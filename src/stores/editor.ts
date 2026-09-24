// UI-only editor state (tool, section disclosure, layout). Engine state
// lives in stores/app.ts + stores/doc.ts; params flow via lib/engine/params.
import { atom, map } from "nanostores";

export type Tool = "edit" | "crop" | "mask" | "presets";
export const activeTool = atom<Tool>("edit");

/** Single source of truth for edit-panel section ids and first-paint open state.
 *  `SectionId` is derived from this object, so a missing map entry is a type
 *  error rather than a permanently-collapsed panel. */
export const DEFAULT_SECTIONS = {
  light: true,
  color: true,
  curve: false,
  detail: false,
  grading: false,
  crop: false,
  mask: false,
  retouch: false,
  camera: false,
  presets: false,
} as const;

export type SectionId = keyof typeof DEFAULT_SECTIONS;

const OPEN_SECTIONS_KEY = "meraraw.openSections.v2";

function readStoredSections(): Record<SectionId, boolean> {
  const next: Record<SectionId, boolean> = { ...DEFAULT_SECTIONS };
  try {
    if (typeof localStorage === "undefined") return next;
    const raw = localStorage.getItem(OPEN_SECTIONS_KEY);
    if (!raw) return next;
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    for (const id of Object.keys(DEFAULT_SECTIONS) as SectionId[]) {
      if (typeof parsed[id] === "boolean") next[id] = parsed[id];
    }
  } catch {
    /* ignore corrupt / unavailable storage */
  }
  return next;
}

export const openSections = map<Record<SectionId, boolean>>(readStoredSections());
openSections.listen((value) => {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(OPEN_SECTIONS_KEY, JSON.stringify(value));
  } catch {
    /* ignore quota / private mode */
  }
});

export const toggleSection = (id: SectionId) =>
  openSections.setKey(id, !openSections.get()[id]);

/** Tone-curve channel selector (matches engine paths). */
export type CurveChannel = "luma" | "red" | "green" | "blue";
export const curveChannel = atom<CurveChannel>("luma");

export const leftRailCollapsed = atom<boolean>(true);
export const isZenMode = atom<boolean>(false);
export const imageBrowserCollapsed = atom<boolean>(false);
export const photoDetailsCollapsed = atom<boolean>(false);

/** Right rail: develop controls vs local mask edits. */
export type RightPanelMode = "edit" | "crop" | "mask";
export const rightPanelMode = atom<RightPanelMode>("edit");

/** Jump target for shortcuts / command palette. Scrolls that accordion into view. */
export type EditFocus =
  | null
  | "light"
  | "color"
  | "curve"
  | "detail"
  | "grading"
  | "crop"
  | "mask"
  | "retouch"
  | "camera"
  | "presets";
export const editFocus = atom<EditFocus>(null);

export const histogramOpen = atom<boolean>(true);
export const commandPaletteOpen = atom<boolean>(false);
