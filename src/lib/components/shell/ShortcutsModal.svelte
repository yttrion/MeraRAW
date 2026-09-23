<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { isShortcutsOpen } from "../../../stores/ui";
  import { shortcutLabels } from "../../shortcuts";

  function close() {
    isShortcutsOpen.set(false);
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") close();
  }

  const shortcutGroups = [
    {
      title: "Navigation & Layout",
      items: [
        { label: "Photo Editor / Video Editor", key: "Title bar" },
        { label: "Go to Library / Clips", key: shortcutLabels.library },
        { label: "Back to Library from a photo", key: "Esc" },
        { label: "Go to Editor / Grade", key: shortcutLabels.edit },
        { label: "Toggle Left Sidebar", key: shortcutLabels.sidebar },
        { label: "Toggle Details Panel (Library)", key: shortcutLabels.details },
        { label: "Toggle Filmstrip (Editor)", key: shortcutLabels.filmstrip },
        { label: "Toggle Zen / Expert Mode", key: shortcutLabels.zenMode },
        { label: "Toggle Fullscreen", key: shortcutLabels.fullscreen },
      ]
    },
    {
      title: "Tool Switching",
      items: [
        { label: "Focus Light section", key: shortcutLabels.toolEdit },
        { label: "Crop tab (Photo)", key: shortcutLabels.toolCrop },
        { label: "Focus Mask section (Photo)", key: shortcutLabels.toolMask },
        { label: "Focus Retouch section (Photo)", key: shortcutLabels.toolRetouch },
        { label: "Focus Presets section", key: shortcutLabels.toolPresets },
      ]
    },
    {
      title: "Image Adjustment & Modals",
      items: [
        { label: "Toggle Zoom 1:1 / Fit", key: shortcutLabels.zoom },
        { label: "Zoom In", key: shortcutLabels.zoomIn },
        { label: "Zoom Out", key: shortcutLabels.zoomOut },
        { label: "Zoom to Fit", key: shortcutLabels.zoomFit },
        { label: "Toggle Before/After Compare", key: shortcutLabels.compare },
        { label: "Open Export Modal", key: shortcutLabels.export },
        { label: "Open Settings Modal", key: shortcutLabels.settings },
        { label: "Undo last change", key: shortcutLabels.undo },
        { label: "Redo last change", key: shortcutLabels.redo },
        { label: "Next / Previous Photo", key: "← / →" },
        { label: "Video frame step (Edit)", key: "← / →" },
        { label: "Play / Pause video", key: shortcutLabels.videoPlay },
        { label: "Reverse play (video)", key: "J" },
        { label: "Pause video", key: "K" },
        { label: "Mark In / Out (video)", key: "I / O" },
        { label: "Previous / Next Look", key: "[ / ]" },
        { label: "Copy Grade", key: shortcutLabels.copyGrade },
        { label: "Paste Grade", key: shortcutLabels.pasteGrade },
        { label: "Open Selected Photo", key: shortcutLabels.openPhoto },
        { label: "Close active dialog / Deselect", key: shortcutLabels.escape },
      ]
    }
  ];
</script>

<svelte:window onkeydown={handleKeyDown} />

<!-- Modal Backdrop -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  transition:fade={{ duration: 200 }}
  class="backdrop absolute inset-0 z-[100] flex items-center justify-center bg-black/60 px-4 py-6"
  onclick={close}
>
  <!-- Modal Content Card -->
  <div
    transition:scale={{ duration: 250, start: 0.95 }}
    class="modal-card relative flex w-[640px] max-h-[90%] flex-col overflow-hidden text-fg"
    onclick={(e) => e.stopPropagation()}
  >
    <!-- Modal Header -->
    <div class="flex shrink-0 items-center justify-between border-b border-border px-8 py-5">
      <h2 class="content-title m-0">Keyboard Shortcuts</h2>
      <button 
        class="flex size-[26px] items-center justify-center rounded-full border border-border bg-hover text-secondary hover:bg-active hover:text-fg active:scale-95 transition-all cursor-pointer"
        onclick={close}
        aria-label="Close dialog"
      >
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5">
          <path d="M1.5 1.5L8.5 8.5M8.5 1.5L1.5 8.5" stroke-linecap="round" />
        </svg>
      </button>
    </div>

    <!-- Scrollable content -->
    <div class="flex-1 overflow-y-auto px-8 py-6 custom-scrollbar flex flex-col gap-6">
      {#each shortcutGroups as group}
        <section class="group-section">
          <h3 class="group-title">{group.title}</h3>
          <div class="shortcuts-grid">
            {#each group.items as item}
              <div class="shortcut-row">
                <span class="shortcut-label">{item.label}</span>
                <kbd class="shortcut-kbd">{item.key}</kbd>
              </div>
            {/each}
          </div>
        </section>
      {/each}
    </div>

    <!-- Footer Actions -->
    <footer class="flex shrink-0 items-center justify-end gap-3 border-t border-border px-8 py-4">
      <button class="footer-btn footer-btn--primary" onclick={close}>
        Close
      </button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    background: rgba(0, 0, 0, 0.35);
    -webkit-backdrop-filter: blur(16px) saturate(120%);
    backdrop-filter: blur(16px) saturate(120%);
  }

  /* Solid panel + hairline border + one shadow layer. */
  .modal-card {
    background: var(--color-panel);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius);
    box-shadow: var(--shadow-popover);
  }

  .content-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--color-fg);
    letter-spacing: -0.01em;
  }

  /* Scrollbar matching modern design language */
  .custom-scrollbar::-webkit-scrollbar {
    width: 4px;
  }
  .custom-scrollbar::-webkit-scrollbar-track {
    background: transparent;
  }
  .custom-scrollbar::-webkit-scrollbar-thumb {
    background: var(--color-active);
    border-radius: 99px;
  }
  .custom-scrollbar::-webkit-scrollbar-thumb:hover {
    background: var(--color-active);
  }

  /* Shortcut Groups */
  .group-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .group-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--color-subtle);
    margin: 0;
  }

  .shortcuts-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px 24px;
  }

  .shortcut-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 0;
    border-bottom: 1px solid var(--color-border);
  }

  .shortcut-label {
    font-size: 12px;
    color: var(--color-secondary);
  }

  .shortcut-kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 24px;
    height: 20px;
    padding: 0 6px;
    font-family: inherit;
    font-size: 10px;
    font-weight: 600;
    color: var(--color-fg);
    background: var(--color-hover);
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    box-shadow: 0 1px 0 rgba(0, 0, 0, 0.2);
  }

  /* Footer Action Button */
  .footer-btn {
    appearance: none;
    -webkit-appearance: none;
    border: none;
    border-radius: 10px;
    padding: 8px 20px;
    font-size: 12px;
    font-family: inherit;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 150ms ease, transform 150ms ease;
  }

  .footer-btn--primary {
    background: var(--color-accent, #ffffff);
    color: #000;
  }

  .footer-btn--primary:hover {
    background: var(--color-border-strong);
    transform: translateY(-0.5px);
  }

  .footer-btn--primary:active {
    transform: scale(0.98) translateY(0);
  }
</style>
