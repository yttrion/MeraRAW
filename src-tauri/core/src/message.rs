//! EngineMsg: the command→engine channel vocabulary (contract C1). Grows
//! per phase (P2 ApplyOp/Undo, P4 AddMask, P5 Import/Search, P7 Export).

use crate::error::CoreError;
use crate::gpu::display::ViewParams;
use crate::raw::{DecodedImage, ImageMeta};
use serde::Serialize;
use std::path::PathBuf;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocRef {
    pub doc_id: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub alive: bool,
    pub gpu_ready: bool,
    pub adapter: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub gpu_adapter: Option<String>,
    pub gpu_backend: Option<String>,
}

/// A CPU-side RGBA8 frame ready for the webview canvas.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampledColor {
    /// linear Rec.2020 working-space value (scene-referred, pre-edit)
    pub working: [f32; 3],
    /// friendly display sRGB 0-255 of the base image at that point
    pub display: [u8; 3],
}

/// Instrumentation readout (spec 7.5; budgets in test strategy §7).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfStats {
    pub last_render_ms: u64,
    pub last_passes: Vec<String>,
    pub renders: u64,
    /// renders that reused at least one upstream cache (cache-the-chain proof)
    pub cached_renders: u64,
    pub last_decode_ms: u64,
}

/// RGB + luma histogram and clip percentages (contract F1).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStats {
    pub bins: usize,
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    pub luma: Vec<u32>,
    pub clip_high_pct: f32,
    pub clip_low_pct: f32,
    /// Compact luma waveform: `waveform_w` columns × `waveform_h` luma rows.
    #[serde(default)]
    pub waveform: Vec<u32>,
    #[serde(default)]
    pub waveform_w: u32,
    #[serde(default)]
    pub waveform_h: u32,
    /// Rec.709-style vectorscope: `vectorscope_size²` bins, U×V from display RGB.
    #[serde(default)]
    pub vectorscope: Vec<u32>,
    #[serde(default)]
    pub vectorscope_size: u32,
    /// RGB parade waveform, 3 planes packed R then G then B, each `waveform_w * waveform_h`.
    #[serde(default)]
    pub parade: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameInfo {
    pub version: u64,
    pub width: u32,
    pub height: u32,
}

/// Engine → frontend push events (Tauri layer forwards as named events,
/// contract C3). Core stays Tauri-free.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum EngineEvent {
    PreviewReady {
        version: u64,
    },
    ImageReady {
        version: u64,
    },
    FrameReady {
        version: u64,
    },
    DecodeError {
        message: String,
    },
    /// Canonical doc changed (op/undo/redo/restore) — frontend reconciles.
    DocUpdated {
        delta: serde_json::Value,
    },
    /// A segmentation mask finished inference and is now rendering.
    MaskReady {
        id: String,
    },
    /// Segmentation failed for a mask (model missing, inference error, …).
    MaskError {
        id: String,
        message: String,
    },
    ImportProgress {
        done: u64,
        total: u64,
    },
    ImportDone {
        total: u64,
    },
    /// Catalog rows changed (ratings/flags/imports) — grids should refresh.
    CatalogChanged,
    /// Tiled export progress (phase: "render" | "encode", done/total tiles or 1/1).
    ExportProgress {
        phase: String,
        done: u32,
        total: u32,
    },
    /// Batch export progress: per-image phase ("decode" | "render" | "encode")
    /// plus the image's position in the queue.
    ExportBatchProgress {
        index: u32,
        count: u32,
        path: String,
        phase: String,
        done: u32,
        total: u32,
    },
    /// Batch export finished (or was cancelled). `failed` = (source, error).
    ExportBatchDone {
        ok: Vec<String>,
        failed: Vec<(String, String)>,
        cancelled: bool,
    },
    /// Engine thread recovered from a panic; UI should prompt restart.
    EngineCrashed {
        message: String,
    },
    /// AI denoise job progress (tiles).
    DenoiseProgress {
        job: u64,
        pct: f32,
        tile: u32,
        tiles: u32,
    },
    DenoiseDone {
        job: u64,
    },
    DenoiseError {
        job: u64,
        message: String,
    },
    /// Object-removal / heal rebuild finished.
    RetouchDone,
    RetouchError {
        message: String,
    },
}

pub enum EngineMsg {
    Ping {
        reply: oneshot::Sender<EngineStatus>,
    },
    Info {
        reply: oneshot::Sender<EngineInfo>,
    },
    /// Test pattern (P0 transport proof; also the no-image fallback).
    TestFrame {
        width: u32,
        height: u32,
        reply: oneshot::Sender<Frame>,
    },
    // ---- Phase 1 ----
    OpenImage {
        path: PathBuf,
        doc_id: Option<String>,
        reply: oneshot::Sender<Result<ImageMeta, CoreError>>,
    },
    RequestFrame {
        view: ViewParams,
        reply: oneshot::Sender<Result<FrameInfo, CoreError>>,
    },
    GetFrame {
        reply: oneshot::Sender<Option<Frame>>,
    },
    GetMetadata {
        reply: oneshot::Sender<Option<ImageMeta>>,
    },
    CloseImage {
        reply: oneshot::Sender<()>,
    },
    // ---- Phase 2: ops on the canonical doc ----
    ApplyOp {
        op: crate::ops::Op,
        /// true during an interactive drag — coalesce into one undo entry.
        live: bool,
        reply: oneshot::Sender<Result<crate::ops::DocDelta, CoreError>>,
    },
    Undo {
        reply: oneshot::Sender<Result<crate::ops::DocDelta, CoreError>>,
    },
    Redo {
        reply: oneshot::Sender<Result<crate::ops::DocDelta, CoreError>>,
    },
    GetDoc {
        reply: oneshot::Sender<Option<serde_json::Value>>,
    },
    GetHistory {
        reply: oneshot::Sender<Vec<String>>,
    },
    Snapshot {
        name: String,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    ListSnapshots {
        reply: oneshot::Sender<Vec<String>>,
    },
    RestoreSnapshot {
        name: String,
        reply: oneshot::Sender<Result<crate::ops::DocDelta, CoreError>>,
    },
    /// Clone the active doc (new id), sharing the decoded base buffer.
    /// `path` set = persist a copy for that file even if it is not the open image.
    VirtualCopy {
        path: Option<String>,
        reply: oneshot::Sender<Result<String, CoreError>>,
    },
    SwitchDoc {
        doc_id: String,
        reply: oneshot::Sender<Result<crate::ops::DocDelta, CoreError>>,
    },
    ListDocs {
        reply: oneshot::Sender<Vec<DocRef>>,
    },
    /// Drop a virtual copy from the open image. Master (first doc) cannot be removed.
    DeleteVirtualCopy {
        doc_id: String,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    ApplyGradeToPaths {
        paths: Vec<String>,
        modules: crate::doc::ModuleParams,
        lut_file: Option<String>,
        reply: oneshot::Sender<Result<u32, CoreError>>,
    },
    /// Extract chosen modules' params as a PartialDoc (preset save).
    SavePreset {
        modules: Vec<String>,
        reply: oneshot::Sender<Result<serde_json::Value, CoreError>>,
    },
    /// Force sidecar write now (close/quit path).
    FlushSidecar {
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    // ---- Phase 3 ----
    /// Histogram + clip stats over the latest rendered frame (contract F1).
    GetStats {
        reply: oneshot::Sender<Option<FrameStats>>,
    },
    /// WB eyedropper: neutralize the sampled (normalized) point via two
    /// guard-walled SetParam ops recorded as one undoable step.
    WbFromPoint {
        x: f32,
        y: f32,
        reply: oneshot::Sender<Result<crate::ops::DocDelta, CoreError>>,
    },
    // ---- Phase 4 ----
    /// Toggle the viewport mask overlay (None = off).
    SetMaskOverlay {
        id: Option<String>,
        /// Tint strength 0..1; None keeps previous strength.
        strength: Option<f32>,
        /// 0 red, 1 white, 2 black, 3 color-on-B&W; None keeps previous.
        mode: Option<u32>,
        reply: oneshot::Sender<()>,
    },
    /// Before/after: render the un-edited base (edit chain bypassed) while on.
    SetPreviewBypass {
        on: bool,
        reply: oneshot::Sender<()>,
    },
    /// Display look: 0 = Neutral, 1 = Camera/punchy, 2 = Filmic/AgX, 4 = Original, 8 = Linear/None.
    SetDisplayLook {
        look: u32,
        reply: oneshot::Sender<()>,
    },
    /// Toggle highlight / shadow clipping blinkies on the viewport present pass.
    SetClipWarnings {
        hi: bool,
        lo: bool,
        reply: oneshot::Sender<()>,
    },
    /// View-only soft proof: 0 off, 1 sRGB, 2 Display P3, 3 Adobe RGB, 4 ProPhoto.
    SetProofTarget {
        space: u32,
        gamut: bool,
        reply: oneshot::Sender<()>,
    },
    // ---- Phase 5: catalog ----
    ImportFolder {
        path: PathBuf,
        reply: oneshot::Sender<Result<u64, CoreError>>,
    },
    ScanImportFolder {
        path: PathBuf,
        reply: oneshot::Sender<Result<Vec<crate::catalog::ImportCandidate>, CoreError>>,
    },
    ImportSelected {
        root: PathBuf,
        paths: Vec<PathBuf>,
        reply: oneshot::Sender<Result<u64, CoreError>>,
    },
    GetAssetDetail {
        id: i64,
        reply: oneshot::Sender<Result<Option<crate::catalog::AssetDetail>, CoreError>>,
    },
    ListAlbums {
        reply: oneshot::Sender<Result<Vec<crate::catalog::AlbumItem>, CoreError>>,
    },
    CreateAlbum {
        name: String,
        reply: oneshot::Sender<Result<i64, CoreError>>,
    },
    DeleteAlbum {
        id: i64,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    AddToAlbum {
        album_id: i64,
        asset_ids: Vec<i64>,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    RemoveFromAlbum {
        album_id: i64,
        asset_ids: Vec<i64>,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    SetCameraProfile {
        profile_file: String,
        reply: oneshot::Sender<Result<ImageMeta, CoreError>>,
    },
    /// Load (Some path) or clear (None) the current image's 3D look LUT.
    SetLut {
        path: Option<String>,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    ListLooks {
        reply: oneshot::Sender<Vec<crate::look::LookInfo>>,
    },
    SeekVideo {
        frame: u32,
        reply: oneshot::Sender<Result<ImageMeta, CoreError>>,
    },
    /// Change the demosaic algorithm for the current image and re-decode it.
    SetDemosaic {
        algo: String,
        reply: oneshot::Sender<Result<ImageMeta, CoreError>>,
    },
    GetGrid {
        query: crate::catalog::GridQuery,
        reply: oneshot::Sender<Result<Vec<crate::catalog::GridItem>, CoreError>>,
    },
    ListFolders {
        reply: oneshot::Sender<Result<Vec<crate::catalog::FolderItem>, CoreError>>,
    },
    ForgetFolder {
        root: String,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    SetAssetMeta {
        ids: Vec<i64>,
        patch: crate::catalog::MetaPatch,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    RebuildIndex {
        reply: oneshot::Sender<Result<u64, CoreError>>,
    },
    GetPreviewFile {
        id: i64,
        tier: String, // "t" | "p"
        reply: oneshot::Sender<Option<PathBuf>>,
    },
    // ---- Phase 6: assistant eyes ----
    /// Small current-state preview as JPEG (the only pixels that ever
    /// leave the device — privacy contract 6.7).
    RenderPreviewJpeg {
        max_dim: u32,
        reply: oneshot::Sender<Result<Vec<u8>, CoreError>>,
    },
    /// Working-space + display color at a normalized point (see-tool).
    SampleColor {
        x: f32,
        y: f32,
        reply: oneshot::Sender<Result<SampledColor, CoreError>>,
    },
    /// Object-pick UI: saliency contours the user can click to mask.
    ProposeObjectMasks {
        reply: oneshot::Sender<Result<Vec<crate::segment::ObjectProposal>, CoreError>>,
    },
    /// Crop-tool auto-level: dominant near-horizontal/vertical edge deviation
    /// (degrees, ORIGINAL image space, mod-90 folded into [-45, 45)).
    /// Returns 0.0 when no dominant line direction is found.
    AutoLevel {
        reply: oneshot::Sender<Result<f32, CoreError>>,
    },
    // ---- Phase 7: export + presets + perf ----
    /// Export the OPEN image: tiled full-res linear render on the actor,
    /// then transform/encode on a worker. Replies when the file is written.
    ExportImage {
        settings: crate::export::ExportSettings,
        reply: oneshot::Sender<Result<String, CoreError>>,
    },
    /// Export many images without touching the open one: each is decoded +
    /// segmented on a worker, tile-rendered on the actor, encoded on a
    /// worker. Replies with the accepted queue length; completion arrives as
    /// EngineEvent::ExportBatchDone.
    ExportBatch {
        items: Vec<crate::export::BatchExportItem>,
        settings: crate::export::ExportSettings,
        reply: oneshot::Sender<Result<u32, CoreError>>,
    },
    ExportBatchCancel {
        reply: oneshot::Sender<()>,
    },
    SavePresetToDisk {
        name: String,
        modules: Vec<String>,
        grade: Option<crate::doc::ModuleParams>,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    ListPresets {
        reply: oneshot::Sender<Vec<String>>,
    },
    ListPresetCatalog {
        reply: oneshot::Sender<Vec<crate::doc::PresetCatalogEntry>>,
    },
    ApplyPresetByName {
        name: String,
        reply: oneshot::Sender<Result<crate::ops::DocDelta, CoreError>>,
    },
    GetPerfStats {
        reply: oneshot::Sender<PerfStats>,
    },
    // ---- Denoise (classical settings via ApplyOp; AI async) ----
    DenoiseEstimateProfile {
        reply: oneshot::Sender<Result<crate::denoise::NoiseProfile, CoreError>>,
    },
    DenoiseModelsList {
        reply: oneshot::Sender<Vec<crate::denoise::ai::ModelInfo>>,
    },
    DenoiseAiStart {
        reply: oneshot::Sender<Result<u64, CoreError>>,
    },
    DenoiseAiCancel {
        job: u64,
        reply: oneshot::Sender<()>,
    },
    /// Revert the working master to a plain re-decode (AI Denoise unchecked).
    DenoiseAiReset {
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    // ---- internal (workers → engine) ----
    PreviewDone {
        generation: u64,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    },
    DecodeDone {
        generation: u64,
        result: Result<Box<DecodedPayload>, CoreError>,
    },
    SegmentDone {
        generation: u64,
        mask_id: String,
        source_hash: u64,
        result: Result<crate::segment::Mask01, CoreError>,
    },
    ImportFileDone {
        import_id: u64,
        file: Box<Result<crate::catalog::ImportedFile, CoreError>>,
    },
    ImportFinished {
        import_id: u64,
    },
    /// Continue incremental tiled export (one tile per actor tick).
    ExportStep,
    /// Batch worker finished decode + segmentation for queue slot `index`.
    BatchImagePrepared {
        batch_id: u64,
        index: usize,
        result: Result<Box<BatchPrepared>, CoreError>,
    },
    /// Batch encode worker finished writing queue slot `index`.
    BatchEncodeDone {
        batch_id: u64,
        index: usize,
        result: Result<String, CoreError>,
    },
    /// Continue incremental tiled batch render (one tile per actor tick).
    ExportBatchStep,
    /// AI denoise worker progress (forwarded to the UI as DenoiseProgress).
    DenoiseJobProgress {
        job: u64,
        pct: f32,
        tile: u32,
        tiles: u32,
    },
    /// AI denoise worker terminal state.
    DenoiseJobFinished {
        job: u64,
        outcome: DenoiseOutcome,
    },
}

/// Terminal outcome carried from the denoise worker thread.
pub enum DenoiseOutcome {
    Done {
        rgb: std::sync::Arc<Vec<f32>>,
        width: u32,
        height: u32,
    },
    Cancelled,
    Failed {
        message: String,
    },
}

/// Carried from the batch-prepare worker: decoded working image plus
/// everything the render needs that `open_image` would normally set up
/// (sidecar doc, DCP, LUT, segmentation masks).
pub struct BatchPrepared {
    pub payload: DecodedPayload,
    pub doc: crate::doc::EditDoc,
    pub dcp: Option<std::sync::Arc<crate::profile::DcpProfile>>,
    pub lut: Option<std::sync::Arc<crate::lut::CubeLut>>,
    /// (mask id, source hash, inferred mask) for doc masks of type "segmented".
    pub masks: Vec<(String, u64, crate::segment::Mask01)>,
    /// Unique stem for a virtual copy (`DSC_001_vc-1`); None = source stem.
    pub output_stem: Option<String>,
}

/// Carried from the decode worker thread: f16-packed working master +
/// retained small CPU copy (histogram/segmentation/fallback) + metadata.
pub struct DecodedPayload {
    pub rgba_f16: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub small_cpu: std::sync::Arc<crate::image::RgbF32Buf>,
    /// Full-res clean master (pre-retouch). Kept so heal spots can rebuild.
    pub clean_rgb: std::sync::Arc<crate::image::RgbF32Buf>,
    pub meta: ImageMeta,
}

impl DecodedPayload {
    pub fn from_decoded(img: DecodedImage) -> Self {
        let rgba_f16 = img.working.to_rgba_f16_bytes();
        let small_cpu = std::sync::Arc::new(img.working.downscale_to(2048));
        let clean_rgb = std::sync::Arc::new(img.working);
        Self {
            rgba_f16,
            width: clean_rgb.width as u32,
            height: clean_rgb.height as u32,
            small_cpu,
            clean_rgb,
            meta: img.meta,
        }
    }
}