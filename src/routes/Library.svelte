<script lang="ts">
  import GlassPanel from "../lib/components/primitives/GlassPanel.svelte";
  import FileBrowser from "../lib/components/file-browser/FileBrowser.svelte";
  import PhotoDetailsPanel from "../lib/components/library/PhotoDetailsPanel.svelte";
  import CollectionFilters from "../lib/components/library/CollectionFilters.svelte";
  import ContextMenu from "../lib/components/primitives/ContextMenu.svelte";
  import type { ContextMenuItem } from "../lib/components/primitives/ContextMenu.svelte";
  import { leftRailCollapsed, photoDetailsCollapsed } from "../stores/editor";
  import { isExportOpen } from "../stores/ui";
  import {
    activePhoto,
    applyLibraryFilters,
    clearLibraryFilters,
    folder,
    folderHydrate,
    folderLeafName,
    folders,
    libraryFilters,
    libraryFiltersActive,
    libraryItems,
    libraryPinFor,
    loadFolder,
    openPhoto as browseOpenPhoto,
    patchPhotoMeta,
    photos,
    sameFolderPath,
    thumbUrl,
    gridKey,
  } from "../stores/browse";
  import type { GridItem } from "../ipc/types";
  import { push, router } from "svelte-spa-router";
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { shortcutLabels } from "../lib/shortcuts";
  import { copyGradeFrom, pasteGradeOnto, pasteGradeOntoPaths } from "../lib/grade";
  import { virtualCopy } from "../ipc/commands";
  import { adoptWorkspaceFromRoute, editorRoute, setWorkspace, workspace } from "../stores/workspace";
  import { isVideoPath } from "../lib/media";

  $effect(() => {
    adoptWorkspaceFromRoute(router.location);
    leftRailCollapsed.set(false);
  });

  const isVideo = $derived($workspace === "video");
  const libraryPin = $derived(libraryPinFor($folder, $folders));
  const showingNestedFolder = $derived(
    Boolean($folder && libraryPin && !sameFolderPath($folder, libraryPin.root)),
  );

  // Window dimensions for responsive boundaries
  let windowWidth = $state(0);

  // Left rail width
  let leftRailWidth = $state(220);
  let isResizingLeft = $state(false);

  const maxLeftRailWidth = $derived(Math.max(180, Math.min(400, windowWidth * 0.35)));

  $effect(() => {
    if (leftRailWidth > maxLeftRailWidth) leftRailWidth = maxLeftRailWidth;
    else if (leftRailWidth < 180) leftRailWidth = 180;
  });

  function handleLeftResizeStart(e: MouseEvent) {
    e.preventDefault();
    isResizingLeft = true;
    const startX = e.clientX;
    const startWidth = leftRailWidth;

    function handleMouseMove(moveEvent: MouseEvent) {
      const deltaX = moveEvent.clientX - startX;
      leftRailWidth = Math.max(180, Math.min(maxLeftRailWidth, startWidth + deltaX));
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

  function handleLeftKeyDown(e: KeyboardEvent) {
    if (e.key === "ArrowRight") leftRailWidth = Math.max(180, Math.min(maxLeftRailWidth, leftRailWidth + 10));
    else if (e.key === "ArrowLeft") leftRailWidth = Math.max(180, Math.min(maxLeftRailWidth, leftRailWidth - 10));
  }

  // Right rail (photo details panel) width
  let rightRailWidth = $state(260);
  let isResizingRight = $state(false);

  const maxRightRailWidth = $derived(Math.max(200, Math.min(450, windowWidth * 0.35)));

  $effect(() => {
    if (rightRailWidth > maxRightRailWidth) rightRailWidth = maxRightRailWidth;
    else if (rightRailWidth < 200) rightRailWidth = 200;
  });

  function handleRightResizeStart(e: MouseEvent) {
    e.preventDefault();
    isResizingRight = true;
    const startX = e.clientX;
    const startWidth = rightRailWidth;

    function handleMouseMove(moveEvent: MouseEvent) {
      const deltaX = startX - moveEvent.clientX; // drag left increases width
      rightRailWidth = Math.max(200, Math.min(maxRightRailWidth, startWidth + deltaX));
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

  function handleRightKeyDown(e: KeyboardEvent) {
    if (e.key === "ArrowLeft") rightRailWidth = Math.max(200, Math.min(maxRightRailWidth, rightRailWidth + 10));
    else if (e.key === "ArrowRight") rightRailWidth = Math.max(200, Math.min(maxRightRailWidth, rightRailWidth - 10));
  }

  // Sort is client-side (name asc/desc isn't a catalog sort). Filters go to get_grid.
  type SortMode = "date-desc" | "date-asc" | "name-asc" | "name-desc" | "rating";
  let sortMode = $state<SortMode>("date-desc");
  let searchQuery = $state($libraryFilters.text);
  let showSortMenu = $state(false);
  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    const t = $libraryFilters.text;
    searchQuery = t;
    if (t === "") window.clearTimeout(searchTimer);
  });

  function onSearchInput(e: Event) {
    const value = (e.currentTarget as HTMLInputElement).value;
    searchQuery = value;
    window.clearTimeout(searchTimer);
    searchTimer = window.setTimeout(() => {
      void applyLibraryFilters({ text: value });
    }, 250);
  }

  const sortLabels: Record<SortMode, string> = {
    "date-desc": "Newest first",
    "date-asc":  "Oldest first",
    "name-asc":  "Name A–Z",
    "name-desc": "Name Z–A",
    "rating":    "Rating",
  };

  let selectedKey = $state<string | null>(null);
  let selectedKeys = $state<string[]>([]);
  let selectAnchor = $state<string | null>(null);

  const selectedSet = $derived(new Set(selectedKeys));

  const filteredPhotos = $derived.by(() => {
    let items = [...$libraryItems];
    switch (sortMode) {
      case "date-desc":
        items = [...items].sort((a, b) =>
          (b.capturedAt ?? "").localeCompare(a.capturedAt ?? ""),
        );
        break;
      case "date-asc":
        items = [...items].sort((a, b) =>
          (a.capturedAt ?? "").localeCompare(b.capturedAt ?? ""),
        );
        break;
      case "name-asc":
        items = [...items].sort((a, b) => a.filename.localeCompare(b.filename));
        break;
      case "name-desc":
        items = [...items].sort((a, b) => b.filename.localeCompare(a.filename));
        break;
      case "rating":
        items = [...items].sort((a, b) => b.rating - a.rating);
        break;
    }
    return items;
  });

  function selectPhoto(photo: GridItem, e?: MouseEvent | KeyboardEvent) {
    const list = filteredPhotos;
    const key = gridKey(photo);
    if (e && "shiftKey" in e && e.shiftKey && selectAnchor) {
      const a = list.findIndex((p) => gridKey(p) === selectAnchor);
      const b = list.findIndex((p) => gridKey(p) === key);
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        selectedKeys = list.slice(lo, hi + 1).map(gridKey);
      }
    } else if (e && "metaKey" in e && (e.metaKey || e.ctrlKey)) {
      selectedKeys = selectedSet.has(key)
        ? selectedKeys.filter((k) => k !== key)
        : [...selectedKeys, key];
      selectAnchor = key;
    } else {
      selectedKeys = [key];
      selectAnchor = key;
    }
    selectedKey = key;
  }

  async function openPhoto(photo: GridItem) {
    selectedKey = gridKey(photo);
    await browseOpenPhoto(photo);
    // browseOpenPhoto already navigates to edit route via openPath;
    // push again with startViewTransition can crash on Linux WebKitGTK.
    // Use a plain push since navigation already completed.
    push(editorRoute());
  }

  async function ratePhoto(photo: GridItem, rating: number, e: MouseEvent) {
    e.stopPropagation();
    const next = photo.rating === rating ? 0 : rating;
    try {
      await patchPhotoMeta([photo.id], { rating: next });
    } catch {
      /* ignore */
    }
  }

  async function cycleFlag(photo: GridItem, e: MouseEvent) {
    e.stopPropagation();
    const order = ["none", "pick", "reject"] as const;
    const i = Math.max(0, order.indexOf(photo.flag as (typeof order)[number]));
    const next = order[(i + 1) % order.length];
    try {
      await patchPhotoMeta([photo.id], { flag: next });
    } catch {
      /* ignore */
    }
  }

  const highlightKey = $derived(selectedKey ?? ($activePhoto ? gridKey($activePhoto) : null));
  const selected = $derived(
    filteredPhotos.find((p) => gridKey(p) === highlightKey) ?? null,
  );

  const selectedItems = $derived(
    selectedKeys
      .map((k) => filteredPhotos.find((p) => gridKey(p) === k))
      .filter((p): p is GridItem => !!p),
  );

  // Keyboard navigation across the photo grid
  let gridEl = $state<HTMLDivElement | null>(null);

  function isEditableTarget(target: EventTarget | null) {
    if (!(target instanceof HTMLElement)) return false;
    return (
      target.isContentEditable ||
      ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)
    );
  }

  function getColumnCount() {
    if (!gridEl) return 1;
    return getComputedStyle(gridEl).gridTemplateColumns.split(" ").length || 1;
  }

  function handleGridKeydown(e: KeyboardEvent) {
    if (isEditableTarget(e.target)) return;
    const list = filteredPhotos;
    if (!list.length) return;

    if (e.key === "Escape") {
      selectedKeys = highlightKey ? [highlightKey] : [];
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      selectedKeys = list.map(gridKey);
      return;
    }

    if (e.key === "Enter") {
      const current = list.find((p) => gridKey(p) === highlightKey);
      if (current) void openPhoto(current);
      return;
    }

    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) return;
    e.preventDefault();

    const currentIndex = highlightKey ? list.findIndex((p) => gridKey(p) === highlightKey) : -1;
    const cols = getColumnCount();
    let nextIndex = currentIndex;

    if (e.key === "ArrowLeft") nextIndex = Math.max(0, currentIndex - 1);
    else if (e.key === "ArrowRight") nextIndex = Math.min(list.length - 1, currentIndex === -1 ? 0 : currentIndex + 1);
    else if (e.key === "ArrowUp") nextIndex = Math.max(0, currentIndex - cols);
    else if (e.key === "ArrowDown") nextIndex = Math.min(list.length - 1, currentIndex === -1 ? 0 : currentIndex + cols);

    if (nextIndex !== currentIndex) selectPhoto(list[nextIndex], e);
  }

  // ── Context Menu ─────────────────────────────────────────
  let ctxMenu = $state<{ x: number; y: number } | null>(null);
  let ctxPhoto = $state<GridItem | null>(null);

  function handlePhotoContextMenu(e: MouseEvent, photo: GridItem) {
    e.preventDefault();
    e.stopPropagation();
    selectPhoto(photo);
    ctxPhoto = photo;
    ctxMenu = { x: e.clientX, y: e.clientY };
  }

  const ctxItems = $derived<ContextMenuItem[]>([
    {
      type: "item",
      label: isVideo ? "Open in video editor" : "Open in editor",
      shortcut: shortcutLabels.openPhoto,
      onclick: () => { if (ctxPhoto) void openPhoto(ctxPhoto); },
    },
    {
      type: "item",
      label: "Export…",
      shortcut: shortcutLabels.export,
      onclick: () => isExportOpen.set(true),
    },
    { type: "separator" },
    {
      type: "item",
      label: "Copy grade",
      shortcut: shortcutLabels.copyGrade,
      onclick: () => {
        if (ctxPhoto) void copyGradeFrom(ctxPhoto.path);
      },
    },
    {
      type: "item",
      label: "Paste grade",
      shortcut: shortcutLabels.pasteGrade,
      onclick: () => {
        if (ctxPhoto) void pasteGradeOnto(ctxPhoto.path);
      },
    },
    {
      type: "item",
      label: selectedKeys.length > 1 ? `Paste grade to ${selectedKeys.length} selected` : "Paste grade to selected",
      disabled: selectedKeys.length < 1,
      onclick: () => {
        const paths = [
          ...new Set(
            (selectedItems.length ? selectedItems : ctxPhoto ? [ctxPhoto] : []).map((p) => p.path),
          ),
        ];
        void pasteGradeOntoPaths(paths);
      },
    },
    {
      type: "item",
      label: "Create virtual copy",
      onclick: () => {
        if (!ctxPhoto) return;
        void virtualCopy(ctxPhoto.path);
      },
    },
    { type: "separator" },
    {
      type: "item",
      label: "Copy file path",
      onclick: () => {
        if (ctxPhoto) navigator.clipboard?.writeText(ctxPhoto.path);
      },
    },
    { type: "separator" },
    {
      type: "item",
      label: "Deselect",
      shortcut: shortcutLabels.escape,
      onclick: () => {
        selectedKey = null;
        selectedKeys = [];
      },
    },
  ]);
</script>

<svelte:window bind:innerWidth={windowWidth} onkeydown={handleGridKeydown} />

<div
  in:fade={{ duration: 200, delay: 100 }}
  out:fade={{ duration: 150 }}
  style="grid-template-columns: auto minmax(0, 1fr) auto;"
  class="relative grid h-full min-h-0 w-full flex-1 gap-[11px] px-[11px] pb-[11px]"
>
  <!-- Animated Left Rail Container -->
  <div 
    style="
      width: {$leftRailCollapsed ? '0px' : `${leftRailWidth}px`};
      margin-right: {$leftRailCollapsed ? '-11px' : '0px'};
      transition: {isResizingLeft ? 'none' : 'width 350ms cubic-bezier(0.16, 1, 0.3, 1), margin-right 350ms cubic-bezier(0.16, 1, 0.3, 1)'};
    "
    class="relative min-h-0 {$leftRailCollapsed ? 'overflow-hidden pointer-events-none' : 'overflow-visible'}"
  >
    <!-- Left Rail: Folder Browser -->
    <FileBrowser class="h-full {isResizingLeft ? 'transition-none' : ''}" />
    <!-- Resize Handle -->
    {#if !$leftRailCollapsed}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        role="separator"
        aria-orientation="vertical"
        aria-valuenow={leftRailWidth}
        aria-valuemin={180}
        aria-valuemax={maxLeftRailWidth}
        tabindex="0"
        class="group absolute -right-[11px] top-0 bottom-0 w-[11px] cursor-col-resize z-50 flex items-center justify-center focus:outline-none"
        onmousedown={handleLeftResizeStart}
        onkeydown={handleLeftKeyDown}
      >
        <div class="w-[2px] h-[40px] rounded-full bg-hover group-hover:bg-border-strong group-active:bg-border-strong transition-all duration-200"></div>
      </div>
    {/if}
  </div>

  <!-- Main Content Area Wrapper -->
  <div class="relative min-h-0 flex flex-col">
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
    {#if $photoDetailsCollapsed}
      <button
        type="button"
        onclick={() => photoDetailsCollapsed.set(false)}
        aria-label="Expand Details"
        title="Expand details ({shortcutLabels.details})"
        class="absolute right-2 top-1/2 z-50 flex size-[28px] -translate-y-1/2 items-center justify-center rounded-md border border-border bg-panel text-fg hover:bg-hover cursor-pointer"
      >
        <svg width="6" height="10" viewBox="0 0 6 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" class="pointer-events-none rotate-180">
          <path d="M1.5 1.5L5 5L1.5 8.5" />
        </svg>
      </button>
    {/if}
    <GlassPanel class="flex-1 flex min-h-0 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div class="flex shrink-0 flex-wrap items-center gap-[8px] px-[18px] pt-[18px] pb-[12px]">
        <!-- Sort / Filter button -->
        <div class="relative">
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <button
            class="toolbar-btn"
            onclick={() => (showSortMenu = !showSortMenu)}
            aria-label="Sort and filter"
            title="Sort and filter"
          >
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M2 3.5H14L9 9V13.5L7 14.5V9L2 3.5Z" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </button>

          {#if showSortMenu}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <!-- svelte-ignore a11y_interactive_supports_focus -->
            <div
              transition:scale={{ duration: 180, start: 0.95, easing: cubicOut }}
              class="sort-menu"
              role="menu"
              tabindex="-1"
              onclick={() => (showSortMenu = false)}
              onkeydown={(e) => e.key === 'Escape' && (showSortMenu = false)}
            >
              {#each Object.entries(sortLabels) as [mode, label]}
                <button
                  role="menuitem"
                  class="sort-item {sortMode === mode ? 'sort-item--active' : ''}"
                  onclick={() => { sortMode = mode as SortMode; showSortMenu = false; }}
                >
                  {label}
                  {#if sortMode === mode}
                    <svg class="ml-auto" width="10" height="8" viewBox="0 0 10 8" fill="none">
                      <path d="M1 4L3.5 6.5L9 1" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
                  {/if}
                </button>
              {/each}
            </div>
          {/if}
        </div>

        <CollectionFilters />

        <div class="flex-1"></div>

        <!-- Search bar -->
        <label class="search-wrap">
          <input
            type="search"
            placeholder="Search filename, camera, keywords"
            value={searchQuery}
            oninput={onSearchInput}
            class="search-input"
          />
          <div class="search-icon-circle">
            <svg width="15" height="15" viewBox="0 0 15 15" fill="none" class="search-icon">
              <circle cx="6.5" cy="6.5" r="4.5" stroke="currentColor" stroke-width="1.5"/>
              <path d="M10 10L13.5 13.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
            </svg>
          </div>
        </label>
      </div>

      {#if showingNestedFolder && libraryPin && $folder}
        <div class="all-photos-bar">
          <button
            type="button"
            class="all-photos-chip"
            title="Show every photo in {libraryPin.name}"
            onclick={() => void loadFolder(libraryPin.root)}
          >
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
              <path d="M7.5 2L3.5 6l4 4" />
            </svg>
            All photos in {libraryPin.name}
          </button>
          <span class="all-photos-here" title={$folder}>{folderLeafName($folder)}</span>
        </div>
      {/if}

      <!-- Photo Grid -->
      <div class="min-h-0 flex-1 overflow-y-auto px-[18px] pb-[18px] pt-[6px]">
        {#if !$folder}
          <!-- Empty state: no folder selected -->
          <div class="flex h-full flex-col items-center justify-center gap-3 text-center">
            <div class="empty-icon-wrap">
              <svg width="36" height="36" viewBox="0 0 36 36" fill="none" xmlns="http://www.w3.org/2000/svg">
                <rect x="1.5" y="8.5" width="33" height="24" rx="4.5" stroke="currentColor" stroke-width="1.5"/>
                <path d="M1.5 15H34.5" stroke="currentColor" stroke-width="1.5"/>
                <path d="M10 3.5H6a4.5 4.5 0 0 0-4.5 4.5V15" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
                <path d="M12 3.5H14" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
                <circle cx="18" cy="23" r="4" stroke="currentColor" stroke-width="1.5"/>
                <path d="M30 20.5L25.5 25" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
              </svg>
            </div>
            <p class="text-[13px] font-medium text-subtle">No folder open</p>
            <p class="text-[11px] text-subtle">
              Select a folder from the sidebar to browse your {isVideo ? "clips" : "photos"}
            </p>
          </div>
        {:else if filteredPhotos.length === 0}
          <!-- Empty state: folder open but no matching photos -->
          <div class="flex h-full flex-col items-center justify-center gap-3 text-center">
            {#if $folderHydrate?.path === $folder && $folderHydrate.status === "importing"}
              <p class="text-[13px] font-medium text-subtle">
                {isVideo ? "Importing clips…" : "Importing photos…"}
              </p>
              <p class="empty-state text-[11px] text-subtle">
                This folder is on disk but not in the library yet. Watch the jobs pill for progress.
              </p>
            {:else if $folderHydrate?.path === $folder && $folderHydrate.status === "error"}
              <p class="text-[13px] font-medium text-subtle">{isVideo ? "No clips found" : "No photos found"}</p>
              <p class="empty-state text-[11px] text-subtle">{$folderHydrate.message}</p>
            {:else}
              <p class="text-[13px] font-medium text-subtle">{isVideo ? "No clips found" : "No photos found"}</p>
            {/if}
            {#if libraryFiltersActive($libraryFilters)}
              <p class="empty-state text-[11px] text-subtle">No results match these filters</p>
              <button
                type="button"
                class="mt-1 rounded-[8px] border border-border px-3 py-[5px] text-[12px] text-fg"
                onclick={() => void clearLibraryFilters()}
              >
                Clear filters
              </button>
            {:else if $photos.length > 0 && isVideo}
              <p class="text-[11px] text-subtle">
                This folder has photos. Switch editors to see them.
              </p>
              <button
                type="button"
                class="mt-1 rounded-[8px] border border-border px-3 py-[5px] text-[12px] text-fg"
                onclick={() => setWorkspace("photo")}
              >
                Open Photo Editor
              </button>
            {/if}
          </div>
        {:else}
          {#if selectedKeys.length === 2}
            {@const pair = selectedItems}
            {#if pair.length === 2}
              <div class="cull-compare">
                {#each pair as photo (gridKey(photo))}
                  <button type="button" class="cull-pane" onclick={() => void openPhoto(photo)}>
                    {#if photo.hasThumb}
                      <img src={thumbUrl(photo.id)} alt={photo.filename} />
                    {/if}
                    <span>{photo.filename}</span>
                  </button>
                {/each}
              </div>
            {/if}
          {/if}
          <!-- Photo Grid -->
          <div class="photo-grid" bind:this={gridEl}>
            {#each filteredPhotos as photo (gridKey(photo))}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <button
                class="photo-card {highlightKey === gridKey(photo) ? 'photo-card--active' : ''} {selectedSet.has(gridKey(photo)) && highlightKey !== gridKey(photo) ? 'photo-card--selected' : ''}"
                onclick={(e) => selectPhoto(photo, e)}
                ondblclick={() => openPhoto(photo)}
                oncontextmenu={(e) => handlePhotoContextMenu(e, photo)}
                title={photo.filename}
              >
                <div class="photo-thumb-wrap">
                  {#if photo.hasThumb}
                    <img
                      src={thumbUrl(photo.id)}
                      alt={photo.filename}
                      loading="lazy"
                      class="photo-thumb"
                      style={highlightKey === gridKey(photo) ? 'view-transition-name: active-image;' : ''}
                    />
                  {:else}
                    <div class="photo-thumb-placeholder">
                      <span class="text-[10px] font-semibold tracking-widest text-subtle">{isVideoPath(photo.filename) || isVideo ? "CLIP" : "RAW"}</span>
                    </div>
                  {/if}
                </div>
                <div class="photo-meta">
                  <span class="photo-name">{photo.filename}</span>
                  <div class="photo-cull">
                    {#if photo.flag === "pick"}
                      <span class="flag-dot flag-dot--pick" title="Pick">●</span>
                    {:else if photo.flag === "reject"}
                      <span class="flag-dot flag-dot--reject" title="Reject">✕</span>
                    {/if}
                    {#if photo.rating > 0}
                      <span class="rating-mini">{"★".repeat(photo.rating)}</span>
                    {/if}
                  </div>
                </div>
              </button>
            {/each}
          </div>
        {/if}
      </div>

      {#if selected}
        <div class="cull-bar">
          <span class="cull-name">{selected.filename}</span>
          <div class="cull-stars" role="group" aria-label="Rating">
            {#each [1, 2, 3, 4, 5] as n}
              <button
                type="button"
                class="star-btn {selected.rating >= n ? 'star-btn--on' : ''}"
                title="{n} star{n === 1 ? '' : 's'}"
                onclick={(e) => void ratePhoto(selected, n, e)}
              >★</button>
            {/each}
          </div>
          <button
            type="button"
            class="flag-btn flag-btn--{selected.flag || 'none'}"
            title="Cycle flag (none → pick → reject)"
            onclick={(e) => void cycleFlag(selected, e)}
          >
            {#if selected.flag === "pick"}Pick
            {:else if selected.flag === "reject"}Reject
            {:else}Flag{/if}
          </button>
        </div>
      {/if}
    </GlassPanel>
  </div>

  <!-- Animated Right Rail Container -->
  <div
    style="
      width: {$photoDetailsCollapsed ? '0px' : `${rightRailWidth}px`};
      margin-left: {$photoDetailsCollapsed ? '-11px' : '0px'};
      transition: {isResizingRight ? 'none' : 'width 350ms cubic-bezier(0.16, 1, 0.3, 1), margin-left 350ms cubic-bezier(0.16, 1, 0.3, 1)'};
    "
    class="relative min-h-0 {$photoDetailsCollapsed ? 'overflow-hidden pointer-events-none' : 'overflow-visible'}"
  >
    <PhotoDetailsPanel class="h-full {isResizingRight ? 'transition-none' : ''}" />
    <!-- Resize Handle -->
    {#if !$photoDetailsCollapsed}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        role="separator"
        aria-orientation="vertical"
        aria-valuenow={rightRailWidth}
        aria-valuemin={200}
        aria-valuemax={maxRightRailWidth}
        tabindex="0"
        class="group absolute -left-[11px] top-0 bottom-0 w-[11px] cursor-col-resize z-50 flex items-center justify-center focus:outline-none"
        onmousedown={handleRightResizeStart}
        onkeydown={handleRightKeyDown}
      >
        <div class="w-[2px] h-[40px] rounded-full bg-hover group-hover:bg-border-strong group-active:bg-border-strong transition-all duration-200"></div>
      </div>
    {/if}
  </div>

</div>

{#if ctxMenu}
  <ContextMenu
    x={ctxMenu.x}
    y={ctxMenu.y}
    items={ctxItems}
    onclose={() => (ctxMenu = null)}
  />
{/if}

<!-- Click-outside to close sort menu -->
{#if showSortMenu}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="fixed inset-0 z-30"
    onclick={() => (showSortMenu = false)}
    onkeydown={(e) => e.key === 'Escape' && (showSortMenu = false)}
    role="presentation"
  ></div>
{/if}

<style>
  /* ── Toolbar ──────────────────────────────────────────── */
  .toolbar-btn {
    appearance: none;
    -webkit-appearance: none;
    border: 1px solid var(--color-border-strong);
    padding: 0;
    margin: 0;
    cursor: pointer;

    display: flex;
    align-items: center;
    justify-content: center;

    width: 36px;
    height: 36px;
    border-radius: 9999px;
    background: var(--color-hover);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.08),
      inset 0 -1px 0 rgba(0, 0, 0, 0.2),
      0 2px 6px rgba(0, 0, 0, 0.25);
    transition: background 200ms ease, transform 200ms cubic-bezier(0.25, 1, 0.5, 1);
  }

  .toolbar-btn:hover {
    background: var(--color-active);
    transform: translateY(-0.5px);
  }

  .toolbar-btn:active {
    transform: scale(0.96);
    background: var(--color-active);
  }

  .all-photos-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 18px 10px;
    min-width: 0;
  }
  .all-photos-chip {
    appearance: none;
    border: 1px solid var(--color-border-strong);
    background: var(--color-hover);
    color: var(--color-fg);
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px;
    border-radius: 999px;
    cursor: pointer;
    white-space: nowrap;
  }
  .all-photos-chip:hover { background: var(--color-active); }
  .all-photos-here {
    color: var(--color-subtle);
    font-size: 12px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ── Sort Menu ────────────────────────────────────────── */
  .sort-menu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 40;
    min-width: 150px;
    background: var(--color-panel);
    -webkit-backdrop-filter: blur(20px);
    backdrop-filter: blur(20px);
    border: 1px solid var(--color-border-strong);
    border-radius: 12px;
    padding: 5px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .sort-item {
    appearance: none;
    -webkit-appearance: none;
    border: none;
    background: transparent;
    font: inherit;
    color: var(--color-secondary);
    cursor: pointer;

    display: flex;
    align-items: center;
    width: 100%;
    padding: 7px 10px;
    border-radius: 8px;
    font-size: 12px;
    text-align: left;
    transition: background 150ms ease, color 150ms ease;
  }

  .sort-item:hover {
    background: var(--color-hover);
    color: var(--color-fg);
  }

  .sort-item--active {
    color: var(--color-fg);
    font-weight: 500;
  }

  /* ── Search ───────────────────────────────────────────── */
  .search-wrap {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    height: 36px;
    width: 240px;
    padding: 0 4px 0 16px;
    border-radius: 9999px;
    border: 1px solid var(--color-border);
    background: var(--color-sunken);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.05),
      inset 0 -1px 0 rgba(0, 0, 0, 0.2),
      0 2px 6px rgba(0, 0, 0, 0.25);
    transition: background 150ms cubic-bezier(0.4, 0, 0.2, 1), border-color 150ms cubic-bezier(0.4, 0, 0.2, 1);
    cursor: text;
  }

  .search-wrap:focus-within {
    background: var(--color-hover);
    border-color: var(--color-border-strong);
  }

  .search-icon-circle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.25);
  }

  .search-icon {
    flex-shrink: 0;
  }

  .search-input {
    appearance: none;
    -webkit-appearance: none;
    border: none;
    background: transparent;
    font: inherit;
    color: var(--color-fg);
    font-size: 12px;
    flex: 1;
    outline: none;
  }

  .search-input::placeholder {
    color: var(--color-subtle);
  }

  /* Remove native search clear button */
  .search-input::-webkit-search-cancel-button {
    display: none;
  }

  /* ── Empty State ──────────────────────────────────────── */
  .empty-icon-wrap {
    width: 64px;
    height: 64px;
    border-radius: 18px;
    background: var(--color-hover);
    border: 1px solid var(--color-border);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  /* ── Photo Grid ───────────────────────────────────────── */
  .photo-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 10px;
  }

  .photo-card {
    appearance: none;
    -webkit-appearance: none;
    border: 1px solid var(--color-border);
    padding: 0;
    margin: 0;
    background: var(--color-hover);
    cursor: pointer;
    overflow: hidden;
    text-align: left;
    transition: border-color 180ms ease, background 180ms ease, box-shadow 200ms ease;
  }

  .photo-card:hover {
    border-color: var(--color-border-strong);
    background: var(--color-active);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.15);
  }

  .photo-card:active {
    transform: scale(0.98);
  }

  .photo-card--active {
    border-color: var(--color-fg);
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.3), 0 2px 8px rgba(0, 0, 0, 0.2);
    background: var(--color-hover);
  }
  .photo-card--selected {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 1px var(--color-accent);
  }
  .cull-compare {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    margin: 0 0 12px;
  }
  .cull-pane {
    appearance: none;
    border: 1px solid var(--color-border-strong);
    border-radius: 10px;
    overflow: hidden;
    background: var(--color-sunken);
    color: var(--color-fg);
    cursor: pointer;
    padding: 0;
    text-align: left;
  }
  .cull-pane img {
    display: block;
    width: 100%;
    aspect-ratio: 3 / 2;
    object-fit: cover;
  }
  .cull-pane span {
    display: block;
    padding: 6px 10px;
    font-size: 11px;
    color: var(--color-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .photo-thumb-wrap {
    width: 100%;
    aspect-ratio: 3 / 2;
    overflow: hidden;
    background: var(--color-hover);
  }

  .photo-thumb {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .photo-thumb-placeholder {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--color-hover);
  }

  .photo-meta {
    padding: 7px 10px 8px;
    border-top: 1px solid var(--color-border);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }

  .photo-name {
    font-size: 10px;
    color: var(--color-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: block;
    letter-spacing: 0.01em;
    min-width: 0;
  }

  .photo-cull {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .flag-dot {
    font-size: 9px;
    line-height: 1;
  }
  .flag-dot--pick { color: rgba(120, 200, 120, 0.9); }
  .flag-dot--reject { color: rgba(220, 120, 120, 0.9); }

  .rating-mini {
    font-size: 8px;
    color: rgba(255, 210, 120, 0.85);
    letter-spacing: -0.5px;
  }

  .cull-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 18px 14px;
    border-top: 1px solid var(--color-border);
    flex-shrink: 0;
  }

  .cull-name {
    font-size: 11px;
    color: var(--color-secondary);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .cull-stars {
    display: flex;
    gap: 2px;
  }

  .star-btn {
    appearance: none;
    border: none;
    background: transparent;
    color: var(--color-subtle);
    font-size: 14px;
    line-height: 1;
    padding: 2px 3px;
    cursor: pointer;
  }
  .star-btn--on { color: rgba(255, 210, 120, 0.95); }
  .star-btn:hover { color: rgba(255, 220, 140, 0.9); }

  .flag-btn {
    appearance: none;
    border: 1px solid var(--color-border-strong);
    background: var(--color-hover);
    color: var(--color-secondary);
    font-size: 10px;
    font-weight: 600;
    padding: 5px 10px;
    border-radius: 8px;
    cursor: pointer;
  }
  .flag-btn--pick {
    border-color: rgba(120, 200, 120, 0.35);
    color: rgba(160, 220, 160, 0.95);
  }
  .flag-btn--reject {
    border-color: rgba(220, 120, 120, 0.35);
    color: rgba(230, 150, 150, 0.95);
  }
</style>

