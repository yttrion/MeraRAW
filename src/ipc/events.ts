// Event names mirrored from src-tauri/src/events.rs. Keep in sync.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const EVENTS = {
  engineReady: "engine-ready",
  log: "log",
  fileOpened: "file-opened",
  folderOpened: "folder-opened",
  previewReady: "preview-ready",
  imageReady: "image-ready",
  frameReady: "frame-ready",
  decodeError: "decode-error",
  docUpdated: "doc-updated",
  maskReady: "mask-ready",
  maskError: "mask-error",
  importProgress: "import-progress",
  importDone: "import-done",
  catalogChanged: "catalog-changed",
  exportProgress: "export-progress",
  exportBatchProgress: "export-batch-progress",
  exportBatchDone: "export-batch-done",
  engineCrashed: "engine-crashed",
  exportRequested: "export-requested",
  importRequested: "import-requested",
  settingsRequested: "settings-requested",
  viewZoomIn: "view-zoom-in",
  viewZoomOut: "view-zoom-out",
  viewZoomFit: "view-zoom-fit",
  photoWorkspace: "photo-workspace",
  videoWorkspace: "video-workspace",
  denoiseProgress: "denoise-progress",
  denoiseDone: "denoise-done",
  denoiseError: "denoise-error",
  retouchDone: "retouch-done",
  retouchError: "retouch-error",
} as const;

export interface DenoiseProgressPayload {
  job: number;
  pct: number;
  tile: number;
  tiles: number;
}

export function onDenoiseProgress(
  cb: (p: DenoiseProgressPayload) => void,
): Promise<UnlistenFn> {
  return listen<DenoiseProgressPayload>(EVENTS.denoiseProgress, (e) =>
    cb(e.payload),
  );
}

export function onDenoiseDone(cb: (job: number) => void): Promise<UnlistenFn> {
  return listen<number>(EVENTS.denoiseDone, (e) => cb(e.payload));
}

export function onDenoiseError(
  cb: (p: { job: number; message: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ job: number; message: string }>(EVENTS.denoiseError, (e) =>
    cb(e.payload),
  );
}

export function onRetouchDone(cb: () => void): Promise<UnlistenFn> {
  return listen(EVENTS.retouchDone, () => cb());
}

export function onRetouchError(cb: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>(EVENTS.retouchError, (e) => cb(e.payload));
}

export function onMaskReady(cb: (id: string) => void): Promise<UnlistenFn> {
  return listen<string>(EVENTS.maskReady, (e) => cb(e.payload));
}

export function onMaskError(
  cb: (p: { id: string; message: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string; message: string }>(EVENTS.maskError, (e) =>
    cb(e.payload),
  );
}

export function onImportProgress(
  cb: (p: { done: number; total: number }) => void,
): Promise<UnlistenFn> {
  return listen<{ done: number; total: number }>(EVENTS.importProgress, (e) =>
    cb(e.payload),
  );
}

export function onImportDone(cb: (total: number) => void): Promise<UnlistenFn> {
  return listen<number>(EVENTS.importDone, (e) => cb(e.payload));
}

export function onCatalogChanged(cb: () => void): Promise<UnlistenFn> {
  return listen<null>(EVENTS.catalogChanged, () => cb());
}

export function onPreviewReady(cb: (version: number) => void): Promise<UnlistenFn> {
  return listen<number>(EVENTS.previewReady, (e) => cb(e.payload));
}

export function onImageReady(cb: (version: number) => void): Promise<UnlistenFn> {
  return listen<number>(EVENTS.imageReady, (e) => cb(e.payload));
}

export function onFrameReady(cb: (version: number) => void): Promise<UnlistenFn> {
  return listen<number>(EVENTS.frameReady, (e) => cb(e.payload));
}

export function onDecodeError(cb: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>(EVENTS.decodeError, (e) => cb(e.payload));
}

export function onExportProgress(
  cb: (p: import("./types").ExportProgress) => void,
): Promise<UnlistenFn> {
  return listen<import("./types").ExportProgress>(EVENTS.exportProgress, (e) =>
    cb(e.payload),
  );
}

export function onExportBatchProgress(
  cb: (p: import("./types").ExportBatchProgress) => void,
): Promise<UnlistenFn> {
  return listen<import("./types").ExportBatchProgress>(
    EVENTS.exportBatchProgress,
    (e) => cb(e.payload),
  );
}

export function onExportBatchDone(
  cb: (p: import("./types").ExportBatchDone) => void,
): Promise<UnlistenFn> {
  return listen<import("./types").ExportBatchDone>(EVENTS.exportBatchDone, (e) =>
    cb(e.payload),
  );
}

export function onEngineCrashed(cb: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>(EVENTS.engineCrashed, (e) => cb(e.payload));
}

export function onDocUpdated(
  cb: (delta: import("./types").DocDelta) => void,
): Promise<UnlistenFn> {
  return listen<import("./types").DocDelta>(EVENTS.docUpdated, (e) =>
    cb(e.payload),
  );
}

export interface EngineReadyPayload {
  adapter: string | null;
  gpuReady: boolean;
}

export function onEngineReady(
  cb: (p: EngineReadyPayload) => void,
): Promise<UnlistenFn> {
  return listen<EngineReadyPayload>(EVENTS.engineReady, (e) => cb(e.payload));
}

export function onFileOpened(cb: (path: string) => void): Promise<UnlistenFn> {
  return listen<string>(EVENTS.fileOpened, (e) => cb(e.payload));
}

export function onFolderOpened(
  cb: (path: string) => void,
): Promise<UnlistenFn> {
  return listen<string>(EVENTS.folderOpened, (e) => cb(e.payload));
}

export function onImportRequested(cb: () => void): Promise<UnlistenFn> {
  return listen<null>(EVENTS.importRequested, () => cb());
}

export function onExportRequested(cb: () => void): Promise<UnlistenFn> {
  return listen<null>(EVENTS.exportRequested, () => cb());
}

export function onSettingsRequested(cb: () => void): Promise<UnlistenFn> {
  return listen<null>(EVENTS.settingsRequested, () => cb());
}

export function onViewZoomIn(cb: () => void): Promise<UnlistenFn> {
  return listen<null>(EVENTS.viewZoomIn, () => cb());
}

export function onViewZoomOut(cb: () => void): Promise<UnlistenFn> {
  return listen<null>(EVENTS.viewZoomOut, () => cb());
}

export function onViewZoomFit(cb: () => void): Promise<UnlistenFn> {
  return listen<null>(EVENTS.viewZoomFit, () => cb());
}

export function onPhotoWorkspace(cb: () => void): Promise<UnlistenFn> {
  return listen<null>(EVENTS.photoWorkspace, () => cb());
}

export function onVideoWorkspace(cb: () => void): Promise<UnlistenFn> {
  return listen<null>(EVENTS.videoWorkspace, () => cb());
}
