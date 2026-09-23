use super::Engine;
use super::*;
use std::path::Path;
use std::sync::Arc;

const TILE: u32 = 1024;

/// Output dims for an export. `crop_preview` matches the viewport crop tool:
/// a rect-only crop while the tool is open is overlay-only (full frame).
fn export_dims(doc: &EditDoc, src_w: u32, src_h: u32, crop_preview: bool) -> (u32, u32) {
    let crop = crate::crop::CropParams::from_doc(doc);
    let mode = crop.mode(crop_preview);
    match mode {
        1 | 2 => {
            let (cw, ch) = crop.content_dims(src_w, src_h, mode);
            ((cw.round() as u32).max(1), (ch.round() as u32).max(1))
        }
        _ => (src_w, src_h),
    }
}

pub(super) struct ExportJob {
    settings: crate::export::ExportSettings,
    reply: Option<oneshot::Sender<Result<String, CoreError>>>,
    full: Vec<f32>,
    /// content-space output dims (see `export_dims`)
    w: u32,
    h: u32,
    tile_tx: u32,
    tile_ty: u32,
    tiles_done: u32,
    tiles_total: u32,
    source_path: String,
    look: u32,
    meta: ImageMeta,
    dcp: Option<Arc<DcpProfile>>,
    lut: Option<Arc<crate::lut::CubeLut>>,
    cct: f32,
    /// User picked Original (look 4) — empty doc, even on JPEG/PNG.
    skip_edits: bool,
    /// Crop tool is open: do not bake an uncommitted rect.
    crop_preview: bool,
    /// Waiting for AI masks started by `ensure_segmentations`.
    waiting_masks: bool,
}

impl Engine {
    pub(super) fn export_image(
        &mut self,
        settings: crate::export::ExportSettings,
        reply: oneshot::Sender<Result<String, CoreError>>,
    ) {
        if self.export_job.is_some() || self.export_batch.is_some() {
            let _ = reply.send(Err(CoreError::Engine("export already in progress".into())));
            return;
        }
        self.flush_sidecar_now();

        if settings.video_clip
            && self
                .current
                .as_ref()
                .is_some_and(|c| c.meta.kind == crate::raw::ImageKind::Video)
        {
            self.export_video_clip(settings, reply);
            return;
        }

        let setup: Result<ExportJob, CoreError> = (|| {
            self.ensure_segmentations();
            let cur = self.current.as_ref().ok_or(CoreError::NoImage)?;
            let source_path = cur.path.to_string_lossy().into_owned();
            let (_, _, w, h) = *cur
                .working
                .as_ref()
                .ok_or(CoreError::Engine("decode not finished".into()))?;
            let gpu = self.gpu.as_ref().ok_or(CoreError::Gpu("no gpu".into()))?;
            if self.export_graph.is_none() {
                self.export_graph = Some(RenderGraph::new(gpu));
            }
            // Viewport keys Original off the raw UI look. present_look_for
            // remaps JPEG/PNG look 4 → 3 (passthrough encode) but must still
            // skip edits so export matches the unedited preview.
            // Linear (8) runs module chain but skips display transform.
            let is_original = self.display_look == 4;
            let is_linear = self.display_look == 8;
            let skip_edits = is_original;
            let crop_preview = !skip_edits
                && self.last_view.is_some_and(|v| v.crop_preview);
            let display_look =
                crate::lut::present_look_for(cur.meta.kind, self.display_look, cur.doc());
            let (out_w, out_h) = if skip_edits {
                (w, h)
            } else {
                export_dims(cur.doc(), w, h, crop_preview)
            };
            let tiles_x = out_w.div_ceil(TILE);
            let tiles_y = out_h.div_ceil(TILE);
            let dcp = if skip_edits || !DcpProfile::applies_to_display_look(display_look)
            {
                None
            } else {
                cur.dcp_profile.clone()
            };
            let lut = if skip_edits {
                None
            } else {
                cur.lut_cube.clone()
            };
            if let Some(g) = self.export_graph.as_mut() {
                g.set_look(display_look);
            }
            let waiting_masks = !skip_edits && !cur.pending_segments.is_empty();
            Ok(ExportJob {
                settings,
                reply: None,
                full: vec![0.0f32; (out_w as usize) * (out_h as usize) * 3],
                w: out_w,
                h: out_h,
                tile_tx: 0,
                tile_ty: 0,
                tiles_done: 0,
                tiles_total: tiles_x * tiles_y,
                source_path,
                look: display_look,
                meta: cur.meta.clone(),
                dcp,
                lut,
                cct: cur.as_shot_cct(),
                skip_edits,
                crop_preview,
                waiting_masks,
            })
        })();

        match setup {
            Err(e) => {
                let _ = reply.send(Err(e));
            }
            Ok(mut job) => {
                job.reply = Some(reply);
                let total = job.tiles_total;
                let waiting = job.waiting_masks;
                self.export_job = Some(job);
                if waiting {
                    self.emit(EngineEvent::ExportProgress {
                        phase: "segment".into(),
                        done: 0,
                        total: 1,
                    });
                } else {
                    self.emit(EngineEvent::ExportProgress {
                        phase: "render".into(),
                        done: 0,
                        total,
                    });
                    let _ = self.self_tx.try_send(EngineMsg::ExportStep);
                }
            }
        }
    }

    fn render_working_linear(&mut self) -> Result<(Vec<f32>, u32, u32), CoreError> {
        let gpu = self.gpu.as_ref().ok_or(CoreError::Gpu("no gpu".into()))?;
        let (display_look, skip_edits, crop_preview, job_w, job_h) = {
            let cur = self.current.as_ref().ok_or(CoreError::NoImage)?;
            let (_, _, src_w, src_h) = cur
                .working
                .as_ref()
                .ok_or(CoreError::Engine("decode not finished".into()))?;
            let is_original = self.display_look == 4;
            let is_linear = self.display_look == 8;
            let skip_edits = is_original;
            let crop_preview = !skip_edits && self.last_view.is_some_and(|v| v.crop_preview);
            let display_look =
                crate::lut::present_look_for(cur.meta.kind, self.display_look, cur.doc());
            let (job_w, job_h) = if skip_edits {
                (*src_w, *src_h)
            } else {
                export_dims(cur.doc(), *src_w, *src_h, crop_preview)
            };
            (display_look, skip_edits, crop_preview, job_w, job_h)
        };
        if self.export_graph.is_none() {
            self.export_graph = Some(RenderGraph::new(gpu));
        }
        let gpu = self.gpu.as_ref().ok_or(CoreError::Gpu("no gpu".into()))?;
        let cur = self.current.as_ref().ok_or(CoreError::NoImage)?;
        let (_, tex_view, src_w, src_h) = cur
            .working
            .as_ref()
            .ok_or(CoreError::Engine("decode not finished".into()))?;
        let lut = if skip_edits {
            None
        } else {
            cur.lut_cube.clone()
        };
        let dcp = if skip_edits || !DcpProfile::applies_to_display_look(display_look) {
            None
        } else {
            cur.dcp_profile.clone()
        };
        let cct = cur.as_shot_cct();
        let original = skip_edits;
        let seg_views: std::collections::HashMap<String, wgpu::TextureView> = cur
            .masks_gpu
            .iter()
            .map(|(id, (_, tex))| (id.clone(), tex.create_view(&Default::default())))
            .collect();
        let graph = self
            .export_graph
            .as_mut()
            .ok_or(CoreError::Engine("export graph".into()))?;
        graph.set_look(display_look);
        graph.invalidate_all();

        let mut full = vec![0.0f32; (job_w as usize) * (job_h as usize) * 3];
        let mut ty = 0u32;
        while ty < job_h {
            let mut tx = 0u32;
            while tx < job_w {
                let tw = TILE.min(job_w - tx);
                let th = TILE.min(job_h - ty);
                let view = ViewParams {
                    out_w: tw,
                    out_h: th,
                    scale: Some(1.0),
                    center_x: (tx as f32 + tw as f32 / 2.0) / job_w as f32,
                    center_y: (ty as f32 + th as f32 / 2.0) / job_h as f32,
                    crop_preview,
                };
                let base_doc;
                let render_doc = if original {
                    base_doc = EditDoc::new(&cur.path.to_string_lossy());
                    &base_doc
                } else {
                    cur.doc()
                };
                let tile = graph.render_linear_tile(
                    gpu,
                    tex_view,
                    *src_w,
                    *src_h,
                    &view,
                    render_doc,
                    cct,
                    &seg_views,
                    lut.as_deref(),
                    dcp.as_deref(),
                )?;
                for row in 0..th as usize {
                    let src = row * tw as usize * 3;
                    let dst = ((ty as usize + row) * job_w as usize + tx as usize) * 3;
                    full[dst..dst + tw as usize * 3]
                        .copy_from_slice(&tile[src..src + tw as usize * 3]);
                }
                tx += TILE;
            }
            ty += TILE;
        }
        Ok((full, job_w, job_h))
    }

    fn export_video_clip(
        &mut self,
        settings: crate::export::ExportSettings,
        reply: oneshot::Sender<Result<String, CoreError>>,
    ) {
        let result = (|| -> Result<String, CoreError> {
            let cur = self.current.as_ref().ok_or(CoreError::NoImage)?;
            let path = cur.path.clone();
            let meta = cur.meta.clone();
            let video = meta
                .video
                .clone()
                .ok_or_else(|| CoreError::InvalidOp("not a video clip".into()))?;
            let restore_frame = video.frame;
            let max = video.frame_count.saturating_sub(1);
            let mut in_f = settings.video_in.unwrap_or(video.in_frame).min(max);
            let mut out_f = settings.video_out.unwrap_or(video.out_frame).min(max);
            if in_f > out_f {
                std::mem::swap(&mut in_f, &mut out_f);
            }
            let fps = video.fps.max(1.0);
            let t = crate::registry::effective_f32(cur.doc(), "input", "transfer").round() as u32;
            let p = crate::registry::effective_f32(cur.doc(), "input", "primaries").round() as u32;
            let (override_t, override_p) = crate::video::overrides_from_input_params(t, p);
            let pr = crate::video::probe(&path)?;
            let land = crate::video::land_from_tags(&pr, override_t, override_p);
            let look = crate::lut::present_look_for(cur.meta.kind, self.display_look, cur.doc());
            let gpu = self.gpu.as_ref().ok_or(CoreError::Gpu("no gpu".into()))?;
            if self.export_graph.is_none() {
                self.export_graph = Some(RenderGraph::new(gpu));
            }
            if let Some(g) = self.export_graph.as_mut() {
                g.set_look(look);
            }

            let dest = if settings.dest_dir.is_empty() {
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("exports")
            } else {
                std::path::PathBuf::from(&settings.dest_dir)
            };
            std::fs::create_dir_all(&dest)?;
            let clip_stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "clip".into());
            let total = out_f.saturating_sub(in_f) + 1;
            let mut last_still = dest.join(format!("{clip_stem}_{in_f:06}.jpg"));

            for (i, frame) in (in_f..=out_f).enumerate() {
                self.emit(EngineEvent::ExportProgress {
                    phase: "render".into(),
                    done: i as u32,
                    total,
                });
                let buf = crate::video::decode_frame(&path, frame, land)?;
                if let Some(cur) = self.current.as_mut() {
                    cur.doc_mut()
                        .unknown
                        .insert("video_frame".into(), serde_json::json!(frame));
                    if let Some(v) = cur.meta.video.as_mut() {
                        v.frame = frame;
                    }
                }
                self.upload_working_rgb(buf);
                let (full, w, h) = self.render_working_linear()?;
                let (lin, w, h) = match settings.max_dim {
                    Some(d) => crate::export::resize_linear(full, w, h, d),
                    None => (full, w, h),
                };
                let mut enc =
                    crate::export::output_transform_look(&lin, w, h, settings.target, false, look);
                crate::export::output_sharpen8(&mut enc.rgb8, w, h, settings.sharpen);
                let mut frame_settings = settings.clone();
                frame_settings.format = crate::export::ExportFormat::Jpeg;
                frame_settings.output_stem = Some(format!("{clip_stem}_{frame:06}"));
                last_still = crate::export::encode_and_write(
                    &enc,
                    &frame_settings,
                    &path.to_string_lossy(),
                    &meta,
                )?;
            }

            self.emit(EngineEvent::ExportProgress {
                phase: "encode".into(),
                done: 0,
                total: 1,
            });
            let mp4 = dest.join(format!("{clip_stem}.mp4"));
            let pattern = dest.join(format!("{clip_stem}_%06d.jpg"));
            let audio = if settings.video_audio && crate::video::has_audio_stream(&path) {
                Some(crate::video::AudioPass {
                    source: path.clone(),
                    start_s: in_f as f32 / fps,
                    duration_s: total as f32 / fps,
                })
            } else {
                None
            };
            let out_path =
                match crate::video::mux_jpeg_sequence(&pattern, in_f, fps, &mp4, audio.as_ref()) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(error = %e, "clip mux failed; jpeg sequence kept");
                        last_still
                    }
                };

            if restore_frame != out_f {
                if let Ok(buf) = crate::video::decode_frame(&path, restore_frame, land) {
                    if let Some(cur) = self.current.as_mut() {
                        cur.doc_mut()
                            .unknown
                            .insert("video_frame".into(), serde_json::json!(restore_frame));
                        if let Some(v) = cur.meta.video.as_mut() {
                            v.frame = restore_frame;
                        }
                    }
                    self.upload_working_rgb(buf);
                    self.schedule_render();
                }
            } else {
                self.schedule_render();
            }
            self.emit(EngineEvent::ExportProgress {
                phase: "encode".into(),
                done: 1,
                total: 1,
            });
            Ok(out_path.to_string_lossy().into_owned())
        })();
        let _ = reply.send(result);
    }

    /// Single export waits here until `ensure_segmentations` workers finish.
    /// Failed / stale segments are already dropped from `pending_segments`.
    pub(super) fn maybe_start_export_after_masks(&mut self) {
        let start = match (self.export_job.as_ref(), self.current.as_ref()) {
            (Some(job), Some(cur)) => job.waiting_masks && cur.pending_segments.is_empty(),
            _ => false,
        };
        if !start {
            return;
        }
        let total = self
            .export_job
            .as_ref()
            .map(|j| j.tiles_total)
            .unwrap_or(0);
        if let Some(job) = self.export_job.as_mut() {
            job.waiting_masks = false;
        }
        self.emit(EngineEvent::ExportProgress {
            phase: "render".into(),
            done: 0,
            total,
        });
        let _ = self.self_tx.try_send(EngineMsg::ExportStep);
    }

    pub(super) fn export_step(&mut self) {
        let Some(job) = self.export_job.as_ref() else {
            return;
        };
        let tx = job.tile_tx;
        let ty = job.tile_ty;
        let job_w = job.w;
        let job_h = job.h;
        let cct = job.cct;
        let dcp = job.dcp.clone();
        let lut = job.lut.clone();
        let original = job.skip_edits;
        let crop_preview = job.crop_preview;

        let tile_result: Result<Vec<f32>, CoreError> = (|| {
            let gpu = self.gpu.as_ref().ok_or(CoreError::Gpu("no gpu".into()))?;
            let cur = self.current.as_ref().ok_or(CoreError::NoImage)?;
            let (_, tex_view, w, h) = cur
                .working
                .as_ref()
                .ok_or(CoreError::Engine("decode not finished".into()))?;
            let seg_views: std::collections::HashMap<String, wgpu::TextureView> = cur
                .masks_gpu
                .iter()
                .map(|(id, (_, tex))| (id.clone(), tex.create_view(&Default::default())))
                .collect();
            let graph = self
                .export_graph
                .as_mut()
                .ok_or(CoreError::Engine("export graph".into()))?;

            // tile grid + centers live in content space (job dims); the
            // working-master dims only parameterize the source texture
            let tw = TILE.min(job_w - tx);
            let th = TILE.min(job_h - ty);
            let view = ViewParams {
                out_w: tw,
                out_h: th,
                scale: Some(1.0),
                center_x: (tx as f32 + tw as f32 / 2.0) / job_w as f32,
                center_y: (ty as f32 + th as f32 / 2.0) / job_h as f32,
                crop_preview,
            };
            let base_doc;
            let render_doc = if original {
                base_doc = EditDoc::new(&cur.path.to_string_lossy());
                &base_doc
            } else {
                cur.doc()
            };
            graph.render_linear_tile(
                gpu,
                tex_view,
                *w,
                *h,
                &view,
                render_doc,
                cct,
                &seg_views,
                lut.as_deref(),
                dcp.as_deref(),
            )
        })();

        let tile = match tile_result {
            Ok(t) => t,
            Err(e) => {
                if let Some(job) = self.export_job.take() {
                    if let Some(reply) = job.reply {
                        let _ = reply.send(Err(e));
                    }
                }
                return;
            }
        };

        {
            let job = self.export_job.as_mut().unwrap();
            let tw = TILE.min(job_w - tx);
            let th = TILE.min(job_h - ty);
            for row in 0..th as usize {
                let src = row * tw as usize * 3;
                let dst = ((ty as usize + row) * job.w as usize + tx as usize) * 3;
                job.full[dst..dst + tw as usize * 3]
                    .copy_from_slice(&tile[src..src + tw as usize * 3]);
            }
            drop(tile);
            job.tiles_done += 1;
            job.tile_tx += TILE;
            if job.tile_tx >= job.w {
                job.tile_tx = 0;
                job.tile_ty += TILE;
            }
        }

        let (done, total, more_tiles) = {
            let j = self.export_job.as_ref().unwrap();
            (j.tiles_done, j.tiles_total, j.tile_ty < j.h)
        };

        self.emit(EngineEvent::ExportProgress {
            phase: "render".into(),
            done,
            total,
        });

        if more_tiles {
            let _ = self.self_tx.try_send(EngineMsg::ExportStep);
            return;
        }

        let mut job = self.export_job.take().unwrap();
        let look = DcpProfile::present_look(job.look, job.dcp.as_deref());
        tracing::info!(
            w = job.w,
            h = job.h,
            tiles = job.tiles_total,
            profile = job
                .dcp
                .as_ref()
                .map(|d| d.profile_name.as_str())
                .unwrap_or("none"),
            "export render (tiled) done"
        );

        self.emit(EngineEvent::ExportProgress {
            phase: "encode".into(),
            done: 0,
            total: 1,
        });

        let settings = job.settings;
        let full = job.full;
        let w = job.w;
        let h = job.h;
        let source_path = job.source_path;
        let meta = job.meta;
        let reply = job.reply.take().expect("export reply");
        let events = self.events.clone();

        std::thread::Builder::new()
            .name("export-worker".into())
            .spawn(move || {
                let t0 = Instant::now();
                let result = (|| {
                    let (lin, w, h) = match settings.max_dim {
                        Some(d) => crate::export::resize_linear(full, w, h, d),
                        None => (full, w, h),
                    };
                    let want16 = settings.format == crate::export::ExportFormat::Tiff16;
                    let mut enc = crate::export::output_transform_look(
                        &lin,
                        w,
                        h,
                        settings.target,
                        want16,
                        look,
                    );
                    if !want16 {
                        crate::export::output_sharpen8(&mut enc.rgb8, w, h, settings.sharpen);
                    }
                    crate::export::encode_and_write(&enc, &settings, &source_path, &meta)
                        .map(|p| p.to_string_lossy().into_owned())
                })();
                tracing::info!(
                    ms = t0.elapsed().as_millis() as u64,
                    ok = result.is_ok(),
                    "export encode done"
                );
                if let Some(tx) = &events {
                    let _ = tx.send(EngineEvent::ExportProgress {
                        phase: "encode".into(),
                        done: 1,
                        total: 1,
                    });
                }
                let _ = reply.send(result);
            })
            .expect("spawn export worker");
    }
}

// ---- Batch export (spec 7.3) ----
//
// The open image is untouched: each queued image gets its own decode +
// segmentation on a worker thread, its own working texture, and a tiled
// render through the shared export_graph. One decode stays in flight ahead
// of the image that owns the GPU (pipelining); encode runs on its own
// worker like single export. Stale worker messages after cancel are dropped
// by batch id.

pub(super) struct BatchState {
    id: u64,
    items: Vec<crate::export::BatchExportItem>,
    settings: crate::export::ExportSettings,
    look: u32,
    ok: Vec<String>,
    failed: Vec<(String, String)>,
    /// queue slot currently decoding on the prepare worker (≤ 1 in flight)
    preparing: Option<usize>,
    /// decode that finished while another image still owned the GPU
    prepared: Option<(usize, Box<BatchPrepared>)>,
    /// image currently tile-rendering / encoding
    cur: Option<BatchImage>,
    /// next queue slot to hand to the prepare worker
    next_prepare: usize,
    /// images fully accounted for (ok or failed)
    finished: usize,
}

struct BatchImage {
    index: usize,
    path: String,
    /// keeps the working texture alive for `view`
    _tex: wgpu::Texture,
    view: wgpu::TextureView,
    /// content-space output dims (see `export_dims`)
    w: u32,
    h: u32,
    /// working-master texture dims
    src_w: u32,
    src_h: u32,
    /// per-image present look (`present_look_for`), not the batch-wide UI look
    look: u32,
    doc: EditDoc,
    dcp: Option<Arc<DcpProfile>>,
    lut: Option<Arc<crate::lut::CubeLut>>,
    cct: f32,
    meta: ImageMeta,
    /// segmentation textures (keep-alive; views rebuilt per tile)
    masks_gpu: Vec<(String, wgpu::Texture)>,
    full: Vec<f32>,
    tile_tx: u32,
    tile_ty: u32,
    tiles_done: u32,
    tiles_total: u32,
    encoding: bool,
    output_stem: Option<String>,
}

impl Engine {
    pub(super) fn export_batch_start(
        &mut self,
        items: Vec<crate::export::BatchExportItem>,
        settings: crate::export::ExportSettings,
        reply: oneshot::Sender<Result<u32, CoreError>>,
    ) {
        if items.is_empty() {
            let _ = reply.send(Err(CoreError::InvalidOp("empty export batch".into())));
            return;
        }
        if self.export_job.is_some() || self.export_batch.is_some() {
            let _ = reply.send(Err(CoreError::Engine("export already in progress".into())));
            return;
        }
        if self.gpu.is_none() {
            let _ = reply.send(Err(CoreError::Gpu("no gpu".into())));
            return;
        }
        // The open image's unsaved edits must land in its sidecar first —
        // the batch reads sidecars as the canonical docs.
        self.flush_sidecar_now();
        self.batch_seq += 1;
        let count = items.len() as u32;
        self.export_batch = Some(BatchState {
            id: self.batch_seq,
            items,
            settings,
            look: self.display_look,
            ok: Vec::new(),
            failed: Vec::new(),
            preparing: None,
            prepared: None,
            cur: None,
            next_prepare: 0,
            finished: 0,
        });
        self.spawn_batch_prepare();
        let _ = reply.send(Ok(count));
    }

    pub(super) fn export_batch_cancel(&mut self, reply: oneshot::Sender<()>) {
        if self.export_batch.is_some() {
            self.batch_finish(true);
        }
        let _ = reply.send(());
    }

    fn emit_batch_progress(&self, index: usize, path: &str, phase: &str, done: u32, total: u32) {
        let count = self
            .export_batch
            .as_ref()
            .map(|b| b.items.len() as u32)
            .unwrap_or(0);
        self.emit(EngineEvent::ExportBatchProgress {
            index: index as u32,
            count,
            path: path.into(),
            phase: phase.into(),
            done,
            total,
        });
    }

    /// Hand the next queue slot to a prepare worker, keeping at most one
    /// decode in flight and one prepared image stashed.
    fn spawn_batch_prepare(&mut self) {
        let job = {
            let Some(batch) = self.export_batch.as_mut() else {
                return;
            };
            if batch.preparing.is_some()
                || batch.prepared.is_some()
                || batch.next_prepare >= batch.items.len()
            {
                return;
            }
            let index = batch.next_prepare;
            batch.next_prepare += 1;
            batch.preparing = Some(index);
            (
                index,
                batch.items[index].clone(),
                batch.id,
                batch.look == 4,
            )
        };
        let (index, item, batch_id, skip_edits) = job;
        let path = item.path.clone();
        let doc_id = item.doc_id.clone();
        self.emit_batch_progress(index, &path.to_string_lossy(), "decode", 0, 1);
        let pre = self.snapshot_current_decode(&path);
        let tx = self.self_tx.clone();
        std::thread::Builder::new()
            .name("batch-prepare".into())
            .spawn(move || {
                let result =
                    prepare_batch_image(&path, skip_edits, doc_id.as_deref(), pre).map(Box::new);
                let _ = tx.blocking_send(EngineMsg::BatchImagePrepared {
                    batch_id,
                    index,
                    result,
                });
            })
            .expect("spawn batch prepare worker");
    }

    pub(super) fn batch_image_prepared(
        &mut self,
        batch_id: u64,
        index: usize,
        result: Result<Box<BatchPrepared>, CoreError>,
    ) {
        {
            let Some(batch) = self.export_batch.as_mut() else {
                return;
            };
            if batch.id != batch_id {
                return;
            }
            batch.preparing = None;
            match result {
                Err(e) => {
                    let path = batch.items[index].path.to_string_lossy().into_owned();
                    tracing::warn!(error = %e, path = %path, "batch image failed at decode");
                    batch.failed.push((path, e.to_string()));
                    batch.finished += 1;
                }
                Ok(p) => {
                    batch.prepared = Some((index, p));
                }
            }
        }
        self.batch_advance();
    }

    /// Drive the state machine: start the next render when the GPU is free,
    /// keep the decode pipeline full, finish when all slots are accounted for.
    fn batch_advance(&mut self) {
        let start = {
            let Some(batch) = self.export_batch.as_mut() else {
                return;
            };
            if batch.cur.is_none() {
                batch.prepared.take()
            } else {
                None
            }
        };
        if let Some((index, p)) = start {
            self.batch_begin_render(index, p);
        }
        self.spawn_batch_prepare();
        let all_done = {
            let Some(batch) = self.export_batch.as_ref() else {
                return;
            };
            batch.finished == batch.items.len()
        };
        if all_done {
            self.batch_finish(false);
        }
    }

    fn batch_begin_render(&mut self, index: usize, p: Box<BatchPrepared>) {
        let (user_look, path) = {
            let batch = self.export_batch.as_ref().expect("batch state");
            (
                batch.look,
                batch.items[index].path.to_string_lossy().into_owned(),
            )
        };
        let Some(gpu) = &self.gpu else {
            if let Some(batch) = self.export_batch.as_mut() {
                batch.failed.push((path, "no gpu".into()));
                batch.finished += 1;
            }
            return;
        };
        let BatchPrepared {
            payload,
            doc,
            dcp,
            lut,
            masks,
            output_stem,
        } = *p;
        let tex = upload_working_texture(gpu, &payload.rgba_f16, payload.width, payload.height);
        let view = tex.create_view(&Default::default());
        let masks_gpu: Vec<(String, wgpu::Texture)> = masks
            .into_iter()
            .map(|(id, _hash, m)| {
                let t =
                    crate::graph::upload_small_mask(gpu, &m.data, m.width as u32, m.height as u32);
                (id, t)
            })
            .collect();
        let look = crate::lut::present_look_for(payload.meta.kind, user_look, &doc);
        if self.export_graph.is_none() {
            self.export_graph = Some(RenderGraph::new(gpu));
        }
        if let Some(g) = self.export_graph.as_mut() {
            // fresh image — never reuse a previous image's cached tiles
            g.invalidate_all();
            g.set_look(look);
        }
        let (w, h) = (payload.width, payload.height);
        let (out_w, out_h) = export_dims(&doc, w, h, false);
        let tiles_x = out_w.div_ceil(TILE);
        let tiles_y = out_h.div_ceil(TILE);
        let cct = payload.meta.estimated_cct.unwrap_or(5200.0);
        let img = BatchImage {
            index,
            path: path.clone(),
            _tex: tex,
            view,
            w: out_w,
            h: out_h,
            src_w: w,
            src_h: h,
            look,
            doc,
            dcp,
            lut,
            cct,
            meta: payload.meta,
            masks_gpu,
            full: vec![0.0f32; (out_w as usize) * (out_h as usize) * 3],
            tile_tx: 0,
            tile_ty: 0,
            tiles_done: 0,
            tiles_total: tiles_x * tiles_y,
            encoding: false,
            output_stem,
        };
        let total = img.tiles_total;
        if let Some(batch) = self.export_batch.as_mut() {
            batch.cur = Some(img);
        }
        self.emit_batch_progress(index, &path, "render", 0, total);
        let _ = self.self_tx.try_send(EngineMsg::ExportBatchStep);
    }

    pub(super) fn export_batch_step(&mut self) {
        let Some(batch) = self.export_batch.as_ref() else {
            return;
        };
        let Some(img) = batch.cur.as_ref() else {
            return;
        };
        if img.encoding {
            return;
        }
        let (index, tx, ty, img_w, img_h, src_w, src_h, cct, look) = (
            img.index,
            img.tile_tx,
            img.tile_ty,
            img.w,
            img.h,
            img.src_w,
            img.src_h,
            img.cct,
            img.look,
        );
        let dcp = img.dcp.clone();
        let lut = img.lut.clone();

        let tile_result: Result<Vec<f32>, CoreError> = (|| {
            let gpu = self.gpu.as_ref().ok_or(CoreError::Gpu("no gpu".into()))?;
            let batch = self.export_batch.as_ref().expect("batch state");
            let img = batch.cur.as_ref().expect("batch image");
            let seg_views: std::collections::HashMap<String, wgpu::TextureView> = img
                .masks_gpu
                .iter()
                .map(|(id, tex)| (id.clone(), tex.create_view(&Default::default())))
                .collect();
            let graph = self
                .export_graph
                .as_mut()
                .ok_or(CoreError::Engine("export graph".into()))?;

            let tw = TILE.min(img_w - tx);
            let th = TILE.min(img_h - ty);
            let view = ViewParams {
                out_w: tw,
                out_h: th,
                scale: Some(1.0),
                center_x: (tx as f32 + tw as f32 / 2.0) / img_w as f32,
                center_y: (ty as f32 + th as f32 / 2.0) / img_h as f32,
                crop_preview: false,
            };
            graph.render_linear_tile(
                gpu,
                &img.view,
                src_w,
                src_h,
                &view,
                &img.doc,
                cct,
                &seg_views,
                lut.as_deref(),
                dcp.as_deref(),
            )
        })();

        let tile = match tile_result {
            Ok(t) => t,
            Err(e) => {
                // fail this image, keep the batch going
                let path = {
                    let batch = self.export_batch.as_mut().expect("batch state");
                    batch.cur = None;
                    let path = batch.items[index].path.to_string_lossy().into_owned();
                    batch.failed.push((path.clone(), e.to_string()));
                    batch.finished += 1;
                    path
                };
                tracing::warn!(path = %path, "batch image failed at render");
                self.batch_advance();
                return;
            }
        };

        let (done, total, more_tiles) = {
            let batch = self.export_batch.as_mut().expect("batch state");
            let img = batch.cur.as_mut().expect("batch image");
            let tw = TILE.min(img_w - tx);
            let th = TILE.min(img_h - ty);
            for row in 0..th as usize {
                let src = row * tw as usize * 3;
                let dst = ((ty as usize + row) * img_w as usize + tx as usize) * 3;
                img.full[dst..dst + tw as usize * 3]
                    .copy_from_slice(&tile[src..src + tw as usize * 3]);
            }
            img.tiles_done += 1;
            img.tile_tx += TILE;
            if img.tile_tx >= img.w {
                img.tile_tx = 0;
                img.tile_ty += TILE;
            }
            (img.tiles_done, img.tiles_total, img.tile_ty < img.h)
        };

        let path = {
            let batch = self.export_batch.as_ref().expect("batch state");
            batch.cur.as_ref().expect("batch image").path.clone()
        };
        self.emit_batch_progress(index, &path, "render", done, total);

        if more_tiles {
            let _ = self.self_tx.try_send(EngineMsg::ExportBatchStep);
            return;
        }

        // render done → encode off-thread; GPU stays busy with the next
        // prepared image as soon as this one's encode is in flight.
        let (batch_id, settings, full, meta) = {
            let batch = self.export_batch.as_mut().expect("batch state");
            let img = batch.cur.as_mut().expect("batch image");
            img.encoding = true;
            let mut settings = batch.settings.clone();
            if img.output_stem.is_some() {
                settings.output_stem = img.output_stem.clone();
            }
            (
                batch.id,
                settings,
                std::mem::take(&mut img.full),
                img.meta.clone(),
            )
        };
        self.emit_batch_progress(index, &path, "encode", 0, 1);
        let enc_look = DcpProfile::present_look(look, dcp.as_deref());
        let tx_chan = self.self_tx.clone();
        std::thread::Builder::new()
            .name("batch-encode".into())
            .spawn(move || {
                let t0 = Instant::now();
                let result = (|| {
                    let (lin, w, h) = match settings.max_dim {
                        Some(d) => crate::export::resize_linear(full, img_w, img_h, d),
                        None => (full, img_w, img_h),
                    };
                    let want16 = settings.format == crate::export::ExportFormat::Tiff16;
                    let mut enc = crate::export::output_transform_look(
                        &lin,
                        w,
                        h,
                        settings.target,
                        want16,
                        enc_look,
                    );
                    if !want16 {
                        crate::export::output_sharpen8(&mut enc.rgb8, w, h, settings.sharpen);
                    }
                    crate::export::encode_and_write(&enc, &settings, &path, &meta)
                        .map(|p| p.to_string_lossy().into_owned())
                })();
                tracing::info!(
                    ms = t0.elapsed().as_millis() as u64,
                    ok = result.is_ok(),
                    "batch encode done"
                );
                let _ = tx_chan.blocking_send(EngineMsg::BatchEncodeDone {
                    batch_id,
                    index,
                    result,
                });
            })
            .expect("spawn batch encode worker");
    }

    pub(super) fn batch_encode_done(
        &mut self,
        batch_id: u64,
        index: usize,
        result: Result<String, CoreError>,
    ) {
        let path = {
            let Some(batch) = self.export_batch.as_mut() else {
                return;
            };
            if batch.id != batch_id {
                return;
            }
            batch.cur = None; // frees this image's GPU resources
            let path = batch.items[index].path.to_string_lossy().into_owned();
            match result {
                Ok(out) => batch.ok.push(out),
                Err(e) => batch.failed.push((path.clone(), e.to_string())),
            }
            batch.finished += 1;
            path
        };
        self.emit_batch_progress(index, &path, "encode", 1, 1);
        self.batch_advance();
    }

    /// Reuse the open image's working master (including in-memory AI denoise)
    /// when the batch path is the file on screen.
    fn snapshot_current_decode(&self, path: &Path) -> Option<DecodedPayload> {
        let cur = self.current.as_ref()?;
        if cur.path != path {
            return None;
        }
        let clean = cur.clean_rgb.clone()?;
        let small = cur
            .small_cpu
            .clone()
            .unwrap_or_else(|| Arc::new(clean.downscale_to(2048)));
        Some(DecodedPayload {
            rgba_f16: clean.to_rgba_f16_bytes(),
            width: clean.width as u32,
            height: clean.height as u32,
            small_cpu: small,
            clean_rgb: clean,
            meta: cur.meta.clone(),
        })
    }

    fn batch_finish(&mut self, cancelled: bool) {
        let Some(batch) = self.export_batch.take() else {
            return;
        };
        tracing::info!(
            ok = batch.ok.len(),
            failed = batch.failed.len(),
            cancelled,
            "batch export finished"
        );
        self.emit(EngineEvent::ExportBatchDone {
            ok: batch.ok,
            failed: batch.failed,
            cancelled,
        });
    }
}

/// Worker-side mirror of `open_image`'s setup for one queued image: sidecar
/// doc (including a virtual copy), camera profile, LUT, demosaic choice,
/// full decode or a snapshot of the open working master, optional AI-denoise
/// cache, and segmentation. Runs entirely off the actor thread.
fn prepare_batch_image(
    path: &Path,
    skip_edits: bool,
    doc_id: Option<&str>,
    pre_payload: Option<DecodedPayload>,
) -> Result<BatchPrepared, CoreError> {
    let decoder = crate::raw::decoder_for(path);
    if !decoder.probe(path) {
        return Err(CoreError::Decode(format!(
            "unsupported file: {}",
            path.display()
        )));
    }
    let meta = match &pre_payload {
        Some(p) => p.meta.clone(),
        None => decoder.metadata(path)?,
    };
    let want_copy = doc_id.map(str::trim).filter(|s| !s.is_empty());
    let primary_id = sidecar::load_edits(path)
        .ok()
        .flatten()
        .map(sidecar::split_copies)
        .and_then(|mut docs| docs.first_mut().map(|d| d.doc_id.clone()));
    let output_stem = sidecar::virtual_copy_stem_suffix(want_copy, primary_id.as_deref()).map(
        |suffix| {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "export".into());
            format!("{stem}_{suffix}")
        },
    );
    // Original look (4) exports the unedited base — skip sidecar + DCP + LUT.
    let doc = if skip_edits {
        EditDoc::new(&path.to_string_lossy())
    } else {
        match sidecar::load_edits_doc(path, want_copy) {
            Ok(Some(d)) => d,
            Ok(None) => EditDoc::new(&path.to_string_lossy()),
            Err(e) => {
                if want_copy.is_some() {
                    return Err(e);
                }
                tracing::warn!(error = %e, path = %path.display(), "batch: sidecar unreadable; exporting unedited");
                EditDoc::new(&path.to_string_lossy())
            }
        }
    };
    let index = crate::profile::ProfileIndex::embedded();
    let chosen = crate::profile::choose_profile(&meta, &index, doc.meta.profile_file.as_deref());
    let profile_path = crate::profile::decode_profile_path(chosen.as_ref());
    let dcp = if skip_edits {
        None
    } else {
        crate::profile::load_dcp_profile(&meta, chosen.as_ref()).map(Arc::new)
    };
    let lut = if skip_edits {
        None
    } else {
        crate::path_safety::sanitize_lut_path(doc.meta.lut_file.clone()).and_then(|p| {
            match crate::lut::CubeLut::load_cube(Path::new(&p)) {
                Ok(c) => Some(Arc::new(c)),
                Err(e) => {
                    tracing::warn!(error = %e, path = %p, "batch: LUT load failed; exporting without it");
                    None
                }
            }
        })
    };
    let mut payload = if let Some(pre) = pre_payload {
        pre
    } else {
        let mut demosaic = crate::raw::Demosaic::parse_or_default(doc.meta.demosaic.as_deref());
        let available = crate::raw::Demosaic::available();
        if !available.iter().any(|n| n == demosaic.name()) {
            demosaic = crate::raw::Demosaic::Rcd;
        }
        let img = decoder.decode_with_options(path, profile_path.as_deref(), demosaic)?;
        let mut decoded = DecodedPayload::from_decoded(img);
        if !skip_edits {
            apply_cached_ai_denoise(&mut decoded, &doc);
        }
        decoded
    };
    // Heal spots rewrite the working master before export (same as viewport).
    if !skip_edits && doc.retouch.iter().any(|s| s.enabled) {
        match crate::retouch::apply_all(&payload.clean_rgb, &doc.retouch) {
            Ok(healed) => {
                payload.rgba_f16 = healed.to_rgba_f16_bytes();
                payload.width = healed.width as u32;
                payload.height = healed.height as u32;
                payload.small_cpu = Arc::new(healed.downscale_to(2048));
            }
            Err(e) => {
                tracing::warn!(error = %e, "batch: retouch apply failed; exporting without heal");
            }
        }
    }

    // Segmented masks are inferred here, synchronously — the render can't
    // start without them and this thread is already off the actor.
    let mut masks = Vec::new();
    let seg_jobs: Vec<(String, String, u64, serde_json::Value)> = if skip_edits {
        Vec::new()
    } else {
        let mut jobs = Vec::new();
        for m in &doc.masks {
            // Mirror live path: top-level segmented + composite children.
            use std::hash::{Hash, Hasher};
            let mut push = |job_id: String, kind: &str, source: &serde_json::Value| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                kind.hash(&mut h);
                source.to_string().hash(&mut h);
                jobs.push((job_id, kind.to_string(), h.finish(), source.clone()));
            };
            match m.source.get("type").and_then(|t| t.as_str()) {
                Some("segmented") => {
                    let kind = crate::segment::kind_from_segmented_source(&m.kind, &m.source);
                    push(
                        crate::segment::segment_cache_key(&m.id, None),
                        &kind,
                        &m.source,
                    );
                }
                Some("composite") => {
                    if let Some(comps) = m.source.get("components").and_then(|c| c.as_array()) {
                        for (i, comp) in comps.iter().enumerate() {
                            let src = comp.get("source").cloned().unwrap_or_else(|| comp.clone());
                            if src.get("type").and_then(|t| t.as_str()) == Some("segmented") {
                                let kind =
                                    crate::segment::kind_from_segmented_source(&m.kind, &src);
                                push(
                                    crate::segment::segment_cache_key(&m.id, Some(i)),
                                    &kind,
                                    &src,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        jobs
    };
    if !seg_jobs.is_empty() {
        let input = payload.small_cpu.downscale_to(768);
        for (id, kind, hash, source) in seg_jobs {
            let hint_point = source
                .get("hint")
                .and_then(|h| h.get("point"))
                .and_then(|pt| pt.as_array())
                .and_then(|a| Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)));
            let depth_near = source
                .get("depth_near")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32);
            let depth_far = source
                .get("depth_far")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32);
            let result =
                crate::segment::run_ai_mask(&kind, &input, hint_point, depth_near, depth_far);
            match result {
                Ok(m) => masks.push((id, hash, m)),
                Err(e) => {
                    return Err(CoreError::Engine(format!(
                        "segmentation for mask {id} failed: {e}"
                    )))
                }
            }
        }
    }

    Ok(BatchPrepared {
        payload,
        doc,
        dcp,
        lut,
        masks,
        output_stem,
    })
}

/// Replay a cached AI denoise base so folder export matches the editor
/// when the file is not the open image (the open image uses a snapshot).
fn apply_cached_ai_denoise(payload: &mut DecodedPayload, doc: &EditDoc) {
    let settings = crate::denoise::DenoiseSettings::from_doc(
        doc,
        &crate::denoise::NoiseProfile::from_iso(payload.meta.iso.unwrap_or(800)),
    );
    if !settings.ai_active() {
        return;
    }
    let small = payload.small_cpu.as_ref();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(small.width as u32).to_le_bytes());
    hasher.update(&(small.height as u32).to_le_bytes());
    hasher.update(bytemuck::cast_slice::<f32, u8>(&small.data));
    let hash = hasher.finalize().to_hex().to_string();
    let Ok(key) = crate::denoise::ai::cache_key(&hash, &settings.ai_model, settings.ai_amount)
    else {
        return;
    };
    let store = crate::denoise::ai::CacheStore::default();
    let Ok(Some(cache_path)) = store.lookup(&key) else {
        return;
    };
    let Ok((w, h, rgb)) = crate::denoise::ai::CacheStore::load(&cache_path) else {
        return;
    };
    let clean = crate::image::RgbF32Buf {
        width: w,
        height: h,
        data: rgb,
    };
    payload.rgba_f16 = clean.to_rgba_f16_bytes();
    payload.width = clean.width as u32;
    payload.height = clean.height as u32;
    payload.small_cpu = Arc::new(clean.downscale_to(2048));
    payload.clean_rgb = Arc::new(clean);
}

#[cfg(test)]
mod tests {
    use super::export_dims;
    use crate::doc::{EditDoc, ParamValue};

    #[test]
    fn crop_preview_keeps_full_frame() {
        let mut doc = EditDoc::new("/x.ARW");
        doc.set("crop", "right", ParamValue::F32(0.5));
        assert_eq!(export_dims(&doc, 6000, 4000, true), (6000, 4000));
        assert_eq!(export_dims(&doc, 6000, 4000, false), (3000, 4000));
    }

    #[test]
    fn crop_preview_honors_live_rotate() {
        let mut doc = EditDoc::new("/x.ARW");
        doc.set("crop", "rotate_90", ParamValue::F32(1.0));
        assert_eq!(export_dims(&doc, 6000, 4000, true), (4000, 6000));
        assert_eq!(export_dims(&doc, 6000, 4000, false), (4000, 6000));
    }
}