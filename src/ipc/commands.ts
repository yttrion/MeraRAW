// Typed wrappers over Tauri invoke. Thin — mirror of src-tauri/src/commands.rs.
import { invoke } from "@tauri-apps/api/core";
import type {
  AppInfo,
  DocDelta,
  DocRef,
  EditDocMirror,
  EngineStatus,
  ExportSettings,
  FileMeta,
  FrameInfo,
  ImageMeta,
  Op,
  ParamSpec,
  ViewParams,
} from "./types";

export function appInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

export function pingEngine(): Promise<EngineStatus> {
  return invoke<EngineStatus>("ping_engine");
}

export function pickFile(kind?: "photo" | "video" | null): Promise<string | null> {
  return invoke<string | null>("pick_file", { kind: kind ?? null });
}

export function pickFiles(kind?: "photo" | "video" | null): Promise<string[] | null> {
  return invoke<string[] | null>("pick_files", { kind: kind ?? null });
}

export function pickFolder(): Promise<string | null> {
  return invoke<string | null>("pick_folder");
}

export function readFileMeta(path: string): Promise<FileMeta> {
  return invoke<FileMeta>("read_file_meta", { path });
}

export function probeOrientation(path: string): Promise<string> {
  return invoke<string>("probe_orientation", { path });
}

export function openImage(path: string, docId?: string | null): Promise<ImageMeta> {
  return invoke<ImageMeta>("open_image", { path, docId: docId ?? null });
}

export function requestFrame(view: ViewParams): Promise<FrameInfo> {
  return invoke<FrameInfo>("request_frame", { view });
}

export function getMetadata(): Promise<ImageMeta | null> {
  return invoke<ImageMeta | null>("get_metadata");
}

export function closeImage(): Promise<void> {
  return invoke<void>("close_image");
}

// ---- Phase 2: doc ops ----

export function applyOp(op: Op, live = false): Promise<DocDelta> {
  return invoke<DocDelta>("apply_op", { op, live });
}

export function setParam(
  path: string,
  value: unknown,
  live = false,
): Promise<DocDelta> {
  return applyOp({ op: "set_param", path, value }, live);
}

export function undo(): Promise<DocDelta> {
  return invoke<DocDelta>("undo");
}

export function redo(): Promise<DocDelta> {
  return invoke<DocDelta>("redo");
}

export function getDoc(): Promise<EditDocMirror | null> {
  return invoke<EditDocMirror | null>("get_doc");
}

export function getHistory(): Promise<string[]> {
  return invoke<string[]>("get_history");
}

export function getRegistry(): Promise<ParamSpec[]> {
  return invoke<ParamSpec[]>("get_registry");
}

export function denoiseEstimateProfile(): Promise<{
  a: number;
  b: number;
  source: string;
}> {
  return invoke("denoise_estimate_profile");
}

export function denoiseModelsList(): Promise<
  Array<{
    id: string;
    name: string;
    sizeMb: number;
    ready: boolean;
    standIn: boolean;
    sha256?: string | null;
    license: string;
  }>
> {
  return invoke("denoise_models_list");
}

export function denoiseAiStart(): Promise<number> {
  return invoke<number>("denoise_ai_start");
}

export function denoiseAiCancel(job: number): Promise<void> {
  return invoke<void>("denoise_ai_cancel", { job });
}

/** Revert the working master to a plain re-decode (AI Denoise unchecked). */
export function denoiseAiReset(): Promise<void> {
  return invoke<void>("denoise_ai_reset");
}

export function snapshot(name: string): Promise<void> {
  return invoke<void>("snapshot", { name });
}

export function listSnapshots(): Promise<string[]> {
  return invoke<string[]>("list_snapshots");
}

export function restoreSnapshot(name: string): Promise<DocDelta> {
  return invoke<DocDelta>("restore_snapshot", { name });
}

export function virtualCopy(path?: string | null): Promise<string> {
  return invoke<string>("virtual_copy", { path: path ?? null });
}

export function switchDoc(docId: string): Promise<DocDelta> {
  return invoke<DocDelta>("switch_doc", { docId });
}

export function listDocs(): Promise<DocRef[]> {
  return invoke<DocRef[]>("list_docs");
}

export function deleteVirtualCopy(docId: string): Promise<void> {
  return invoke<void>("delete_virtual_copy", { docId });
}

export function applyGradeToPaths(
  paths: string[],
  modules: Record<string, Record<string, unknown>>,
  lutFile?: string | null,
): Promise<number> {
  return invoke<number>("apply_grade_to_paths", {
    paths,
    modules,
    lutFile: lutFile ?? null,
  });
}

export function savePreset(modules: string[]): Promise<unknown> {
  return invoke<unknown>("save_preset", { modules });
}

export function getStats(): Promise<import("./types").FrameStats | null> {
  return invoke("get_stats");
}

/** WB eyedropper: neutralize the sampled normalized image point. */
export function wbFromPoint(x: number, y: number): Promise<DocDelta> {
  return invoke<DocDelta>("wb_from_point", { x, y });
}

/** Crop auto-level: dominant line deviation (deg, original space); 0 = none. */
export function autoLevel(): Promise<number> {
  return invoke<number>("auto_level");
}

// ---- Phase 5: catalog ----

export function importFolder(path: string): Promise<number> {
  return invoke<number>("import_folder", { path });
}

export function scanImportFolder(
  path: string,
): Promise<import("./types").ImportCandidate[]> {
  return invoke("scan_import_folder", { path });
}

export function importSelected(root: string, paths: string[]): Promise<number> {
  return invoke<number>("import_selected", { root, paths });
}

export function getAssetDetail(
  id: number,
): Promise<import("./types").AssetDetail | null> {
  return invoke("get_asset_detail", { id });
}

export function listAlbums(): Promise<import("./types").AlbumItem[]> {
  return invoke("list_albums");
}

export function createAlbum(name: string): Promise<number> {
  return invoke<number>("create_album", { name });
}

export function deleteAlbum(id: number): Promise<void> {
  return invoke<void>("delete_album", { id });
}

export function addToAlbum(albumId: number, assetIds: number[]): Promise<void> {
  return invoke<void>("add_to_album", { albumId, assetIds });
}

export function removeFromAlbum(
  albumId: number,
  assetIds: number[],
): Promise<void> {
  return invoke<void>("remove_from_album", { albumId, assetIds });
}

export function setCameraProfile(
  profileFile: string,
): Promise<ImageMeta> {
  return invoke<ImageMeta>("set_camera_profile", { profileFile });
}

/** Native picker for a 3D look LUT (.cube). */
export function pickLut(): Promise<string | null> {
  return invoke<string | null>("pick_lut");
}

/** Load (path) or clear (null) the current image's 3D look LUT. */
export function setLut(path: string | null): Promise<void> {
  return invoke<void>("set_lut", { path });
}

export interface LookInfo {
  id: string;
  name: string;
  description: string;
  category: string;
  kind: number;
  intensityDefault: number;
  grainAmount: number;
  grainSize: number;
  /** sRGB 8-bit probes: shadow, skin, sky. */
  preview: [[number, number, number], [number, number, number], [number, number, number]];
}

export function listLooks(): Promise<LookInfo[]> {
  return invoke<LookInfo[]>("list_looks");
}

export function userLooksDir(): Promise<string> {
  return invoke<string>("user_looks_dir");
}

export function deleteUserLook(id: string): Promise<void> {
  return invoke<void>("delete_user_look", { id });
}

export function seekVideo(frame: number): Promise<import("./types").ImageMeta> {
  return invoke("seek_video", { frame });
}

/** Change the demosaic algorithm and re-decode the current RAW. */
export function setDemosaic(algo: string): Promise<ImageMeta> {
  return invoke<ImageMeta>("set_demosaic", { algo });
}

export function getGrid(
  query: import("./types").GridQuery,
): Promise<import("./types").GridItem[]> {
  return invoke("get_grid", { query });
}

export function listFolders(): Promise<import("./types").FolderItem[]> {
  return invoke("list_folders");
}

export function discoverMediaFolders(): Promise<import("./types").DiscoveredFolder[]> {
  return invoke("discover_media_folders");
}

export function listFolderChildren(path: string): Promise<import("./types").FolderChild[]> {
  return invoke("list_folder_children", { path });
}

export function forgetFolder(root: string): Promise<void> {
  return invoke("forget_folder", { root });
}

export function setAssetMeta(
  ids: number[],
  patch: import("./types").MetaPatch,
): Promise<void> {
  return invoke("set_asset_meta", { ids, patch });
}

export function rebuildIndex(confirm = true): Promise<number> {
  return invoke<number>("rebuild_index", { confirm });
}

/** Toggle viewport mask overlay (null = off). Optional strength 0..1 and mode 0..3. */
export function setMaskOverlay(
  id: string | null,
  opts?: { strength?: number; mode?: number },
): Promise<void> {
  return invoke<void>("set_mask_overlay", {
    id,
    strength: opts?.strength ?? null,
    mode: opts?.mode ?? null,
  });
}

export function sampleColor(
  x: number,
  y: number,
): Promise<{ working: [number, number, number]; display: [number, number, number] }> {
  return invoke("sample_color", { x, y });
}

export interface ObjectProposal {
  id: number;
  path: [number, number][];
  centroid: [number, number];
  area: number;
}

/** Saliency contours for object-pick UI (dotted outlines). */
export function proposeObjectMasks(): Promise<ObjectProposal[]> {
  return invoke<ObjectProposal[]>("propose_object_masks");
}

/** Before/after: render the un-edited base while on. */
export function setPreviewBypass(on: boolean): Promise<void> {
  return invoke<void>("set_preview_bypass", { on });
}

/** Display look: 0 = Neutral, 1 = Camera, 2 = Filmic (AgX). */
export function setDisplayLook(look: number): Promise<void> {
  return invoke<void>("set_display_look", { look });
}

export function setClipWarnings(hi: boolean, lo: boolean): Promise<void> {
  return invoke<void>("set_clip_warnings", { hi, lo });
}

/** View-only soft proof. `space`: 0 off, 1 sRGB, 2 P3, 3 Adobe RGB, 4 ProPhoto. */
export function setProofTarget(space: number, gamut: boolean): Promise<void> {
  return invoke<void>("set_proof_target", { space, gamut });
}

// ---- Phase 7: export + presets ----

export function exportImage(settings: ExportSettings): Promise<string> {
  return invoke<string>("export_image", { settings });
}

/** Engine-side batch export: returns the accepted queue length; progress
 * arrives via export-batch-progress / export-batch-done events. */
export function exportBatch(
  items: import("./types").BatchExportItem[],
  settings: ExportSettings,
): Promise<number> {
  return invoke<number>("export_batch", { items, settings });
}

export function cancelExportBatch(): Promise<void> {
  return invoke<void>("cancel_export_batch");
}

export function revealInFinder(path: string): Promise<void> {
  return invoke<void>("reveal_in_finder", { path });
}

export function listPresets(): Promise<string[]> {
  return invoke<string[]>("list_presets");
}

export interface PresetCatalogEntry {
  id: string;
  label: string;
  tags: string[];
  modules?: Record<string, Record<string, unknown>>;
}

/** Preset style categories (searchable in Presets panel). */
export const PRESET_TAGS = [
  "Natural",
  "Bright & Airy",
  "Moody",
  "Cinematic",
  "Film",
  "Vintage",
  "Matte",
  "Warm",
  "Cool",
  "Vibrant",
  "Muted",
  "Pastel",
  "High Contrast",
  "Black & White",
  "Dark",
  "Dreamy",
  "Editorial",
  "Clean",
  "Earthy",
  "Teal & Orange",
] as const;

export function listPresetCatalog(): Promise<PresetCatalogEntry[]> {
  return invoke<PresetCatalogEntry[]>("list_preset_catalog");
}

export function applyPreset(name: string): Promise<DocDelta> {
  return invoke<DocDelta>("apply_preset", { name });
}

/** Save the listed modules' current params as a named preset (P7).
 *  Pass `grade` to persist a copied look without requiring the current doc. */
export function savePresetNamed(
  name: string,
  modules: string[],
  grade?: Record<string, Record<string, unknown>> | null,
): Promise<void> {
  return invoke<void>("save_preset", {
    name,
    modules,
    grade: grade ?? null,
  });
}

/** Batch a set of param changes as ONE undoable history step (apply_preset). */
export function applyParamBatch(
  modules: Record<string, Record<string, number>>,
): Promise<DocDelta> {
  return applyOp({ op: "apply_preset", preset: { modules } });
}

/** Dev/test hook: returns MERATECH_OPEN env path if set. */
export function autoopenPath(): Promise<string | null> {
  return invoke<string | null>("autoopen_path");
}

/** Intentional-error probe: backend always returns AppError. DoD item 7. */
export function failOnPurpose(): Promise<void> {
  return invoke<void>("fail_on_purpose");
}

/** Frontend self-report → Rust log (lets headless test runs verify the webview booted + frame path). */
export function reportFrontendStatus(status: string): Promise<void> {
  return invoke<void>("report_frontend_status", { status });
}

export function getPerfStats(): Promise<{
  decodeMs: number;
  renderMs: number;
  [k: string]: unknown;
}> {
  return invoke("get_perf_stats");
}

export function reportProblem(message: string, from?: string | null): Promise<void> {
  return invoke<void>("report_problem", { message, from: from ?? null });
}

export interface LicenseCheck {
  licensed: boolean;
  userId: string | null;
  email: string | null;
  reason: string | null;
}

export function licenseCheckLocal(): Promise<LicenseCheck> {
  return invoke<LicenseCheck>("license_check_local");
}

export function licenseClearToken(): Promise<void> {
  return invoke<void>("license_clear_token");
}

export function licenseVerifyTokenLocally(token: string): Promise<boolean> {
  return invoke<boolean>("license_verify_token_locally", { token });
}

export function licenseRequestOtp(email: string): Promise<void> {
  return invoke<void>("license_request_otp", { email });
}

export function licenseVerifyOtp(email: string, code: string): Promise<LicenseCheck> {
  return invoke<LicenseCheck>("license_verify_otp", { email, code });
}

export function licenseSignInAndActivate(
  email: string,
  password: string,
): Promise<string> {
  return invoke<string>("license_sign_in_and_activate", { email, password });
}

export function licenseSupporterStatus(): Promise<{
  licensed: boolean;
  isEarlySupporter: boolean;
  userId: string | null;
  reason: string | null;
}> {
  return invoke("license_supporter_status");
}

export function licenseStartCheckout(
  email: string,
  password: string,
): Promise<string> {
  return invoke<string>("license_start_checkout", { email, password });
}

export function openExternalUrl(url: string): Promise<void> {
  return invoke<void>("open_external_url", { url });
}

export function selftestEnabled(): Promise<string> {
  return invoke<string>("selftest_enabled");
}

export function verifySliderEnabled(): Promise<boolean> {
  return invoke<boolean>("verify_slider_enabled");
}
