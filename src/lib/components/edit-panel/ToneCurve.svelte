<script lang="ts">
  import { curveChannel, type CurveChannel } from "../../../stores/editor";
  import { doc, reconcile } from "../../../stores/doc";
  import { selectedMask } from "../../../stores/app";
  import { beginMaskAdjust, endMaskAdjust } from "../../../stores/mask";
  import { setParam } from "../../../ipc/commands";
  import CollapsibleSection from "./CollapsibleSection.svelte";

  const channels: { id: CurveChannel; color: string; stroke: string }[] = [
    { id: "luma", color: "#e8e8e8", stroke: "rgba(255, 255, 255, 0.85)" },
    { id: "red", color: "#e64f4f", stroke: "#e64f4f" },
    { id: "green", color: "#0fb327", stroke: "#0fb327" },
    { id: "blue", color: "#3b82f6", stroke: "#3b82f6" },
  ];

  const PATH: Record<CurveChannel, string> = {
    luma: "tone_curve.points",
    red: "tone_curve.points_r",
    green: "tone_curve.points_g",
    blue: "tone_curve.points_b",
  };

  const IDENTITY = [
    { x: 0, y: 0 },
    { x: 1, y: 1 },
  ];

  function readChannel(ch: CurveChannel): { x: number; y: number }[] {
    const key =
      ch === "luma" ? "points" : ch === "red" ? "points_r" : ch === "green" ? "points_g" : "points_b";
    const raw = $doc?.modules?.tone_curve?.[key];
    if (!Array.isArray(raw) || raw.length < 2) return [...IDENTITY];
    return (raw as [number, number][]).map(([x, y]) => ({ x, y }));
  }

  let pointsByChannel = $state<Record<CurveChannel, { x: number; y: number }[]>>({
    luma: [...IDENTITY],
    red: [...IDENTITY],
    green: [...IDENTITY],
    blue: [...IDENTITY],
  });

  // Keep local points in sync when the doc mirror changes (undo/redo/open).
  $effect(() => {
    void $doc;
    pointsByChannel = {
      luma: readChannel("luma"),
      red: readChannel("red"),
      green: readChannel("green"),
      blue: readChannel("blue"),
    };
  });

  const activePoints = $derived(pointsByChannel[$curveChannel]);
  const activeChannelColor = $derived(
    channels.find((c) => c.id === $curveChannel)?.stroke || "rgba(255,255,255,0.7)",
  );

  const splinePath = $derived.by(() => {
    const pts = activePoints;
    if (pts.length === 0) return "";
    let path = `M ${pts[0].x * 256} ${(1 - pts[0].y) * 256}`;
    if (pts.length === 2) {
      path += ` L ${pts[1].x * 256} ${(1 - pts[1].y) * 256}`;
      return path;
    }
    for (let i = 0; i < pts.length - 1; i++) {
      const p0 = pts[i - 1] || pts[i];
      const p1 = pts[i];
      const p2 = pts[i + 1];
      const p3 = pts[i + 2] || p2;
      const cp1x = p1.x + (p2.x - p0.x) / 6;
      const cp1y = p1.y + (p2.y - p0.y) / 6;
      const cp2x = p2.x - (p3.x - p1.x) / 6;
      const cp2y = p2.y - (p3.y - p1.y) / 6;
      path += ` C ${cp1x * 256} ${(1 - cp1y) * 256}, ${cp2x * 256} ${(1 - cp2y) * 256}, ${p2.x * 256} ${(1 - p2.y) * 256}`;
    }
    return path;
  });

  let draggedIndex = $state<number | null>(null);
  let dragAxis = $state<'x' | 'y' | null>(null);
  let dragStartPoint = $state<{ x: number; y: number } | null>(null);
  let dragStartMouse = $state<{ x: number; y: number } | null>(null);
  let selectedPointIndex = $state<number | null>(null);
  let pointInputX = $state<number>(0);
  let pointInputY = $state<number>(0);

  function payload(pts: { x: number; y: number }[]): [number, number][] | [] {
    const isIdentity =
      pts.length === 2 &&
      pts[0].x === 0 &&
      pts[0].y === 0 &&
      pts[1].x === 1 &&
      pts[1].y === 1;
    if (isIdentity) return [];
    return pts.map((p) => [p.x, p.y] as [number, number]);
  }

  function commit(ch: CurveChannel = $curveChannel) {
    const pts = pointsByChannel[ch];
    void setParam(PATH[ch], payload(pts))
      .then(reconcile)
      .catch(() => {});
  }

  function startDrag(e: PointerEvent, index: number) {
    e.preventDefault();
    if ($selectedMask) beginMaskAdjust();
    draggedIndex = index;
    dragAxis = null;
    dragStartPoint = { ...activePoints[index] };
    dragStartMouse = { x: e.clientX, y: e.clientY };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function handlePointerMove(e: PointerEvent) {
    if (draggedIndex === null || !dragStartPoint || !dragStartMouse) return;
    const svgElement = e.currentTarget as SVGSVGElement;
    const rect = svgElement.getBoundingClientRect();
    const currentX = (e.clientX - rect.left) / rect.width;
    const currentY = 1 - (e.clientY - rect.top) / rect.height;
    const updated = [...activePoints];
    const pt = { ...updated[draggedIndex] };

    const isFirst = draggedIndex === 0;
    const isLast = draggedIndex === activePoints.length - 1;

    if (isFirst || isLast) {
      if (dragAxis === null) {
        const mouseDx = Math.abs(e.clientX - dragStartMouse.x);
        const mouseDy = Math.abs(e.clientY - dragStartMouse.y);
        if (mouseDx > 3 || mouseDy > 3) {
          dragAxis = mouseDx > mouseDy ? 'x' : 'y';
        }
      }
      if (dragAxis === 'x') {
        pt.y = dragStartPoint.y;
        if (isFirst) pt.x = Math.max(0, Math.min(0.5, currentX));
        else pt.x = Math.max(0.5, Math.min(1, currentX));
      } else if (dragAxis === 'y') {
        pt.x = dragStartPoint.x;
        pt.y = Math.max(0, Math.min(1, currentY));
      } else {
        // Not decided yet — keep original position
        pt.x = dragStartPoint.x;
        pt.y = dragStartPoint.y;
      }
    } else {
      pt.y = Math.max(0, Math.min(1, currentY));
      const minX = updated[draggedIndex - 1].x + 0.05;
      const maxX = updated[draggedIndex + 1].x - 0.05;
      pt.x = Math.max(minX, Math.min(maxX, currentX));
    }
    updated[draggedIndex] = pt;
    pointsByChannel[$curveChannel] = updated;
    // Real-time preview during drag
    void setParam(PATH[$curveChannel], payload(updated))
      .then(reconcile)
      .catch(() => {});
  }

  function stopDrag(e: PointerEvent) {
    if (draggedIndex !== null) {
      try {
        (e.target as HTMLElement).releasePointerCapture(e.pointerId);
      } catch {
        /* ignore */
      }
      draggedIndex = null;
      dragAxis = null;
      dragStartPoint = null;
      dragStartMouse = null;
      if ($selectedMask) endMaskAdjust();
    }
  }

  function handleSvgDoubleClick(e: MouseEvent) {
    if ((e.target as SVGElement).tagName === "circle") return;
    const svgElement = e.currentTarget as SVGSVGElement;
    const rect = svgElement.getBoundingClientRect();
    const x = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const y = Math.max(0, Math.min(1, 1 - (e.clientY - rect.top) / rect.height));
    let insertIdx = 0;
    while (insertIdx < activePoints.length && activePoints[insertIdx].x < x) insertIdx++;
    const updated = [...activePoints];
    updated.splice(insertIdx, 0, { x, y });
    pointsByChannel[$curveChannel] = updated;
    if ($selectedMask) beginMaskAdjust();
    commit();
  }

  function removePoint(index: number) {
    if (index === 0 || index === activePoints.length - 1) return;
    const updated = [...activePoints];
    updated.splice(index, 1);
    pointsByChannel[$curveChannel] = updated;
    if (selectedPointIndex === index) {
      selectedPointIndex = null;
    } else if (selectedPointIndex !== null && selectedPointIndex > index) {
      selectedPointIndex--;
    }
    commit();
  }

  function selectPoint(index: number) {
    selectedPointIndex = index;
    const pt = activePoints[index];
    pointInputX = Math.round(pt.x * 255);
    pointInputY = Math.round(pt.y * 255);
  }

  function updatePointInput(field: 'x' | 'y', value: string) {
    if (selectedPointIndex === null) return;
    const num = Math.max(0, Math.min(255, parseInt(value) || 0));
    if (field === 'x') {
      pointInputX = num;
    } else {
      pointInputY = num;
    }
    const updated = [...activePoints];
    const pt = { ...updated[selectedPointIndex] };
    const isFirst = selectedPointIndex === 0;
    const isLast = selectedPointIndex === activePoints.length - 1;

    if (field === 'x') {
      pt.x = num / 255;
      // For endpoints, maintain axis constraint
      if (isFirst) {
        pt.x = Math.max(0, Math.min(0.5, pt.x));
        pt.y = 0;
      } else if (isLast) {
        pt.x = Math.max(0.5, Math.min(1, pt.x));
        pt.y = 1;
      }
    } else {
      pt.y = num / 255;
      if (isFirst) {
        pt.x = 0;
        pt.y = Math.max(0, Math.min(1, pt.y));
      } else if (isLast) {
        pt.x = 1;
        pt.y = Math.max(0, Math.min(1, pt.y));
      }
    }
    updated[selectedPointIndex] = pt;
    pointsByChannel[$curveChannel] = updated;
    void setParam(PATH[$curveChannel], payload(updated))
      .then(reconcile)
      .catch(() => {});
  }

  function handlePointInputBlur(field: 'x' | 'y') {
    // Sync displayed value to actual clamped value
    if (selectedPointIndex !== null) {
      const pt = activePoints[selectedPointIndex];
      pointInputX = Math.round(pt.x * 255);
      pointInputY = Math.round(pt.y * 255);
    }
  }
</script>

<CollapsibleSection id="curve" title="Tone Curve">
<div class="curve">
  <div class="dots">
    {#each channels as ch (ch.id)}
      <button
        aria-label="{ch.id} channel"
        class="dot"
        class:on={$curveChannel === ch.id}
        style="background: {ch.color}"
        onclick={() => curveChannel.set(ch.id)}
      ></button>
    {/each}
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="canvas">
    <div class="svg-wrapper">
      <svg
        viewBox="0 0 256 256"
        class="svg"
        ondblclick={handleSvgDoubleClick}
        onpointermove={handlePointerMove}
        onpointerup={stopDrag}
        onpointercancel={stopDrag}
        onclick={() => {
          selectedPointIndex = null;
          pointInputX = 0;
          pointInputY = 0;
        }}
      >
      <line x1="64" y1="0" x2="64" y2="256" stroke="white" stroke-opacity="0.05" stroke-dasharray="2 2" />
      <line x1="128" y1="0" x2="128" y2="256" stroke="white" stroke-opacity="0.05" stroke-dasharray="2 2" />
      <line x1="192" y1="0" x2="192" y2="256" stroke="white" stroke-opacity="0.05" stroke-dasharray="2 2" />
      <line x1="0" y1="64" x2="256" y2="64" stroke="white" stroke-opacity="0.05" stroke-dasharray="2 2" />
      <line x1="0" y1="128" x2="256" y2="128" stroke="white" stroke-opacity="0.05" stroke-dasharray="2 2" />
      <line x1="0" y1="192" x2="256" y2="192" stroke="white" stroke-opacity="0.05" stroke-dasharray="2 2" />
      <path
        d={splinePath}
        fill="none"
        stroke={activeChannelColor}
        stroke-width="1.8"
        stroke-linecap="round"
      />
      {#each activePoints as p, i}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <circle
          cx={p.x * 256}
          cy={(1 - p.y) * 256}
          r={draggedIndex === i ? 5.775 : selectedPointIndex === i ? 5.775 : 4.2}
          class="cursor-pointer fill-fg stroke-black/60 stroke-[1.5px] transition-all duration-100 hover:fill-[#a8a8a8] active:fill-[#8c8c8c]"
          onpointerdown={(e) => startDrag(e, i)}
          onclick={(e) => {
            e.stopPropagation();
            selectPoint(i);
          }}
          ondblclick={(e) => {
            e.stopPropagation();
            removePoint(i);
          }}
          oncontextmenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            removePoint(i);
          }}
        />
      {/each}
    </svg>
    </div>
  </div>

  <div class="point-inputs">
    <label>
      <span>Input</span>
      <input
        type="number"
        min="0"
        max="255"
        value={pointInputX}
        oninput={(e) => updatePointInput('x', e.currentTarget.value)}
        onblur={() => handlePointInputBlur('x')}
        onkeydown={(e) => { if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur(); }}
      />
    </label>
    <label>
      <span>Output</span>
      <input
        type="number"
        min="0"
        max="255"
        value={pointInputY}
        oninput={(e) => updatePointInput('y', e.currentTarget.value)}
        onblur={() => handlePointInputBlur('y')}
        onkeydown={(e) => { if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur(); }}
      />
    </label>
  </div>
</div>
</CollapsibleSection>

<style>
  .curve { display: flex; flex-direction: column; gap: var(--space-2); }
  .canvas {
    width: 100%;
    aspect-ratio: 1 / 1;
    position: relative;
    overflow: hidden;
    border-radius: 0;
    border: 1px solid var(--color-border);
    background: var(--color-active);
    touch-action: none;
    user-select: none;
  }
  .svg-wrapper {
    position: absolute;
    inset: 0;
    box-sizing: border-box;
  }
  .svg { width: 100%; height: 100%; cursor: crosshair; }
  .dots {
    display: flex;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-1) 0;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 0;
    padding: 0;
    cursor: pointer;
    opacity: 0.4;
  }
  .dot.on {
    opacity: 1;
    outline: 1px solid var(--color-fg);
    outline-offset: 2px;
  }
  .point-inputs {
    display: flex;
    gap: var(--space-3);
    padding: var(--space-2);
    background: var(--color-sunken);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    font-size: var(--text-group);
  }
  .point-inputs label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: center;
  }
  .point-inputs span {
    color: var(--color-subtle);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    font-size: 10px;
    text-align: center;
  }
  .point-inputs input {
    width: 60px;
    height: 28px;
    border: 1px solid var(--color-border-strong);
    border-radius: 6px;
    background: var(--color-active);
    color: var(--color-fg);
    font-size: var(--text-ui);
    font-family: var(--font-mono);
    text-align: center;
    padding: 0 8px;
  }
  .point-inputs input:focus {
    outline: none;
    border-color: var(--color-fg);
  }
  /* Hide spinner arrows on number inputs */
  .point-inputs input::-webkit-outer-spin-button,
  .point-inputs input::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }
  .point-inputs input[type=number] {
    -moz-appearance: textfield;
  }
</style>
