<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { histogramOpen, rightPanelMode } from "../../../stores/editor";
  import { imageOpen } from "../../../stores/app";
  import { applyTool, showCropPanel, showMaskPanel } from "../../editor/focus";
  import {
    colorPickActive,
    stopGeomPlacement,
    stopInstancePick,
  } from "../../../stores/mask";
  import LightSettings from "./LightSettings.svelte";
  import ColorSettings from "./ColorSettings.svelte";
  import ToneCurve from "./ToneCurve.svelte";
  import DetailSettings from "./DetailSettings.svelte";
  import GradingSettings from "./GradingSettings.svelte";
  import CropSettings from "./CropSettings.svelte";
  import RetouchSettings from "./RetouchSettings.svelte";
  import CameraSettings from "./CameraSettings.svelte";
  import PresetSettings from "./PresetSettings.svelte";
  import MaskPanel from "./MaskPanel.svelte";
  import Histogram from "../histogram/Histogram.svelte";
  import { workspace } from "../../../stores/workspace";

  const isVideo = $derived($workspace === "video");

  $effect(() => {
    if (isVideo && ($rightPanelMode === "mask" || $rightPanelMode === "crop")) {
      rightPanelMode.set("edit");
    }
  });

  function openEditTab() {
    // Leaving Mask must drop pick tools + red overlay (setMaskOverlay alone is not enough —
    // selectedMask stayed set and the next sync resurrected the overlay).
    stopInstancePick();
    stopGeomPlacement();
    colorPickActive.set(false);
    applyTool("edit");
    rightPanelMode.set("edit");
  }
</script>

<div class="edit-rail">
  <div class="edit-tabs" role="tablist" aria-label="Workspace">
    <button
      type="button"
      role="tab"
      aria-selected={$rightPanelMode === "edit"}
      class="edit-tab"
      class:is-active={$rightPanelMode === "edit"}
      onclick={openEditTab}
    >
      {isVideo ? "Grade" : "Edit"}
    </button>
    {#if !isVideo}
      <button
        type="button"
        role="tab"
        aria-selected={$rightPanelMode === "crop"}
        class="edit-tab"
        class:is-active={$rightPanelMode === "crop"}
        onclick={(e) => {
          e.stopPropagation();
          showCropPanel();
        }}
      >
        Crop
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={$rightPanelMode === "mask"}
        class="edit-tab"
        class:is-active={$rightPanelMode === "mask"}
        onclick={(e) => {
          e.stopPropagation();
          showMaskPanel();
        }}
      >
        Mask
      </button>
      {/if}
  </div>

  {#if $rightPanelMode === "crop"}
    <div class="hist-dock">
      <button
        type="button"
        class="group-label hist-toggle"
        aria-expanded={$histogramOpen}
        onclick={() => histogramOpen.set(!$histogramOpen)}
      >
        <span>Histogram</span>
        <span class="hist-chevron" class:open={$histogramOpen}>›</span>
      </button>
      {#if $histogramOpen}
        <div class="hist-body" transition:fly={{ y: 6, duration: 180, easing: cubicOut }}>
          <Histogram embedded />
        </div>
      {/if}
    </div>
    <div class="edit-body" in:fade={{ duration: 160 }}>
      {#if !$imageOpen}
        <p class="rail-empty empty">Open a photo to crop.</p>
      {:else}
        <CropSettings />
      {/if}
    </div>
  {:else if $rightPanelMode === "mask"}
    <div class="edit-body" in:fade={{ duration: 160 }}>
      {#if !$imageOpen}
        <p class="rail-empty empty">Open a photo to start masking.</p>
      {:else}
        <MaskPanel />
      {/if}
    </div>
  {:else}
    <div class="hist-dock">
      <button
        type="button"
        class="group-label hist-toggle"
        aria-expanded={$histogramOpen}
        onclick={() => histogramOpen.set(!$histogramOpen)}
      >
        <span>Histogram</span>
        <span class="hist-chevron" class:open={$histogramOpen}>›</span>
      </button>
      {#if $histogramOpen}
        <div class="hist-body" transition:fly={{ y: 6, duration: 180, easing: cubicOut }}>
          <Histogram embedded />
        </div>
      {/if}
    </div>

    <div class="edit-body">
      {#if !$imageOpen}
        <p class="rail-empty empty">
          {isVideo ? "Open a clip to start grading." : "Open a photo to start editing."}
        </p>
      {:else}
        <div class="acc-list custom-scrollbar">
          {#if isVideo}
            <LightSettings />
            <ColorSettings />
            <GradingSettings />
            <CameraSettings mode="video" />
            <PresetSettings />
          {:else}
            <LightSettings />
            <ColorSettings />
            <ToneCurve />
            <DetailSettings />
            <GradingSettings />
            <RetouchSettings />
            <CameraSettings mode="photo" />
            <PresetSettings />
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .edit-rail {
    position: relative;
    z-index: 2;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    pointer-events: auto;
    background: var(--color-sidebar);
  }
  .edit-tabs {
    display: flex;
    flex: none;
    gap: 2px;
    padding: var(--space-2) var(--space-3);
  }
  .edit-tab {
    flex: 1;
    height: 28px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--color-subtle);
    font-size: var(--text-ui);
    font-weight: 400;
    cursor: pointer;
    transition: background-color 0.15s var(--ease-std), color 0.15s var(--ease-std);
  }
  .edit-tab:hover { color: var(--color-fg); background: var(--color-hover); }
  .edit-tab.is-active {
    background: var(--color-active);
    color: var(--color-fg);
    font-weight: 500;
  }
  .edit-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .acc-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: var(--space-4);
  }
  .empty { padding: var(--space-4) var(--space-3); }
  .hist-dock {
    flex: none;
    border-bottom: 1px solid var(--color-border);
  }
  .hist-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-height: 32px;
    margin: 0;
    padding: 0 var(--space-3);
    border: 0;
    background: transparent;
    cursor: pointer;
    text-align: left;
  }
  .hist-toggle:hover { color: var(--color-fg); }
  .hist-chevron {
    color: var(--color-subtle);
    font-size: 16px;
    line-height: 1;
    transform: rotate(0deg);
    transition: transform 0.15s var(--ease-std);
  }
  .hist-chevron.open { transform: rotate(90deg); }
  .hist-dock :global(.rail-chip) {
    height: 22px;
    padding: 0 6px;
    border-radius: 5px;
    font-size: 10.5px;
  }
  .hist-body { height: 150px; padding: 0 10px 10px; }
</style>
