<script lang="ts">
  import FileBrowser from "../lib/components/file-browser/FileBrowser.svelte";
  import MainViewport from "../lib/components/viewport/MainViewport.svelte";
  import EditPanel from "../lib/components/edit-panel/EditPanel.svelte";
  import ImageBrowser from "../lib/components/image-browser/ImageBrowser.svelte";
  import BottomBar from "../lib/components/shell/BottomBar.svelte";
  import { leftRailCollapsed, isZenMode, imageBrowserCollapsed } from "../stores/editor";
  import { shortcutLabels } from "../lib/shortcuts";
  import { fade } from "svelte/transition";
  import { router } from "svelte-spa-router";
  import { adoptWorkspaceFromRoute } from "../stores/workspace";

  const isZen = $derived($isZenMode);

  $effect(() => {
    adoptWorkspaceFromRoute(router.location);
    leftRailCollapsed.set(true);
  });

  let windowWidth = $state(0);
  let windowHeight = $state(0);

  let leftRailWidth = $state(240);
  let rightRailWidth = $state(280);
  let bottomRailHeight = $state(104);

  const maxLeftRailWidth = $derived(Math.max(200, Math.min(360, windowWidth * 0.28)));
  const maxRightRailWidth = $derived(Math.max(240, Math.min(380, windowWidth * 0.32)));
  const maxBottomRailHeight = $derived(Math.max(72, Math.min(220, windowHeight * 0.28)));

  $effect(() => {
    if (leftRailWidth > maxLeftRailWidth) leftRailWidth = maxLeftRailWidth;
    else if (leftRailWidth < 200) leftRailWidth = 200;
  });
  $effect(() => {
    if (rightRailWidth > maxRightRailWidth) rightRailWidth = maxRightRailWidth;
    else if (rightRailWidth < 240) rightRailWidth = 240;
  });
  $effect(() => {
    if (bottomRailHeight > maxBottomRailHeight) bottomRailHeight = maxBottomRailHeight;
    else if (bottomRailHeight < 72) bottomRailHeight = 72;
  });

  let isResizingLeft = $state(false);
  let isResizingRight = $state(false);
  let isResizingBottom = $state(false);

  function handleLeftResizeStart(e: MouseEvent) {
    e.preventDefault();
    isResizingLeft = true;
    const startX = e.clientX;
    const startWidth = leftRailWidth;
    function handleMouseMove(moveEvent: MouseEvent) {
      leftRailWidth = Math.max(200, Math.min(maxLeftRailWidth, startWidth + (moveEvent.clientX - startX)));
    }
    function handleMouseUp() {
      isResizingLeft = false;
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
      document.body.style.cursor = "";
    }
    document.body.style.cursor = "col-resize";
    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
  }

  function handleRightResizeStart(e: MouseEvent) {
    e.preventDefault();
    isResizingRight = true;
    const startX = e.clientX;
    const startWidth = rightRailWidth;
    function handleMouseMove(moveEvent: MouseEvent) {
      rightRailWidth = Math.max(240, Math.min(maxRightRailWidth, startWidth + (startX - moveEvent.clientX)));
    }
    function handleMouseUp() {
      isResizingRight = false;
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
      document.body.style.cursor = "";
    }
    document.body.style.cursor = "col-resize";
    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
  }

  function handleBottomResizeStart(e: MouseEvent) {
    e.preventDefault();
    isResizingBottom = true;
    const startY = e.clientY;
    const startHeight = bottomRailHeight;
    function handleMouseMove(moveEvent: MouseEvent) {
      bottomRailHeight = Math.max(72, Math.min(maxBottomRailHeight, startHeight + (startY - moveEvent.clientY)));
    }
    function handleMouseUp() {
      isResizingBottom = false;
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
      document.body.style.cursor = "";
    }
    document.body.style.cursor = "row-resize";
    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
  }
</script>

<svelte:window bind:innerWidth={windowWidth} bind:innerHeight={windowHeight} />

<div
  in:fade={{ duration: 180, delay: 60 }}
  out:fade={{ duration: 120 }}
  style="
    display: grid;
    grid-template-columns: {$leftRailCollapsed ? 0 : leftRailWidth}px minmax(0, 1fr) {isZen ? 0 : rightRailWidth}px;
    grid-template-rows: minmax(0, 1fr) auto 22px;
    transition: {isResizingLeft || isResizingRight || isResizingBottom ? 'none' : 'grid-template-columns 220ms var(--ease-out)'};
  "
  class="relative h-full w-full min-h-0 min-w-0 overflow-hidden"
>
  <div
    class="relative z-10 row-span-2 flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-sidebar border-r border-border {$leftRailCollapsed ? 'pointer-events-none' : ''}"
  >
    <FileBrowser class="h-full w-full min-h-0 min-w-0" />
    {#if !$leftRailCollapsed}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        role="separator"
        aria-orientation="vertical"
        class="group absolute right-0 top-0 bottom-0 w-[8px] cursor-col-resize z-50"
        onmousedown={handleLeftResizeStart}
      ></div>
    {/if}
  </div>

  <div class="relative flex min-h-0 min-w-0 flex-col overflow-hidden bg-canvas">
    {#if $leftRailCollapsed}
      <button
        type="button"
        onclick={() => leftRailCollapsed.set(false)}
        aria-label="Expand Sidebar"
        title="Expand sidebar ({shortcutLabels.sidebar})"
        class="absolute left-2 top-1/2 z-50 flex size-[28px] -translate-y-1/2 items-center justify-center rounded-md border border-border bg-panel text-fg hover:bg-hover cursor-pointer"
      >
        <svg width="6" height="10" viewBox="0 0 6 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" class="pointer-events-none">
          <path d="M1.5 1.5L5 5L1.5 8.5" />
        </svg>
      </button>
    {/if}
    <MainViewport />
  </div>

  <div class="relative z-20 row-span-2 flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-sidebar border-l border-border {isZen ? 'pointer-events-none' : ''}">
    {#if !isZen}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        role="separator"
        aria-orientation="vertical"
        class="group absolute left-0 top-0 bottom-0 w-[8px] cursor-col-resize z-50"
        onmousedown={handleRightResizeStart}
      ></div>
    {/if}
    <div class="flex h-full min-h-0 min-w-0 flex-col {isZen ? 'opacity-0' : 'opacity-100'}">
      <EditPanel />
    </div>
  </div>

  <div
    style="
      height: {$imageBrowserCollapsed ? '36px' : `${bottomRailHeight}px`};
      transition: {isResizingBottom ? 'none' : 'height 220ms var(--ease-out)'};
    "
    class="relative z-10 min-w-0 overflow-hidden border-t border-border bg-sidebar"
  >
    <ImageBrowser
      onResizeStart={$imageBrowserCollapsed ? undefined : handleBottomResizeStart}
      class="h-full"
    />
  </div>
  <BottomBar />
</div>
