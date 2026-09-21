//! Per-node render configuration computed from the EditDoc each render.
//! Every node must be EXACTLY identity at registry defaults (the graph
//! skips it — cache-the-chain depends on this; test enforced).

use crate::color::{self, Mat3};
use crate::curve::{self, ToneParams};
use crate::doc::{EditDoc, ParamValue};
use crate::registry::effective_f32 as eff;

/// (node name, doc module) in fixed pipeline order (spec 3.2).
pub const NODES: &[(&str, &str)] = &[
    ("exposure", "exposure"),
    ("highlights", "highlights"),
    ("white_balance", "white_balance"),
    ("calibration", "calibration"),
    ("noise", "detail"),
    ("color_grade", "color_grade"),
    ("hsl", "hsl"),
    ("tone_curve", "tone_curve"),
    ("lut", "lut"),
    ("sharpen", "detail"),
    ("effects", "effects"),
];

pub enum NodeConfig {
    Skip,
    Run {
        /// bytemuck-cast uniform bytes for the node's shader
        uniforms: Vec<u8>,
        /// tone-curve LUT data when this node carries one
        lut: Option<Vec<f32>>,
    },
}

fn run<T: bytemuck::Pod>(u: T) -> NodeConfig {
    NodeConfig::Run {
        uniforms: bytemuck::bytes_of(&u).to_vec(),
        lut: None,
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MatrixU {
    m0: [f32; 4],
    m1: [f32; 4],
    m2: [f32; 4],
    gain: f32,
    width: u32,
    height: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HighlightsU {
    amount: f32,
    clip: f32,
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CalibU {
    m0: [f32; 4],
    m1: [f32; 4],
    m2: [f32; 4],
    shadow_tint: [f32; 4],
    width: u32,
    height: u32,
    _p0: u32,
    _p1: u32,
}

/// Classical denoise uniforms — must match `NoiseUniforms` in noise.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct NoiseU {
    /// Profile (a, b) already strength-scaled.
    profile_a: f32,
    profile_b: f32,
    /// Per-level Wiener s multipliers (luma / chroma), packed.
    luma_s0: f32,
    luma_s1: f32,
    luma_s2: f32,
    luma_s3: f32,
    luma_s4: f32,
    chroma_s0: f32,
    chroma_s1: f32,
    chroma_s2: f32,
    chroma_s3: f32,
    chroma_s4: f32,
    chroma_s5: f32,
    detail: f32,
    impulse_k: f32,
    texture_k: f32,
    sigma_scale: f32,
    use_nlm: f32,
    nlm_patch: f32,
    nlm_search: f32,
    nlm_h: f32,
    nlm_center: f32,
    width: u32,
    height: u32,
    _p0: u32,
    _p1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GradeU {
    k0: [f32; 4],
    k1: [f32; 4],
    k2: [f32; 4],
    ki0: [f32; 4],
    ki1: [f32; 4],
    ki2: [f32; 4],
    m0: [f32; 4],
    m1: [f32; 4],
    m2r: [f32; 4],
    mi0: [f32; 4],
    mi1: [f32; 4],
    mi2: [f32; 4],
    shadows: [f32; 4],
    midtones: [f32; 4],
    highlights: [f32; 4],
    ranges: [f32; 4],
    width: u32,
    height: u32,
    model: u32,
    _p1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HslU {
    k0: [f32; 4],
    k1: [f32; 4],
    k2: [f32; 4],
    ki0: [f32; 4],
    ki1: [f32; 4],
    ki2: [f32; 4],
    m0: [f32; 4],
    m1: [f32; 4],
    m2r: [f32; 4],
    mi0: [f32; 4],
    mi1: [f32; 4],
    mi2: [f32; 4],
    bands: [[f32; 4]; 8],
    width: u32,
    height: u32,
    _p0: u32,
    _p1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CurveU {
    width: u32,
    height: u32,
    lut_size: u32,
    /// bit0=luma (RGB+parametric), bit1=R, bit2=G, bit3=B
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SharpenU {
    amount: f32,
    sigma: f32,
    threshold: f32,
    _pad: f32,
    width: u32,
    height: u32,
    _p0: u32,
    _p1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EffectsU {
    clarity: f32,
    grain_amount: f32,
    grain_size: f32,
    vignette_amount: f32,
    vignette_midpoint: f32,
    dehaze: f32,
    _pad0: f32,
    _pad1: f32,
    width: u32,
    height: u32,
    frame_index: u32,
    _pad2: u32,
}

fn rows(m: &Mat3) -> ([f32; 4], [f32; 4], [f32; 4]) {
    (
        [m[0][0], m[0][1], m[0][2], 0.0],
        [m[1][0], m[1][1], m[1][2], 0.0],
        [m[2][0], m[2][1], m[2][2], 0.0],
    )
}

/// 3D-LUT node uniform — field order/layout matches `struct LutU` in lut.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LutU {
    size: u32,
    width: u32,
    height: u32,
    shaper: u32,
    dmin: [f32; 4],
    dmax: [f32; 4],
    opacity: f32,
    interp: u32,
    kind: u32,
    _p0: u32,
    m_in0: [f32; 4],
    m_in1: [f32; 4],
    m_in2: [f32; 4],
    m_out0: [f32; 4],
    m_out1: [f32; 4],
    m_out2: [f32; 4],
}

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn module_enabled(doc: &EditDoc, module: &str) -> bool {
    doc.get(module, "enabled")
        .and_then(|v| v.as_f32())
        .map(|v| v >= 0.5)
        .unwrap_or(true)
}

/// Rotate `v` around the gray axis (1,1,1)/√3 by `deg` (Rodrigues).
fn rotate_gray(v: [f32; 3], deg: f32) -> [f32; 3] {
    let k = 1.0 / 3f32.sqrt();
    let (kx, ky, kz) = (k, k, k);
    let th = deg.to_radians();
    let (s, c) = th.sin_cos();
    let dot = kx * v[0] + ky * v[1] + kz * v[2];
    let cross = [
        ky * v[2] - kz * v[1],
        kz * v[0] - kx * v[2],
        kx * v[1] - ky * v[0],
    ];
    [
        v[0] * c + cross[0] * s + kx * dot * (1.0 - c),
        v[1] * c + cross[1] * s + ky * dot * (1.0 - c),
        v[2] * c + cross[2] * s + kz * dot * (1.0 - c),
    ]
}

/// Calibration matrix: per-primary hue rotation (±100 → ±30°) + sat scale
/// (±100 → ±50%), rows renormalized so white stays white.
pub fn calibration_matrix(
    red_hue: f32,
    red_sat: f32,
    green_hue: f32,
    green_sat: f32,
    blue_hue: f32,
    blue_sat: f32,
) -> Mat3 {
    let prims = [
        ([1.0, 0.0, 0.0], red_hue, red_sat),
        ([0.0, 1.0, 0.0], green_hue, green_sat),
        ([0.0, 0.0, 1.0], blue_hue, blue_sat),
    ];
    let mut cols = [[0.0f32; 3]; 3];
    for (i, (e, hue, sat)) in prims.iter().enumerate() {
        let rotated = rotate_gray(*e, hue * 0.30);
        let gray = (rotated[0] + rotated[1] + rotated[2]) / 3.0;
        let s = 1.0 + sat * 0.005;
        for r in 0..3 {
            cols[i][r] = gray + (rotated[r] - gray) * s;
        }
    }
    // M[r][c] = cols[c][r]
    let mut m = [[0.0f32; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            m[r][c] = cols[c][r];
        }
        let sum: f32 = m[r].iter().sum();
        if sum.abs() > 1e-6 {
            for v in m[r].iter_mut() {
                *v /= sum;
            }
        }
    }
    m
}

fn oklab_rows() -> (
    [f32; 4],
    [f32; 4],
    [f32; 4],
    [f32; 4],
    [f32; 4],
    [f32; 4],
    [f32; 4],
    [f32; 4],
    [f32; 4],
    [f32; 4],
    [f32; 4],
    [f32; 4],
) {
    let m = color::oklab_mats();
    let (k0, k1, k2) = rows(&m.rec2020_to_lms);
    let (ki0, ki1, ki2) = rows(&m.lms_to_rec2020);
    let (m0, m1, m2r) = rows(&m.m2);
    let (mi0, mi1, mi2) = rows(&m.m2_inv);
    (k0, k1, k2, ki0, ki1, ki2, m0, m1, m2r, mi0, mi1, mi2)
}

pub fn node_configs(
    doc: &EditDoc,
    as_shot_cct: f32,
    w: u32,
    h: u32,
    lut: Option<&crate::lut::CubeLut>,
) -> Vec<NodeConfig> {
    let mut out = Vec::with_capacity(NODES.len());

    // exposure
    {
        let stops = eff(doc, "exposure", "stops");
        if !module_enabled(doc, "exposure") || stops == 0.0 {
            out.push(NodeConfig::Skip);
        } else {
            let (m0, m1, m2) = rows(&IDENTITY);
            out.push(run(MatrixU {
                m0,
                m1,
                m2,
                gain: 2f32.powf(stops),
                width: w,
                height: h,
                _pad: 0,
            }));
        }
    }

    // highlights (clipped-channel reconstruct)
    {
        let amount = eff(doc, "highlights", "amount") / 100.0;
        if !module_enabled(doc, "highlights") || amount <= 1e-4 {
            out.push(NodeConfig::Skip);
        } else {
            out.push(run(HighlightsU {
                amount,
                clip: eff(doc, "highlights", "clip").max(0.5),
                width: w,
                height: h,
            }));
        }
    }

    // white_balance (absent = as-shot = identity)
    {
        let temp = doc
            .get("white_balance", "temp")
            .and_then(|v| v.as_f32())
            .unwrap_or(as_shot_cct);
        let tint = doc
            .get("white_balance", "tint")
            .and_then(|v| v.as_f32())
            .unwrap_or(0.0);
        let m = color::wb_matrix_rec2020(temp, tint, as_shot_cct);
        if !module_enabled(doc, "white_balance") || color::mat_is_identity(&m, 1e-4) {
            out.push(NodeConfig::Skip);
        } else {
            let (m0, m1, m2) = rows(&m);
            out.push(run(MatrixU {
                m0,
                m1,
                m2,
                gain: 1.0,
                width: w,
                height: h,
                _pad: 0,
            }));
        }
    }

    // calibration
    {
        let p = |n: &str| eff(doc, "calibration", n);
        let (rh, rs, gh, gs, bh, bs, st) = (
            p("red_hue"),
            p("red_sat"),
            p("green_hue"),
            p("green_sat"),
            p("blue_hue"),
            p("blue_sat"),
            p("shadow_tint"),
        );
        if !module_enabled(doc, "calibration")
            || [rh, rs, gh, gs, bh, bs, st].iter().all(|v| *v == 0.0)
        {
            out.push(NodeConfig::Skip);
        } else {
            let m = calibration_matrix(rh, rs, gh, gs, bh, bs);
            let (m0, m1, m2) = rows(&m);
            // green↔magenta axis: + = magenta
            let t = st * 0.002;
            out.push(run(CalibU {
                m0,
                m1,
                m2,
                shadow_tint: [t * 0.5, -t, t * 0.5, 0.0],
                width: w,
                height: h,
                _p0: 0,
                _p1: 0,
            }));
        }
    }

    // noise (detail slot 4) — classical VST + à-trous / NLM chain
    {
        // ISO is stashed on open into doc.unknown["denoise_iso"] (engine/decode).
        let iso_hint = doc
            .unknown
            .get("denoise_iso")
            .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
            .unwrap_or(800) as u32;
        let profile = crate::denoise::NoiseProfile::from_iso(iso_hint);
        let settings = crate::denoise::DenoiseSettings::from_doc(doc, &profile);
        if !module_enabled(doc, "detail") || !settings.classical_active() {
            out.push(NodeConfig::Skip);
        } else {
            // Preview is already downsampled by extract; treat σ as full-res
            // (sigma_scale=1). Viewport-scale correction is a future polish item.
            let p = settings.to_chain_params(&profile, 1.0);
            out.push(run(NoiseU {
                profile_a: p.profile.a,
                profile_b: p.profile.b,
                luma_s0: p.luma_s[0],
                luma_s1: p.luma_s[1],
                luma_s2: p.luma_s[2],
                luma_s3: p.luma_s[3],
                luma_s4: p.luma_s[4],
                chroma_s0: p.chroma_s[0],
                chroma_s1: p.chroma_s[1],
                chroma_s2: p.chroma_s[2],
                chroma_s3: p.chroma_s[3],
                chroma_s4: p.chroma_s[4],
                chroma_s5: p.chroma_s[5],
                detail: p.detail,
                impulse_k: p.impulse_k,
                texture_k: p.texture_k,
                sigma_scale: p.sigma_scale,
                use_nlm: if p.use_nlm { 1.0 } else { 0.0 },
                nlm_patch: p.nlm_patch as f32,
                nlm_search: p.nlm_search as f32,
                nlm_h: p.nlm_h,
                nlm_center: p.nlm_center,
                width: w,
                height: h,
                _p0: 0,
                _p1: 0,
            }));
        }
    }

    // color_grade
    {
        let p = |n: &str| eff(doc, "color_grade", n);
        let zones = [
            (p("shadows_hue"), p("shadows_sat"), p("shadows_lum")),
            (p("midtones_hue"), p("midtones_sat"), p("midtones_lum")),
            (
                p("highlights_hue"),
                p("highlights_sat"),
                p("highlights_lum"),
            ),
        ];
        let gc = p("global_chroma");
        let ps = p("perceptual_sat");
        let active = zones.iter().any(|(_, s, l)| *s != 0.0 || *l != 0.0) || gc != 0.0 || ps != 0.0;
        if !module_enabled(doc, "color_grade") || !active {
            out.push(NodeConfig::Skip);
        } else {
            let (k0, k1, k2, ki0, ki1, ki2, m0, m1, m2r, mi0, mi1, mi2) = oklab_rows();
            let z = |i: usize| [zones[i].0.to_radians(), zones[i].1, zones[i].2, 0.0];
            out.push(run(GradeU {
                k0,
                k1,
                k2,
                ki0,
                ki1,
                ki2,
                m0,
                m1,
                m2r,
                mi0,
                mi1,
                mi2,
                shadows: z(0),
                midtones: z(1),
                highlights: z(2),
                ranges: [p("shadow_range"), p("highlight_range"), gc, ps],
                width: w,
                height: h,
                model: p("model").round().clamp(0.0, 3.0) as u32,
                _p1: 0,
            }));
        }
    }

    // hsl
    {
        const BANDS: [&str; 8] = [
            "red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta",
        ];
        let mut bands = [[0.0f32; 4]; 8];
        let mut active = false;
        for (i, b) in BANDS.iter().enumerate() {
            let hue = eff_dyn(doc, "hsl", &format!("{b}.hue"));
            let sat = eff_dyn(doc, "hsl", &format!("{b}.sat"));
            let lum = eff_dyn(doc, "hsl", &format!("{b}.lum"));
            bands[i] = [hue, sat, lum, 0.0];
            active |= hue != 0.0 || sat != 0.0 || lum != 0.0;
        }
        if !module_enabled(doc, "hsl") || !active {
            out.push(NodeConfig::Skip);
        } else {
            let (k0, k1, k2, ki0, ki1, ki2, m0, m1, m2r, mi0, mi1, mi2) = oklab_rows();
            out.push(run(HslU {
                k0,
                k1,
                k2,
                ki0,
                ki1,
                ki2,
                m0,
                m1,
                m2r,
                mi0,
                mi1,
                mi2,
                bands,
                width: w,
                height: h,
                _p0: 0,
                _p1: 0,
            }));
        }
    }

    // tone_curve — RGB luma + optional per-channel R/G/B point curves
    {
        let rgb: Vec<[f32; 2]> = match doc.get("tone_curve", "points") {
            Some(ParamValue::Curve(p)) => p.clone(),
            _ => vec![],
        };
        let r_pts: Vec<[f32; 2]> = match doc.get("tone_curve", "points_r") {
            Some(ParamValue::Curve(p)) => p.clone(),
            _ => vec![],
        };
        let g_pts: Vec<[f32; 2]> = match doc.get("tone_curve", "points_g") {
            Some(ParamValue::Curve(p)) => p.clone(),
            _ => vec![],
        };
        let b_pts: Vec<[f32; 2]> = match doc.get("tone_curve", "points_b") {
            Some(ParamValue::Curve(p)) => p.clone(),
            _ => vec![],
        };
        let tp = ToneParams {
            contrast: eff(doc, "tone_curve", "contrast"),
            shadows: eff(doc, "tone_curve", "shadows"),
            darks: eff(doc, "tone_curve", "darks"),
            lights: eff(doc, "tone_curve", "lights"),
            highlights: eff(doc, "tone_curve", "highlights"),
        };
        let sigmoid = eff(doc, "tone_curve", "sigmoid");
        if !module_enabled(doc, "tone_curve") {
            out.push(NodeConfig::Skip);
        } else if curve::should_run(&rgb, &r_pts, &g_pts, &b_pts, &tp) || sigmoid > 0.0 {
            let mut luma = curve::build_lut(&rgb, &tp);
            curve::apply_sigmoid(&mut luma, sigmoid);
            let r_lut = if r_pts.is_empty() {
                curve::identity_lut()
            } else {
                curve::build_lut(&r_pts, &ToneParams::default())
            };
            let g_lut = if g_pts.is_empty() {
                curve::identity_lut()
            } else {
                curve::build_lut(&g_pts, &ToneParams::default())
            };
            let b_lut = if b_pts.is_empty() {
                curve::identity_lut()
            } else {
                curve::build_lut(&b_pts, &ToneParams::default())
            };
            let mut packed = Vec::with_capacity(curve::LUT_SIZE * 4);
            packed.extend(luma);
            packed.extend(r_lut);
            packed.extend(g_lut);
            packed.extend(b_lut);
            let mut flags = 0u32;
            if !curve::is_identity(&rgb, &tp) || sigmoid > 0.0 {
                flags |= 1;
            }
            if !r_pts.is_empty() {
                flags |= 2;
            }
            if !g_pts.is_empty() {
                flags |= 4;
            }
            if !b_pts.is_empty() {
                flags |= 8;
            }
            out.push(NodeConfig::Run {
                uniforms: bytemuck::bytes_of(&CurveU {
                    width: w,
                    height: h,
                    lut_size: curve::LUT_SIZE as u32,
                    flags,
                })
                .to_vec(),
                lut: Some(packed),
            });
        } else {
            out.push(NodeConfig::Skip);
        }
    }

    // lut (3D look LUT) — active only when a .cube is loaded + opacity > 0
    // + lut.enabled. Cube lives in the engine cache; doc holds path + params.
    {
        let p = crate::lut::params_from_doc(doc, eff(doc, "lut", "opacity"));
        match lut {
            Some(cube) if p.opacity > 0.0 => {
                let (m_in, m_out) = crate::lut::CubeLut::shader_mats(&p);
                let (in0, in1, in2) = rows(&m_in);
                let (out0, out1, out2) = rows(&m_out);
                let u = LutU {
                    size: cube.size as u32,
                    width: w,
                    height: h,
                    shaper: p.shaper as u32,
                    dmin: [
                        cube.domain_min[0],
                        cube.domain_min[1],
                        cube.domain_min[2],
                        0.0,
                    ],
                    dmax: [
                        cube.domain_max[0],
                        cube.domain_max[1],
                        cube.domain_max[2],
                        0.0,
                    ],
                    opacity: p.opacity,
                    interp: p.interp as u32,
                    kind: p.kind as u32,
                    _p0: 0,
                    m_in0: in0,
                    m_in1: in1,
                    m_in2: in2,
                    m_out0: out0,
                    m_out1: out1,
                    m_out2: out2,
                };
                out.push(NodeConfig::Run {
                    uniforms: bytemuck::bytes_of(&u).to_vec(),
                    lut: Some(cube.flatten()),
                });
            }
            _ => out.push(NodeConfig::Skip),
        }
    }

    // sharpen (detail slot 8)
    {
        let amount = eff(doc, "detail", "sharpen_amount");
        if !module_enabled(doc, "detail") || amount == 0.0 {
            out.push(NodeConfig::Skip);
        } else {
            let radius = eff(doc, "detail", "sharpen_radius");
            let detail = eff(doc, "detail", "sharpen_detail");
            out.push(run(SharpenU {
                amount: amount * 0.02,
                sigma: radius,
                threshold: (1.0 - detail / 100.0) * 0.08 + 0.002,
                _pad: 0.0,
                width: w,
                height: h,
                _p0: 0,
                _p1: 0,
            }));
        }
    }

    // effects (dehaze / grain / vignette / clarity) — identity when all amounts are 0
    {
        let clarity = eff(doc, "effects", "clarity");
        let frame = doc
            .unknown
            .get("video_frame")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let grain = eff(doc, "effects", "grain_amount");
        let vignette = eff(doc, "effects", "vignette_amount");
        let dehaze = eff(doc, "effects", "dehaze");
        if !module_enabled(doc, "effects")
            || (clarity == 0.0 && grain == 0.0 && vignette == 0.0 && dehaze == 0.0)
        {
            out.push(NodeConfig::Skip);
        } else {
            out.push(run(EffectsU {
                clarity: clarity / 100.0,
                grain_amount: grain / 100.0,
                grain_size: eff(doc, "effects", "grain_size"),
                vignette_amount: vignette / 100.0,
                vignette_midpoint: eff(doc, "effects", "vignette_midpoint") / 100.0,
                dehaze: dehaze / 100.0,
                _pad0: 0.0,
                _pad1: 0.0,
                width: w,
                height: h,
                frame_index: frame,
                _pad2: 0,
            }));
        }
    }

    out
}

/// Node configs for a MASK's scoped module stack (spec 4.5: identical
/// passes, scoped params — one code path, global and local).
pub fn mask_node_configs(
    mask: &crate::doc::Mask,
    as_shot_cct: f32,
    w: u32,
    h: u32,
) -> Vec<NodeConfig> {
    let mut pseudo = EditDoc::new("mask://scoped");
    pseudo.modules = mask.modules.clone();
    // Masks don't carry a scoped 3D LUT (v1) — the global LUT applies once.
    node_configs(&pseudo, as_shot_cct, w, h, None)
}

/// effective_f32 for dynamic (leaked-str registry) param names.
fn eff_dyn(doc: &EditDoc, module: &str, param: &str) -> f32 {
    doc.get(module, param)
        .and_then(|v| v.as_f32())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::mat_vec;

    #[test]
    fn default_doc_skips_every_node() {
        let doc = EditDoc::new("/x.ARW");
        let configs = node_configs(&doc, 5200.0, 100, 100, None);
        assert!(
            configs.iter().all(|c| matches!(c, NodeConfig::Skip)),
            "all nodes must be identity at defaults"
        );
    }

    #[test]
    fn calibration_matrix_identity_at_zero() {
        let m = calibration_matrix(0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert!(color::mat_is_identity(&m, 1e-5), "{m:?}");
    }

    #[test]
    fn calibration_matrix_preserves_white() {
        let m = calibration_matrix(50.0, 30.0, -20.0, 10.0, 40.0, -25.0);
        let w = mat_vec(&m, [1.0, 1.0, 1.0]);
        for c in w {
            assert!((c - 1.0).abs() < 1e-4, "white shifted: {w:?}");
        }
    }

    #[test]
    fn calibration_red_hue_rotates_red_not_white() {
        let m = calibration_matrix(100.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        let r = mat_vec(&m, [1.0, 0.0, 0.0]);
        // red moved (some energy into another channel)
        assert!((r[1].abs() + r[2].abs()) > 0.05, "red unchanged: {r:?}");
        // row renorm spreads a little cross-effect; green must stay dominant
        let g = mat_vec(&m, [0.0, 1.0, 0.0]);
        assert!(
            g[1] > 0.5 && g[1] > g[0] && g[1] > g[2],
            "green broke: {g:?}"
        );
    }

    #[test]
    fn each_node_activates_from_its_params() {
        let mut doc = EditDoc::new("/x.ARW");
        doc.set("detail", "sharpen_amount", ParamValue::F32(50.0));
        let configs = node_configs(&doc, 5200.0, 10, 10, None);
        // sharpen node (idx 9 after highlights+lut); noise (idx 4) skipped
        assert!(matches!(configs[9], NodeConfig::Run { .. }));
        assert!(matches!(configs[4], NodeConfig::Skip));
        // lut node (idx 8) is skipped when no cube is loaded
        assert!(matches!(configs[8], NodeConfig::Skip));
    }

    #[test]
    fn disabled_module_skips_even_when_params_are_set() {
        let mut doc = EditDoc::new("/x.ARW");
        doc.set("exposure", "stops", ParamValue::F32(1.0));
        doc.set("exposure", "enabled", ParamValue::F32(0.0));
        let configs = node_configs(&doc, 5200.0, 10, 10, None);
        assert!(matches!(configs[0], NodeConfig::Skip));
    }

    #[test]
    fn dehaze_activates_effects_node() {
        let mut doc = EditDoc::new("/x.ARW");
        doc.set("effects", "dehaze", ParamValue::F32(40.0));
        let configs = node_configs(&doc, 5200.0, 10, 10, None);
        assert!(matches!(configs[10], NodeConfig::Run { .. }));
        assert_eq!(std::mem::size_of::<EffectsU>() % 16, 0);
    }

    #[test]
    fn tone_curve_enabled_no_points_skips() {
        let mut doc = EditDoc::new("/x.ARW");
        doc.set("tone_curve", "enabled", ParamValue::F32(1.0));
        let configs = node_configs(&doc, 5200.0, 10, 10, None);
        // tone_curve is at index 7 in NODES
        assert!(matches!(configs[7], NodeConfig::Skip), "tone_curve should skip when enabled but no points");
    }

    #[test]
    fn tone_curve_identity_points_skips() {
        let mut doc = EditDoc::new("/x.ARW");
        doc.set("tone_curve", "enabled", ParamValue::F32(1.0));
        // Identity points (what UI might save)
        doc.set("tone_curve", "points", ParamValue::Curve(vec![[0.0, 0.0], [1.0, 1.0]]));
        let configs = node_configs(&doc, 5200.0, 10, 10, None);
        // tone_curve is at index 7 in NODES
        assert!(matches!(configs[7], NodeConfig::Skip), "tone_curve should skip with identity points");
    }
}
