// Terminal display-transform node (contract F2 screen side), 1:1 over the
// chain output: gamut Rec2020→sRGB → pinned neutral view transform
// (Reinhard-extended Lw=4) → clamp → sRGB OETF. alpha 0 → letterbox bg.
// Constants mirror gpu/display.wgsl; golden tests pin them.

struct PresentUniforms {
  width: u32,
  height: u32,
  overlay: f32, // 0 = off; else mask-overlay tint strength
  look: u32,    // 0 = Neutral, 1 = Camera (punchy), 2 = Filmic, 3 = Passthrough, 4 = Original, 8 = Linear
  clip_hi: u32, // 1 = highlight blinkies on
  clip_lo: u32, // 1 = shadow blinkies on
  _p0: u32,     // millis for blink pulse
  _p1: u32,     // bits 0-3 proof space (0 = off), bit 4 = gamut check
  m0: vec4<f32>,
  m1: vec4<f32>,
  m2: vec4<f32>,
  n0: vec4<f32>,
  n1: vec4<f32>,
  n2: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> u: PresentUniforms;
@group(0) @binding(3) var overlay_mask: texture_2d<f32>;

const REC2020_TO_SRGB = mat3x3<f32>(
  vec3<f32>( 1.6605, -0.1246, -0.0182),
  vec3<f32>(-0.5876,  1.1329, -0.1006),
  vec3<f32>(-0.0728, -0.0083,  1.1187),
);

const VIEW_LW: f32 = 4.0;
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// After Rec.2020→sRGB, Fuji cyans/blues often land with R≤0 (clipped to neon).
// Affinity keeps real red + lower sat. Operates in *linear* sRGB (pre-OETF).
fn fix_srgb_cyan(c: vec3<f32>) -> vec3<f32> {
  let r = c.x;
  let g = c.y;
  let b = c.z;
  // B must dominate; allow slightly negative R from the gamut matrix.
  if (b < 0.02 || b < g) {
    return c;
  }
  let mx = max(r, max(g, b));
  let mn = min(r, min(g, b));
  let chroma = (mx - mn) / max(mx, 1e-6);
  // Near-neutral / near-white: leave alone (RW2 water + coot shield → magenta).
  if (chroma < 0.18 || (mx > 0.55 && chroma < 0.28)) {
    return c;
  }
  let rb = r / max(b, 1e-6);
  let gb = g / max(b, 1e-6);
  // Already has enough red (Affinity sky ~0.20–0.35 linear R/B).
  if (rb > 0.38) {
    return c;
  }
  // Skip near-neutrals and pure deep blue with healthy channel balance.
  if (gb < 0.12 && rb > 0.15) {
    return c;
  }
  let luma = dot(max(c, vec3<f32>(0.0)), LUMA);
  let crush = clamp((0.38 - rb) / 0.38, 0.0, 1.0);
  // Stronger on cyan (high G/B) and mid blues (G/B ~0.2–0.6).
  let cyan = clamp(gb / 0.85, 0.0, 1.0);
  let strength = crush * max(cyan, 0.45);
  if (strength < 0.08) {
    return c;
  }
  var out = mix(c, vec3<f32>(luma), 0.50 * strength);
  // Restore Affinity-like R/B (~0.22 for sky, higher for bluish-green).
  let want_rb = select(0.22, 0.45, gb > 0.55);
  out.x = max(out.x, want_rb * b * strength);
  out.y = out.y * (1.0 - 0.18 * strength * clamp(gb / 0.6, 0.0, 1.0));
  return out;
}

// Clipped-green magenta highlights (Panasonic RW2 water / coot shield): when R≈B
// both sit well above G in brights, lift G. Must NOT touch real blues (B≫R) or
// warm hues (R≫B) — that washed Nikon/Sony skies toward grey-magenta.
fn fix_magenta_highlights(c: vec3<f32>) -> vec3<f32> {
  let r = c.x;
  var g = c.y;
  let b = c.z;
  let mx = max(r, max(g, b));
  if (mx < 0.40) {
    return c;
  }
  // Strong hue → leave alone (sky blue, orange, etc.).
  if (abs(r - b) > 0.08 * mx) {
    return c;
  }
  // Both R and B must beat G (true magenta / clipped-green white).
  let rb_min = min(r, b);
  if (rb_min < g + 0.02) {
    return c;
  }
  let rb = 0.5 * (r + b);
  let lag = rb - g;
  if (lag < 0.02) {
    return c;
  }
  let hi = clamp((mx - 0.40) / 0.40, 0.0, 1.0);
  let t = clamp(lag / max(rb, 1e-6), 0.0, 1.0) * hi;
  g = mix(g, rb, t * 0.90);
  return vec3<f32>(r, g, b);
}

// Post-DCP Camera display — MIRRORED in profile/dcp.rs `view_look_display`.
// Look 5: ACR-default curve (Affinity-matched darken). Look 6: embedded curve.
fn view_look_dcp_grade(c: vec3<f32>, gain: f32, luma_contrast: f32, chroma: f32) -> vec3<f32> {
  var outc = max(c, vec3<f32>(0.0)) * gain;
  let l = dot(outc, LUMA);
  if (l <= 1e-8) {
    return vec3<f32>(0.0);
  }
  let s = 0.5 - 0.5 * cos(min(l, 1.0) * 3.14159265);
  let ld = l + luma_contrast * (s - l);
  outc = outc * (ld / l);
  let l2 = dot(max(outc, vec3<f32>(0.0)), LUMA);
  outc = mix(vec3<f32>(l2), outc, chroma);
  // Warm highlight orange (SR2 poppies) — MIRRORED in dcp.rs refine_display_hue.
  if (outc.r > outc.g && l2 > 0.35) {
    let warm = clamp((outc.r - outc.b) / max(outc.r, 1e-6), 0.0, 1.0);
    let hi = clamp((l2 - 0.35) / 0.45, 0.0, 1.0);
    let t = warm * hi;
    let target_gr = 0.73;
    let gr = outc.g / max(outc.r, 1e-6);
    if (gr < target_gr) {
      outc.g = outc.g + (target_gr * outc.r - outc.g) * t * 0.40;
    }
    if (l2 > 0.72) {
      let cap = l2 * 1.04 + 0.06;
      outc.r = min(outc.r, cap);
    }
  }
  return max(outc, vec3<f32>(0.0));
}

fn view_look_dcp(c: vec3<f32>) -> vec3<f32> {
  // ACR-default on-disk — Affinity PNG (_DSC1477.SR2 / PEF / ORF).
  return view_look_dcp_grade(c, 0.82, 0.11, 1.02);
}

fn view_look_dcp_embedded(c: vec3<f32>) -> vec3<f32> {
  // Embedded ProfileToneCurve fallback (RT only when no Adobe Standard).
  return view_look_dcp_grade(c, 1.10, 0.08, 1.05);
}

fn view_look_dcp_builtin(c: vec3<f32>) -> vec3<f32> {
  let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
  let t = clamp((l - 0.08) / 0.38, 0.0, 1.0);
  let gain = 0.58 + 0.28 * t;
  return view_look_dcp_grade(c, gain, 0.14, 0.96);
}

// shared look operator — MIRRORED in export.rs::view_look (WYSIWYG).
// Reinhard-extended shoulder (headroom-safe) + a gentle S-curve, all on
// luminance so hue is preserved; then a saturation scale.
fn view_look(c: vec3<f32>, look: u32) -> vec3<f32> {
  var gain = 1.15;     // Neutral: a touch of lift so the base isn't dark
  var contrast = 0.12;
  var sat = 1.0;
  if (look == 1u) {    // Camera fallback when no DCP — Affinity PNG (S2 / iPhone)
    gain = 1.08;
    contrast = 0.16;
    sat = 1.02;
  }
  let l = dot(max(c, vec3<f32>(0.0)), LUMA);
  if (l <= 1e-8) {
    return vec3<f32>(0.0);
  }
  let x = l * gain;
  let r = x * (1.0 + x / (VIEW_LW * VIEW_LW)) / (1.0 + x);
  let s = 0.5 - 0.5 * cos(clamp(r, 0.0, 1.0) * 3.14159265);
  let ld = clamp(mix(r, s, contrast), 0.0, 1.0);
  var outc = c * (ld / l);
  let l2 = dot(max(outc, vec3<f32>(0.0)), LUMA);
  outc = mix(vec3<f32>(l2), outc, sat);
  return max(outc, vec3<f32>(0.0));
}

fn oetf_srgb(c: vec3<f32>) -> vec3<f32> {
  let lo = c * 12.92;
  let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
  return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// ---- AgX filmic display transform (look == 2) ----
// "Minimal AgX" (Benjamin Wrensch / iolite-engine.com), the implementation
// Blender ships. Input: linear sRGB (Rec.709 primaries). Output: sRGB
// display-encoded — graceful highlight desaturation + clean out-of-gamut,
// unlike the luminance-only Reinhard above. Matrices are the published AgX
// inset/outset (column-major here for `M * v`); min/max log2 exposure pinned.
const AGX_INSET = mat3x3<f32>(
  vec3<f32>(0.8424790622, 0.0423282423, 0.0423756549),
  vec3<f32>(0.0784336000, 0.8784686365, 0.0784336000),
  vec3<f32>(0.0792237451, 0.0791661275, 0.8791429738),
);
const AGX_OUTSET = mat3x3<f32>(
  vec3<f32>( 1.1968790051, -0.0528968518, -0.0529716355),
  vec3<f32>(-0.0980208811,  1.1519031299, -0.0980434501),
  vec3<f32>(-0.0990297441, -0.0989611768,  1.1510736726),
);
const AGX_MIN_EV: f32 = -12.47393;
const AGX_MAX_EV: f32 = 4.026069;

// 6th-order polynomial approximation of the AgX log-encoded contrast sigmoid.
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
  let x2 = x * x;
  let x4 = x2 * x2;
  return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4
       - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(srgb_lin: vec3<f32>) -> vec3<f32> {
  var v = AGX_INSET * srgb_lin;
  v = clamp((log2(max(v, vec3<f32>(1e-10))) - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV),
            vec3<f32>(0.0), vec3<f32>(1.0));
  v = agx_contrast(v);
  v = AGX_OUTSET * v;
  return clamp(v, vec3<f32>(0.0), vec3<f32>(1.0)); // already display-encoded
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x >= u.width || gid.y >= u.height) {
    return;
  }
  let p = textureLoad(src, vec2<i32>(gid.xy), 0);
  var encoded = vec3<f32>(0.0863, 0.0863, 0.0941); // app bg
  if (p.a > 0.0) {
    var c: vec3<f32>;
    var paint_gamut = false;
    let space = u._p1 & 7u;
    if (space == 0u) {
      c = REC2020_TO_SRGB * max(p.rgb, vec3<f32>(0.0));
      c = fix_magenta_highlights(c);
      c = fix_srgb_cyan(c);
    } else {
      let M = mat3x3<f32>(u.m0.xyz, u.m1.xyz, u.m2.xyz);
      let N = mat3x3<f32>(u.n0.xyz, u.n1.xyz, u.n2.xyz);
      let t = M * p.rgb;
      if ((u._p1 & 16u) != 0u && (t.x < 0.0 || t.y < 0.0 || t.z < 0.0)) {
        paint_gamut = true;
      }
      c = N * max(t, vec3<f32>(0.0));
      c = fix_magenta_highlights(c);
      c = fix_srgb_cyan(c);
    }
    if (paint_gamut) {
      encoded = vec3<f32>(1.0, 0.0, 1.0);
    } else if (u.look == 2u) {
      // AgX already outputs display-encoded sRGB — no second OETF.
      encoded = agx(max(c, vec3<f32>(0.0)));
    } else if (u.look == 5u) {
      // Post-DCP Camera (ACR default curve): Affinity-matched grade + OETF.
      encoded = oetf_srgb(clamp(view_look_dcp(max(c, vec3<f32>(0.0))), vec3<f32>(0.0), vec3<f32>(1.0)));
    } else if (u.look == 6u) {
      // Post-DCP Camera (embedded ProfileToneCurve): mild grade + OETF.
      encoded = oetf_srgb(clamp(view_look_dcp_embedded(max(c, vec3<f32>(0.0))), vec3<f32>(0.0), vec3<f32>(1.0)));
    } else if (u.look == 7u) {
      // Built-in ACR curve only (no on-disk DCP HueSatMap).
      encoded = oetf_srgb(clamp(view_look_dcp_builtin(max(c, vec3<f32>(0.0))), vec3<f32>(0.0), vec3<f32>(1.0)));
    } else if (u.look == 3u || u.look == 4u || u.look == 8u) {
      // 3 = raster zero-edit (JPEG/PNG already display-referred).
      // 4 = Original RAW (demosaic only). Rec.2020→sRGB + OETF, no view look.
      // 8 = Linear/None. Rec.2020→sRGB + OETF, no tone mapping.
      encoded = oetf_srgb(clamp(max(c, vec3<f32>(0.0)), vec3<f32>(0.0), vec3<f32>(1.0)));
    } else {
      let looked = view_look(max(c, vec3<f32>(0.0)), u.look);
      encoded = oetf_srgb(clamp(looked, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    if (u.overlay > 0.0) {
      let m = clamp(textureLoad(overlay_mask, vec2<i32>(gid.xy), 0).r, 0.0, 1.0);
      let mode = (u._p1 >> 8u) & 0xfu;
      var tint = vec3<f32>(1.0, 0.15, 0.15);
      if (mode == 1u) {
        tint = vec3<f32>(1.0, 1.0, 1.0);
      } else if (mode == 2u) {
        tint = vec3<f32>(0.0, 0.0, 0.0);
      }
      if (mode == 3u) {
        let gray = dot(encoded, LUMA);
        let base = vec3<f32>(gray, gray, gray);
        encoded = mix(base, vec3<f32>(1.0, 0.15, 0.15), m * u.overlay);
      } else {
        encoded = mix(encoded, tint, m * u.overlay);
      }
    }
    // Clipping blinkies on display-encoded output (matches histogram clip %).
    // Pulse via _p0 = millis so warnings flash while frames keep updating.
    if (!paint_gamut) {
      let pulse = 0.55 + 0.45 * abs(sin(f32(u._p0) * 0.012566)); // ~2 Hz
      if (u.clip_hi != 0u) {
        let hi = encoded.r >= 0.995 || encoded.g >= 0.995 || encoded.b >= 0.995;
        if (hi) {
          encoded = mix(encoded, vec3<f32>(1.0, 0.05, 0.05), pulse);
        }
      }
      if (u.clip_lo != 0u) {
        let lo = encoded.r <= 0.004 && encoded.g <= 0.004 && encoded.b <= 0.004;
        if (lo) {
          encoded = mix(encoded, vec3<f32>(0.15, 0.45, 1.0), pulse);
        }
      }
    }
  }
  textureStore(dst, vec2<i32>(gid.xy), vec4<f32>(encoded, 1.0));
}