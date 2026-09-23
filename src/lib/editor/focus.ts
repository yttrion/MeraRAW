import { setMaskOverlay } from "../../ipc/commands";
import {
  cropActive,
  selectedMask,
  selectedRetouch,
  viewportTool,
} from "../../stores/app";
import { doc } from "../../stores/doc";
import {
  colorPickActive,
  stopGeomPlacement,
  stopInstancePick,
  syncMaskOverlay,
  syncViewportToolForMask,
} from "../../stores/mask";
import {
  activeTool,
  editFocus,
  openSections,
  rightPanelMode,
  type EditFocus,
  type SectionId,
  type Tool,
} from "../../stores/editor";
import { abandonCropSession, flushCropDraft } from "../../crop/cropSession";

export function applyTool(id: Tool) {
  if (id !== "crop" && cropActive.get()) {
    void flushCropDraft()
      .catch(() => {})
      .then(() => applyToolNow(id));
    return;
  }
  applyToolNow(id);
}

function applyToolNow(id: Tool) {
  activeTool.set(id);
  cropActive.set(id === "crop");
  if (id === "crop") {
    selectedMask.set(null);
    selectedRetouch.set(null);
    viewportTool.set("crop");
    void setMaskOverlay(null);
  } else if (id === "mask") {
    selectedRetouch.set(null);
    const mid = selectedMask.get();
    const m = mid ? doc.get()?.masks?.find((x) => x.id === mid) : null;
    syncViewportToolForMask(m ?? null);
    syncMaskOverlay();
  } else {
    selectedRetouch.set(null);
    selectedMask.set(null);
    viewportTool.set("pan");
    void setMaskOverlay(null);
  }
}

const FOCUS_TO_SECTION: Record<Exclude<EditFocus, null>, SectionId> = {
  light: "light",
  color: "color",
  curve: "curve",
  detail: "detail",
  grading: "grading",
  crop: "crop",
  mask: "mask",
  retouch: "retouch",
  camera: "camera",
  presets: "presets",
};

export function applyEditFocus(focus: EditFocus) {
  editFocus.set(focus);
  if (focus === "mask") {
    showMaskPanel();
  } else if (focus === "crop") {
    showCropPanel();
  } else if (focus === "retouch") {
    rightPanelMode.set("edit");
    applyTool("edit");
  } else if (focus === "presets") {
    rightPanelMode.set("edit");
    applyTool("presets");
  } else {
    rightPanelMode.set("edit");
    applyTool("edit");
  }

  if (!focus) return;
  const id = FOCUS_TO_SECTION[focus];
  openSections.setKey(id, true);
  if (focus !== "mask" && focus !== "crop") {
    queueMicrotask(() => {
      document
        .querySelector(`[data-section="${id}"]`)
        ?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    });
  }
}

/** Lightroom-style masking rail: tools + scoped adjustments. */
export function showMaskPanel() {
  rightPanelMode.set("mask");
  applyTool("mask");
}

/** Crop as its own develop tab; overlay lives on the photo. */
export function showCropPanel() {
  stopInstancePick();
  stopGeomPlacement();
  colorPickActive.set(false);
  rightPanelMode.set("crop");
  applyTool("crop");
}

export function leaveCropTool() {
  if (rightPanelMode.get() !== "crop" && !cropActive.get()) return;
  rightPanelMode.set("edit");
  applyTool("edit");
}

/** Esc: drop in-progress overlay edits instead of committing them. */
export function cancelCropTool() {
  abandonCropSession();
  leaveCropTool();
}
