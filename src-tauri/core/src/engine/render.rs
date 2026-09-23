use super::Engine;
use super::*;

impl Engine {
    pub(super) fn compute_stats(&self) -> Option<crate::message::FrameStats> {
        // 256 bins match 8-bit display encoding 1:1 (phase 12).
        const BINS: usize = 256;
        // Match present.wgsl blinkies: hi ≥ 0.995 (~254/255), lo ≤ 0.004 (~1/255).
        const HI_THR: u8 = 254;
        const LO_THR: u8 = 1;
        let frame = self.latest_frame.as_ref()?;
        let mut r = vec![0u32; BINS];
        let mut g = vec![0u32; BINS];
        let mut b = vec![0u32; BINS];
        let mut luma = vec![0u32; BINS];
        let (mut hi, mut lo, mut n) = (0u64, 0u64, 0u64);
        for px in frame.rgba.chunks_exact(4) {
            if px[0] == 22 && px[1] == 22 && px[2] == 24 {
                continue; // letterbox bg
            }
            n += 1;
            r[px[0] as usize] += 1;
            g[px[1] as usize] += 1;
            b[px[2] as usize] += 1;
            let l =
                (0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32) as usize;
            luma[l.min(BINS - 1)] += 1;
            if px[0] >= HI_THR || px[1] >= HI_THR || px[2] >= HI_THR {
                hi += 1;
            }
            if px[0] <= LO_THR && px[1] <= LO_THR && px[2] <= LO_THR {
                lo += 1;
            }
        }
        let n = n.max(1) as f32;
        const WW: u32 = 128;
        const WH: u32 = 64;
        let mut waveform = vec![0u32; (WW * WH) as usize];
        let mut parade = vec![0u32; (WW * WH * 3) as usize];
        const VS: u32 = 64;
        let mut vectorscope = vec![0u32; (VS * VS) as usize];
        if let Some(frame) = self.latest_frame.as_ref() {
            for y in 0..frame.height {
                for x in 0..frame.width {
                    let i = ((y * frame.width + x) * 4) as usize;
                    let px = &frame.rgba[i..];
                    if px[0] == 22 && px[1] == 22 && px[2] == 24 {
                        continue;
                    }
                    let rf = px[0] as f32;
                    let gf = px[1] as f32;
                    let bf = px[2] as f32;
                    let l = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
                    let col = (x as f32 / frame.width.max(1) as f32 * WW as f32) as u32;
                    let col = col.min(WW - 1);
                    let row_of = |v: f32| {
                        WH.saturating_sub(1)
                            - ((v / 255.0) * (WH - 1) as f32).clamp(0.0, (WH - 1) as f32) as u32
                    };
                    let idx = (row_of(l) * WW + col) as usize;
                    waveform[idx] = waveform[idx].saturating_add(1);
                    for (plane, v) in [rf, gf, bf].into_iter().enumerate() {
                        let pidx = (plane as u32 * WW * WH + row_of(v) * WW + col) as usize;
                        parade[pidx] = parade[pidx].saturating_add(1);
                    }
                    // Rec.709 luma-subtracted chroma, origin at center.
                    let cb = (bf - l) / 255.0; // ~[-1,1]
                    let cr = (rf - l) / 255.0;
                    let vx =
                        ((cr * 0.5 + 0.5) * (VS - 1) as f32).clamp(0.0, (VS - 1) as f32) as u32;
                    let vy = ((1.0 - (cb * 0.5 + 0.5)) * (VS - 1) as f32)
                        .clamp(0.0, (VS - 1) as f32) as u32;
                    let vsi = (vy * VS + vx) as usize;
                    vectorscope[vsi] = vectorscope[vsi].saturating_add(1);
                }
            }
        }
        Some(crate::message::FrameStats {
            bins: BINS,
            r,
            g,
            b,
            luma,
            clip_high_pct: 100.0 * hi as f32 / n,
            clip_low_pct: 100.0 * lo as f32 / n,
            waveform,
            waveform_w: WW,
            waveform_h: WH,
            vectorscope,
            vectorscope_size: VS,
            parade,
        })
    }
    pub(super) fn wb_from_point(&mut self, x: f32, y: f32) -> Result<DocDelta, CoreError> {
        let c = self.current.as_mut().ok_or(CoreError::NoImage)?;
        let small = c.small_cpu.as_ref().ok_or(CoreError::NoImage)?;
        let (w, h) = (small.width as i64, small.height as i64);
        let cx = ((x.clamp(0.0, 1.0) * w as f32) as i64).clamp(0, w - 1);
        let cy = ((y.clamp(0.0, 1.0) * h as f32) as i64).clamp(0, h - 1);
        let mut acc = [0.0f64; 3];
        let mut count = 0.0f64;
        for dy in -2..=2i64 {
            for dx in -2..=2i64 {
                let px = (cx + dx).clamp(0, w - 1);
                let py = (cy + dy).clamp(0, h - 1);
                let i = ((py * w + px) * 3) as usize;
                acc[0] += small.data[i] as f64;
                acc[1] += small.data[i + 1] as f64;
                acc[2] += small.data[i + 2] as f64;
                count += 1.0;
            }
        }
        let sample = [
            (acc[0] / count) as f32,
            (acc[1] / count) as f32,
            (acc[2] / count) as f32,
        ];
        if sample[1] <= 1e-5 {
            return Err(CoreError::InvalidOp("sampled patch too dark".into()));
        }
        let (temp, tint) = crate::color::solve_wb_for_neutral(sample, c.as_shot_cct());
        tracing::info!(temp, tint, ?sample, "wb eyedropper");

        let before = c.doc().clone();
        ops::apply_op(
            c.doc_mut(),
            &Op::SetParam {
                path: "white_balance.temp".into(),
                value: serde_json::json!(temp),
            },
        )?;
        ops::apply_op(
            c.doc_mut(),
            &Op::SetParam {
                path: "white_balance.tint".into(),
                value: serde_json::json!(tint),
            },
        )?;
        c.history.record(before, "wb eyedropper".into());
        c.doc_dirty = true;
        let delta = c.delta("wb eyedropper".into(), None, None);
        if let Some(g) = &mut self.graph {
            g.invalidate_from_module("white_balance");
        }
        self.schedule_render();
        self.schedule_settle();
        Ok(delta)
    }
    pub(super) fn full_redraw(&mut self) {
        if let Some(g) = &mut self.graph {
            g.invalidate_all();
        }
        self.schedule_render();
    }

    pub(super) fn on_settle(&mut self) {
        self.flush_sidecar_now();
    }
    pub(super) fn render_view(&mut self, view: ViewParams) -> Result<FrameInfo, CoreError> {
        self.ensure_segmentations();
        let Some(cur) = &self.current else {
            return Err(CoreError::NoImage);
        };
        let Some((_, tex_view, w, h)) = &cur.working else {
            return self
                .latest_frame
                .as_ref()
                .map(|f| FrameInfo {
                    version: f.version,
                    width: f.width,
                    height: f.height,
                })
                .ok_or(CoreError::NoImage);
        };
        let gpu = self.gpu.as_ref().ok_or(CoreError::Gpu("no gpu".into()))?;
        if self.graph.is_none() {
            self.graph = Some(RenderGraph::new(gpu));
        }
        // Rendered rasters skip Neutral/Camera/Original view-looks (those are
        // RAW rendering). See `effective_display_look`.
        let display_look =
            crate::lut::present_look_for(cur.meta.kind, self.display_look, cur.doc());
        {
            let g = self.graph.as_mut().unwrap();
            g.set_look(display_look);
            g.set_clip_warnings(self.clip_hi, self.clip_lo);
            g.set_mask_overlay_style(self.overlay_strength, self.overlay_mode);
            g.set_proof(self.proof_space, self.proof_gamut);
        }
        let (w, h) = (*w, *h);
        let seg_views: std::collections::HashMap<String, wgpu::TextureView> = cur
            .masks_gpu
            .iter()
            .map(|(id, (_, tex))| (id.clone(), tex.create_view(&Default::default())))
            .collect();
        // before/after + Original look: render without saved edits.
        // Linear (8) runs module chain but skips display transform.
        let base_doc;
        let is_original = self.display_look == 4;
        let is_linear = self.display_look == 8;
        let render_doc = if self.preview_bypass || is_original {
            base_doc = EditDoc::new(&cur.path.to_string_lossy());
            &base_doc
        } else {
            cur.doc()
        };
        let as_shot_cct = cur.as_shot_cct();
        let dcp = if cur.meta.kind.allows_raw_only_stages()
            && crate::profile::DcpProfile::applies_to_display_look(self.display_look)
        {
            cur.dcp_profile.clone()
        } else {
            None
        };
        let lut = if is_original {
            None
        } else {
            cur.lut_cube.clone()
        };
        let started = Instant::now();
        let rgba = {
            let graph = self.graph.as_mut().unwrap();
            graph.render(
                gpu,
                tex_view,
                w,
                h,
                &view,
                render_doc,
                as_shot_cct,
                &seg_views,
                self.overlay_mask.as_deref(),
                dcp.as_deref(),
                lut.as_deref(),
            )?
        };
        let elapsed = started.elapsed();
        if let Some(g) = &self.graph {
            tracing::debug!(
                ms = elapsed.as_millis() as u64,
                passes = ?g.last_passes_run,
                "render"
            );
        }
        self.last_view = Some(view);
        // perf instrumentation (spec 7.5)
        self.perf.last_render_ms = elapsed.as_millis() as u64;
        self.perf.renders += 1;
        if let Some(g) = &self.graph {
            self.perf.last_passes = g.last_passes_run.clone();
            // no extract pass ⇒ upstream caches were reused this render
            if !g.last_passes_run.iter().any(|p| p == "extract") {
                self.perf.cached_renders += 1;
            }
        }
        let version = self.next_version();
        let frame = Frame {
            width: view.out_w,
            height: view.out_h,
            rgba,
            version,
        };
        let info = FrameInfo {
            version,
            width: frame.width,
            height: frame.height,
        };
        self.latest_frame = Some(frame);
        Ok(info)
    }
    pub(super) fn render_preview_jpeg(&mut self, max_dim: u32) -> Result<Vec<u8>, CoreError> {
        self.ensure_segmentations();
        let Some(cur) = &self.current else {
            return Err(CoreError::NoImage);
        };
        let Some((_, tex_view, w, h)) = &cur.working else {
            return Err(CoreError::NoImage);
        };
        let gpu = self.gpu.as_ref().ok_or(CoreError::Gpu("no gpu".into()))?;
        if self.preview_graph.is_none() {
            self.preview_graph = Some(RenderGraph::new(gpu));
        }
        let (w, h) = (*w, *h);
        let max_dim = max_dim.clamp(256, 1600);
        // previews reflect a committed crop: size against the content dims
        let crop = crate::crop::CropParams::from_doc(cur.doc());
        let (cw, ch) = crop.content_dims(w, h, crop.mode(false));
        let scale = (max_dim as f32 / cw.max(ch)).min(1.0);
        let view = ViewParams {
            out_w: ((cw * scale) as u32).max(1),
            out_h: ((ch * scale) as u32).max(1),
            scale: Some(scale),
            center_x: 0.5,
            center_y: 0.5,
            crop_preview: false,
        };
        let seg_views: std::collections::HashMap<String, wgpu::TextureView> = cur
            .masks_gpu
            .iter()
            .map(|(id, (_, tex))| (id.clone(), tex.create_view(&Default::default())))
            .collect();
        let graph = self.preview_graph.as_mut().unwrap();
        let display_look =
            crate::lut::present_look_for(cur.meta.kind, self.display_look, cur.doc());
        graph.set_look(display_look);
        graph.set_mask_overlay_style(self.overlay_strength, self.overlay_mode);
        graph.invalidate_all();
        let dcp = if cur.meta.kind.allows_raw_only_stages()
            && crate::profile::DcpProfile::applies_to_display_look(self.display_look)
        {
            cur.dcp_profile.as_deref()
        } else {
            None
        };
        let rgba = graph.render(
            gpu,
            tex_view,
            w,
            h,
            &view,
            cur.doc(),
            cur.as_shot_cct(),
            &seg_views,
            None,
            dcp,
            cur.lut_cube.as_deref(),
        )?;
        let rgb: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let img = image::RgbImage::from_raw(view.out_w, view.out_h, rgb)
            .ok_or_else(|| CoreError::Engine("preview buffer".into()))?;
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82)
            .encode_image(&img)
            .map_err(|e| CoreError::Engine(e.to_string()))?;
        Ok(out)
    }
    pub(super) fn sample_color(
        &self,
        x: f32,
        y: f32,
    ) -> Result<crate::message::SampledColor, CoreError> {
        let cur = self.current.as_ref().ok_or(CoreError::NoImage)?;
        let small = cur.small_cpu.as_ref().ok_or(CoreError::NoImage)?;
        let (w, h) = (small.width, small.height);
        let cx = ((x.clamp(0.0, 1.0) * w as f32) as usize).min(w - 1);
        let cy = ((y.clamp(0.0, 1.0) * h as f32) as usize).min(h - 1);
        let i = (cy * w + cx) * 3;
        let lin = [small.data[i], small.data[i + 1], small.data[i + 2]];
        // friendly display value (same math as the present shader)
        let m = crate::color::mat_mul(&crate::color::XYZ_TO_SRGB, &crate::color::REC2020_TO_XYZ);
        let srgb_lin = crate::color::mat_vec(&m, lin);
        let l = 0.2126 * srgb_lin[0].max(0.0)
            + 0.7152 * srgb_lin[1].max(0.0)
            + 0.0722 * srgb_lin[2].max(0.0);
        let k = if l > 1e-8 {
            (l * (1.0 + l / 16.0) / (1.0 + l)) / l
        } else {
            0.0
        };
        let display = srgb_lin.map(|c| {
            let c = (c.max(0.0) * k).clamp(0.0, 1.0);
            let enc = if c <= 0.0031308 {
                12.92 * c
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            (enc * 255.0).round() as u8
        });
        Ok(crate::message::SampledColor {
            working: lin,
            display,
        })
    }
    /// Crop-tool auto-level (plan P6): Sobel gradients on a downscaled luma,
    /// magnitude-weighted histogram of edge orientation folded mod 90° into
    /// [-45°, 45°), smoothed; the peak is the dominant line deviation.
    /// Returns 0.0 when no direction clearly dominates.
    pub(super) fn auto_level(&self) -> Result<f32, CoreError> {
        let cur = self.current.as_ref().ok_or(CoreError::NoImage)?;
        let small = cur.small_cpu.as_ref().ok_or(CoreError::NoImage)?;
        let img = small.downscale_to(768);
        let (w, h) = (img.width, img.height);
        if w < 16 || h < 16 {
            return Ok(0.0);
        }
        let luma: Vec<f32> = img
            .data
            .chunks_exact(3)
            .map(|p| {
                let l = 0.2627 * p[0].max(0.0) + 0.678 * p[1].max(0.0) + 0.0593 * p[2].max(0.0);
                // log-ish tone compression so bright skies don't drown edges
                (1.0 + l * 64.0).ln()
            })
            .collect();

        const BINS: usize = 360; // 0.25° over [-45, 45)
        let mut hist = vec![0f64; BINS];
        let mut total_mag = 0f64;
        let mut count = 0usize;
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let px = |dx: i32, dy: i32| {
                    luma[(y as i32 + dy) as usize * w + (x as i32 + dx) as usize]
                };
                let gx = (px(1, -1) + 2.0 * px(1, 0) + px(1, 1))
                    - (px(-1, -1) + 2.0 * px(-1, 0) + px(-1, 1));
                let gy = (px(-1, 1) + 2.0 * px(0, 1) + px(1, 1))
                    - (px(-1, -1) + 2.0 * px(0, -1) + px(1, -1));
                let mag = (gx * gx + gy * gy).sqrt();
                if mag < 1e-4 {
                    continue;
                }
                // edge direction = gradient rotated 90°; fold into [-45, 45)
                let edge_deg = gy.atan2(gx).to_degrees() + 90.0;
                let mut d = edge_deg.rem_euclid(90.0);
                if d >= 45.0 {
                    d -= 90.0;
                }
                let bin = (((d + 45.0) / 90.0 * BINS as f32) as usize).min(BINS - 1);
                hist[bin] += mag as f64;
                total_mag += mag as f64;
                count += 1;
            }
        }
        if count == 0 || total_mag <= 0.0 {
            return Ok(0.0);
        }
        // smooth (±2 bins ≈ ±0.5°)
        let smoothed: Vec<f64> = (0..BINS)
            .map(|i| {
                (-2i32..=2)
                    .map(|o| hist[(i as i32 + o).rem_euclid(BINS as i32) as usize])
                    .sum::<f64>()
                    / 5.0
            })
            .collect();
        let (peak_bin, peak_val) = smoothed
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        // confidence: the peak must beat the average bin mass by a clear margin
        let avg = total_mag / BINS as f64;
        if *peak_val < avg * 2.5 {
            return Ok(0.0);
        }
        let d = (peak_bin as f32 + 0.5) / BINS as f32 * 90.0 - 45.0;
        // tiny deviations aren't worth a resample; huge ones are usually a
        // diagonal composition, not a crooked horizon
        if d.abs() < 0.05 || d.abs() > 20.0 {
            return Ok(0.0);
        }
        Ok(d)
    }

    pub(super) fn render_now(&mut self) {
        let Some(view) = self.last_view else { return };
        match self.render_view(view) {
            Ok(info) => self.emit(EngineEvent::FrameReady {
                version: info.version,
            }),
            Err(CoreError::NoImage) => {}
            Err(e) => tracing::error!(error = %e, "debounced render failed"),
        }
    }
}