/** Gaps the registry coverage test is allowed to ignore — each needs a reason.
 *  Deleting a reason without exposing or removing the param fails the contract. */
export const KNOWN_UNEXPOSED: { path: string; reason: string }[] = [
  { path: "detail.nr_engine", reason: "NR engine selector; Phase 8 — expose or delete" },
  { path: "detail.nr_aggressive", reason: "advanced NR; Phase 8 — expose or delete" },
  { path: "detail.nlm_patch", reason: "NLM internals; Phase 8 — expose or delete" },
  { path: "detail.nlm_search", reason: "NLM internals; Phase 8 — expose or delete" },
  { path: "detail.nlm_center", reason: "NLM internals; Phase 8 — expose or delete" },
  { path: "detail.chroma_auto", reason: "auto chroma NR; Phase 8 — expose or delete" },
];

/** ui.group → panel files that consume it. Color Grade / Tone span two panels. */
export const GROUP_PANELS: Record<string, string[]> = {
  Light: ["LightSettings.svelte"],
  "White Balance": ["ColorSettings.svelte"],
  Calibration: ["CalibrationSettings.svelte"],
  "Noise Reduction": ["DetailSettings.svelte"],
  Detail: ["DetailSettings.svelte"],
  "Color Grade": ["ColorSettings.svelte", "GradingSettings.svelte"],
  Tone: ["LightSettings.svelte", "ToneCurve.svelte"],
  LUT: ["LutSettings.svelte", "GradingSettings.svelte"],
  Input: ["CameraSettings.svelte"],
  HSL: ["ColorSettings.svelte"],
  Crop: ["CropSettings.svelte"],
  Perspective: ["CropSettings.svelte"],
  Effects: ["LightSettings.svelte"],
};

/** Registered Rust commands that must not appear in the release ACL. */
export const DEBUG_COMMANDS = [
  "fail_on_purpose",
  "autoopen_path",
  "selftest_enabled",
  "verify_slider_enabled",
  "report_frontend_status",
] as const;

/** Rust commands with a TS wrapper but no product caller yet. */
export const UNCALLED_OK: { command: string; reason: string }[] = [
  {
    command: "fail_on_purpose",
    reason: "debug probe; wrapper only, ACL is capabilities-dev",
  },
  {
    command: "license_verify_token_locally",
    reason: "sign-in path verifies on the Rust side; TS wrapper for the contract",
  },
  {
    command: "app_info",
    reason: "version/build probe; no settings row reads it yet",
  },
  {
    command: "close_image",
    reason: "no close-document control in the shell yet",
  },
  {
    command: "denoise_estimate_profile",
    reason: "profile-estimate control not exposed; AI denoise uses start/cancel",
  },
  {
    command: "scan_import_folder",
    reason: "selective import UI not built; import_selected is the live path",
  },
  {
    command: "list_albums",
    reason: "albums UI not built",
  },
  {
    command: "create_album",
    reason: "albums UI not built",
  },
  {
    command: "delete_album",
    reason: "albums UI not built",
  },
  {
    command: "add_to_album",
    reason: "albums UI not built",
  },
  {
    command: "remove_from_album",
    reason: "albums UI not built",
  },
  {
    command: "rebuild_index",
    reason: "no settings control to rebuild the catalog index",
  },
];
