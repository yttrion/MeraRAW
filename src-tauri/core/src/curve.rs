//! Tone-curve LUT builder (slot 7). The GPU pass applies the LUT with the
//! film-like constant-hue method (RawTherapee / Adobe DNG reference —
//! curve the max & min channel, interpolate the middle by ratio).
//!
//! Domain: the LUT operates on shutter-compressed t = x/(1+x) ∈ [0,1) so
//! scene-referred values >1 stay curve-addressable; the shader maps back
//! with y/(1-y). Identity LUT ⇒ exact identity end-to-end.
//!
//! Composition order (pinned): base point-curve B → parametric region
//! deltas → contrast S-curve. All steps identity at defaults.

pub const LUT_SIZE: usize = 512;

/// Monotonic cubic interpolation (Fritsch–Carlson) through control points.
/// Points must have strictly increasing x in [0,1] (guard-wall enforces).
/// Unlike before, we no longer force endpoints at x=0 and x=1 —
/// users can position them freely for clipping (Lightroom-style).
#[derive(Clone, Debug)]
struct MonotonicCubic {
    xs: Vec<f32>,
    ys: Vec<f32>,
    ms: Vec<f32>, // tangents
    first_x: f32,  // user's first point x (black clip threshold)
    last_x: f32,   // user's last point x (white clip threshold)
}

impl MonotonicCubic {
    fn new(mut pts: Vec<[f32; 2]>) -> Self {
        // Sort by x to be safe (UI should maintain order, but defend anyway)
        pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());

        let n = pts.len();
        // Need at least 2 points
        if n < 2 {
            return Self {
                xs: vec![0.0, 1.0],
                ys: vec![0.0, 1.0],
                ms: vec![1.0, 1.0],
                first_x: 0.0,
                last_x: 1.0,
            };
        }

        let first_x = pts[0][0].clamp(0.0, 1.0);
        let last_x = pts[n - 1][0].clamp(0.0, 1.0);

        let xs: Vec<f32> = pts.iter().map(|p| p[0]).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p[1]).collect();

        // secant slopes
        let mut d = vec![0.0f32; n - 1];
        for i in 0..n - 1 {
            d[i] = (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]).max(1e-6);
        }
        let mut ms = vec![0.0f32; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            ms[i] = if d[i - 1] * d[i] <= 0.0 {
                0.0
            } else {
                (d[i - 1] + d[i]) * 0.5
            };
        }
        // Fritsch–Carlson limiter
        for i in 0..n - 1 {
            if d[i].abs() < 1e-9 {
                ms[i] = 0.0;
                ms[i + 1] = 0.0;
            } else {
                let a = ms[i] / d[i];
                let b = ms[i + 1] / d[i];
                let s = a * a + b * b;
                if s > 9.0 {
                    let tau = 3.0 / s.sqrt();
                    ms[i] = tau * a * d[i];
                    ms[i + 1] = tau * b * d[i];
                }
            }
        }
        Self { xs, ys, ms, first_x, last_x }
    }

    /// Evaluate spline at x ∈ [0,1].
    /// For x ≤ first_x: returns first point's y (extended flat for clipping)
    /// For x ≥ last_x: returns last point's y (extended flat for clipping)
    fn eval(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1];
        }
        let mut i = 0;
        while i < n - 2 && x > self.xs[i + 1] {
            i += 1;
        }
        let h = (self.xs[i + 1] - self.xs[i]).max(1e-6);
        let t = (x - self.xs[i]) / h;
        let t2 = t * t;
        let t3 = t2 * t;
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        h00 * self.ys[i] + h10 * h * self.ms[i] + h01 * self.ys[i + 1] + h11 * h * self.ms[i + 1]
    }

    /// Get the user's first point x (black clip threshold)
    fn first_x(&self) -> f32 {
        self.first_x
    }

    /// Get the user's last point x (white clip threshold)
    fn last_x(&self) -> f32 {
        self.last_x
    }
}

/// Raised-cosine region weight centered at `c` with half-width `w`.
fn region_w(t: f32, c: f32, w: f32) -> f32 {
    let d = (t - c).abs();
    if d >= w {
        0.0
    } else {
        0.5 * (1.0 + (std::f32::consts::PI * d / w).cos())
    }
}

/// Convert the tone shader's compressed LUT coordinate back to a normalized
/// scene-light coordinate for the parametric tonal regions. The shader looks
/// up `t = x / (1 + x)`, so using `t` directly would put Highlights around
/// scene value 7.0 instead of the visible 0.875 region.
fn scene_region_t(t: f32) -> f32 {
    if t >= 0.5 {
        1.0
    } else {
        (t / (1.0 - t)).clamp(0.0, 1.0)
    }
}

/// Contrast S-curve: blend toward a cosine ease (k>0) or its inverse (k<0).
fn contrast_curve(v: f32, contrast: f32) -> f32 {
    let k = (contrast / 100.0).clamp(-1.0, 1.0);
    if k > 0.0 {
        let ease = 0.5 - 0.5 * (std::f32::consts::PI * v).cos();
        v + (ease - v) * k
    } else if k < 0.0 {
        let inv = (1.0 - 2.0 * v.clamp(0.0, 1.0)).acos() / std::f32::consts::PI;
        v + (inv - v) * (-k)
    } else {
        v
    }
}

/// DCP ProfileToneCurve — cubic spline through linear (input, output) pairs.
#[derive(Debug, Clone)]
pub struct ProfileToneCurve {
    spline: MonotonicCubic,
    pub embedded: bool,
}

impl ProfileToneCurve {
    pub fn from_points(pts: Vec<[f32; 2]>) -> Self {
        Self {
            spline: MonotonicCubic::new(pts),
            embedded: true,
        }
    }

    /// Adobe Camera Raw default curve (used when a profile omits ProfileToneCurve).
    pub fn adobe_default() -> Self {
        Self {
            spline: MonotonicCubic::new(vec![
                [0.0, 0.0],
                [0.014539, 0.015553],
                [0.051668, 0.105943],
                [0.117937, 0.314067],
                [0.217703, 0.564130],
                [0.354594, 0.763154],
                [0.531769, 0.884435],
                [0.752050, 0.947136],
                [1.0, 1.0],
            ]),
            embedded: false,
        }
    }

    pub fn eval(&self, x: f32) -> f32 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            // Preserve scene headroom above 1.0 (linear extension from white point).
            return self.spline.eval(1.0) + (x - 1.0);
        }
        self.spline.eval(x).max(0.0)
    }

    /// Apply the profile tone curve the way Adobe Camera Raw does in the
    /// midtones (per-channel — that is what gives Camera look its saturation),
    /// then soft-blend toward a luminance-preserving remap as channels approach
    /// clip. Pure per-channel on speculars is the Fujifilm magenta/green failure
    /// mode; pure luminance remap looks muted next to Lightroom / Affinity.
    pub fn apply_rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        let r = rgb[0].max(0.0);
        let g = rgb[1].max(0.0);
        let b = rgb[2].max(0.0);
        let per = [self.eval(r), self.eval(g), self.eval(b)];

        let y = 0.2627 * r + 0.6780 * g + 0.0593 * b;
        let luma = if y <= 1e-8 {
            let v = self.eval(y);
            [v, v, v]
        } else {
            let s = self.eval(y) / y;
            [r * s, g * s, b * s]
        };

        // Per-channel midtones; luminance blend near clip. Warm orange petals
        // keep per-channel further into highlights; near-neutrals always use the
        // luma curve (cool sat_keep previously magenta'd RW2 water/whites).
        let peak = r.max(g).max(b);
        let chroma = if peak > 1e-6 {
            (peak - r.min(g).min(b)) / peak
        } else {
            0.0
        };
        // Near-white / grey: Affinity stays neutral — never per-channel here.
        if chroma < 0.20 {
            return luma;
        }
        let warmth = if peak > 1e-6 {
            ((r - b).max(0.0) / peak).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let sat_keep = warmth * ((chroma - 0.20) / 0.25).clamp(0.0, 1.0);
        let blend_start = 0.72 + 0.12 * sat_keep;
        let t = ((peak - blend_start) / (1.05 - blend_start).max(0.05)).clamp(0.0, 1.0);
        // Smoothstep
        let mut w = t * t * (3.0 - 2.0 * t);
        w *= 1.0 - sat_keep * 0.45;
        [
            per[0] + (luma[0] - per[0]) * w,
            per[1] + (luma[1] - per[1]) * w,
            per[2] + (luma[2] - per[2]) * w,
        ]
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ToneParams {
    pub contrast: f32,
    pub shadows: f32,
    pub darks: f32,
    pub lights: f32,
    pub highlights: f32,
}

pub fn is_identity(points: &[[f32; 2]], p: &ToneParams) -> bool {
    points.is_empty() && *p == ToneParams::default()
}

/// Whether the tone-curve GPU pass should run (any channel or parametric active).
pub fn should_run(
    rgb: &[[f32; 2]],
    r: &[[f32; 2]],
    g: &[[f32; 2]],
    b: &[[f32; 2]],
    p: &ToneParams,
) -> bool {
    !r.is_empty() || !g.is_empty() || !b.is_empty() || !is_identity(rgb, p)
}

/// Identity ramp for an unused LUT slot.
pub fn identity_lut() -> Vec<f32> {
    (0..LUT_SIZE)
        .map(|i| i as f32 / (LUT_SIZE - 1) as f32)
        .collect()
}

/// Scene-referred sigmoid (x / (x + 0.18)) mixed by `amount` 0..100.
pub fn apply_sigmoid(lut: &mut [f32], amount: f32) {
    let w = (amount / 100.0).clamp(0.0, 1.0);
    if w <= 0.0 {
        return;
    }
    const MID: f32 = 0.18;
    for v in lut.iter_mut() {
        let x = *v;
        let s = x / (x + MID);
        *v = x * (1.0 - w) + s * w;
    }
}

/// Build the 512-entry LUT over t∈[0,1]. Guaranteed monotonic
/// non-decreasing (cumulative max) and clamped to [0, 0.9995] so the
/// shader's y/(1-y) un-compression stays finite.
///
/// **Lightroom-style clipping**: if user's first point has x > 0, all input
/// values below that x map to output 0 (black clip). If last point has x < 1,
/// all input values above that x map to output 1 (white clip).
///
/// User points are in scene-referred [0,1] (1 = reference white).
/// Convert BOTH x and y to compressed domain for the shader:
///   t = x/(1+x)  (compressed input)
///   y_c = y/(1+y)  (compressed output)
fn scene_to_compressed(v: f32) -> f32 {
    v / (1.0 + v)
}

pub fn build_lut(points: &[[f32; 2]], p: &ToneParams) -> Vec<f32> {
    // Convert user points from scene-referred to compressed domain
    let compressed_points: Vec<[f32; 2]> = points
        .iter()
        .map(|&[x, y]| [scene_to_compressed(x), scene_to_compressed(y)])
        .collect();

    let base: Option<MonotonicCubic> = if compressed_points.is_empty() {
        None
    } else {
        Some(MonotonicCubic::new(compressed_points))
    };

    let first_x = base.as_ref().map(|c| c.first_x()).unwrap_or(0.0);
    let last_x = base.as_ref().map(|c| c.last_x()).unwrap_or(1.0);

    // Only apply clipping if user explicitly moved endpoints away from 0/1
    let clip_black = first_x > 0.0;
    let clip_white = last_x < 1.0;

    let mut lut = Vec::with_capacity(LUT_SIZE);
    // parametric region deltas (pinned placement; max shift 0.12)
    const SCALE: f32 = 0.12 / 100.0;

    for i in 0..LUT_SIZE {
        let t = i as f32 / (LUT_SIZE - 1) as f32;

        // --- CLIPPING LOGIC ---
        // Below first_x: hard clip to 0 (black) — only if user moved first point right
        // Above last_x: hard clip to 1 (white) — only if user moved last point left
        // Between: normal curve evaluation (clamped to 0.9995 for shader safety)
        let v = if clip_black && t <= first_x {
            0.0
        } else if clip_white && t >= last_x {
            1.0
        } else {
            let region_t = scene_region_t(t);
            let mut v = match &base {
                Some(c) => c.eval(t).clamp(0.0, 1.0),
                None => t,
            };
            v += p.shadows * SCALE * region_w(region_t, 0.125, 0.25)
                + p.darks * SCALE * region_w(region_t, 0.30, 0.40)
                + p.lights * SCALE * region_w(region_t, 0.70, 0.40)
                + p.highlights * SCALE * region_w(region_t, 0.875, 0.25);
            v = contrast_curve(v.clamp(0.0, 1.0), p.contrast);
            v.clamp(0.0, 0.9995)
        };
        lut.push(v);
    }
    // enforce monotonic non-decreasing (parametric deltas could dent it)
    for i in 1..LUT_SIZE {
        if lut[i] < lut[i - 1] {
            lut[i] = lut[i - 1];
        }
    }
    lut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_lut_is_exact_ramp() {
        let lut = build_lut(&[], &ToneParams::default());
        for (i, v) in lut.iter().enumerate() {
            let t = i as f32 / (LUT_SIZE - 1) as f32;
            assert!((v - t.min(0.9995)).abs() < 1e-5, "lut[{i}]={v} t={t}");
        }
    }

    #[test]
    fn contrast_is_s_shaped_and_monotonic() {
        let lut = build_lut(
            &[],
            &ToneParams {
                contrast: 50.0,
                ..Default::default()
            },
        );
        let at = |t: f32| lut[(t * (LUT_SIZE - 1) as f32) as usize];
        assert!(at(0.25) < 0.25, "darks darker");
        assert!(at(0.75) > 0.75, "lights lighter");
        assert!((at(0.5) - 0.5).abs() < 0.01, "pivot stays");
        for i in 1..LUT_SIZE {
            assert!(lut[i] >= lut[i - 1], "monotonic");
        }
    }

    #[test]
    fn sigmoid_compresses_highlights() {
        let mut lut = identity_lut();
        apply_sigmoid(&mut lut, 100.0);
        let hi = lut[LUT_SIZE - 1];
        assert!(hi < 0.9, "full sigmoid should compress 1.0, got {hi}");
        assert!(lut[0] >= 0.0);
        for i in 1..LUT_SIZE {
            assert!(lut[i] >= lut[i - 1], "sigmoid must stay monotonic");
        }
    }

    #[test]
    fn point_curve_passes_through_points() {
        let lut = build_lut(
            &[[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]],
            &ToneParams::default(),
        );
        let mid = lut[LUT_SIZE / 2];
        assert!((mid - 0.7).abs() < 0.01, "mid={mid}");
    }

    #[test]
    fn parametric_highlights_lift_top_only() {
        let lut = build_lut(
            &[],
            &ToneParams {
                highlights: 100.0,
                ..Default::default()
            },
        );
        let at = |t: f32| lut[(t * (LUT_SIZE - 1) as f32) as usize];
        assert!(at(0.9) > 0.9 + 0.04, "highlights lifted");
        assert!((at(0.2) - 0.2).abs() < 1e-3, "shadows untouched");
    }

    #[test]
    fn parametric_highlights_affect_visible_scene_values() {
        fn apply_scene(lut: &[f32], x: f32) -> f32 {
            let t = x / (1.0 + x);
            let pos = t * (LUT_SIZE - 1) as f32;
            let i = pos.floor() as usize;
            let j = (i + 1).min(LUT_SIZE - 1);
            let y = lut[i] + (lut[j] - lut[i]) * (pos - i as f32);
            y / (1.0 - y).max(5e-4)
        }

        let lut = build_lut(
            &[],
            &ToneParams {
                highlights: 100.0,
                ..Default::default()
            },
        );

        assert!(
            apply_scene(&lut, 0.8) > 0.9,
            "visible highlights should lift"
        );
        assert!((apply_scene(&lut, 0.2) - 0.2).abs() < 1e-3);
    }

    #[test]
    fn monotonic_cubic_no_overshoot() {
        // steep step must not overshoot above 1
        let lut = build_lut(
            &[[0.0, 0.0], [0.4, 0.05], [0.6, 0.95], [1.0, 1.0]],
            &ToneParams::default(),
        );
        assert!(lut.iter().all(|v| (0.0..=0.9995).contains(v)));
        for i in 1..LUT_SIZE {
            assert!(lut[i] >= lut[i - 1]);
        }
    }

    // --- NEW TESTS FOR CLIPPING ---

    #[test]
    fn black_clip_first_point_x_gt_zero() {
        // First point at x=0.1, y=0.2 → everything below 0.1 maps to 0
        let lut = build_lut(
            &[[0.1, 0.2], [0.5, 0.5], [1.0, 1.0]],
            &ToneParams::default(),
        );
        // t=0.05 (below first_x=0.1) should be clipped to 0
        let idx = (0.05 * (LUT_SIZE - 1) as f32) as usize;
        assert_eq!(lut[idx], 0.0, "black clip at t=0.05");

        // t=0.15 (above first_x) should be on curve
        let idx2 = (0.15 * (LUT_SIZE - 1) as f32) as usize;
        assert!(lut[idx2] > 0.0, "above first_x follows curve");
    }

    #[test]
    fn white_clip_last_point_x_lt_one() {
        // Last point at x=0.9, y=0.8 → everything above 0.9 maps to 1
        let lut = build_lut(
            &[[0.0, 0.0], [0.5, 0.5], [0.9, 0.8]],
            &ToneParams::default(),
        );
        // t=0.95 (above last_x=0.9) should be clipped to 1
        let idx = (0.95 * (LUT_SIZE - 1) as f32) as usize;
        assert!((lut[idx] - 1.0).abs() < 1e-5, "white clip at t=0.95");

        // t=0.85 (below last_x) should be on curve
        let idx2 = (0.85 * (LUT_SIZE - 1) as f32) as usize;
        assert!(lut[idx2] < 1.0, "below last_x follows curve");
    }

    #[test]
    fn both_clips_active() {
        // First at x=0.1, last at x=0.9
        let lut = build_lut(
            &[[0.1, 0.1], [0.5, 0.5], [0.9, 0.9]],
            &ToneParams::default(),
        );
        // Below 0.1 → 0
        assert_eq!(lut[(0.05 * (LUT_SIZE - 1) as f32) as usize], 0.0);
        // Above 0.9 → 1
        assert!((lut[(0.95 * (LUT_SIZE - 1) as f32) as usize] - 1.0).abs() < 1e-5);
        // Middle follows curve
        let mid = lut[LUT_SIZE / 2];
        assert!((mid - 0.5).abs() < 0.02);
    }

    #[test]
    fn clip_with_parametric_still_monotonic() {
        // Clipping + parametric should not break monotonicity
        let lut = build_lut(
            &[[0.15, 0.1], [0.5, 0.5], [0.85, 0.9]],
            &ToneParams {
                contrast: 50.0,
                shadows: 30.0,
                highlights: -30.0,
                ..Default::default()
            },
        );
        for i in 1..LUT_SIZE {
            assert!(lut[i] >= lut[i - 1], "monotonic at i={i}: {} >= {}", lut[i], lut[i-1]);
        }
        // Black clip region
        assert_eq!(lut[0], 0.0);
        // White clip region
        assert!((lut[LUT_SIZE - 1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn first_point_y_not_zero_still_clips_black() {
        // First point at x=0.2, y=0.3 → below 0.2 still clips to 0 (not 0.3)
        let lut = build_lut(
            &[[0.2, 0.3], [0.5, 0.5], [1.0, 1.0]],
            &ToneParams::default(),
        );
        let idx = (0.1 * (LUT_SIZE - 1) as f32) as usize;
        assert_eq!(lut[idx], 0.0, "black clip ignores first point's y");
    }

    #[test]
    fn last_point_y_not_one_still_clips_white() {
        // Last point at x=0.8, y=0.7 → above 0.8 clips to 1 (not 0.7)
        let lut = build_lut(
            &[[0.0, 0.0], [0.5, 0.5], [0.8, 0.7]],
            &ToneParams::default(),
        );
        let idx = (0.9 * (LUT_SIZE - 1) as f32) as usize;
        assert!((lut[idx] - 1.0).abs() < 1e-5, "white clip ignores last point's y");
    }
}