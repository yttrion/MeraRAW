//! Engine actor: dedicated thread + current-thread tokio runtime owning all
//! heavy state (GPU, images, the canonical EditDoc). Single owner. Commands
//! talk via mpsc + oneshot; decode runs on worker threads posting results
//! back as internal messages; renders are throttled (op storms coalesce —
//! latest doc wins) and sidecar writes happen on settle.

use crate::doc::EditDoc;
use crate::error::CoreError;
use crate::gpu::display::{upload_working_texture, ViewParams};
use crate::gpu::GpuContext;
use crate::graph::RenderGraph;
use crate::image::RgbF32Buf;
use crate::message::{
    BatchPrepared, DecodedPayload, EngineEvent, EngineInfo, EngineMsg, EngineStatus, Frame,
    FrameInfo,
};
use crate::ops::{self, DocDelta, History, Op};
use crate::profile::DcpProfile;
use crate::raw::ImageMeta;
use crate::sidecar;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

mod catalog_ops;
mod decode;
mod denoise_ops;
mod doc_ops;
mod export;
mod render;
mod retouch_ops;

use doc_ops::{list_preset_catalog, list_presets};

// Small enough to feel immediate on a slider drag, large enough to coalesce
// op storms. Once scheduled, the deadline is not pushed back by later ops;
// otherwise a continuous drag can starve rendering until the pointer stops.
const RENDER_THROTTLE: Duration = Duration::from_millis(3);
const SETTLE_DEBOUNCE: Duration = Duration::from_millis(600);

fn next_render_deadline(current: Option<Instant>, now: Instant) -> Instant {
    current.unwrap_or(now + RENDER_THROTTLE)
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine channel closed")]
    ChannelClosed,
    #[error("engine dropped reply")]
    ReplyDropped,
}

#[derive(Clone)]
pub struct EngineHandle {
    tx: mpsc::Sender<EngineMsg>,
}

impl EngineHandle {
    async fn request<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<T>) -> EngineMsg,
    ) -> Result<T, EngineError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(build(tx))
            .await
            .map_err(|_| EngineError::ChannelClosed)?;
        rx.await.map_err(|_| EngineError::ReplyDropped)
    }

    pub async fn ping(&self) -> Result<EngineStatus, EngineError> {
        self.request(|reply| EngineMsg::Ping { reply }).await
    }

    pub async fn info(&self) -> Result<EngineInfo, EngineError> {
        self.request(|reply| EngineMsg::Info { reply }).await
    }

    pub async fn test_frame(&self, width: u32, height: u32) -> Result<Frame, EngineError> {
        self.request(|reply| EngineMsg::TestFrame {
            width,
            height,
            reply,
        })
        .await
    }

    pub async fn open_image(
        &self,
        path: PathBuf,
        doc_id: Option<String>,
    ) -> Result<Result<ImageMeta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::OpenImage {
            path,
            doc_id,
            reply,
        })
        .await
    }

    pub async fn request_frame(
        &self,
        view: ViewParams,
    ) -> Result<Result<FrameInfo, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::RequestFrame { view, reply })
            .await
    }

    pub async fn get_frame(&self) -> Result<Option<Frame>, EngineError> {
        self.request(|reply| EngineMsg::GetFrame { reply }).await
    }

    pub async fn get_metadata(&self) -> Result<Option<ImageMeta>, EngineError> {
        self.request(|reply| EngineMsg::GetMetadata { reply }).await
    }

    pub async fn close_image(&self) -> Result<(), EngineError> {
        self.request(|reply| EngineMsg::CloseImage { reply }).await
    }

    pub async fn apply_op(&self, op: Op) -> Result<Result<DocDelta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ApplyOp {
            op,
            live: false,
            reply,
        })
        .await
    }

    /// Live (interactive drag) variant — coalesced into one undo entry per
    /// gesture; the committing `apply_op` closes the gesture.
    pub async fn apply_op_live(&self, op: Op) -> Result<Result<DocDelta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ApplyOp {
            op,
            live: true,
            reply,
        })
        .await
    }

    pub async fn undo(&self) -> Result<Result<DocDelta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::Undo { reply }).await
    }

    pub async fn redo(&self) -> Result<Result<DocDelta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::Redo { reply }).await
    }

    pub async fn get_doc(&self) -> Result<Option<serde_json::Value>, EngineError> {
        self.request(|reply| EngineMsg::GetDoc { reply }).await
    }

    pub async fn get_history(&self) -> Result<Vec<String>, EngineError> {
        self.request(|reply| EngineMsg::GetHistory { reply }).await
    }

    pub async fn snapshot(&self, name: String) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::Snapshot { name, reply })
            .await
    }

    pub async fn list_snapshots(&self) -> Result<Vec<String>, EngineError> {
        self.request(|reply| EngineMsg::ListSnapshots { reply })
            .await
    }

    pub async fn restore_snapshot(
        &self,
        name: String,
    ) -> Result<Result<DocDelta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::RestoreSnapshot { name, reply })
            .await
    }

    pub async fn virtual_copy(
        &self,
        path: Option<String>,
    ) -> Result<Result<String, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::VirtualCopy { path, reply })
            .await
    }

    pub async fn switch_doc(
        &self,
        doc_id: String,
    ) -> Result<Result<DocDelta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::SwitchDoc { doc_id, reply })
            .await
    }

    pub async fn list_docs(&self) -> Result<Vec<crate::message::DocRef>, EngineError> {
        self.request(|reply| EngineMsg::ListDocs { reply }).await
    }

    pub async fn delete_virtual_copy(
        &self,
        doc_id: String,
    ) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::DeleteVirtualCopy { doc_id, reply })
            .await
    }

    pub async fn apply_grade_to_paths(
        &self,
        paths: Vec<String>,
        modules: crate::doc::ModuleParams,
        lut_file: Option<String>,
    ) -> Result<Result<u32, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ApplyGradeToPaths {
            paths,
            modules,
            lut_file,
            reply,
        })
        .await
    }

    pub async fn save_preset(
        &self,
        modules: Vec<String>,
    ) -> Result<Result<serde_json::Value, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::SavePreset { modules, reply })
            .await
    }

    pub async fn flush_sidecar(&self) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::FlushSidecar { reply })
            .await
    }

    pub async fn get_stats(&self) -> Result<Option<crate::message::FrameStats>, EngineError> {
        self.request(|reply| EngineMsg::GetStats { reply }).await
    }

    pub async fn wb_from_point(
        &self,
        x: f32,
        y: f32,
    ) -> Result<Result<DocDelta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::WbFromPoint { x, y, reply })
            .await
    }

    pub async fn set_mask_overlay(
        &self,
        id: Option<String>,
        strength: Option<f32>,
        mode: Option<u32>,
    ) -> Result<(), EngineError> {
        self.request(|reply| EngineMsg::SetMaskOverlay {
            id,
            strength,
            mode,
            reply,
        })
        .await
    }

    pub async fn set_preview_bypass(&self, on: bool) -> Result<(), EngineError> {
        self.request(|reply| EngineMsg::SetPreviewBypass { on, reply })
            .await
    }

    pub async fn set_display_look(&self, look: u32) -> Result<(), EngineError> {
        self.request(|reply| EngineMsg::SetDisplayLook { look, reply })
            .await
    }

    pub async fn set_clip_warnings(&self, hi: bool, lo: bool) -> Result<(), EngineError> {
        self.request(|reply| EngineMsg::SetClipWarnings { hi, lo, reply })
            .await
    }

    pub async fn set_proof_target(&self, space: u32, gamut: bool) -> Result<(), EngineError> {
        self.request(|reply| EngineMsg::SetProofTarget {
            space,
            gamut,
            reply,
        })
        .await
    }

    // ---- Phase 5: catalog ----

    pub async fn import_folder(
        &self,
        path: PathBuf,
    ) -> Result<Result<u64, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ImportFolder { path, reply })
            .await
    }

    pub async fn scan_import_folder(
        &self,
        path: PathBuf,
    ) -> Result<Result<Vec<crate::catalog::ImportCandidate>, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ScanImportFolder { path, reply })
            .await
    }

    pub async fn import_selected(
        &self,
        root: PathBuf,
        paths: Vec<PathBuf>,
    ) -> Result<Result<u64, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ImportSelected { root, paths, reply })
            .await
    }

    pub async fn get_asset_detail(
        &self,
        id: i64,
    ) -> Result<Result<Option<crate::catalog::AssetDetail>, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::GetAssetDetail { id, reply })
            .await
    }

    pub async fn list_albums(
        &self,
    ) -> Result<Result<Vec<crate::catalog::AlbumItem>, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ListAlbums { reply }).await
    }

    pub async fn create_album(&self, name: String) -> Result<Result<i64, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::CreateAlbum { name, reply })
            .await
    }

    pub async fn delete_album(&self, id: i64) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::DeleteAlbum { id, reply })
            .await
    }

    pub async fn add_to_album(
        &self,
        album_id: i64,
        asset_ids: Vec<i64>,
    ) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::AddToAlbum {
            album_id,
            asset_ids,
            reply,
        })
        .await
    }

    pub async fn remove_from_album(
        &self,
        album_id: i64,
        asset_ids: Vec<i64>,
    ) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::RemoveFromAlbum {
            album_id,
            asset_ids,
            reply,
        })
        .await
    }

    pub async fn set_camera_profile(
        &self,
        profile_file: String,
    ) -> Result<Result<ImageMeta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::SetCameraProfile {
            profile_file,
            reply,
        })
        .await
    }

    /// Load (Some) or clear (None) the current image's 3D look LUT (`.cube`).
    pub async fn set_lut(
        &self,
        path: Option<String>,
    ) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::SetLut { path, reply })
            .await
    }

    pub async fn list_looks(&self) -> Result<Vec<crate::look::LookInfo>, EngineError> {
        self.request(|reply| EngineMsg::ListLooks { reply }).await
    }

    pub async fn seek_video(
        &self,
        frame: u32,
    ) -> Result<Result<ImageMeta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::SeekVideo { frame, reply })
            .await
    }

    /// Change the demosaic algorithm and re-decode the current image.
    pub async fn set_demosaic(
        &self,
        algo: String,
    ) -> Result<Result<ImageMeta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::SetDemosaic { algo, reply })
            .await
    }

    pub async fn get_grid(
        &self,
        query: crate::catalog::GridQuery,
    ) -> Result<Result<Vec<crate::catalog::GridItem>, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::GetGrid { query, reply })
            .await
    }

    pub async fn list_folders(
        &self,
    ) -> Result<Result<Vec<crate::catalog::FolderItem>, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ListFolders { reply }).await
    }

    pub async fn forget_folder(&self, root: String) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ForgetFolder { root, reply })
            .await
    }

    pub async fn set_asset_meta(
        &self,
        ids: Vec<i64>,
        patch: crate::catalog::MetaPatch,
    ) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::SetAssetMeta { ids, patch, reply })
            .await
    }

    pub async fn rebuild_index(&self) -> Result<Result<u64, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::RebuildIndex { reply })
            .await
    }

    pub async fn get_preview_file(
        &self,
        id: i64,
        tier: String,
    ) -> Result<Option<PathBuf>, EngineError> {
        self.request(|reply| EngineMsg::GetPreviewFile { id, tier, reply })
            .await
    }

    // ---- Phase 6: assistant eyes ----

    pub async fn render_preview_jpeg(
        &self,
        max_dim: u32,
    ) -> Result<Result<Vec<u8>, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::RenderPreviewJpeg { max_dim, reply })
            .await
    }

    pub async fn sample_color(
        &self,
        x: f32,
        y: f32,
    ) -> Result<Result<crate::message::SampledColor, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::SampleColor { x, y, reply })
            .await
    }

    pub async fn propose_object_masks(
        &self,
    ) -> Result<Result<Vec<crate::segment::ObjectProposal>, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ProposeObjectMasks { reply })
            .await
    }

    pub async fn auto_level(&self) -> Result<Result<f32, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::AutoLevel { reply }).await
    }

    // ---- Phase 7 ----

    pub async fn export_image(
        &self,
        settings: crate::export::ExportSettings,
    ) -> Result<Result<String, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ExportImage { settings, reply })
            .await
    }

    /// Queue a batch export. Replies with the accepted queue length;
    /// progress + completion arrive as ExportBatchProgress/ExportBatchDone.
    pub async fn export_batch(
        &self,
        items: Vec<crate::export::BatchExportItem>,
        settings: crate::export::ExportSettings,
    ) -> Result<Result<u32, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ExportBatch {
            items,
            settings,
            reply,
        })
        .await
    }

    pub async fn export_batch_cancel(&self) -> Result<(), EngineError> {
        self.request(|reply| EngineMsg::ExportBatchCancel { reply })
            .await
    }

    pub async fn save_preset_to_disk(
        &self,
        name: String,
        modules: Vec<String>,
        grade: Option<crate::doc::ModuleParams>,
    ) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::SavePresetToDisk {
            name,
            modules,
            grade,
            reply,
        })
        .await
    }

    pub async fn list_presets(&self) -> Result<Vec<String>, EngineError> {
        self.request(|reply| EngineMsg::ListPresets { reply }).await
    }

    pub async fn list_preset_catalog(
        &self,
    ) -> Result<Vec<crate::doc::PresetCatalogEntry>, EngineError> {
        self.request(|reply| EngineMsg::ListPresetCatalog { reply })
            .await
    }

    pub async fn apply_preset_by_name(
        &self,
        name: String,
    ) -> Result<Result<DocDelta, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::ApplyPresetByName { name, reply })
            .await
    }

    pub async fn get_perf_stats(&self) -> Result<crate::message::PerfStats, EngineError> {
        self.request(|reply| EngineMsg::GetPerfStats { reply })
            .await
    }

    pub async fn denoise_estimate_profile(
        &self,
    ) -> Result<Result<crate::denoise::NoiseProfile, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::DenoiseEstimateProfile { reply })
            .await
    }

    pub async fn denoise_models_list(
        &self,
    ) -> Result<Vec<crate::denoise::ai::ModelInfo>, EngineError> {
        self.request(|reply| EngineMsg::DenoiseModelsList { reply })
            .await
    }

    pub async fn denoise_ai_start(&self) -> Result<Result<u64, CoreError>, EngineError> {
        self.request(|reply| EngineMsg::DenoiseAiStart { reply })
            .await
    }

    pub async fn denoise_ai_reset(&self) -> Result<Result<(), CoreError>, EngineError> {
        self.request(|reply| EngineMsg::DenoiseAiReset { reply })
            .await
    }

    pub async fn denoise_ai_cancel(&self, job: u64) -> Result<(), EngineError> {
        self.request(|reply| EngineMsg::DenoiseAiCancel { job, reply })
            .await
    }
}

pub fn spawn_with_events(events: Option<mpsc::UnboundedSender<EngineEvent>>) -> EngineHandle {
    let (tx, rx) = mpsc::channel::<EngineMsg>(256);
    let self_tx = tx.clone();
    std::thread::Builder::new()
        .name("meratech-engine".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("engine runtime");
            rt.block_on(run(rx, self_tx, events));
        })
        .expect("spawn engine thread");
    EngineHandle { tx }
}

pub fn spawn() -> EngineHandle {
    spawn_with_events(None)
}

struct CurrentImage {
    path: PathBuf,
    meta: ImageMeta,
    /// Parsed DCP for viewport look application (matrix is baked in at decode).
    dcp_profile: Option<std::sync::Arc<DcpProfile>>,
    /// Parsed 3D look LUT (cached; keyed by doc.meta.lut_file). None = no LUT.
    lut_cube: Option<std::sync::Arc<crate::lut::CubeLut>>,
    /// (texture, view, w, h) — the working master (contract A4).
    working: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    /// Retained downsized CPU copy (histogram / segmentation / fallback).
    small_cpu: Option<std::sync::Arc<RgbF32Buf>>,
    /// Full-res clean master (pre-retouch). None until decode finishes.
    clean_rgb: Option<std::sync::Arc<RgbF32Buf>>,
    /// All docs for this image: [0] = primary, rest = virtual copies.
    docs: Vec<EditDoc>,
    active_doc: usize,
    history: History,
    snapshots: Vec<(String, EditDoc)>,
    /// Doc state at the start of an interactive (live) gesture; on commit it
    /// becomes the single undo entry for the whole drag. None when idle.
    gesture_before: Option<EditDoc>,
    doc_dirty: bool, // unsaved sidecar changes (primary doc only)
    /// Segmentation cache: mask id → (source hash, uploaded small mask).
    masks_gpu: std::collections::HashMap<String, (u64, wgpu::Texture)>,
    pending_segments: std::collections::HashSet<String>,
}

impl CurrentImage {
    fn doc(&self) -> &EditDoc {
        &self.docs[self.active_doc]
    }
    fn doc_mut(&mut self) -> &mut EditDoc {
        &mut self.docs[self.active_doc]
    }
    fn as_shot_cct(&self) -> f32 {
        self.meta.estimated_cct.unwrap_or(5200.0)
    }
    fn delta(
        &self,
        label: String,
        new_mask_id: Option<String>,
        new_retouch_id: Option<String>,
    ) -> DocDelta {
        let (u, r) = self.history.depths();
        DocDelta {
            doc: self.doc().to_json(),
            label,
            undo_depth: u,
            redo_depth: r,
            new_mask_id,
            new_retouch_id,
        }
    }
}

struct Engine {
    gpu: Option<GpuContext>,
    graph: Option<RenderGraph>,
    events: Option<mpsc::UnboundedSender<EngineEvent>>,
    self_tx: mpsc::Sender<EngineMsg>,
    current: Option<CurrentImage>,
    latest_frame: Option<Frame>,
    frame_version: u64,
    generation: u64,
    last_view: Option<ViewParams>,
    render_at: Option<Instant>,
    settle_at: Option<Instant>,
    overlay_mask: Option<String>,
    /// Overlay tint strength (0..1), default 0.55.
    overlay_strength: f32,
    /// 0 red, 1 white, 2 black, 3 color-on-B&W.
    overlay_mode: u32,
    catalog: Option<crate::catalog::Catalog>,
    import_state: Option<ImportState>,
    /// Before/after: when true, render the un-edited base.
    preview_bypass: bool,
    /// Display look applied to preview + export: 0 Neutral, 1 Camera, 2 AgX, 4 Original.
    display_look: u32,
    /// Viewport clipping blinkies (highlight / shadow).
    clip_hi: bool,
    clip_lo: bool,
    /// Soft-proof target: 0 off, 1 sRGB, 2 P3, 3 Adobe, 4 ProPhoto.
    proof_space: u32,
    proof_gamut: bool,
    /// Dedicated graph for assistant previews (own small caches — never
    /// thrashes the viewport graph).
    preview_graph: Option<RenderGraph>,
    /// Dedicated graph for tiled export (tile-sized caches).
    export_graph: Option<RenderGraph>,
    export_job: Option<export::ExportJob>,
    /// Batch export state machine (spec 7.3); shares export_graph with
    /// single export — the two are mutually exclusive.
    export_batch: Option<export::BatchState>,
    /// Monotonic batch id — stale worker messages after cancel are dropped.
    batch_seq: u64,
    perf: crate::message::PerfStats,
    /// AI denoise job manager (tiled inference + cache).
    ai_denoise: crate::denoise::ai::AiJobManager,
    /// (job id, image path) of the in-flight AI denoise job — results for a
    /// different job or a since-closed image are dropped.
    pending_denoise: Option<(u64, PathBuf)>,
}

struct ImportState {
    id: u64,
    total: u64,
    done: u64,
    /// further roots to import once this one finishes (rebuild path)
    queued_roots: Vec<PathBuf>,
}

async fn run(
    mut rx: mpsc::Receiver<EngineMsg>,
    self_tx: mpsc::Sender<EngineMsg>,
    events: Option<mpsc::UnboundedSender<EngineEvent>>,
) {
    let gpu = match GpuContext::init().await {
        Ok(g) => {
            tracing::info!(adapter = %g.adapter_name(), backend = %g.backend_name(), "gpu ready");
            Some(g)
        }
        Err(e) => {
            tracing::error!(error = %e, "gpu init failed; engine continues without gpu");
            None
        }
    };
    // Worker → engine adapter: denoise notifications become internal actor
    // messages (fired from the worker thread only, so blocking_send is safe).
    let mut ai_denoise = crate::denoise::ai::AiJobManager::new();
    {
        let tx = self_tx.clone();
        ai_denoise.set_notify(move |n| {
            use crate::denoise::ai::JobNotify as N;
            use crate::message::DenoiseOutcome as O;
            let msg = match n {
                N::Progress(p) => EngineMsg::DenoiseJobProgress {
                    job: p.job.0,
                    pct: p.pct,
                    tile: p.tile,
                    tiles: p.tiles,
                },
                N::Done {
                    job,
                    rgb,
                    width,
                    height,
                } => EngineMsg::DenoiseJobFinished {
                    job: job.0,
                    outcome: O::Done {
                        rgb,
                        width: width as u32,
                        height: height as u32,
                    },
                },
                N::Cancelled { job } => EngineMsg::DenoiseJobFinished {
                    job: job.0,
                    outcome: O::Cancelled,
                },
                N::Failed { job, message } => EngineMsg::DenoiseJobFinished {
                    job: job.0,
                    outcome: O::Failed { message },
                },
            };
            let _ = tx.blocking_send(msg);
        });
    }
    let mut engine = Engine {
        gpu,
        graph: None,
        events,
        self_tx,
        current: None,
        latest_frame: None,
        frame_version: 0,
        generation: 0,
        last_view: None,
        render_at: None,
        settle_at: None,
        overlay_mask: None,
        overlay_strength: 0.55,
        overlay_mode: 0,
        catalog: None,
        import_state: None,
        preview_bypass: false,
        display_look: 1, // Camera by default (matches export; preview now WYSIWYG)
        clip_hi: false,
        clip_lo: false,
        proof_space: 0,
        proof_gamut: false,
        preview_graph: None,
        export_graph: None,
        export_job: None,
        export_batch: None,
        batch_seq: 0,
        perf: Default::default(),
        ai_denoise,
        pending_denoise: None,
    };
    tracing::info!("engine actor up");

    loop {
        let next_deadline = [engine.render_at, engine.settle_at]
            .into_iter()
            .flatten()
            .min();
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(m) => {
                        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            engine.handle(m);
                        }))
                        .is_err();
                        if crashed {
                            tracing::error!("engine panic during message handling");
                            engine.export_job = None;
                            engine.export_batch = None;
                            engine.emit(EngineEvent::EngineCrashed {
                                message: "GPU/render panic — restart the app".into(),
                            });
                        }
                    }
                    None => break,
                }
            }
            _ = deadline_sleep(next_deadline) => {
                let now = Instant::now();
                if engine.render_at.is_some_and(|t| t <= now) {
                    engine.render_at = None;
                    engine.render_now();
                }
                if engine.settle_at.is_some_and(|t| t <= now) {
                    engine.settle_at = None;
                    engine.on_settle();
                }
            }
        }
    }
    tracing::info!("engine actor shut down");
}

async fn deadline_sleep(deadline: Option<Instant>) {
    match deadline {
        Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t)).await,
        None => std::future::pending().await,
    }
}

impl Engine {
    pub(super) fn emit(&self, ev: EngineEvent) {
        if let Some(tx) = &self.events {
            let _ = tx.send(ev);
        }
    }

    pub(super) fn next_version(&mut self) -> u64 {
        self.frame_version += 1;
        self.frame_version
    }

    pub(super) fn schedule_render(&mut self) {
        self.render_at = Some(next_render_deadline(self.render_at, Instant::now()));
    }

    pub(super) fn schedule_settle(&mut self) {
        self.settle_at = Some(Instant::now() + SETTLE_DEBOUNCE);
    }

    pub(super) fn handle(&mut self, msg: EngineMsg) {
        match msg {
            EngineMsg::Ping { reply } => {
                let _ = reply.send(EngineStatus {
                    alive: true,
                    gpu_ready: self.gpu.is_some(),
                    adapter: self.gpu.as_ref().map(|g| g.adapter_name()),
                });
            }
            EngineMsg::Info { reply } => {
                let _ = reply.send(EngineInfo {
                    gpu_adapter: self.gpu.as_ref().map(|g| g.adapter_name()),
                    gpu_backend: self.gpu.as_ref().map(|g| g.backend_name()),
                });
            }
            EngineMsg::TestFrame {
                width,
                height,
                reply,
            } => {
                let v = self.frame_version;
                let _ = reply.send(render_test_frame(width, height, v));
            }
            EngineMsg::OpenImage {
                path,
                doc_id,
                reply,
            } => self.open_image(path, doc_id, reply),
            EngineMsg::PreviewDone {
                generation,
                rgba,
                width,
                height,
            } => {
                if generation != self.generation {
                    return;
                }
                if self
                    .current
                    .as_ref()
                    .map(|c| c.working.is_some())
                    .unwrap_or(true)
                {
                    return;
                }
                let version = self.next_version();
                self.latest_frame = Some(Frame {
                    width,
                    height,
                    rgba,
                    version,
                });
                tracing::info!(version, width, height, "preview frame ready");
                self.emit(EngineEvent::PreviewReady { version });
                self.emit(EngineEvent::FrameReady { version });
            }
            EngineMsg::DecodeDone { generation, result } => {
                if generation != self.generation {
                    return;
                }
                match result {
                    Ok(payload) => self.finish_decode(*payload),
                    Err(e) => {
                        tracing::error!(error = %e, "decode failed");
                        self.emit(EngineEvent::DecodeError {
                            message: e.to_string(),
                        });
                    }
                }
            }
            EngineMsg::RequestFrame { view, reply } => {
                self.last_view = Some(view);
                let _ = reply.send(self.render_view(view));
            }
            EngineMsg::GetFrame { reply } => {
                let _ = reply.send(self.latest_frame.clone());
            }
            EngineMsg::GetMetadata { reply } => {
                let _ = reply.send(self.current.as_ref().map(|c| c.meta.clone()));
            }
            EngineMsg::CloseImage { reply } => {
                self.flush_sidecar_now();
                self.generation += 1;
                self.current = None;
                self.latest_frame = None;
                self.render_at = None;
                self.settle_at = None;
                if let Some(g) = &mut self.graph {
                    g.invalidate_all();
                }
                let _ = reply.send(());
            }
            // ---- ops ----
            EngineMsg::ApplyOp { op, live, reply } => {
                let result = self.do_apply_op(op, live);
                if let Ok(delta) = &result {
                    self.emit(EngineEvent::DocUpdated {
                        delta: serde_json::to_value(delta).unwrap_or_default(),
                    });
                }
                let _ = reply.send(result);
            }
            EngineMsg::Undo { reply } => {
                let _ = reply.send(self.do_undo(true));
            }
            EngineMsg::Redo { reply } => {
                let _ = reply.send(self.do_undo(false));
            }
            EngineMsg::GetDoc { reply } => {
                let _ = reply.send(self.current.as_ref().map(|c| c.doc().to_json()));
            }
            EngineMsg::GetHistory { reply } => {
                let _ = reply.send(
                    self.current
                        .as_ref()
                        .map(|c| c.history.labels())
                        .unwrap_or_default(),
                );
            }
            EngineMsg::Snapshot { name, reply } => {
                let _ = reply.send(match &mut self.current {
                    Some(c) => {
                        let doc = c.doc().clone();
                        c.snapshots.retain(|(n, _)| *n != name);
                        c.snapshots.push((name, doc));
                        Ok(())
                    }
                    None => Err(CoreError::NoImage),
                });
            }
            EngineMsg::ListSnapshots { reply } => {
                let _ = reply.send(
                    self.current
                        .as_ref()
                        .map(|c| c.snapshots.iter().map(|(n, _)| n.clone()).collect())
                        .unwrap_or_default(),
                );
            }
            EngineMsg::RestoreSnapshot { name, reply } => {
                let result = (|| {
                    let c = self.current.as_mut().ok_or(CoreError::NoImage)?;
                    let snap = c
                        .snapshots
                        .iter()
                        .find(|(n, _)| *n == name)
                        .map(|(_, d)| d.clone())
                        .ok_or_else(|| CoreError::InvalidOp(format!("no snapshot '{name}'")))?;
                    let before = c.doc().clone();
                    c.history.record(before, format!("restore '{name}'"));
                    *c.doc_mut() = snap;
                    c.doc_dirty = true;
                    Ok(c.delta(format!("restore '{name}'"), None, None))
                })();
                if result.is_ok() {
                    self.full_redraw();
                }
                if let Ok(delta) = &result {
                    self.emit(EngineEvent::DocUpdated {
                        delta: serde_json::to_value(delta).unwrap_or_default(),
                    });
                }
                let _ = reply.send(result);
            }
            EngineMsg::VirtualCopy { path, reply } => {
                let result = self.make_virtual_copy(path);
                let _ = reply.send(result);
            }
            EngineMsg::SwitchDoc { doc_id, reply } => {
                let result = (|| {
                    let c = self.current.as_mut().ok_or(CoreError::NoImage)?;
                    let idx = c
                        .docs
                        .iter()
                        .position(|d| d.doc_id == doc_id)
                        .ok_or_else(|| CoreError::InvalidOp(format!("no doc {doc_id}")))?;
                    c.active_doc = idx;
                    c.history.clear();
                    Ok(c.delta(format!("switch to {doc_id}"), None, None))
                })();
                if result.is_ok() {
                    self.full_redraw();
                }
                let _ = reply.send(result);
            }
            EngineMsg::ListDocs { reply } => {
                let list = self
                    .current
                    .as_ref()
                    .map(|c| {
                        c.docs
                            .iter()
                            .enumerate()
                            .map(|(i, d)| crate::message::DocRef {
                                doc_id: d.doc_id.clone(),
                                active: i == c.active_doc,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = reply.send(list);
            }
            EngineMsg::DeleteVirtualCopy { doc_id, reply } => {
                let result = self.delete_virtual_copy(&doc_id);
                let _ = reply.send(result);
            }
            EngineMsg::ApplyGradeToPaths {
                paths,
                modules,
                lut_file,
                reply,
            } => {
                let result = self.apply_grade_to_paths(paths, modules, lut_file);
                let _ = reply.send(result);
            }
            EngineMsg::SavePreset { modules, reply } => {
                let _ = reply.send(match &self.current {
                    Some(c) => {
                        let mut partial = crate::doc::PartialDoc {
                            modules: Default::default(),
                        };
                        for m in modules {
                            if let Some(params) = c.doc().modules.get(&m) {
                                partial.modules.insert(m, params.clone());
                            }
                        }
                        serde_json::to_value(&partial).map_err(|e| CoreError::Engine(e.to_string()))
                    }
                    None => Err(CoreError::NoImage),
                });
            }
            EngineMsg::FlushSidecar { reply } => {
                self.flush_sidecar_now();
                let _ = reply.send(Ok(()));
            }
            EngineMsg::GetStats { reply } => {
                let _ = reply.send(self.compute_stats());
            }
            EngineMsg::SetMaskOverlay {
                id,
                strength,
                mode,
                reply,
            } => {
                self.overlay_mask = id;
                if let Some(s) = strength {
                    self.overlay_strength = s.clamp(0.0, 1.0);
                }
                if let Some(m) = mode {
                    self.overlay_mode = m.min(3);
                }
                if let Some(g) = &mut self.graph {
                    g.invalidate_from_module("masks");
                }
                self.schedule_render();
                let _ = reply.send(());
            }
            EngineMsg::SetPreviewBypass { on, reply } => {
                if self.preview_bypass != on {
                    self.preview_bypass = on;
                    if let Some(g) = &mut self.graph {
                        g.invalidate_all();
                    }
                    self.render_now();
                }
                let _ = reply.send(());
            }
            EngineMsg::SetDisplayLook { look, reply } => {
                if self.display_look != look {
                    let was_dcp =
                        crate::profile::DcpProfile::applies_to_display_look(self.display_look);
                    let now_dcp = crate::profile::DcpProfile::applies_to_display_look(look);
                    self.display_look = look;
                    // Any look change requires re-running the present shader
                    // (it uses the look uniform). Also invalidate DCP path if
                    // switching between Camera and non-Camera.
                    if let Some(g) = &mut self.graph {
                        g.invalidate_all();
                    }
                    self.render_now();
                }
                let _ = reply.send(());
            }
            EngineMsg::SetClipWarnings { hi, lo, reply } => {
                if self.clip_hi != hi || self.clip_lo != lo {
                    self.clip_hi = hi;
                    self.clip_lo = lo;
                    if let Some(g) = &mut self.graph {
                        g.set_clip_warnings(hi, lo);
                    }
                    self.render_now();
                }
                let _ = reply.send(());
            }
            EngineMsg::SetProofTarget {
                space,
                gamut,
                reply,
            } => {
                let space = space.min(4);
                if self.proof_space != space || self.proof_gamut != gamut {
                    self.proof_space = space;
                    self.proof_gamut = gamut && space > 0;
                    if let Some(g) = &mut self.graph {
                        g.set_proof(self.proof_space, self.proof_gamut);
                    }
                    self.render_now();
                }
                let _ = reply.send(());
            }
            // ---- Phase 5: catalog ----
            EngineMsg::ImportFolder { path, reply } => {
                let _ = reply.send(self.start_import(path, None));
            }
            EngineMsg::ScanImportFolder { path, reply } => {
                let result = self
                    .catalog_mut()
                    .and_then(|c| c.scan_import_candidates(&path));
                let _ = reply.send(result);
            }
            EngineMsg::ImportSelected { root, paths, reply } => {
                let _ = reply.send(self.start_import(root, Some(paths)));
            }
            EngineMsg::GetAssetDetail { id, reply } => {
                let _ = reply.send(self.catalog_mut().and_then(|c| c.asset_detail(id)));
            }
            EngineMsg::ListAlbums { reply } => {
                let _ = reply.send(self.catalog_mut().and_then(|c| c.list_albums()));
            }
            EngineMsg::CreateAlbum { name, reply } => {
                let result = self.catalog_mut().and_then(|c| {
                    let id = c.create_album(&name)?;
                    Ok(id)
                });
                if result.is_ok() {
                    self.emit(EngineEvent::CatalogChanged);
                }
                let _ = reply.send(result);
            }
            EngineMsg::DeleteAlbum { id, reply } => {
                let result = self.catalog_mut().and_then(|c| c.delete_album(id));
                if result.is_ok() {
                    self.emit(EngineEvent::CatalogChanged);
                }
                let _ = reply.send(result);
            }
            EngineMsg::AddToAlbum {
                album_id,
                asset_ids,
                reply,
            } => {
                let result = self
                    .catalog_mut()
                    .and_then(|c| c.add_to_album(album_id, &asset_ids));
                if result.is_ok() {
                    self.emit(EngineEvent::CatalogChanged);
                }
                let _ = reply.send(result);
            }
            EngineMsg::RemoveFromAlbum {
                album_id,
                asset_ids,
                reply,
            } => {
                let result = self
                    .catalog_mut()
                    .and_then(|c| c.remove_from_album(album_id, &asset_ids));
                if result.is_ok() {
                    self.emit(EngineEvent::CatalogChanged);
                }
                let _ = reply.send(result);
            }
            EngineMsg::SetCameraProfile {
                profile_file,
                reply,
            } => {
                self.set_camera_profile(profile_file, reply);
            }
            EngineMsg::SetLut { path, reply } => {
                self.set_lut(path, reply);
            }
            EngineMsg::ListLooks { reply } => {
                let _ = reply.send(crate::look::all_looks());
            }
            EngineMsg::SeekVideo { frame, reply } => {
                self.seek_video(frame, reply);
            }
            EngineMsg::SetDemosaic { algo, reply } => {
                self.set_demosaic(algo, reply);
            }
            EngineMsg::GetGrid { query, reply } => {
                let _ = reply.send(self.catalog_mut().and_then(|c| c.grid(&query)));
            }
            EngineMsg::ListFolders { reply } => {
                let _ = reply.send(self.catalog_mut().and_then(|c| c.list_folders()));
            }
            EngineMsg::ForgetFolder { root, reply } => {
                let result = self.catalog_mut().and_then(|c| c.forget_folder(&root));
                if result.is_ok() {
                    self.emit(EngineEvent::CatalogChanged);
                }
                let _ = reply.send(result);
            }
            EngineMsg::SetAssetMeta { ids, patch, reply } => {
                let _ = reply.send(self.set_asset_meta(&ids, &patch));
            }
            EngineMsg::RebuildIndex { reply } => {
                let _ = reply.send(self.rebuild_index());
            }
            EngineMsg::GetPreviewFile { id, tier, reply } => {
                let p = self.catalog_mut().ok().and_then(|c| {
                    if tier == "p" {
                        let path = c.preview_path(id);
                        if path.exists() {
                            Some(path)
                        } else {
                            // Fall back to thumb; generate if needed.
                            c.ensure_thumb(id)
                        }
                    } else {
                        c.ensure_thumb(id)
                    }
                });
                let _ = reply.send(p);
            }
            EngineMsg::ImportFileDone { import_id, file } => {
                self.import_file_done(import_id, *file);
            }
            EngineMsg::ImportFinished { import_id } => {
                if let Some(st) = &self.import_state {
                    if st.id == import_id {
                        let total = st.done;
                        let queued = self.import_state.take().unwrap().queued_roots;
                        tracing::info!(total, "import finished");
                        self.emit(EngineEvent::ImportDone { total });
                        self.emit(EngineEvent::CatalogChanged);
                        // continue a multi-root rebuild
                        if let Some(next) = queued.first().cloned() {
                            match self.start_import(next, None) {
                                Ok(_) => {
                                    if let Some(st) = &mut self.import_state {
                                        st.queued_roots = queued[1..].to_vec();
                                    }
                                }
                                Err(e) => tracing::error!(error = %e, "queued import failed"),
                            }
                        }
                    }
                }
            }
            // ---- Phase 7 ----
            EngineMsg::ExportImage { settings, reply } => {
                self.export_image(settings, reply);
            }
            EngineMsg::ExportStep => {
                self.export_step();
            }
            EngineMsg::ExportBatch {
                items,
                settings,
                reply,
            } => {
                self.export_batch_start(items, settings, reply);
            }
            EngineMsg::ExportBatchCancel { reply } => {
                self.export_batch_cancel(reply);
            }
            EngineMsg::BatchImagePrepared {
                batch_id,
                index,
                result,
            } => {
                self.batch_image_prepared(batch_id, index, result);
            }
            EngineMsg::BatchEncodeDone {
                batch_id,
                index,
                result,
            } => {
                self.batch_encode_done(batch_id, index, result);
            }
            EngineMsg::ExportBatchStep => {
                self.export_batch_step();
            }
            EngineMsg::SavePresetToDisk {
                name,
                modules,
                grade,
                reply,
            } => {
                let _ = reply.send(self.save_preset_to_disk(&name, &modules, grade.as_ref()));
            }
            EngineMsg::ListPresets { reply } => {
                let _ = reply.send(list_presets());
            }
            EngineMsg::ListPresetCatalog { reply } => {
                let _ = reply.send(list_preset_catalog());
            }
            EngineMsg::ApplyPresetByName { name, reply } => {
                let result = self.apply_named_preset(&name);
                if let Ok(delta) = &result {
                    self.emit(EngineEvent::DocUpdated {
                        delta: serde_json::to_value(delta).unwrap_or_default(),
                    });
                }
                let _ = reply.send(result);
            }
            EngineMsg::GetPerfStats { reply } => {
                let _ = reply.send(self.perf.clone());
            }
            EngineMsg::DenoiseEstimateProfile { reply } => {
                let result = (|| {
                    let cur = self.current.as_ref().ok_or(CoreError::NoImage)?;
                    let small = cur
                        .small_cpu
                        .as_ref()
                        .ok_or_else(|| CoreError::Decode("image not fully decoded".into()))?;
                    let iso = cur.meta.iso.unwrap_or(800);
                    let seed = crate::denoise::NoiseProfile::from_iso(iso);
                    Ok(
                        crate::denoise::estimate_from_image(&small.data, small.width, small.height)
                            .unwrap_or(seed),
                    )
                })();
                let _ = reply.send(result);
            }
            EngineMsg::DenoiseModelsList { reply } => {
                let _ = reply.send(self.ai_denoise.models());
            }
            EngineMsg::DenoiseAiStart { reply } => {
                let _ = reply.send(self.denoise_ai_start());
            }
            EngineMsg::DenoiseAiCancel { job, reply } => {
                self.ai_denoise.cancel(crate::denoise::ai::JobId(job));
                let _ = reply.send(());
            }
            EngineMsg::DenoiseAiReset { reply } => {
                self.denoise_ai_reset(reply);
            }
            EngineMsg::DenoiseJobProgress {
                job,
                pct,
                tile,
                tiles,
            } => {
                self.emit(EngineEvent::DenoiseProgress {
                    job,
                    pct,
                    tile,
                    tiles,
                });
            }
            EngineMsg::DenoiseJobFinished { job, outcome } => {
                self.finish_denoise(job, outcome);
            }
            // ---- Phase 6: assistant eyes ----
            EngineMsg::RenderPreviewJpeg { max_dim, reply } => {
                let _ = reply.send(self.render_preview_jpeg(max_dim));
            }
            EngineMsg::SampleColor { x, y, reply } => {
                let _ = reply.send(self.sample_color(x, y));
            }
            EngineMsg::ProposeObjectMasks { reply } => {
                let result = (|| {
                    let cur = self.current.as_ref().ok_or(CoreError::NoImage)?;
                    let small = cur.small_cpu.as_ref().ok_or(CoreError::NoImage)?;
                    let input = small.downscale_to(768);
                    crate::segment::propose_objects(&input)
                })();
                let _ = reply.send(result);
            }
            EngineMsg::AutoLevel { reply } => {
                let _ = reply.send(self.auto_level());
            }
            EngineMsg::SegmentDone {
                generation,
                mask_id,
                source_hash,
                result,
            } => {
                if generation != self.generation {
                    return;
                }
                let Some(cur) = &mut self.current else { return };
                cur.pending_segments.remove(&mask_id);
                let parent_id = crate::segment::segment_parent_mask_id(&mask_id).to_string();
                // Drop stale results if the doc source changed while the worker ran.
                let expected = current_segment_hash(cur, &mask_id);
                if expected.is_some_and(|h| h != source_hash) {
                    tracing::debug!(
                        mask_id = %mask_id,
                        "dropping stale SegmentDone (source changed)"
                    );
                    self.maybe_start_export_after_masks();
                    return;
                }
                match result {
                    Ok(mask) => {
                        let Some(gpu) = &self.gpu else {
                            self.maybe_start_export_after_masks();
                            return;
                        };
                        let tex = crate::graph::upload_small_mask(
                            gpu,
                            &mask.data,
                            mask.width as u32,
                            mask.height as u32,
                        );
                        if mask_id.contains("#c") {
                            cur.masks_gpu.remove(&parent_id);
                        }
                        cur.masks_gpu.insert(mask_id.clone(), (source_hash, tex));
                        let parent_still_pending = cur.pending_segments.iter().any(|id| {
                            crate::segment::segment_parent_mask_id(id) == parent_id
                        });
                        if let Some(g) = &mut self.graph {
                            g.invalidate_from_module("masks");
                        }
                        self.schedule_render();
                        if !parent_still_pending {
                            self.emit(EngineEvent::MaskReady { id: parent_id });
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, mask_id, "segmentation failed");
                        self.emit(EngineEvent::MaskError {
                            id: parent_id,
                            message: e.to_string(),
                        });
                    }
                }
                self.maybe_start_export_after_masks();
            }
            EngineMsg::WbFromPoint { x, y, reply } => {
                let result = self.wb_from_point(x, y);
                if let Ok(delta) = &result {
                    self.emit(EngineEvent::DocUpdated {
                        delta: serde_json::to_value(delta).unwrap_or_default(),
                    });
                }
                let _ = reply.send(result);
            }
        }
    }
}

/// Recompute the source hash for a segmentation cache key against the live doc.
/// Used to drop stale SegmentDone results after the user changed the mask source.
fn current_segment_hash(cur: &CurrentImage, cache_key: &str) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let parent = crate::segment::segment_parent_mask_id(cache_key);
    let m = cur.doc().masks.iter().find(|m| m.id == parent)?;
    let (kind, source) = if cache_key.contains("#c") {
        let idx: usize = cache_key.rsplit("#c").next()?.parse().ok()?;
        let comps = m.source.get("components")?.as_array()?;
        let comp = comps.get(idx)?;
        let src = comp.get("source").cloned().unwrap_or_else(|| comp.clone());
        if src.get("type").and_then(|t| t.as_str()) != Some("segmented") {
            return None;
        }
        (
            crate::segment::kind_from_segmented_source(&m.kind, &src),
            src,
        )
    } else {
        if m.source.get("type").and_then(|t| t.as_str()) != Some("segmented") {
            return None;
        }
        (
            crate::segment::kind_from_segmented_source(&m.kind, &m.source),
            m.source.clone(),
        )
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut h);
    source.to_string().hash(&mut h);
    Some(h.finish())
}

fn render_test_frame(width: u32, height: u32, version: u64) -> Frame {
    let (w, h) = (width.max(1), height.max(1));
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            rgba[i] = (fx * 200.0) as u8 + 20;
            rgba[i + 1] = 40;
            rgba[i + 2] = (fy * 200.0) as u8 + 35;
            rgba[i + 3] = 255;
            let (cx, cy) = (w / 2, h / 2);
            if (x == cx && y.abs_diff(cy) < 40) || (y == cy && x.abs_diff(cx) < 40) {
                rgba[i] = 255;
                rgba[i + 1] = 255;
                rgba[i + 2] = 255;
            }
        }
    }
    Frame {
        width: w,
        height: h,
        rgba,
        version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_deadline_is_not_postponed_by_continuous_input() {
        let now = Instant::now();
        let first_deadline = next_render_deadline(None, now);

        assert_eq!(first_deadline, now + RENDER_THROTTLE);
        assert_eq!(
            next_render_deadline(Some(first_deadline), now + Duration::from_millis(1)),
            first_deadline
        );
    }
}