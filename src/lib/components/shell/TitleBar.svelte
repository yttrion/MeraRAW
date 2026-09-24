<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import IconButton from "../primitives/IconButton.svelte";
  import settingsIcon from "../../icons/settings.svg";
  import helpIcon from "../../icons/help.svg";
  import { isSettingsOpen, isExportOpen, isShortcutsOpen } from "../../../stores/ui";
  import JobsPill from "./JobsPill.svelte";
  import VersionsButton from "./VersionsButton.svelte";
  import { commandPaletteOpen } from "../../../stores/editor";
  import { undo, redo } from "../../../ipc/commands";
  import { reconcile } from "../../../stores/doc";
  import {
    isLibraryRoute,
    workspace,
  } from "../../../stores/workspace";
  import { router } from "svelte-spa-router";

  // Lazy so the page still renders in a plain browser (no Tauri runtime)
  const isLibrary = $derived(isLibraryRoute(router.location));
  const isVideo = $derived($workspace === "video");

  let isFullscreen = $state(false);

  onMount(() => {
    if ((window as any).__TAURI_INTERNALS__) {
      const windowInstance = getCurrentWindow();
      
      // Initial check
      windowInstance.isFullscreen().then((val) => {
        isFullscreen = val;
      });

      // Update fullscreen state on window resize
      const unlistenPromise = windowInstance.onResized(() => {
        windowInstance.isFullscreen().then((val) => {
          isFullscreen = val;
        });
      });

      return () => {
        unlistenPromise.then((unlisten) => unlisten());
      };
    }
  });

  function beginDrag(e: MouseEvent) {
    if (e.button !== 0) return;
    if (!(window as any).__TAURI_INTERNALS__) return;
    const t = e.target as HTMLElement | null;
    if (!t) return;
    if (t.closest("[data-tauri-drag-region='false']")) return;
    if (t.closest("button, input, a, select, textarea")) return;
    void getCurrentWindow().startDragging();
  }
</script>

<header
  data-tauri-drag-region
  class="titlebar relative z-20 flex h-full min-h-0 items-center bg-bg pr-3 {isFullscreen ? 'pl-4' : 'pl-[100px]'} border-b border-border/80"
  onmousedown={beginDrag}
>
  <div class="titlebar-grip" data-tauri-drag-region></div>

  <div class="flex items-center gap-[4px]" data-tauri-drag-region="false">
    <JobsPill />
    <VersionsButton />
    <button type="button" class="quiet-btn" onclick={() => void undo().then(reconcile).catch(() => {})} title="Undo">Undo</button>
    <button type="button" class="quiet-btn" onclick={() => void redo().then(reconcile).catch(() => {})} title="Redo">Redo</button>
    <button type="button" class="quiet-btn" onclick={() => commandPaletteOpen.set(true)} title="Commands (⌘K)">⌘K</button>
    <IconButton
      icon={settingsIcon}
      label="Settings"
      title="Settings"
      iconClass="size-[18px]"
      onclick={() => isSettingsOpen.set(true)}
    />
    <IconButton
      icon={helpIcon}
      label="Help"
      title="Keyboard Shortcuts"
      iconClass="h-[16px] w-[11px]"
      onclick={() => isShortcutsOpen.set(true)}
    />

    {#if !isLibrary}
      <button
        type="button"
        class="export-btn"
        title="Export"
        onclick={() => isExportOpen.set(true)}
      >
        Export
      </button>
    {/if}
  </div>
</header>

<style>
  .titlebar {
    -webkit-app-region: drag;
    app-region: drag;
  }
  .titlebar :global(button),
  .titlebar [data-tauri-drag-region="false"] {
    -webkit-app-region: no-drag;
    app-region: no-drag;
  }
  .titlebar-grip {
    flex: 1 1 auto;
    align-self: stretch;
    min-width: 48px;
    min-height: 100%;
  }
  .quiet-btn {
    height: 26px;
    padding: 0 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--color-secondary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .quiet-btn:hover {
    background: var(--color-hover);
    color: var(--color-fg);
  }
  .export-btn {
    height: 26px;
    padding: 0 11px;
    border: 1px solid var(--color-border);
    border-radius: 7px;
    background: var(--color-fg);
    color: var(--color-panel);
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
  }
  .export-btn:hover { opacity: 0.88; }
</style>
