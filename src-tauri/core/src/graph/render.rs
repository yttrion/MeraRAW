//! Render pass execution, cache invalidation, and module chain.

use super::config::{mask_node_configs, node_configs, NodeConfig};
use super::mask_stage_index;
use super::resources::{
    make_chain_tex, make_mask_tex, make_tex, BlendUniforms, CropUniform, DcpLookUniforms, DcpMeta,
    ExtractUniforms, MaskCombineUniforms, MaskFinalizeUniforms, MaskGeomUniforms,
    MaskSampleUniforms, PassResources, PipeKind, PresentUniforms, MAX_STROKE_POINTS, NODE_PIPES,
};
use super::{FinalTag, RenderGraph, NODES};
use crate::doc::EditDoc;
use crate::error::CoreError;
use crate::gpu::display::ViewParams;
use crate::gpu::GpuContext;
use crate::profile::hue_sat_map::cct_weight;
use crate::profile::DcpProfile;
use std::collections::HashMap;

fn blend_mode_id(blend: &str) -> u32 {
    match blend {
        "multiply" => 1,
        "screen" => 2,
        _ => 0,
    }
}

fn pack_mat3_cols(m: &crate::color::Mat3) -> [[f32; 4]; 3] {
    [
        [m[0][0], m[1][0], m[2][0], 0.0],
        [m[0][1], m[1][1], m[2][1], 0.0],
        [m[0][2], m[1][2], m[2][2], 0.0],
    ]
}

fn proof_matrices(space: u32) -> ([[f32; 4]; 3], [[f32; 4]; 3], u32) {
    if space == 0 {
        let z = [0.0f32; 4];
        return ([z, z, z], [z, z, z], 0);
    }
    let target = match space {
        2 => crate::export::TargetSpace::DisplayP3,
        3 => crate::export::TargetSpace::AdobeRgb,
        4 => crate::export::TargetSpace::ProPhoto,
        _ => crate::export::TargetSpace::Srgb,
    };
    let to_target = crate::export::target_from_rec2020(target);
    let to_srgb = crate::color::mat_mul(&crate::color::XYZ_TO_SRGB, &crate::color::REC2020_TO_XYZ);
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let inv = crate::color::mat_inverse(&to_target).unwrap_or(identity);
    let target_to_srgb = crate::color::mat_mul(&to_srgb, &inv);
    (
        pack_mat3_cols(&to_target),
        pack_mat3_cols(&target_to_srgb),
        space,
    )
}

fn mask_sample_uniforms(
    source: &serde_json::Value,
    out_w: u32,
    out_h: u32,
    img_w: f32,
    img_h: f32,
    scale: f32,
    view: &ViewParams,
    feather: f32,
    opacity: f32,
    invert: u32,
    crop: CropUniform,
    param_mode: u32,
) -> MaskSampleUniforms {
    let f = |k: &str, d: f64| source.get(k).and_then(|v| v.as_f64()).unwrap_or(d) as f32;
    MaskSampleUniforms {
        out_w,
        out_h,
        img_w,
        img_h,
        scale,
        center_x: view.center_x,
        center_y: view.center_y,
        feather,
        opacity,
        invert,
        crop,
        luma_lo: f("luma_lo", 0.0),
        luma_hi: f("luma_hi", 1.0),
        chroma_lo: f("chroma_lo", 0.0),
        chroma_hi: f("chroma_hi", 1.0),
        hue_lo: f("hue_lo", 0.0),
        hue_hi: f("hue_hi", 0.0),
        softness: f("softness", 0.05),
        param_mode,
    }
}

impl RenderGraph {
    /// Upload the DCP look's HSV delta tables (map1|map2|look, concatenated)
    /// and tone LUT once per profile. cct only affects the per-render blend
    /// weight (a uniform), so it is NOT part of the cache key.
    fn ensure_dcp_tables(&mut self, gpu: &GpuContext, dcp: &DcpProfile) {
        let sig = format!("{}|{}", dcp.unique_camera_model, dcp.profile_name);
        if self.dcp_sig.as_deref() == Some(sig.as_str()) {
            return;
        }
        fn pack(
            tables: &mut Vec<[f32; 4]>,
            m: &Option<(u32, u32, u32, Vec<[f32; 4]>)>,
        ) -> (u32, [u32; 4]) {
            match m {
                Some((hd, sd, vd, deltas)) => {
                    let off = tables.len() as u32;
                    tables.extend_from_slice(deltas);
                    (1, [off, *hd, *sd, *vd])
                }
                None => (0, [0, 0, 0, 0]),
            }
        }
        let data = dcp.look_data();
        let mut tables: Vec<[f32; 4]> = Vec::new();
        let (has1, m1) = pack(&mut tables, &data.map1);
        let (has2, m2) = pack(&mut tables, &data.map2);
        let (hasl, lk) = pack(&mut tables, &data.look);
        if tables.is_empty() {
            tables.push([0.0; 4]); // storage buffers must be non-empty
        }
        let make_storage = |label: &str, bytes: &[u8]| -> wgpu::Buffer {
            let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            gpu.queue.write_buffer(&buf, 0, bytes);
            buf
        };
        self.dcp_tables_buf = Some(make_storage("dcp-tables", bytemuck::cast_slice(&tables)));
        self.dcp_tone_buf = Some(make_storage(
            "dcp-tone",
            bytemuck::cast_slice(&data.tone_lut),
        ));
        self.dcp_meta = Some(DcpMeta {
            has_map1: has1,
            has_map2: if has1 == 1 { has2 } else { 0 },
            has_look: hasl,
            tone_size: data.tone_lut.len() as u32,
            baseline_gain: data.baseline_gain,
            t1: data.ill1_cct,
            t2: data.ill2_cct,
            m1,
            m2,
            lk,
        });
        self.dcp_sig = Some(sig);
    }

    /// Set the display look (0 = Neutral, 1 = Camera/punchy, 2 = Filmic/AgX,
    /// 4 = Original — demosaic only, no DCP profile look, 8 = Linear/None — no tone mapping).
    pub fn set_look(&mut self, look: u32) {
        self.look = look;
    }

    pub fn set_clip_warnings(&mut self, hi: bool, lo: bool) {
        self.clip_hi = hi;
        self.clip_lo = lo;
    }

    pub fn set_mask_overlay_style(&mut self, strength: f32, mode: u32) {
        self.overlay_strength = strength.clamp(0.0, 1.0);
        self.overlay_mode = mode.min(3);
    }

    pub fn set_proof(&mut self, space: u32, gamut: bool) {
        self.proof_space = space.min(4);
        self.proof_gamut = gamut && space > 0;
    }

    pub fn invalidate_from_module(&mut self, module: &str) {
        if module == "crop" {
            self.last_crop_key = None;
            self.last_view_key = None;
            self.dirty_from = 0;
            return;
        }
        let idx = if module == "masks" {
            mask_stage_index()
        } else {
            NODES.iter().position(|(_, m)| *m == module).unwrap_or(0)
        };
        self.dirty_from = self.dirty_from.min(idx);
    }

    pub fn invalidate_all(&mut self) {
        self.dirty_from = 0;
        self.last_view_key = None;
        self.last_crop_key = None;
    }

    fn ensure_pools(&mut self, gpu: &GpuContext, uniforms: usize, luts: usize, strokes: usize) {
        while self.pool.len() < uniforms {
            self.pool
                .push(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("mask-pool-u"),
                    size: 512,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }
        while self.lut_pool.len() < luts {
            self.lut_pool
                .push(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("mask-pool-lut"),
                    size: (crate::curve::LUT_SIZE * 4 * 4) as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }
        while self.strokes_pool.len() < strokes {
            self.strokes_pool
                .push(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("mask-pool-strokes"),
                    size: (MAX_STROKE_POINTS * 16) as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gpu: &GpuContext,
        working_view: &wgpu::TextureView,
        img_w: u32,
        img_h: u32,
        view: &ViewParams,
        doc: &EditDoc,
        as_shot_cct: f32,
        seg_masks: &HashMap<String, wgpu::TextureView>,
        overlay_mask: Option<&str>,
        dcp_profile: Option<&DcpProfile>,
        lut: Option<&crate::lut::CubeLut>,
    ) -> Result<Vec<u8>, CoreError> {
        self.last_passes_run.clear();
        // clamp to a safe texture size — never exceed the GPU 2D limit (the
        // viewport is a screen preview; 8192 is ample and panic-proof)
        const MAX_VIEW: u32 = 8192;
        let out_w = view.out_w.clamp(1, MAX_VIEW);
        let out_h = view.out_h.clamp(1, MAX_VIEW);
        let crop = crate::crop::CropParams::from_doc(doc);
        let crop_key = crop.signature(view.crop_preview);
        let scale = view.effective_scale_crop(img_w, img_h, &crop);
        let view_key = [
            out_w,
            out_h,
            scale.to_bits(),
            view.center_x.to_bits(),
            view.center_y.to_bits(),
        ];
        let view_changed = self.last_view_key != Some(view_key);
        let crop_changed = self.last_crop_key != Some(crop_key);

        // totally clean → out_tex still holds the right pixels
        if !view_changed && self.dirty_from == usize::MAX && self.out_tex.is_some() {
            return self.readback(gpu, out_w, out_h);
        }

        if self.cache_size != Some((out_w, out_h)) {
            self.extract_tex = Some(make_chain_tex(gpu, out_w, out_h, "extract-out"));
            self.look_tex = Some(make_chain_tex(gpu, out_w, out_h, "dcp-look-out"));
            for (i, slot) in self.node_tex.iter_mut().enumerate() {
                *slot = Some(make_chain_tex(gpu, out_w, out_h, NODES[i].0));
            }
            self.scratch = (0..2)
                .map(|i| {
                    make_chain_tex(
                        gpu,
                        out_w,
                        out_h,
                        if i == 0 { "scratch-a" } else { "scratch-b" },
                    )
                })
                .collect();
            self.composite = (0..2)
                .map(|i| {
                    make_chain_tex(gpu, out_w, out_h, if i == 0 { "comp-a" } else { "comp-b" })
                })
                .collect();
            // A/B = accumulator ping-pong; C = per-component produce target
            // (must not alias with acc — 2-tex ping-pong wiped layers at 3+ comps).
            self.mask_scratch = (0..3)
                .map(|i| {
                    make_mask_tex(
                        gpu,
                        out_w,
                        out_h,
                        match i {
                            0 => "msk-scratch-a",
                            1 => "msk-scratch-b",
                            _ => "msk-scratch-c",
                        },
                    )
                })
                .collect();
            self.mask_tex.clear();
            self.out_tex = Some(make_tex(
                gpu,
                out_w,
                out_h,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                "present-out",
            ));
            self.cache_size = Some((out_w, out_h));
            self.dirty_from = 0;
        } else if view_changed {
            self.dirty_from = 0;
        }

        // mask textures + pools sized up-front (avoids borrow tangles)
        for m in &doc.masks {
            if !self.mask_tex.contains_key(&m.id) {
                self.mask_tex
                    .insert(m.id.clone(), make_mask_tex(gpu, out_w, out_h, "mask"));
            }
        }
        let n_masks = doc.masks.len();
        // Composites need one uniform + optional stroke slot per component.
        let n_comps: usize = doc
            .masks
            .iter()
            .map(|m| {
                m.source
                    .get("components")
                    .and_then(|c| c.as_array())
                    .map(|a| a.len().max(1))
                    .unwrap_or(1)
            })
            .sum();
        let n_brush: usize = doc
            .masks
            .iter()
            .map(|m| {
                let comps = m.source.get("components").and_then(|c| c.as_array());
                match comps {
                    Some(arr) => arr
                        .iter()
                        .filter(|c| {
                            c.get("source")
                                .and_then(|s| s.get("type"))
                                .and_then(|t| t.as_str())
                                == Some("brush")
                                || c.get("type").and_then(|t| t.as_str()) == Some("brush")
                        })
                        .count()
                        .max(1),
                    None => {
                        if m.source.get("type").and_then(|t| t.as_str()) == Some("brush") {
                            1
                        } else {
                            0
                        }
                    }
                }
            })
            .sum::<usize>()
            .max(n_masks.max(1));
        self.ensure_pools(
            gpu,
            (n_comps * 4 + n_masks * (2 + NODES.len())).max(16),
            n_masks.max(1),
            n_brush,
        );

        let configs = node_configs(doc, as_shot_cct, out_w, out_h, lut);

        // Upload the DCP look tables once per profile (cheap signature check).
        // Camera look (1) is the only display mode that consumes them.
        if let Some(dcp) = dcp_profile.filter(|d| d.has_look()) {
            if view_changed || DcpProfile::applies_to_display_look(self.look) {
                self.ensure_dcp_tables(gpu, dcp);
            }
        }

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("graph-encoder"),
            });

        // ---- extract ----
        // Avoid `let extract_tex = …` — a long-lived ref blocks `&mut self` in composite masks.
        // Extract + dcp_look depend ONLY on the view (their inputs are the
        // working master + view params, never the doc). So they re-run on a
        // view change — which `invalidate_all` also forces via last_view_key =
        // None. Gating on `dirty_from == 0` was wrong: editing module 0
        // (exposure) sets dirty_from = 0 and needlessly re-ran the ~1.3s CPU
        // dcp_look every slider tick.
        let crop_mode = crop.mode(view.crop_preview);
        let crop_u = CropUniform::new(&crop, crop_mode);
        let run_extract = view_changed || crop_changed;
        if run_extract {
            let u = ExtractUniforms {
                out_w,
                out_h,
                img_w: img_w as f32,
                img_h: img_h as f32,
                scale,
                center_x: view.center_x,
                center_y: view.center_y,
                crop: crop_u,
            };
            gpu.queue
                .write_buffer(&self.extract_uniforms, 0, bytemuck::bytes_of(&u));
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("extract-bind"),
                layout: &self.extract.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(working_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(
                            &self
                                .extract_tex
                                .as_ref()
                                .unwrap()
                                .create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.extract_uniforms.as_entire_binding(),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("extract"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.extract.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
            self.last_passes_run.push("extract".into());
        }

        // DCP look on the extracted viewport, on the GPU (no readback). Mirrors
        // the CPU apply_look; runs only on a view change, result persists in
        // look_tex. Stays in the same encoder — the extract→look read hazard is
        // handled by the compute-pass boundary.
        // Camera look only. Neutral/Filmic/Original stay scene-referred so
        // the Adobe default tone curve isn't stacked under AgX or Reinhard.
        let dcp_active = DcpProfile::applies_to_display_look(self.look)
            && dcp_profile.filter(|d| d.has_look()).is_some()
            && self.dcp_meta.is_some();
        if run_extract && dcp_active {
            let meta = self.dcp_meta.clone().unwrap();
            let look_tex = self.look_tex.as_ref().unwrap();
            let u = DcpLookUniforms {
                width: out_w,
                height: out_h,
                has_map1: meta.has_map1,
                has_map2: meta.has_map2,
                has_look: meta.has_look,
                tone_size: meta.tone_size,
                cct_weight: cct_weight(as_shot_cct, meta.t1, meta.t2),
                baseline_gain: meta.baseline_gain,
                m1: meta.m1,
                m2: meta.m2,
                lk: meta.lk,
            };
            gpu.queue
                .write_buffer(&self.dcp_look_uniforms, 0, bytemuck::bytes_of(&u));
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dcp-look-bind"),
                layout: &self.dcp_look.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &self
                                .extract_tex
                                .as_ref()
                                .unwrap()
                                .create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &look_tex.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.dcp_look_uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.dcp_tables_buf.as_ref().unwrap().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.dcp_tone_buf.as_ref().unwrap().as_entire_binding(),
                    },
                ],
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dcp-look"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.dcp_look.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
            }
            self.last_passes_run.push("dcp_look".into());
        }

        // ---- global module chain ----
        // When the profile has a look, the chain reads the looked extract.
        let mut upstream: &wgpu::Texture = if dcp_active {
            self.look_tex.as_ref().unwrap()
        } else {
            self.extract_tex.as_ref().unwrap()
        };
        // Export readback uses last_final. If DCP ran and every module is
        // identity, present still reads look_tex via `upstream` — last_final
        // must point at the same texture or zero-edit exports drop the DCP.
        let mut final_tag = if dcp_active {
            FinalTag::Look
        } else {
            FinalTag::Extract
        };
        for (i, cfg) in configs.iter().enumerate() {
            let NodeConfig::Run { uniforms, lut } = cfg else {
                continue;
            };
            let must_run = i >= self.dirty_from || run_extract;
            let out = self.node_tex[i].as_ref().unwrap();
            if must_run {
                gpu.queue.write_buffer(&self.node_uniforms[i], 0, uniforms);
                // The 3D LUT node has its own buffer; every other LUT-carrying
                // node (tone_curve) shares self.lut_buffer.
                let lut_buf = if NODE_PIPES[i] == PipeKind::Lut3d {
                    &self.lut3d_buffer
                } else {
                    &self.lut_buffer
                };
                if let Some(lut_data) = lut {
                    gpu.queue
                        .write_buffer(lut_buf, 0, bytemuck::cast_slice(lut_data));
                }
                let pipe = self.pipe_for(NODE_PIPES[i]);
                dispatch_node(
                    gpu,
                    &mut encoder,
                    pipe,
                    NODE_PIPES[i].has_lut(),
                    upstream,
                    out,
                    &self.node_uniforms[i],
                    lut_buf,
                    NODES[i].0,
                    out_w,
                    out_h,
                );
                self.last_passes_run.push(NODES[i].0.to_string());
            }
            upstream = out;
            final_tag = FinalTag::Node(i);
        }

        // ---- mask stage ----
        let masks_run = !doc.masks.is_empty();
        if masks_run {
            let mut pool_i = 0usize;
            let mut lut_i = 0usize;
            let mut strokes_i = 0usize;
            let mut comp_flip = 0usize;
            let extract_view = self
                .extract_tex
                .as_ref()
                .unwrap()
                .create_view(&Default::default());
            for mask in &doc.masks {
                if !mask.enabled {
                    continue;
                }
                let mtex = &self.mask_tex[&mask.id];
                let opacity = (mask.opacity / 100.0).clamp(0.0, 1.0);
                let feather = (mask.feather / 100.0).clamp(0.0, 1.0);
                let src_type = mask
                    .source
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let produced = match src_type {
                    "radial" | "linear" | "brush" => {
                        let (kind, pa, pb, rotation, strokes) = parse_geometry(&mask.source);
                        let ub = &self.pool[pool_i];
                        pool_i += 1;
                        let sb = &self.strokes_pool[strokes_i];
                        strokes_i += 1;
                        let u = MaskGeomUniforms {
                            out_w,
                            out_h,
                            img_w: img_w as f32,
                            img_h: img_h as f32,
                            scale,
                            center_x: view.center_x,
                            center_y: view.center_y,
                            kind,
                            pa,
                            pb,
                            rotation,
                            feather,
                            opacity,
                            invert: mask.invert as u32,
                            stroke_count: strokes.len() as u32,
                            crop: crop_u,
                            _p0: 0,
                            _p1: 0,
                            _p2: 0,
                            _p3: 0,
                        };
                        gpu.queue.write_buffer(ub, 0, bytemuck::bytes_of(&u));
                        if !strokes.is_empty() {
                            gpu.queue
                                .write_buffer(sb, 0, bytemuck::cast_slice(&strokes));
                        }
                        let mview = mtex.create_view(&Default::default());
                        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("mask-geom-bind"),
                            layout: &self.mask_geom.layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(&mview),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: ub.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: sb.as_entire_binding(),
                                },
                            ],
                        });
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("mask-geom"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(&self.mask_geom.pipeline);
                        pass.set_bind_group(0, &bind, &[]);
                        pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
                        self.last_passes_run.push(format!("mask:{}", mask.kind));
                        true
                    }
                    "segmented" => {
                        if let Some(small_view) = seg_masks.get(&mask.id) {
                            let ub = &self.pool[pool_i];
                            pool_i += 1;
                            // background kind = inverse of the subject mask
                            let invert = (mask.invert ^ (mask.kind == "background")) as u32;
                            let u = mask_sample_uniforms(
                                &mask.source,
                                out_w,
                                out_h,
                                img_w as f32,
                                img_h as f32,
                                scale,
                                view,
                                feather,
                                opacity,
                                invert,
                                crop_u,
                                0,
                            );
                            gpu.queue.write_buffer(ub, 0, bytemuck::bytes_of(&u));
                            let mview = mtex.create_view(&Default::default());
                            let eview = self
                                .extract_tex
                                .as_ref()
                                .unwrap()
                                .create_view(&Default::default());
                            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("mask-sample-bind"),
                                layout: &self.mask_sample.layout,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::TextureView(small_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::TextureView(&eview),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 3,
                                        resource: wgpu::BindingResource::TextureView(&mview),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 4,
                                        resource: ub.as_entire_binding(),
                                    },
                                ],
                            });
                            let mut pass =
                                encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                    label: Some("mask-sample"),
                                    timestamp_writes: None,
                                });
                            pass.set_pipeline(&self.mask_sample.pipeline);
                            pass.set_bind_group(0, &bind, &[]);
                            pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
                            self.last_passes_run.push(format!("mask:{}", mask.kind));
                            true
                        } else {
                            false // inference pending — mask contributes nothing yet
                        }
                    }
                    "parametric" => {
                        let ub = &self.pool[pool_i];
                        pool_i += 1;
                        let u = mask_sample_uniforms(
                            &mask.source,
                            out_w,
                            out_h,
                            img_w as f32,
                            img_h as f32,
                            scale,
                            view,
                            feather,
                            opacity,
                            mask.invert as u32,
                            crop_u,
                            1,
                        );
                        gpu.queue.write_buffer(ub, 0, bytemuck::bytes_of(&u));
                        let mview = mtex.create_view(&Default::default());
                        let eview = self
                            .extract_tex
                            .as_ref()
                            .unwrap()
                            .create_view(&Default::default());
                        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("mask-param-bind"),
                            layout: &self.mask_sample.layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    // R32 dummy is not filterable; param_mode does not sample this.
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(&eview),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::TextureView(&eview),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(&mview),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: ub.as_entire_binding(),
                                },
                            ],
                        });
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("mask-param"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(&self.mask_sample.pipeline);
                        pass.set_bind_group(0, &bind, &[]);
                        pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
                        self.last_passes_run.push(format!("mask:{}", mask.kind));
                        true
                    }
                    "composite" => produce_composite_mask(
                        gpu,
                        &mut encoder,
                        &self.mask_geom,
                        &self.mask_sample,
                        &self.mask_combine,
                        &self.mask_finalize,
                        &self.sampler,
                        &mut self.pool,
                        &mut self.strokes_pool,
                        &self.mask_scratch,
                        &mut self.last_passes_run,
                        mask,
                        mtex,
                        &extract_view,
                        &seg_masks,
                        out_w,
                        out_h,
                        img_w,
                        img_h,
                        scale,
                        view,
                        feather,
                        crop_u,
                        &mut pool_i,
                        &mut strokes_i,
                    ),
                    _ => false,
                };
                if !produced {
                    continue;
                }

                // local module stack (same passes, scoped params)
                let cfgs = mask_node_configs(mask, as_shot_cct, out_w, out_h);
                let mut local_src: &wgpu::Texture = upstream;
                let mut flip = 0usize;
                let mut ran_local = false;
                for (i, cfg) in cfgs.iter().enumerate() {
                    let NodeConfig::Run { uniforms, lut } = cfg else {
                        continue;
                    };
                    let ub = &self.pool[pool_i];
                    pool_i += 1;
                    gpu.queue.write_buffer(ub, 0, uniforms);
                    let lut_buf = if let Some(lut_data) = lut {
                        let lb = &self.lut_pool[lut_i];
                        lut_i += 1;
                        gpu.queue
                            .write_buffer(lb, 0, bytemuck::cast_slice(lut_data));
                        lb
                    } else {
                        &self.lut_buffer
                    };
                    let out = &self.scratch[flip];
                    flip ^= 1;
                    let pipe = self.pipe_for(NODE_PIPES[i]);
                    dispatch_node(
                        gpu,
                        &mut encoder,
                        pipe,
                        NODE_PIPES[i].has_lut(),
                        local_src,
                        out,
                        ub,
                        lut_buf,
                        NODES[i].0,
                        out_w,
                        out_h,
                    );
                    self.last_passes_run
                        .push(format!("mask-local:{}", NODES[i].0));
                    local_src = out;
                    ran_local = true;
                }
                if !ran_local {
                    continue; // mask has no edits — nothing to blend
                }

                // blend over the running composite
                let dst = &self.composite[comp_flip];
                comp_flip ^= 1;
                let ub = &self.pool[pool_i];
                pool_i += 1;
                gpu.queue.write_buffer(
                    ub,
                    0,
                    bytemuck::bytes_of(&BlendUniforms {
                        width: out_w,
                        height: out_h,
                        mode: blend_mode_id(&mask.blend),
                        _p1: 0,
                    }),
                );
                let base_view = upstream.create_view(&Default::default());
                let local_view = local_src.create_view(&Default::default());
                let mview = mtex.create_view(&Default::default());
                let dview = dst.create_view(&Default::default());
                let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("blend-bind"),
                    layout: &self.blend.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&base_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&local_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&mview),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&dview),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: ub.as_entire_binding(),
                        },
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("blend"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.blend.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
                self.last_passes_run.push("blend".into());
                upstream = dst;
                final_tag = FinalTag::Comp(comp_flip ^ 1);
            }
        }
        self.last_final = final_tag;

        // ---- present (terminal) ----
        let out_tex = self.out_tex.as_ref().unwrap();
        {
            let overlay_view = overlay_mask
                .and_then(|id| self.mask_tex.get(id))
                .unwrap_or(&self.dummy_mask)
                .create_view(&Default::default());
            let millis = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| (d.as_millis() % u32::MAX as u128) as u32)
                .unwrap_or(0);
            let (m, n, space) = proof_matrices(self.proof_space);
            let u = PresentUniforms {
                width: out_w,
                height: out_h,
                overlay: if overlay_mask.is_some() {
                    self.overlay_strength
                } else {
                    0.0
                },
                look: DcpProfile::present_look(self.look, dcp_profile.filter(|_| dcp_active)),
                clip_hi: u32::from(self.clip_hi),
                clip_lo: u32::from(self.clip_lo),
                _p0: millis,
                _p1: space
                    | if self.proof_gamut { 16 } else { 0 }
                    | ((self.overlay_mode & 0xf) << 8),
                m0: m[0],
                m1: m[1],
                m2: m[2],
                n0: n[0],
                n1: n[1],
                n2: n[2],
            };
            gpu.queue
                .write_buffer(&self.present_uniforms, 0, bytemuck::bytes_of(&u));
            let upstream_view = upstream.create_view(&Default::default());
            let out_view = out_tex.create_view(&Default::default());
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("present-bind"),
                layout: &self.present.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&upstream_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&out_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.present_uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&overlay_view),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("present"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.present.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
            self.last_passes_run.push("present".into());
        }

        gpu.queue.submit([encoder.finish()]);
        self.last_view_key = Some(view_key);
        self.last_crop_key = Some(crop_key);
        self.dirty_from = usize::MAX;
        self.readback(gpu, out_w, out_h)
    }

    fn pipe_for(&self, kind: PipeKind) -> &PassResources {
        match kind {
            PipeKind::Matrix => &self.simple_pipes["matrix"],
            PipeKind::Highlights => &self.simple_pipes["highlights"],
            PipeKind::Calibration => &self.simple_pipes["calibration"],
            PipeKind::Noise => &self.simple_pipes["noise"],
            PipeKind::Grade => &self.simple_pipes["grade"],
            PipeKind::Hsl => &self.simple_pipes["hsl"],
            PipeKind::Sharpen => &self.simple_pipes["sharpen"],
            PipeKind::Effects => &self.simple_pipes["effects"],
            PipeKind::Curve => &self.curve_pipe,
            PipeKind::Lut3d => &self.lut_pipe,
        }
    }

    fn readback(&self, gpu: &GpuContext, out_w: u32, out_h: u32) -> Result<Vec<u8>, CoreError> {
        let out_tex = self
            .out_tex
            .as_ref()
            .ok_or_else(|| CoreError::Gpu("no output texture".into()))?;
        let bytes_per_row_packed = out_w * 4;
        let bytes_per_row = bytes_per_row_packed.div_ceil(256) * 256;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("graph-readback"),
            size: (bytes_per_row * out_h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback-encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: out_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(out_h),
                },
            },
            wgpu::Extent3d {
                width: out_w,
                height: out_h,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        gpu.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|_| CoreError::Gpu("readback dropped".into()))?
            .map_err(|e| CoreError::Gpu(format!("map: {e:?}")))?;
        let data = slice.get_mapped_range();
        let mut out = vec![0u8; (bytes_per_row_packed * out_h) as usize];
        for row in 0..out_h {
            let s = (row * bytes_per_row) as usize;
            let d = (row * bytes_per_row_packed) as usize;
            out[d..d + bytes_per_row_packed as usize]
                .copy_from_slice(&data[s..s + bytes_per_row_packed as usize]);
        }
        drop(data);
        readback.unmap();
        Ok(out)
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_node(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    pipe: &PassResources,
    has_lut: bool,
    input: &wgpu::Texture,
    output: &wgpu::Texture,
    uniforms: &wgpu::Buffer,
    lut: &wgpu::Buffer,
    label: &str,
    out_w: u32,
    out_h: u32,
) {
    let in_view = input.create_view(&Default::default());
    let out_view = output.create_view(&Default::default());
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&in_view),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::TextureView(&out_view),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: uniforms.as_entire_binding(),
        },
    ];
    if has_lut {
        entries.push(wgpu::BindGroupEntry {
            binding: 3,
            resource: lut.as_entire_binding(),
        });
    }
    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout: &pipe.layout,
        entries: &entries,
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some(label),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipe.pipeline);
    pass.set_bind_group(0, &bind, &[]);
    pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
}

/// Parse geometry source → (shader kind, pa, pb, rotation, stroke points).
fn parse_geometry(source: &serde_json::Value) -> (u32, [f32; 2], [f32; 2], f32, Vec<[f32; 4]>) {
    let get2 = |key: &str, default: [f32; 2]| -> [f32; 2] {
        source
            .get(key)
            .and_then(|v| v.as_array())
            .and_then(|a| Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32]))
            .unwrap_or(default)
    };
    match source.get("type").and_then(|t| t.as_str()) {
        Some("radial") => {
            let center = if let (Some(cx), Some(cy)) = (
                source.get("cx").and_then(|v| v.as_f64()),
                source.get("cy").and_then(|v| v.as_f64()),
            ) {
                [cx as f32, cy as f32]
            } else {
                get2("center", [0.5, 0.5])
            };
            let radii = if let (Some(rx), Some(ry)) = (
                source.get("rx").and_then(|v| v.as_f64()),
                source.get("ry").and_then(|v| v.as_f64()),
            ) {
                [rx as f32, ry as f32]
            } else {
                get2("radii", [0.25, 0.25])
            };
            let rotation = source
                .get("rotation")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32;
            (0, center, radii, rotation, vec![])
        }
        Some("linear") => {
            let start = if source.get("x0").is_some() {
                [
                    source.get("x0").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32,
                    source.get("y0").and_then(|v| v.as_f64()).unwrap_or(0.2) as f32,
                ]
            } else {
                get2("start", [0.5, 0.0])
            };
            let end = if source.get("x1").is_some() {
                [
                    source.get("x1").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32,
                    source.get("y1").and_then(|v| v.as_f64()).unwrap_or(0.8) as f32,
                ]
            } else {
                get2("end", [0.5, 1.0])
            };
            (1, start, end, 0.0, vec![])
        }
        Some("brush") => {
            let mut pts = Vec::new();
            if let Some(strokes) = source.get("strokes").and_then(|s| s.as_array()) {
                for stroke in strokes {
                    let radius = stroke
                        .get("radius")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.05) as f32;
                    let hardness = stroke
                        .get("hardness")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.5) as f32;
                    let flow = stroke
                        .get("flow")
                        .or_else(|| stroke.get("density"))
                        .or_else(|| stroke.get("amount"))
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0) as f32;
                    let sub = stroke.get("mode").and_then(|m| m.as_str()) == Some("subtract");
                    let r = if sub { -radius } else { radius };
                    // Pack hardness (0..1 → 0..100) + flow fraction into .w for the shader.
                    let packed =
                        (hardness.clamp(0.0, 1.0) * 100.0).floor() + flow.clamp(0.0, 0.999);
                    if let Some(points) = stroke.get("points").and_then(|p| p.as_array()) {
                        for p in points {
                            if let Some(a) = p.as_array() {
                                if let (Some(x), Some(y)) = (
                                    a.first().and_then(|v| v.as_f64()),
                                    a.get(1).and_then(|v| v.as_f64()),
                                ) {
                                    if pts.len() < MAX_STROKE_POINTS {
                                        pts.push([x as f32, y as f32, r, packed]);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            (2, [0.0; 2], [0.0; 2], 0.0, pts)
        }
        _ => (0, [0.5, 0.5], [0.0, 0.0], 0.0, vec![]),
    }
}

fn combine_op_id(op: &str) -> u32 {
    match op {
        "subtract" => 2,
        "intersect" => 3,
        _ => 1,
    }
}

#[allow(clippy::too_many_arguments)]
fn produce_composite_mask(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    mask_geom: &PassResources,
    mask_sample: &PassResources,
    mask_combine: &PassResources,
    mask_finalize: &PassResources,
    sampler: &wgpu::Sampler,
    pool: &mut Vec<wgpu::Buffer>,
    strokes_pool: &mut Vec<wgpu::Buffer>,
    mask_scratch: &[wgpu::Texture],
    last_passes_run: &mut Vec<String>,
    mask: &crate::doc::Mask,
    mtex: &wgpu::Texture,
    extract_view: &wgpu::TextureView,
    seg_masks: &HashMap<String, wgpu::TextureView>,
    out_w: u32,
    out_h: u32,
    img_w: u32,
    img_h: u32,
    scale: f32,
    view: &ViewParams,
    feather: f32,
    crop_u: CropUniform,
    pool_i: &mut usize,
    strokes_i: &mut usize,
) -> bool {
    let Some(comps) = mask.source.get("components").and_then(|c| c.as_array()) else {
        return false;
    };
    if comps.is_empty() || mask_scratch.len() < 3 {
        return false;
    }
    let scratch_a = &mask_scratch[0];
    let scratch_b = &mask_scratch[1];
    let scratch_c = &mask_scratch[2];
    let mut acc_in_a = true;
    let mut any = false;

    for (i, comp) in comps.iter().enumerate() {
        let op = comp.get("op").and_then(|o| o.as_str()).unwrap_or("add");
        let src = comp.get("source").cloned().unwrap_or_else(|| comp.clone());
        let src_type = src.get("type").and_then(|t| t.as_str()).unwrap_or("");
        // Produce into C so the component never aliases the A/B accumulator.
        let comp_tex = scratch_c;
        let produced = match src_type {
            "radial" | "linear" | "brush" => {
                if *pool_i >= pool.len() || *strokes_i >= strokes_pool.len() {
                    return false;
                }
                let (kind, pa, pb, rotation, strokes) = parse_geometry(&src);
                let ub = &pool[*pool_i];
                *pool_i += 1;
                let sb = &strokes_pool[*strokes_i];
                *strokes_i += 1;
                let u = MaskGeomUniforms {
                    out_w,
                    out_h,
                    img_w: img_w as f32,
                    img_h: img_h as f32,
                    scale,
                    center_x: view.center_x,
                    center_y: view.center_y,
                    kind,
                    pa,
                    pb,
                    rotation,
                    feather,
                    opacity: 1.0,
                    invert: 0,
                    stroke_count: strokes.len() as u32,
                    crop: crop_u,
                    _p0: 0,
                    _p1: 0,
                    _p2: 0,
                    _p3: 0,
                };
                gpu.queue.write_buffer(ub, 0, bytemuck::bytes_of(&u));
                if !strokes.is_empty() {
                    gpu.queue
                        .write_buffer(sb, 0, bytemuck::cast_slice(&strokes));
                }
                let mview = comp_tex.create_view(&Default::default());
                let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mask-composite-geom"),
                    layout: &mask_geom.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&mview),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: ub.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: sb.as_entire_binding(),
                        },
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("mask-composite-geom"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&mask_geom.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
                true
            }
            "segmented" => {
                let key = crate::segment::segment_cache_key(&mask.id, Some(i));
                // First wrapped component is the original subject — texture is
                // cached as `mask.id`, not `mask.id#c0`. Falling back here is
                // required or Subtract/Add refine finds no AI mask and panics
                // the combine pass (empty accumulator).
                let small_view = seg_masks.get(&key).or_else(|| {
                    if i == 0 {
                        seg_masks.get(&mask.id)
                    } else {
                        None
                    }
                });
                if let Some(small_view) = small_view {
                    if *pool_i >= pool.len() {
                        return false;
                    }
                    let ub = &pool[*pool_i];
                    *pool_i += 1;
                    let child_kind = crate::segment::kind_from_segmented_source(&mask.kind, &src);
                    // Invert from this component's model, not the parent mask kind.
                    let invert = (child_kind == "background") as u32;
                    let u = mask_sample_uniforms(
                        &src,
                        out_w,
                        out_h,
                        img_w as f32,
                        img_h as f32,
                        scale,
                        view,
                        feather,
                        1.0,
                        invert,
                        crop_u,
                        0,
                    );
                    gpu.queue.write_buffer(ub, 0, bytemuck::bytes_of(&u));
                    let mview = comp_tex.create_view(&Default::default());
                    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("mask-composite-seg"),
                        layout: &mask_sample.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(small_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(extract_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&mview),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: ub.as_entire_binding(),
                            },
                        ],
                    });
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("mask-composite-seg"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&mask_sample.pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
                    true
                } else {
                    false
                }
            }
            "parametric" => {
                if *pool_i >= pool.len() {
                    return false;
                }
                let ub = &pool[*pool_i];
                *pool_i += 1;
                let u = mask_sample_uniforms(
                    &src,
                    out_w,
                    out_h,
                    img_w as f32,
                    img_h as f32,
                    scale,
                    view,
                    feather,
                    1.0,
                    0,
                    crop_u,
                    1,
                );
                gpu.queue.write_buffer(ub, 0, bytemuck::bytes_of(&u));
                let mview = comp_tex.create_view(&Default::default());
                let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mask-composite-param"),
                    layout: &mask_sample.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(extract_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(extract_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&mview),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: ub.as_entire_binding(),
                        },
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("mask-composite-param"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&mask_sample.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
                true
            }
            _ => false,
        };
        if !produced {
            continue;
        }
        if *pool_i >= pool.len() {
            return false;
        }
        // First produced layer: replace into A. Acc and dst must be different
        // textures — wgpu rejects the same resource as sampled + storage, which
        // is what crashed Subtract on a freshly wrapped subject mask.
        let (acc_tex, dst_tex, next_acc_in_a) = if !any {
            (scratch_c, scratch_a, true)
        } else if acc_in_a {
            (scratch_a, scratch_b, false)
        } else {
            (scratch_b, scratch_a, true)
        };
        let combine_op = if !any { 0 } else { combine_op_id(op) };
        any = true;
        acc_in_a = next_acc_in_a;
        let acc_view = acc_tex.create_view(&Default::default());
        let src_view = comp_tex.create_view(&Default::default());
        let dst_view = dst_tex.create_view(&Default::default());
        let ub = &pool[*pool_i];
        *pool_i += 1;
        let cu = MaskCombineUniforms {
            width: out_w,
            height: out_h,
            op: combine_op,
            _pad: 0,
        };
        gpu.queue.write_buffer(ub, 0, bytemuck::bytes_of(&cu));
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mask-combine"),
            layout: &mask_combine.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&acc_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&dst_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ub.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("mask-combine"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&mask_combine.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
    }

    if !any {
        return false;
    }
    if *pool_i >= pool.len() {
        return false;
    }

    let acc_tex = if acc_in_a { scratch_a } else { scratch_b };
    let acc_view = acc_tex.create_view(&Default::default());
    let out_view = mtex.create_view(&Default::default());
    let ub = &pool[*pool_i];
    *pool_i += 1;
    let fu = MaskFinalizeUniforms {
        width: out_w,
        height: out_h,
        opacity: (mask.opacity / 100.0).clamp(0.0, 1.0),
        invert: mask.invert as u32,
        _pad: 0,
    };
    gpu.queue.write_buffer(ub, 0, bytemuck::bytes_of(&fu));
    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mask-finalize"),
        layout: &mask_finalize.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&acc_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&out_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: ub.as_entire_binding(),
            },
        ],
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("mask-finalize"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&mask_finalize.pipeline);
    pass.set_bind_group(0, &bind, &[]);
    pass.dispatch_workgroups(out_w.div_ceil(16), out_h.div_ceil(16), 1);
    last_passes_run.push("mask:composite".into());
    true
}

#[cfg(test)]
impl RenderGraph {
    /// Run only the DCP look pass on a provided RGBA buffer and read it back as
    /// RGB f32. Used by the GPU-vs-CPU equivalence test.
    pub fn run_dcp_look_test(
        &mut self,
        gpu: &GpuContext,
        dcp: &DcpProfile,
        cct: f32,
        rgba_in: &[f32],
        w: u32,
        h: u32,
    ) -> Vec<f32> {
        use crate::gpu::texture_io::{readback_rgba16f_from_texture, upload_rgba16f};
        self.ensure_dcp_tables(gpu, dcp);
        let meta = self.dcp_meta.clone().unwrap();
        let in_tex = make_chain_tex(gpu, w, h, "test-dcp-in");
        let out_tex = make_chain_tex(gpu, w, h, "test-dcp-out");
        upload_rgba16f(gpu, &in_tex, w, h, rgba_in);
        let u = DcpLookUniforms {
            width: w,
            height: h,
            has_map1: meta.has_map1,
            has_map2: meta.has_map2,
            has_look: meta.has_look,
            tone_size: meta.tone_size,
            cct_weight: cct_weight(cct, meta.t1, meta.t2),
            baseline_gain: meta.baseline_gain,
            m1: meta.m1,
            m2: meta.m2,
            lk: meta.lk,
        };
        gpu.queue
            .write_buffer(&self.dcp_look_uniforms, 0, bytemuck::bytes_of(&u));
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test-dcp-bind"),
            layout: &self.dcp_look.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &in_tex.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &out_tex.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.dcp_look_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.dcp_tables_buf.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.dcp_tone_buf.as_ref().unwrap().as_entire_binding(),
                },
            ],
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("test-dcp-look"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.dcp_look.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(16), h.div_ceil(16), 1);
        }
        gpu.queue.submit([encoder.finish()]);
        readback_rgba16f_from_texture(gpu, &out_tex, w, h).unwrap()
    }

    /// Dispatch just the 3D LUT node on `rgba_in` (linear Rec.2020) and read the
    /// result back. Test-only mirror of the in-chain lut node.
    pub fn run_lut_test(
        &mut self,
        gpu: &GpuContext,
        cube: &crate::lut::CubeLut,
        opacity: f32,
        rgba_in: &[f32],
        w: u32,
        h: u32,
    ) -> Vec<f32> {
        use crate::gpu::texture_io::{readback_rgba16f_from_texture, upload_rgba16f};
        let lut_idx = NODES.iter().position(|(n, _)| *n == "lut").unwrap();
        let configs = {
            let mut doc = EditDoc::new("/lut-test.ARW");
            doc.set("lut", "opacity", crate::doc::ParamValue::F32(opacity));
            // Working-linear interpretation so CPU/GPU share CubeLut::apply_working.
            doc.set("lut", "input_primaries", crate::doc::ParamValue::F32(0.0));
            doc.set("lut", "output_primaries", crate::doc::ParamValue::F32(0.0));
            doc.set("lut", "shaper", crate::doc::ParamValue::F32(0.0));
            doc.set("lut", "interpolation", crate::doc::ParamValue::F32(0.0));
            super::config::node_configs(&doc, 5200.0, w, h, Some(cube))
        };
        let NodeConfig::Run { uniforms, lut } = &configs[lut_idx] else {
            panic!("lut node did not activate");
        };
        gpu.queue
            .write_buffer(&self.node_uniforms[lut_idx], 0, uniforms);
        gpu.queue.write_buffer(
            &self.lut3d_buffer,
            0,
            bytemuck::cast_slice(lut.as_ref().unwrap()),
        );
        let in_tex = make_chain_tex(gpu, w, h, "test-lut-in");
        let out_tex = make_chain_tex(gpu, w, h, "test-lut-out");
        upload_rgba16f(gpu, &in_tex, w, h, rgba_in);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        dispatch_node(
            gpu,
            &mut encoder,
            &self.lut_pipe,
            true,
            &in_tex,
            &out_tex,
            &self.node_uniforms[lut_idx],
            &self.lut3d_buffer,
            "test-lut",
            w,
            h,
        );
        gpu.queue.submit([encoder.finish()]);
        readback_rgba16f_from_texture(gpu, &out_tex, w, h).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::DcpProfile;

    /// The crop path must be a pure re-window of the uncropped render:
    /// a rect-only crop equals the matching sub-region pixel-for-pixel, and
    /// rotate-90 transposes (with the correct output dims). Guards the
    /// content-space mapping in crop_common.wgsl.
    #[test]
    fn extract_crop_rect_matches_subregion() {
        use crate::doc::{EditDoc, ParamValue};
        use crate::gpu::texture_io::upload_rgba16f;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let gpu = match rt.block_on(GpuContext::init()) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("skip: no GPU: {e}");
                return;
            }
        };
        let (w, h) = (64u32, 48u32);
        // distinct value per texel so any mapping slip shows up
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[
                    x as f32 / w as f32,
                    y as f32 / h as f32,
                    (x + y) as f32 / (w + h) as f32,
                    1.0,
                ]);
            }
        }
        let tex = make_chain_tex(&gpu, w, h, "crop-test-src");
        upload_rgba16f(&gpu, &tex, w, h, &rgba);
        let tex_view = tex.create_view(&Default::default());
        let seg = HashMap::new();

        let render_with = |doc: &EditDoc, out_w: u32, out_h: u32| -> Vec<u8> {
            let mut graph = RenderGraph::new(&gpu);
            let view = ViewParams {
                out_w,
                out_h,
                scale: Some(1.0),
                center_x: 0.5,
                center_y: 0.5,
                crop_preview: false,
            };
            graph
                .render(
                    &gpu, &tex_view, w, h, &view, doc, 5200.0, &seg, None, None, None,
                )
                .unwrap()
        };

        let base_doc = EditDoc::new("/crop-test.ARW");
        let base = render_with(&base_doc, w, h);

        // rect-only crop, texel-aligned: left/top/right/bottom quarters
        let mut doc = EditDoc::new("/crop-test.ARW");
        doc.set("crop", "left", ParamValue::F32(0.25));
        doc.set("crop", "top", ParamValue::F32(0.25));
        doc.set("crop", "right", ParamValue::F32(0.75));
        doc.set("crop", "bottom", ParamValue::F32(0.75));
        let cropped = render_with(&doc, w / 2, h / 2);
        let (x0, y0) = (w / 4, h / 4);
        for y in 0..h / 2 {
            for x in 0..w / 2 {
                let c = ((y * (w / 2) + x) * 4) as usize;
                let b = (((y + y0) * w + (x + x0)) * 4) as usize;
                for ch in 0..3 {
                    let d = (cropped[c + ch] as i32 - base[b + ch] as i32).abs();
                    assert!(
                        d <= 1,
                        "crop({x},{y}) ch{ch}: {} vs base {} (Δ{d})",
                        cropped[c + ch],
                        base[b + ch]
                    );
                }
            }
        }

        // rotate-90 CW: output dims swap; out(x,y) = src(y, h-1-x)... verify
        // via the inverse map used by the shader: uv' = (v, 1-u).
        let mut doc = EditDoc::new("/crop-test.ARW");
        doc.set("crop", "rotate_90", ParamValue::F32(1.0));
        let rot = render_with(&doc, h, w); // content dims swap
        for y in (0..w).step_by(7) {
            for x in (0..h).step_by(7) {
                // out px (x,y) in 48×64 → uv (u,v) → src uv (v, 1-u)
                let sx = ((y as f32 + 0.5) / w as f32 * w as f32 - 0.5).round() as i64;
                let sy = ((1.0 - (x as f32 + 0.5) / h as f32) * h as f32 - 0.5).round() as i64;
                let sx = sx.clamp(0, (w - 1) as i64) as u32;
                let sy = sy.clamp(0, (h - 1) as i64) as u32;
                let r = ((y * h + x) * 4) as usize;
                let b = ((sy * w + sx) * 4) as usize;
                for ch in 0..3 {
                    let d = (rot[r + ch] as i32 - base[b + ch] as i32).abs();
                    assert!(
                        d <= 1,
                        "rot90({x},{y}) ch{ch}: {} vs src({sx},{sy}) {} (Δ{d})",
                        rot[r + ch],
                        base[b + ch]
                    );
                }
            }
        }
    }

    /// The GPU look pass must reproduce the CPU `apply_look` to within f16 +
    /// LUT-interp tolerance, across a spread of colors including HDR (>1).
    #[test]
    fn gpu_dcp_look_matches_cpu() {
        // find any profile that actually exercises the HSV tables
        let dir = crate::profile::profiles_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skip: no profiles dir");
            return;
        };
        let mut chosen = None;
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("dcp") {
                continue;
            }
            if let Ok(dcp) = DcpProfile::load(&p) {
                let d = dcp.look_data();
                if d.map1.is_some() || d.look.is_some() {
                    chosen = Some(dcp);
                    break;
                }
            }
        }
        let Some(dcp) = chosen else {
            eprintln!("skip: no profile with HSV maps");
            return;
        };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let gpu = match rt.block_on(GpuContext::init()) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("skip: no GPU: {e}");
                return;
            }
        };
        let mut graph = RenderGraph::new(&gpu);

        let cct = 5200.0;
        let samples = [0.02f32, 0.1, 0.18, 0.45, 0.8, 1.25];
        let mut rgba = Vec::new();
        let mut cpu: Vec<[f32; 3]> = Vec::new();
        for &r in &samples {
            for &g in &samples {
                for &b in &samples {
                    rgba.extend_from_slice(&[r, g, b, 1.0]);
                    cpu.push(dcp.apply_look([r, g, b], cct));
                }
            }
        }
        let w = cpu.len() as u32;
        let out = graph.run_dcp_look_test(&gpu, &dcp, cct, &rgba, w, 1);

        let mut max_err = 0f32;
        for (i, exp) in cpu.iter().enumerate() {
            for c in 0..3 {
                max_err = max_err.max((out[i * 3 + c] - exp[c]).abs());
            }
        }
        assert!(
            max_err < 0.01,
            "GPU vs CPU dcp_look max abs err = {max_err}"
        );
    }

    /// The 3D-LUT shader must match CubeLut::apply_working (tetrahedral,
    /// tagged spaces) within f16 + interpolation tolerance.
    #[test]
    fn gpu_lut_matches_cpu_reference() {
        // A "swap R and B" 3D LUT — strong, unambiguous, easy to reason about.
        let size = 5usize;
        let n = (size - 1) as f32;
        let mut s = format!("LUT_3D_SIZE {size}\n");
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    // output = (b, g, r): channel swap in the encoded domain
                    s.push_str(&format!(
                        "{} {} {}\n",
                        b as f32 / n,
                        g as f32 / n,
                        r as f32 / n
                    ));
                }
            }
        }
        let cube = crate::lut::CubeLut::parse_cube(&s).unwrap();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let gpu = match rt.block_on(GpuContext::init()) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("skip: no GPU: {e}");
                return;
            }
        };
        let mut graph = RenderGraph::new(&gpu);

        let opacity = 100.0f32;
        let samples = [0.05f32, 0.2, 0.5, 0.85];
        let mut rgba = Vec::new();
        let mut cpu: Vec<[f32; 3]> = Vec::new();
        let params = crate::lut::LutParams::working_linear();
        for &r in &samples {
            for &g in &samples {
                for &b in &samples {
                    rgba.extend_from_slice(&[r, g, b, 1.0]);
                    cpu.push(cube.apply_working([r, g, b], &params));
                }
            }
        }
        let w = cpu.len() as u32;
        let out = graph.run_lut_test(&gpu, &cube, opacity, &rgba, w, 1);

        let mut max_err = 0f32;
        let mut moved = 0f32;
        for (i, exp) in cpu.iter().enumerate() {
            for c in 0..3 {
                max_err = max_err.max((out[i * 3 + c] - exp[c]).abs());
                moved = moved.max((out[i * 3 + c] - rgba[i * 4 + c]).abs());
            }
        }
        assert!(max_err < 0.01, "GPU vs CPU lut max abs err = {max_err}");
        assert!(moved > 0.1, "channel-swap LUT should visibly move pixels");
    }
}