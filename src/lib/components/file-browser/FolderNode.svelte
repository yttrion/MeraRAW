<script lang="ts">
  import { lastOpenedPath } from "../../../stores/app";
  import { openLibraryFile, loadFolder, sameFolderPath } from "../../../stores/browse";
  import { listFolderChildren } from "../../../ipc/commands";
  import type { FolderChild } from "../../../ipc/types";
  import FolderNode from "./FolderNode.svelte";

  const FILE_CAP = 80;

  let {
    name,
    path,
    isDir = true,
    kind = null,
    depth = 0,
    count = null,
    selectOnClick = true,
    showPin = false,
    accessible = true,
    section = "photo",
    activePath = null,
    onPin,
    onSelectFolder,
    onContextMenu,
  }: {
    name: string;
    path: string;
    isDir?: boolean;
    kind?: string | null;
    depth?: number;
    count?: number | null;
    selectOnClick?: boolean;
    showPin?: boolean;
    accessible?: boolean;
    section?: "photo" | "video" | "found";
    activePath?: string | null;
    onPin?: () => void;
    onSelectFolder?: (path: string, section: "photo" | "video") => void;
    onContextMenu?: (e: MouseEvent, path: string) => void;
  } = $props();

  let expanded = $state(false);
  let loading = $state(false);
  let kids = $state<FolderChild[] | null>(null);
  let error = $state<string | null>(null);
  let fetchGen = 0;

  const dirs = $derived((kids ?? []).filter((k) => k.isDir));
  const sectionFiles = $derived(
    (kids ?? []).filter((k) => {
      if (k.isDir) return false;
      if (section === "photo") return k.kind === "photo";
      if (section === "video") return k.kind === "video";
      return true;
    }),
  );
  const visibleFiles = $derived(sectionFiles.slice(0, FILE_CAP));
  const extra = $derived(Math.max(0, sectionFiles.length - FILE_CAP));
  const isOpen = $derived(Boolean(path && $lastOpenedPath && sameFolderPath($lastOpenedPath, path)));
  const isActive = $derived(Boolean(isDir && activePath && sameFolderPath(activePath, path)));
  const indent = $derived(6 + depth * 12);

  async function toggle() {
    if (!isDir) return;
    if (expanded) {
      expanded = false;
      return;
    }
    expanded = true;
    const gen = ++fetchGen;
    loading = true;
    error = null;
    try {
      const next = await listFolderChildren(path);
      if (gen !== fetchGen) return;
      kids = next;
    } catch (e) {
      if (gen !== fetchGen) return;
      error = e instanceof Error ? e.message : "Could not open folder.";
      kids = [];
    } finally {
      if (gen === fetchGen) loading = false;
    }
  }

  function clickRow() {
    if (isDir) {
      if (selectOnClick && (section === "photo" || section === "video")) {
        onSelectFolder?.(path, section);
      } else if (!expanded) {
        void toggle();
      }
    } else {
      // Load parent folder so filmstrip shows all images in that folder
      const parentPath = path.replace(/\\/g, "/").replace(/\/[^/]+$/, "");
      if (parentPath) {
        void loadFolder(parentPath);
      }
      void openLibraryFile(path);
    }
  }

  function onKey(e: KeyboardEvent) {
    if (!isDir) return;
    if (e.key === "ArrowRight" && !expanded) {
      e.preventDefault();
      void toggle();
    } else if (e.key === "ArrowLeft" && expanded) {
      e.preventDefault();
      void toggle();
    }
  }
</script>

<div class="node">
  <div class="row-wrap" style="padding-left: {indent}px">
    <button
      type="button"
      class="rail-item tree-row"
      class:rail-item--active={isActive}
      class:is-file={!isDir}
      class:is-open={!isDir && isOpen}
      class:is-offline={!accessible}
      title={accessible ? path : `${path} (offline)`}
      aria-expanded={isDir ? expanded : undefined}
      onclick={(e) => {
        if ((e.target as HTMLElement).closest(".chev")) {
          void toggle();
          return;
        }
        clickRow();
      }}
      ondblclick={() => {
        if (isDir && selectOnClick) void toggle();
      }}
      onkeydown={onKey}
      oncontextmenu={(e) => {
        if (isDir) onContextMenu?.(e, path);
      }}
    >
      <span class="chev" class:open={expanded} class:spacer={!isDir}>›</span>
      <span class="rail-icon">
        {#if isDir}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            {#if expanded}
              <path d="M6 14l1.45-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.55 6A2 2 0 0 1 18.45 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v2"/>
            {:else}
              <path d="M20 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2z"/>
            {/if}
          </svg>
        {:else if kind === "video"}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="5" width="20" height="14" rx="2"/>
            <path d="M10 9.5v5l5-2.5-5-2.5z"/>
          </svg>
        {:else}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="5" width="18" height="14" rx="2"/>
            <circle cx="8" cy="10" r="1.5"/>
            <path d="M21 16l-5-5-4 4-2-2-5 5"/>
          </svg>
        {/if}
      </span>
      <span class="rail-label">{name}</span>
      {#if isDir && count != null}
        <span class="rail-count">{count}</span>
      {/if}
    </button>
    {#if showPin}
      <button
        type="button"
        class="pin-btn"
        title="Add shortcut"
        onclick={() => onPin?.()}
      >+</button>
    {/if}
  </div>

  {#if isDir && expanded}
    <div class="kids">
      {#if loading && !kids}
        <p class="empty-state nested">Loading…</p>
      {:else if error}
        <p class="empty-state nested">{error}</p>
      {:else if kids && kids.length === 0}
        <p class="empty-state nested">No photos or clips here.</p>
      {:else if dirs.length === 0 && sectionFiles.length === 0}
        <p class="empty-state nested">{section === "video" ? "No clips here." : "No photos here."}</p>
      {:else}
        {#each dirs as child (child.path)}
          <FolderNode
            name={child.name}
            path={child.path}
            isDir={true}
            depth={depth + 1}
            count={
              section === "video"
                ? child.videoCount || null
                : child.photoCount || child.videoCount || null
            }
            {selectOnClick}
            {section}
            {activePath}
            {onSelectFolder}
          />
        {/each}
        {#each visibleFiles as child (child.path)}
          <FolderNode
            name={child.name}
            path={child.path}
            isDir={false}
            kind={child.kind}
            depth={depth + 1}
            selectOnClick={false}
            {section}
          />
        {/each}
        {#if extra > 0}
          <button
            type="button"
            class="empty-state nested more"
            onclick={() => {
              if (section === "photo" || section === "video") onSelectFolder?.(path, section);
            }}
          >{extra} more — select the folder to browse them</button>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .node { min-width: 0; }
  .row-wrap {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    min-width: 0;
  }
  .tree-row {
    display: grid;
    grid-template-columns: 12px 14px minmax(0, 1fr) auto;
    column-gap: 6px;
    width: 100%;
    padding-left: 0;
  }
  .tree-row.is-file { font-size: 11.5px; min-height: 28px; }
  .tree-row.is-open { color: var(--color-fg); background: var(--color-hover); }
  .tree-row.is-offline { opacity: 0.55; }
  .chev {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    color: var(--color-subtle);
    font-size: 13px;
    line-height: 1;
    cursor: pointer;
    transform: rotate(0deg);
    transition: transform 0.15s var(--ease-std);
  }
  .chev.open { transform: rotate(90deg); }
  .chev.spacer { visibility: hidden; pointer-events: none; }
  .more {
    display: block;
    width: 100%;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .more:hover { color: var(--color-fg); }
  .pin-btn {
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--color-secondary);
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
  }
  .pin-btn:hover { background: var(--color-hover); color: var(--color-fg); }
  .nested {
    padding: 4px 10px 6px 28px;
    font-size: 11px;
  }
</style>
