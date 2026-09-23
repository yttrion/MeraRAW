//! Camera profile resolution — match RAW metadata to bundled `.dcp` files.

pub mod dcp;
pub mod hue_sat_map;
pub mod tone_curve;

pub use dcp::DcpProfile;

use crate::error::CoreError;
use crate::raw::ImageMeta;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static PROFILES_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Called once at app startup (Tauri main) with the resolved profiles folder.
pub fn set_profiles_dir(dir: PathBuf) {
    let _ = PROFILES_DIR.set(dir);
}

pub fn profiles_dir() -> PathBuf {
    PROFILES_DIR.get().cloned().unwrap_or_else(|| {
        if let Ok(d) = std::env::var("MERARAW_PROFILES_DIR") {
            return PathBuf::from(d);
        }
        for candidate in [
            "meraraw-derivatives",
            "../meraraw-derivatives",
            "../../meraraw-derivatives",
        ] {
            let p = PathBuf::from(candidate);
            if p.is_dir() {
                return p.canonicalize().unwrap_or(p);
            }
        }
        PathBuf::from("meraraw-derivatives")
    })
}

pub fn normalize_key(s: &str) -> String {
    s.split_whitespace()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Alphanumeric-only key so EXIF "C5050Z" matches index "C-5050Z".
pub(crate) fn compact_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub fn camera_model_key(make: &str, model: &str) -> String {
    normalize_key(&format!("{} {}", make.trim(), model.trim()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRef {
    #[serde(default)]
    pub name: String,
    pub file: String,
}

fn profile_name_from_file(file: &str) -> String {
    Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.to_string())
}

fn profile_ref_from_value(v: serde_json::Value) -> Result<ProfileRef, String> {
    if let Some(s) = v.as_str() {
        return Ok(ProfileRef {
            name: profile_name_from_file(s),
            file: s.to_string(),
        });
    }
    if let Some(obj) = v.as_object() {
        let file = obj
            .get("file")
            .and_then(|f| f.as_str())
            .ok_or("profile entry missing file")?
            .to_string();
        let name = obj
            .get("name")
            .and_then(|n| n.as_str())
            .map(|s| s.trim_end_matches('\0').trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| profile_name_from_file(&file));
        return Ok(ProfileRef { name, file });
    }
    Err("invalid profile entry".into())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileIndex {
    #[serde(default, deserialize_with = "deserialize_cameras")]
    pub cameras: BTreeMap<String, Vec<ProfileRef>>,
}

fn deserialize_cameras<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, Vec<ProfileRef>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::deserialize(deserializer)?;
    raw.into_iter()
        .map(|(k, vals)| {
            let refs = vals
                .into_iter()
                .map(|v| profile_ref_from_value(v).map_err(serde::de::Error::custom))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((k, refs))
        })
        .collect()
}

impl ProfileIndex {
    pub fn embedded() -> Self {
        serde_json::from_str(include_str!("../../models/profile_index.json"))
            .unwrap_or_else(|e| panic!("profile_index.json: {e}"))
    }

    /// Lookup profiles for a camera, case-insensitive on the index key.
    /// Also matches when EXIF omits hyphens (`C5050Z` ↔ `C-5050Z`), and
    /// marketing names that Adobe indexes by internal id (`iPhone 6s Plus` →
    /// `iPhone8,2 back camera`).
    pub fn profiles_for(&self, make: &str, model: &str) -> Option<&[ProfileRef]> {
        let mut candidates = vec![
            camera_model_key(make, model),
            normalize_key(model),
            normalize_key(&format!("{} {}", title_case_word(make), model.trim())),
        ];
        for alias in camera_model_aliases(make, model) {
            candidates.push(normalize_key(&alias));
            candidates.push(compact_key(&alias));
        }
        let compact_candidates: Vec<String> = candidates.iter().map(|c| compact_key(c)).collect();
        for key in &self.cameras {
            let nk = normalize_key(key.0);
            if candidates.iter().any(|c| c == &nk) {
                return Some(key.1.as_slice());
            }
            let ck = compact_key(key.0);
            if compact_candidates.iter().any(|c| c == &ck) {
                return Some(key.1.as_slice());
            }
        }
        None
    }
}

/// Map EXIF marketing names → Adobe `UniqueCameraModel` / index keys.
pub(crate) fn camera_model_aliases(make: &str, model: &str) -> Vec<String> {
    let mk = make.to_ascii_lowercase();
    let md = model.to_ascii_lowercase().replace('\u{00a0}', " ");
    let mut out = Vec::new();
    // Apple: EXIF "iPhone 6s Plus" vs Adobe "iPhone8,2 back camera".
    if mk.contains("apple") || md.contains("iphone") {
        let map = [
            ("iphone 6s plus", "iPhone8,2 back camera"),
            ("iphone 6s", "iPhone8,1 back camera"),
            ("iphone 6 plus", "iPhone7,1 back camera"),
            ("iphone 6", "iPhone7,2 back camera"),
            ("iphone se", "iPhone8,4 back camera"),
            ("iphone 7 plus", "iPhone9,4 back camera"),
            ("iphone 7", "iPhone9,1 back camera"),
            ("iphone 8 plus", "iPhone10,2 back camera"),
            ("iphone 8", "iPhone10,1 back camera"),
            ("iphone x", "iPhone10,3 back camera"),
        ];
        for (name, adobe) in map {
            if md.contains(name) {
                out.push(adobe.to_string());
                break;
            }
        }
    }
    // Nikon 1 AW1 is indexed as "Nikon 1 AW1".
    if md.contains("aw1") {
        out.push("Nikon 1 AW1".into());
    }
    // Panasonic FZ45 is the EU name for FZ40 — Adobe only ships DMC-FZ40.
    if mk.contains("panasonic") && (md.contains("fz45") || md.contains("fz 45")) {
        out.push("Panasonic DMC-FZ40".into());
        out.push("DMC-FZ40".into());
    }
    out
}

fn title_case_word(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        return String::new();
    }
    let mut chars = t.chars();
    let first = chars.next().unwrap().to_uppercase().collect::<String>();
    format!("{first}{}", chars.as_str().to_lowercase())
}

/// Index / DCP `UniqueCameraModel` label (e.g. "Fujifilm X-T2").
pub fn unique_camera_model_label(make: &str, model: &str) -> String {
    format!("{} {}", title_case_word(make), model.trim())
}

/// Profiles available for this raw's camera metadata.
/// Rendered rasters never list DCPs — EXIF camera tags on a JPEG are metadata,
/// not a license to apply a sensor matrix.
pub fn resolve_profiles(meta: &ImageMeta, index: &ProfileIndex) -> Vec<ProfileRef> {
    if !meta.kind.allows_raw_only_stages() {
        return Vec::new();
    }
    index
        .profiles_for(&meta.camera_make, &meta.camera_model)
        .map(|v| v.to_vec())
        .unwrap_or_default()
}

const DEFAULT_PROFILE_NAMES: &[&str] = &[
    "MeraRAW Standard",
    "Adobe Standard (MeraRAW)",
    "Adobe Standard",
    "Camera Standard",
];

/// Pick the default profile for autoload — prefer Adobe Standard (Affinity match).
pub fn default_profile(profiles: &[ProfileRef]) -> Option<&ProfileRef> {
    // Affinity Develop tracks Adobe Standard LookTables / ACR curves.
    for want in [
        "Adobe Standard",
        "Adobe Standard (MeraRAW)",
        "MeraRAW Standard",
        "Camera Standard",
    ] {
        if let Some(p) = profiles.iter().find(|p| {
            p.name.eq_ignore_ascii_case(want)
                || p.file.to_ascii_lowercase().contains(&want.to_ascii_lowercase())
        }) {
            return Some(p);
        }
    }
    profiles.first()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveProfile {
    pub name: String,
    pub file: String,
    pub unique_camera_model: String,
}

/// Resolve + load the default DCP for a raw file.
pub fn autoload_profile(
    meta: &ImageMeta,
    index: &ProfileIndex,
) -> Result<Option<ActiveProfile>, CoreError> {
    let profiles = resolve_profiles(meta, index);
    if profiles.is_empty() {
        tracing::info!(
            make = %meta.camera_make,
            model = %meta.camera_model,
            "no DCP profiles matched"
        );
        return Ok(None);
    }
    let Some(chosen) = default_profile(&profiles) else {
        return Ok(None);
    };
    let Some(dcp) = load_dcp_profile(meta, Some(chosen)) else {
        if !profile_path(&chosen.file).is_file() {
            tracing::warn!(file = %chosen.file, "DCP file missing and no built-in fallback");
        }
        return Ok(None);
    };
    tracing::info!(
        profile = %dcp.profile_name,
        camera = %dcp.unique_camera_model,
        file = %chosen.file,
        "autoloaded camera profile"
    );
    Ok(Some(ActiveProfile {
        name: profile_display_name(&chosen.file, &dcp.unique_camera_model),
        file: chosen.file.clone(),
        unique_camera_model: dcp.unique_camera_model,
    }))
}

pub fn profile_path(file: &str) -> PathBuf {
    resolve_profile_file(file).unwrap_or_else(|| profiles_dir().join(file))
}

/// All candidate `.dcp` paths for an indexed profile filename.
pub fn profile_candidates(file: &str) -> Vec<PathBuf> {
    let file = file.trim();
    if file.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let primary = profiles_dir().join(file);
    if primary.is_file() {
        out.push(primary);
    }
    for cand in adobe_dcp_fallbacks(file) {
        if !out.iter().any(|p| p == &cand) {
            out.push(cand);
        }
    }
    out
}

/// Prefer Adobe Standard (LookTable, ACR default curve) over RawTherapee
/// embedded ProfileToneCurve — Affinity Develop tracks Adobe-style profiles.
fn dcp_quality_score(dcp: &DcpProfile) -> u64 {
    let look = dcp.look_data();
    let mut s = 0u64;
    if look.map1.is_some() {
        s += 1_000;
    }
    if look.map2.is_some() {
        s += 500;
    }
    if let Some((hd, sd, vd, _)) = &look.look {
        // Adobe Standard LookTables are the Affinity/Lightroom match.
        s += 100_000 + (*hd as u64) * (*sd as u64) * (*vd as u64);
    }
    if dcp.tone_curve_embedded() {
        // RT dense curves help when no Adobe pack exists, but must not beat
        // Adobe Standard ACR-default (was forcing Look 6 on A7III / Fuji).
        s += 5_000;
    } else {
        // ACR default tone curve — Affinity Develop baseline.
        s += 80_000;
    }
    // Prefer profiles literally named Adobe Standard when both exist.
    let name = dcp.profile_name.to_ascii_lowercase();
    if name.contains("adobe standard") {
        s += 200_000;
    }
    s
}

/// Locate a `.dcp` on disk: `meraraw-derivatives` first, then the richest
/// Adobe / RawTherapee fallback (not merely the first Adobe Standard hit).
pub fn resolve_profile_file(file: &str) -> Option<PathBuf> {
    let file = file.trim();
    if file.is_empty() {
        return None;
    }
    let mut best: Option<(PathBuf, u64)> = None;
    for path in profile_candidates(file) {
        if !path.is_file() {
            continue;
        }
        let Ok(dcp) = DcpProfile::load(&path) else {
            continue;
        };
        let score = dcp_quality_score(&dcp);
        if best.as_ref().is_none_or(|(_, s)| score > *s) {
            best = Some((path, score));
        }
    }
    best.map(|(p, _)| p)
}

fn adobe_camera_profiles_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(d) = std::env::var("MERARAW_ADOBE_PROFILES") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            roots.push(p);
        }
    }
    #[cfg(target_os = "macos")]
    {
        for base in [
            "/Applications/Adobe Lightroom.app/Contents/Resources/CameraProfiles",
            "/Applications/Adobe Lightroom Classic.app/Contents/Resources/CameraProfiles",
            "/Library/Application Support/Adobe/CameraRaw/CameraProfiles",
        ] {
            let p = PathBuf::from(base);
            if p.is_dir() {
                roots.push(p);
            }
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if let Some(home) = home {
            let p = home.join("Library/Application Support/Adobe/CameraRaw/CameraProfiles");
            if p.is_dir() {
                roots.push(p);
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            let p = PathBuf::from(pf).join("Adobe/CameraRaw/CameraProfiles");
            if p.is_dir() {
                roots.push(p);
            }
        }
    }
    roots
}

fn rawtherapee_dcp_roots() -> Vec<PathBuf> {
    let roots = Vec::new();
    #[cfg(target_os = "macos")]
    {
        let p = PathBuf::from(
            "/Applications/RawTherapee.app/Contents/Resources/share/dcpprofiles",
        );
        if p.is_dir() {
            roots.push(p);
        }
    }
    roots
}

/// Candidate paths for an indexed profile filename when the local pack is missing.
fn adobe_dcp_fallbacks(file: &str) -> Vec<PathBuf> {
    let stem = file.strip_suffix(".dcp").unwrap_or(file);
    let mut out = Vec::new();

    // "Fujifilm X-T2 MeraRAW Standard" / "... MeraRAW Standard v2"
    // → Adobe Standard/<model> Adobe Standard[.dcp|/ v2.dcp]
    if let Some(model) = stem
        .strip_suffix(" MeraRAW Standard v2")
        .or_else(|| stem.strip_suffix(" MeraRAW Standard"))
    {
        // RawTherapee ships one DCP per body (often with HueSatMap + look).
        // Try RT *before* Adobe Standard so resolve_profile_file can still pick
        // Adobe when RT is absent, but RT wins when both exist (embedded curve).
        for root in rawtherapee_dcp_roots() {
            out.push(root.join(format!("{model}.dcp")));
            out.push(root.join(format!("{}.dcp", model.to_uppercase())));
            // "Fujifilm X-T2" vs RT "FUJIFILM X-T2"
            let upper_make = model
                .split_once(' ')
                .map(|(m, rest)| format!("{} {}", m.to_uppercase(), rest))
                .unwrap_or_else(|| model.to_uppercase());
            out.push(root.join(format!("{upper_make}.dcp")));
        }
        for root in adobe_camera_profiles_roots() {
            let adobe = root.join("Adobe Standard");
            // Prefer v2 when the index asked for v2, else try both (v2 first for Canon).
            if stem.ends_with("v2") {
                out.push(adobe.join(format!("{model} Adobe Standard v2.dcp")));
                out.push(adobe.join(format!("{model} Adobe Standard.dcp")));
            } else {
                out.push(adobe.join(format!("{model} Adobe Standard.dcp")));
                out.push(adobe.join(format!("{model} Adobe Standard v2.dcp")));
            }
        }
        return out;
    }

    // "Canon EOS 5DS Camera Standard" → Camera/<model>/<file>
    if let Some((model, _look)) = stem.split_once(" Camera ") {
        for root in adobe_camera_profiles_roots() {
            out.push(root.join("Camera").join(model).join(file));
        }
    }

    out
}

/// Path for the RAW decoder's color-matrix selection — only when a real `.dcp`
/// file exists on disk. Built-in profiles are look-only (tone curve on GPU).
pub fn decode_profile_path(chosen: Option<&ProfileRef>) -> Option<PathBuf> {
    chosen.and_then(|p| resolve_profile_file(&p.file))
}

/// Load a DCP from disk (including Adobe/RT fallbacks), or synthesize the
/// built-in MeraRAW Standard look when nothing usable is installed.
pub fn load_dcp_profile(meta: &ImageMeta, chosen: Option<&ProfileRef>) -> Option<DcpProfile> {
    let chosen = chosen?;
    let mut scored: Vec<(PathBuf, u64)> = profile_candidates(&chosen.file)
        .into_iter()
        .filter(|p| p.is_file())
        .filter_map(|path| {
            DcpProfile::load(&path)
                .ok()
                .filter(|d| d.matches_camera(&meta.camera_make, &meta.camera_model))
                .map(|d| (path, dcp_quality_score(&d)))
        })
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    if let Some((path, _)) = scored.first() {
        if let Ok(dcp) = DcpProfile::load(path) {
            tracing::info!(
                file = %chosen.file,
                path = %path.display(),
                "loaded camera profile from disk"
            );
            let mut dcp = dcp.with_affinity_ev_bias(&meta.camera_make, &meta.camera_model);
            // FZ45 is EU FZ40 hardware-adjacent but Adobe only ships FZ40 LookTable —
            // that LUT shifts FZ45 toward purple/magenta vs Affinity. Keep matrix+curve.
            let md = meta.camera_model.to_ascii_lowercase();
            if md.contains("fz45") {
                dcp = dcp.without_look_table();
                tracing::info!("FZ45: dropping FZ40 ProfileLookTable (Affinity match)");
            }
            return Some(dcp);
        }
    }
    // No on-disk DCP: decode still uses rawler's matrix. Provide Adobe ACR
    // default tone curve on the GPU so Camera look matches Affinity Develop
    // instead of the Reinhard punchy fallback (which was ~20–60 luma dark
    // and oversaturated on CRW/DCR/3FR/RW2 vs Affinity PNG exports).
    let dcp = DcpProfile::builtin_standard(&meta.camera_make, &meta.camera_model);
    tracing::info!(
        camera = %dcp.unique_camera_model,
        file = %chosen.file,
        "using built-in ACR tone curve (no .dcp on disk)"
    );
    Some(dcp)
}

pub fn find_profile<'a>(profiles: &'a [ProfileRef], name_or_file: &str) -> Option<&'a ProfileRef> {
    let key = normalize_key(name_or_file);
    profiles.iter().find(|p| {
        normalize_key(&p.name) == key
            || normalize_key(&p.file) == key
            || normalize_key(p.file.strip_suffix(".dcp").unwrap_or(&p.file)) == key
    })
}

/// Pick a profile from sidecar override, explicit name, or the default autoload.
/// Rendered rasters (JPEG/PNG/…) never receive a DCP — EXIF camera tags on a
/// JPEG must not trigger sensor-matrix / profile-tone-curve application.
pub fn choose_profile(
    meta: &ImageMeta,
    index: &ProfileIndex,
    override_file: Option<&str>,
) -> Option<ProfileRef> {
    if !meta.kind.allows_raw_only_stages() {
        return None;
    }
    let profiles = resolve_profiles(meta, index);
    if profiles.is_empty() {
        return None;
    }
    if let Some(want) = override_file.filter(|s| !s.is_empty()) {
        if let Some(p) = find_profile(&profiles, want) {
            return Some(p.clone());
        }
    }
    default_profile(&profiles).cloned()
}

/// User-facing label from the `.dcp` filename (e.g. "MeraRAW Standard"), not the
/// internal Adobe ProfileName tag (e.g. "Adobe Standard (MeraRAW)").
pub fn profile_display_name(file: &str, unique_camera_model: &str) -> String {
    let stem = file.strip_suffix(".dcp").unwrap_or(file);
    if let Some(rest) = stem.strip_prefix(&format!("{unique_camera_model} ")) {
        return rest.to_string();
    }
    stem.to_string()
}

pub fn matched_camera_key(meta: &ImageMeta, index: &ProfileIndex) -> Option<String> {
    let candidates = [
        camera_model_key(&meta.camera_make, &meta.camera_model),
        normalize_key(&meta.camera_model),
        normalize_key(&format!(
            "{} {}",
            title_case_word(&meta.camera_make),
            meta.camera_model.trim()
        )),
    ];
    for key in index.cameras.keys() {
        if candidates.iter().any(|c| normalize_key(key) == *c) {
            return Some(key.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_matches_sony_ilce7m4() {
        let index = ProfileIndex::embedded();
        let meta = ImageMeta {
            path: "/x.ARW".into(),
            kind: crate::raw::ImageKind::Raw,
            format: "ARW".into(),
            bit_depth: 0,
            camera_make: "SONY".into(),
            camera_model: "ILCE-7M4".into(),
            lens: None,
            iso: None,
            shutter: None,
            aperture: None,
            focal_mm: None,
            captured_at: None,
            width: 100,
            height: 100,
            orientation: "Normal".into(),
            as_shot_wb: [1.0, 1.0, 1.0],
            estimated_cct: Some(5500.0),
            camera_profile: None,
            available_profiles: Vec::new(),
            available_profile_files: Vec::new(),
            demosaic: String::new(),
            available_demosaic: Vec::new(),
            gps_lat: None,
            gps_lon: None,
            input_color_space: None,
            video: None,
        };
        let profiles = resolve_profiles(&meta, &index);
        assert!(!profiles.is_empty(), "expected ILCE-7M4 profiles in index");
        assert!(default_profile(&profiles).is_some());
    }

    #[test]
    fn jpeg_exif_camera_does_not_autoload_dcp() {
        let index = ProfileIndex::embedded();
        let mut meta = ImageMeta {
            path: "/x.jpg".into(),
            kind: crate::raw::ImageKind::Rendered,
            format: "JPEG".into(),
            bit_depth: 8,
            camera_make: "SONY".into(),
            camera_model: "ILCE-7M4".into(),
            lens: None,
            iso: None,
            shutter: None,
            aperture: None,
            focal_mm: None,
            captured_at: None,
            width: 100,
            height: 100,
            orientation: "Normal".into(),
            as_shot_wb: [1.0, 1.0, 1.0],
            estimated_cct: Some(5500.0),
            camera_profile: None,
            available_profiles: Vec::new(),
            available_profile_files: Vec::new(),
            demosaic: String::new(),
            available_demosaic: Vec::new(),
            gps_lat: None,
            gps_lon: None,
            input_color_space: Some("sRGB".into()),
            video: None,
        };
        assert!(resolve_profiles(&meta, &index).is_empty());
        assert!(choose_profile(&meta, &index, None).is_none());
        meta.kind = crate::raw::ImageKind::Raw;
        // RAW of the same camera still resolves (when the index has it).
        let _ = resolve_profiles(&meta, &index);
    }

    #[test]
    fn adobe_lightroom_fallback_resolves_fuji_xt2() {
        let file = "Fujifilm X-T2 MeraRAW Standard.dcp";
        // Skip when Lightroom/RT packs aren't installed (CI without Adobe).
        let Some(path) = resolve_profile_file(file) else {
            eprintln!("skip: no Adobe/RT DCP fallback on this machine");
            return;
        };
        assert!(path.is_file(), "{}", path.display());
        assert!(
            path.to_string_lossy().contains("dcpprofiles")
                || path.to_string_lossy().contains("Adobe Standard"),
            "unexpected path {}",
            path.display()
        );
        let meta = ImageMeta {
            path: "/x.RAF".into(),
            kind: crate::raw::ImageKind::Raw,
            format: "RAF".into(),
            bit_depth: 14,
            camera_make: "FUJIFILM".into(),
            camera_model: "X-T2".into(),
            lens: None,
            iso: None,
            shutter: None,
            aperture: None,
            focal_mm: None,
            captured_at: None,
            width: 100,
            height: 100,
            orientation: "Normal".into(),
            as_shot_wb: [1.0, 1.0, 1.0],
            estimated_cct: Some(5500.0),
            camera_profile: None,
            available_profiles: Vec::new(),
            available_profile_files: Vec::new(),
            demosaic: String::new(),
            available_demosaic: Vec::new(),
            gps_lat: None,
            gps_lon: None,
            input_color_space: None,
            video: None,
        };
        let chosen = ProfileRef {
            name: "MeraRAW Standard".into(),
            file: file.into(),
        };
        let dcp = load_dcp_profile(&meta, Some(&chosen)).expect("load Adobe/RT DCP");
        let look = dcp.look_data();
        assert!(
            look.map1.is_some() || look.look.is_some(),
            "Adobe/RT Fuji profile should carry HueSatMap or LookTable (not mute builtin)"
        );
    }

    #[test]
    fn builtin_acr_curve_when_dcp_file_missing() {
        // Without an on-disk DCP, Camera look still gets Adobe ACR default tone
        // (decode matrix stays rawler's) so Affinity-like rendering works.
        let file = "Fujifilm X-T2 MeraRAW Standard.dcp";
        if resolve_profile_file(file).is_some() {
            eprintln!("skip: Adobe/RT DCP present on this machine");
            return;
        }
        let index = ProfileIndex::embedded();
        let meta = ImageMeta {
            path: "/x.RAF".into(),
            kind: crate::raw::ImageKind::Raw,
            format: "RAF".into(),
            bit_depth: 14,
            camera_make: "FUJIFILM".into(),
            camera_model: "X-T2".into(),
            lens: None,
            iso: None,
            shutter: None,
            aperture: None,
            focal_mm: None,
            captured_at: None,
            width: 100,
            height: 100,
            orientation: "Normal".into(),
            as_shot_wb: [1.0, 1.0, 1.0],
            estimated_cct: Some(5500.0),
            camera_profile: None,
            available_profiles: Vec::new(),
            available_profile_files: Vec::new(),
            demosaic: String::new(),
            available_demosaic: Vec::new(),
            gps_lat: None,
            gps_lon: None,
            input_color_space: None,
            video: None,
        };
        let profiles = resolve_profiles(&meta, &index);
        assert!(!profiles.is_empty());
        let chosen = default_profile(&profiles).unwrap();
        let dcp = load_dcp_profile(&meta, Some(chosen));
        assert!(dcp.is_some(), "expected built-in ACR tone when .dcp missing");
        assert!(!dcp.unwrap().tone_curve_embedded());
    }

    #[test]
    fn display_name_from_filename_not_adobe_tag() {
        assert_eq!(
            profile_display_name("Sony ILCE-7M4 MeraRAW Standard.dcp", "Sony ILCE-7M4"),
            "MeraRAW Standard"
        );
    }
}
