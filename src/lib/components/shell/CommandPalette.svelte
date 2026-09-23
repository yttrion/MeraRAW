<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { commandPaletteOpen, leftRailCollapsed, imageBrowserCollapsed, isZenMode } from "../../../stores/editor";
  import { isExportOpen, isSettingsOpen, isShortcutsOpen } from "../../../stores/ui";
  import { applyEditFocus } from "../../editor/focus";
  import { push } from "svelte-spa-router";
  import { undo, redo } from "../../../ipc/commands";
  import { reconcile } from "../../../stores/doc";
  import { copyGrade, pasteGrade } from "../../grade";
  import { shortcutLabels } from "../../shortcuts";
  import { editorRoute, libraryRoute, setWorkspace, workspace } from "../../../stores/workspace";

  interface Cmd {
    id: string;
    group: string;
    label: string;
    hint?: string;
    keywords?: string;
    photoOnly?: boolean;
    run: () => void;
  }

  const commands: Cmd[] = [
    { id: "light", group: "Edit", label: "Adjust Light", hint: "1", keywords: "exposure contrast highlights shadows", run: () => applyEditFocus("light") },
    { id: "color", group: "Edit", label: "Adjust Color", keywords: "temp tint vibrance saturation mixer", run: () => applyEditFocus("color") },
    { id: "curve", group: "Edit", label: "Tone Curve", keywords: "rgb curve", photoOnly: true, run: () => applyEditFocus("curve") },
    { id: "detail", group: "Edit", label: "Adjust Detail", keywords: "sharp sharpen denoise noise", photoOnly: true, run: () => applyEditFocus("detail") },
    { id: "grading", group: "Edit", label: "Color Grading", keywords: "split toning shadows midtones highlights looks", run: () => applyEditFocus("grading") },
    { id: "crop", group: "Edit", label: "Crop tab", hint: "2", photoOnly: true, run: () => applyEditFocus("crop") },
    { id: "mask", group: "Edit", label: "Mask", hint: "3", photoOnly: true, run: () => applyEditFocus("mask") },
    { id: "retouch", group: "Edit", label: "Retouch", hint: "4", keywords: "heal spot object removal", photoOnly: true, run: () => applyEditFocus("retouch") },
    { id: "camera", group: "Edit", label: "Camera", keywords: "profile demosaic lut calibration log", run: () => applyEditFocus("camera") },
    { id: "presets", group: "Edit", label: "Presets", hint: "5", run: () => applyEditFocus("presets") },
    { id: "compare", group: "View", label: "Compare Before / After", hint: shortcutLabels.compare, run: () => window.dispatchEvent(new CustomEvent("meraraw:toggle-compare")) },
    { id: "filmstrip", group: "View", label: "Toggle Filmstrip", hint: shortcutLabels.filmstrip, run: () => imageBrowserCollapsed.set(!imageBrowserCollapsed.get()) },
    { id: "sidebar", group: "View", label: "Toggle Sidebar", hint: shortcutLabels.sidebar, run: () => leftRailCollapsed.set(!leftRailCollapsed.get()) },
    { id: "zen", group: "View", label: "Toggle Zen Mode", hint: shortcutLabels.zenMode, run: () => isZenMode.set(!isZenMode.get()) },
    { id: "library", group: "View", label: "Go to Library", hint: shortcutLabels.library, run: () => push(libraryRoute()) },
    { id: "grade", group: "View", label: "Go to Editor", hint: shortcutLabels.edit, run: () => push(editorRoute()) },
    { id: "ws-photo", group: "View", label: "Switch to Photo Editor", keywords: "stills raw meraraw original", run: () => setWorkspace("photo") },
    { id: "export", group: "File", label: "Export", hint: shortcutLabels.export, run: () => isExportOpen.set(true) },
    { id: "settings", group: "File", label: "Settings", hint: shortcutLabels.settings, run: () => isSettingsOpen.set(true) },
    { id: "shortcuts", group: "File", label: "Keyboard Shortcuts", run: () => isShortcutsOpen.set(true) },
    { id: "copy-grade", group: "Edit", label: "Copy Grade", hint: shortcutLabels.copyGrade, keywords: "look lut cdl", run: () => void copyGrade() },
    { id: "paste-grade", group: "Edit", label: "Paste Grade", hint: shortcutLabels.pasteGrade, keywords: "look lut cdl", run: () => void pasteGrade() },
    { id: "undo", group: "Edit", label: "Undo", hint: shortcutLabels.undo, run: () => void undo().then(reconcile).catch(() => {}) },
    { id: "redo", group: "Edit", label: "Redo", hint: shortcutLabels.redo, run: () => void redo().then(reconcile).catch(() => {}) },
  ];

  let query = $state("");
  let active = $state(0);
  let inputEl = $state<HTMLInputElement | null>(null);

  const filtered = $derived.by(() => {
    const video = $workspace === "video";
    const q = query.trim().toLowerCase();
    return commands.filter((c) => {
      if (c.photoOnly && video) return false;
      if (!q) return true;
      return `${c.group} ${c.label} ${c.keywords ?? ""}`.toLowerCase().includes(q);
    });
  });

  $effect(() => {
    void filtered;
    active = 0;
  });

  $effect(() => {
    if ($commandPaletteOpen) {
      query = "";
      active = 0;
      requestAnimationFrame(() => inputEl?.focus());
    }
  });

  function close() {
    commandPaletteOpen.set(false);
  }

  function run(cmd: Cmd) {
    close();
    cmd.run();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      active = Math.min(filtered.length - 1, active + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = Math.max(0, active - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const cmd = filtered[active];
      if (cmd) run(cmd);
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="cmd-scrim" transition:fade={{ duration: 120 }} onclick={close}>
  <div
    class="cmd-card"
    role="dialog"
    tabindex="-1"
    aria-label="Command palette"
    transition:scale={{ duration: 160, start: 0.98 }}
    onclick={(e) => e.stopPropagation()}
  >
    <div class="cmd-search">
      <svg width="13" height="13" viewBox="0 0 15 15" fill="none" class="shrink-0 text-subtle">
        <circle cx="6.5" cy="6.5" r="4.5" stroke="currentColor" stroke-width="1.5" />
        <path d="M10 10L13.5 13.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
      <input
        bind:this={inputEl}
        bind:value={query}
        onkeydown={onKey}
        placeholder="Search commands…"
        aria-label="Search commands"
      />
    </div>
    <div class="cmd-list" role="listbox">
      {#if filtered.length === 0}
        <p class="empty-state">No matching commands.</p>
      {:else}
        {#each filtered as cmd, i (cmd.id)}
          {#if i === 0 || filtered[i - 1].group !== cmd.group}
            <div class="cmd-group">{cmd.group}</div>
          {/if}
          <button
            type="button"
            class="cmd-row"
            class:is-active={i === active}
            role="option"
            aria-selected={i === active}
            onmouseenter={() => (active = i)}
            onclick={() => run(cmd)}
          >
            <span>{cmd.label}</span>
            {#if cmd.hint}
              <span class="cmd-hint">{cmd.hint}</span>
            {/if}
          </button>
        {/each}
      {/if}
    </div>
  </div>
</div>

<style>
  .cmd-scrim {
    position: absolute;
    inset: 0;
    z-index: 80;
    display: flex;
    justify-content: center;
    padding-top: 12vh;
    background: rgba(0, 0, 0, 0.35);
  }
  .cmd-card {
    width: min(480px, calc(100% - 32px));
    max-height: min(420px, 70vh);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--color-panel);
    border: 1px solid var(--color-border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow-popover);
  }
  .cmd-search {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 44px;
    padding: 0 14px;
    border-bottom: 1px solid var(--color-border);
    color: var(--color-subtle);
  }
  .cmd-search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--color-fg);
    font: inherit;
    font-size: 14px;
  }
  .cmd-list {
    overflow-y: auto;
    padding: 6px;
  }
  .cmd-group {
    padding: 8px 8px 4px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--color-subtle);
  }
  .cmd-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    padding: 7px 8px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--color-fg);
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .cmd-row.is-active { background: var(--color-hover); }
  .cmd-hint {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--color-subtle);
  }
</style>
