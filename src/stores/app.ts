// Engine/app state shared across the Svelte UI (successor to the old
// zustand uiStore — only what the new shell actually consumes).
import { atom } from "nanostores";
import type { ImageMeta } from "../ipc/types";

export const engineReady = atom(false);
export const gpuAdapter = atom<string | null>(null);
export const statusMessage = atom("");

export const imageOpen = atom(false);
export const lastOpenedPath = atom<string | null>(null);
export const lastOpenedDocId = atom<string | null>(null);
export const imageMeta = atom<ImageMeta | null>(null);
export const imageDims = atom<{ w: number; h: number } | null>(null);
export type DecodeState = "idle" | "preview" | "ready" | "error";
export const decodeState = atom<DecodeState>("idle");
/** Cached catalog image shown instantly while the RAW engine opens a photo. */
export const openingPreviewUrl = atom<string | null>(null);
/** Ungraded catalog thumb for preset list snapshots. Stays after the opening preview clears. */
export const presetSnapshotUrl = atom<string | null>(null);
/** Catalog size (+ optional EXIF label) so a sideways thumb can stand up
 *  before `open_image` metadata arrives. */
export const openingPreviewHint = atom<{
  w: number;
  h: number;
  orientation?: string;
} | null>(null);

/** Latest engine frame version (bumped by frame-ready / image-ready). */
export const frameVersion = atom(0);
export const zoomLabel = atom("fit");

/** Display look: 1 Camera, 2 Filmic/AgX, 4 Original, 8 Linear. */
export const displayLook = atom(1);
/** Before/after: render the un-edited base when true. */
export const previewBypass = atom(false);

/** Histogram display mode (phase 12). Persisted via settings. */
export type HistMode = "rgb" | "luma" | "parade" | "wave" | "scope";
export type HistScale = "sqrt" | "linear" | "log";

function readHistMode(): HistMode {
  try {
    const v = localStorage.getItem("hist-mode");
    if (v === "luma" || v === "parade" || v === "rgb" || v === "wave" || v === "scope") return v;
  } catch {
    /* ignore */
  }
  return "rgb";
}
function readHistScale(): HistScale {
  try {
    const v = localStorage.getItem("hist-scale");
    if (v === "linear" || v === "log" || v === "sqrt") return v;
  } catch {
    /* ignore */
  }
  return "sqrt";
}

export const histMode = atom<HistMode>(readHistMode());
export const histScale = atom<HistScale>(readHistScale());

export function setHistMode(mode: HistMode): void {
  histMode.set(mode);
  try {
    localStorage.setItem("hist-mode", mode);
  } catch {
    /* ignore */
  }
}
export function setHistScale(scale: HistScale): void {
  histScale.set(scale);
  try {
    localStorage.setItem("hist-scale", scale);
  } catch {
    /* ignore */
  }
}

/** Mask scoping for panel edits: params target mask.<id>.<path> when set. */
export const selectedMask = atom<string | null>(null);

/** Selected object-removal / heal spot (phase 10). */
export const selectedRetouch = atom<string | null>(null);

/** Viewport tool (pan / white-balance eyedropper / brush / crop / mask geometry). */
export type ViewportTool = "pan" | "wb" | "brush" | "crop" | "mask-geo";
export const viewportTool = atom<ViewportTool>("pan");
export const brushRadius = atom(0.05);
export const cropActive = atom(false);

/** One-shot viewport commands from the chrome bar. */
export type ViewCmd = "fit" | "oneToOne" | "zoomIn" | "zoomOut" | null;
export const viewCmd = atom<ViewCmd>(null);
export const viewCmdNonce = atom(0);
export function sendViewCmd(cmd: Exclude<ViewCmd, null>): void {
  viewCmd.set(cmd);
  viewCmdNonce.set(viewCmdNonce.get() + 1);
}

/** Current browse folder (catalog-backed). */
export const currentFolder = atom<string | null>(null);

/** Inclusive in/out marks for the open video clip (UI + clip export). */
export const videoMark = atom<{ inFrame: number; outFrame: number }>({
  inFrame: 0,
  outFrame: 0,
});