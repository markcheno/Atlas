<script lang="ts">
  // The Explore map (Slice 4.6.1), ported from the spike's page: one dot a
  // book, coloured by its area, numbers at the areas' middles. Drag to
  // move, scroll or the buttons to zoom; a book opens its details panel;
  // a number chooses its area. The list beside the map says the same in
  // words, and is how the map is used from the keyboard.
  import { onMount } from 'svelte';
  import type { ExploreMap } from '../api/bindings/ExploreMap';
  import type { ExplorePoint } from '../api/bindings/ExplorePoint';
  import { areaLabel } from '../exploreCopy';
  import { hueColour, isDark } from '../exploreColours';
  import { bounds, fit, Grid, toScreen, zoomAt, type View } from '../exploreMath';
  import { theme } from '../stores/theme.svelte';

  interface Props {
    map: ExploreMap;
    /** The chosen area or sub-area, lit; the rest dimmed. */
    selected: number | null;
    /** Documents a search found, lit; the rest dimmed. */
    found: Set<string> | null;
    onselect: (id: number | null) => void;
    onopen: (point: ExplorePoint, trigger: HTMLElement | null) => void;
  }

  let { map, selected, found, onselect, onopen }: Props = $props();

  let canvas = $state<HTMLCanvasElement | null>(null);
  let tip = $state<HTMLDivElement | null>(null);
  let tipText = $state<{ title: string; area: string } | null>(null);
  let width = 0;
  let height = 0;
  let view: View = { scale: 1, x: 0, y: 0 };
  let hovered = -1;
  let fitted = false;

  const points = $derived(map.points);
  const grid = $derived(new Grid(map.points));
  const areaById = $derived(new Map(map.areas.map((a) => [a.id, a])));
  // A sub-area's parent, for its colour and its words.
  const parentOf = $derived(
    new Map(map.areas.flatMap((a) => a.subs.map((s) => [s.id, a] as const))),
  );
  /** Each area's middle: the median of its books, inside its crowd. */
  const middles = $derived(
    map.areas.map((a) => {
      const xs = points.filter((p) => p.area === a.id).map((p) => p.x);
      const ys = points.filter((p) => p.area === a.id).map((p) => p.y);
      xs.sort((p, q) => p - q);
      ys.sort((p, q) => p - q);
      return { area: a, x: xs[xs.length >> 1] ?? 0, y: ys[ys.length >> 1] ?? 0 };
    }),
  );

  interface Colours {
    surface: string;
    text: string;
    muted: string;
    dim: string;
    accent: string;
    dark: boolean;
  }
  let colours: Colours = {
    surface: '#fff',
    text: '#1d1d1f',
    muted: '#5f5f66',
    dim: 'rgba(128,128,128,0.22)',
    accent: '#2f5fd0',
    dark: false,
  };

  function readColours() {
    const css = getComputedStyle(document.documentElement);
    const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
    const dark = isDark(theme.preference);
    colours = {
      surface: v('--surface', dark ? '#1c1c1e' : '#fff'),
      text: v('--text', dark ? '#f2f2f2' : '#1d1d1f'),
      muted: v('--text-muted', '#8a8a90'),
      dim: dark ? 'rgba(160,160,160,0.18)' : 'rgba(120,120,120,0.2)',
      accent: v('--accent', '#2f5fd0'),
      dark,
    };
  }

  function hueOf(p: ExplorePoint): string {
    const area = areaById.get(p.area);
    return hueColour(area?.hue ?? 0, colours.dark);
  }

  function lit(p: ExplorePoint): boolean | null {
    if (found) return found.has(p.documentId);
    if (selected !== null) return p.area === selected || p.sub === selected;
    return null;
  }

  function draw() {
    const ctx = canvas?.getContext('2d');
    if (!ctx || width === 0) return;
    ctx.clearRect(0, 0, width, height);
    const r = Math.max(2, Math.min(5, 2 + Math.log2(Math.max(1, view.scale / 20))));
    const some = found !== null || selected !== null;
    const dot = (p: ExplorePoint, radius: number, fill: string, ring: boolean) => {
      const [x, y] = toScreen(view, p);
      if (x < -radius || y < -radius || x > width + radius || y > height + radius) return;
      ctx.beginPath();
      ctx.arc(x, y, radius, 0, Math.PI * 2);
      ctx.fillStyle = fill;
      ctx.fill();
      if (ring) {
        ctx.lineWidth = 1.5;
        ctx.strokeStyle = colours.surface;
        ctx.stroke();
      }
    };
    ctx.globalAlpha = some ? 1 : 0.8;
    for (const p of points) {
      if (lit(p) === true) continue;
      dot(p, r, some ? colours.dim : hueOf(p), false);
    }
    ctx.globalAlpha = 1;
    if (some) for (const p of points) if (lit(p) === true) dot(p, r + 0.5, hueOf(p), true);

    // Area numbers on pills, edged in their colour.
    ctx.font = '600 12px system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    for (const m of middles) {
      const [x, y] = toScreen(view, m);
      const text = String(m.area.number);
      const w = Math.max(20, ctx.measureText(text).width + 10);
      ctx.beginPath();
      ctx.roundRect(x - w / 2, y - 10, w, 20, 10);
      ctx.fillStyle = colours.surface;
      ctx.fill();
      const chosen = selected === m.area.id || parentOf.get(selected ?? -1)?.id === m.area.id;
      ctx.lineWidth = chosen ? 2.5 : 2;
      ctx.strokeStyle = some && !chosen ? colours.muted : hueColour(m.area.hue, colours.dark);
      ctx.stroke();
      ctx.fillStyle = colours.text;
      ctx.fillText(text, x, y);
    }
    if (hovered >= 0 && points[hovered]) {
      const [x, y] = toScreen(view, points[hovered]);
      ctx.beginPath();
      ctx.arc(x, y, r + 3, 0, Math.PI * 2);
      ctx.lineWidth = 2;
      ctx.strokeStyle = colours.text;
      ctx.stroke();
    }
  }

  function resize() {
    if (!canvas) return;
    const box = canvas.getBoundingClientRect();
    const ratio = window.devicePixelRatio || 1;
    width = box.width;
    height = box.height;
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
    canvas.getContext('2d')?.setTransform(ratio, 0, 0, ratio, 0, 0);
    if (!fitted && width > 0) {
      view = fit(bounds(points), width, height);
      fitted = true;
    }
    draw();
  }

  /** Shows the whole map. */
  export function showAll() {
    view = fit(bounds(points), width, height);
    draw();
  }

  export function zoom(factor: number) {
    view = zoomAt(view, factor, width / 2, height / 2);
    draw();
  }

  /** The area number under (x, y) on the canvas, or the nearest book. */
  function at(x: number, y: number): { area: number | null; book: number } {
    for (const m of middles) {
      const [mx, my] = toScreen(view, m);
      if (Math.abs(x - mx) <= 14 && Math.abs(y - my) <= 12) return { area: m.area.id, book: -1 };
    }
    const book = grid.nearest(
      (x - view.x) / view.scale,
      (y - view.y) / view.scale,
      14 / view.scale,
    );
    return { area: null, book };
  }

  let drag: { x: number; y: number; moved: boolean } | null = null;

  function down(e: PointerEvent) {
    canvas?.setPointerCapture?.(e.pointerId);
    drag = { x: e.offsetX, y: e.offsetY, moved: false };
  }

  function move(e: PointerEvent) {
    if (drag) {
      const dx = e.offsetX - drag.x;
      const dy = e.offsetY - drag.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
      if (drag.moved) {
        view = { ...view, x: view.x + dx, y: view.y + dy };
        drag.x = e.offsetX;
        drag.y = e.offsetY;
        hovered = -1;
        tipText = null;
        draw();
      }
      return;
    }
    const found = at(e.offsetX, e.offsetY);
    hovered = found.book;
    const p = found.book >= 0 ? points[found.book] : null;
    const area = p ? areaById.get(p.area) : found.area !== null ? areaById.get(found.area) : null;
    tipText = area
      ? { title: p ? p.title : `Area ${area.number}`, area: `${area.number} · ${areaLabel(area)}` }
      : null;
    if (tip && tipText) {
      // CSSOM, not a style attribute: the window's CSP allows it.
      const left = Math.min(e.offsetX + 14, width - 260);
      const top = Math.min(e.offsetY + 14, height - 60);
      tip.style.translate = `${Math.max(4, left)}px ${Math.max(4, top)}px`;
    }
    draw();
  }

  function up(e: PointerEvent) {
    const wasDrag = drag?.moved;
    drag = null;
    if (wasDrag) return;
    const hit = at(e.offsetX, e.offsetY);
    if (hit.book >= 0) onopen(points[hit.book], canvas);
    else onselect(hit.area);
  }

  function leave() {
    hovered = -1;
    tipText = null;
    draw();
  }

  function wheel(e: WheelEvent) {
    e.preventDefault();
    view = zoomAt(view, Math.exp(-e.deltaY * 0.0015), e.offsetX, e.offsetY);
    draw();
  }

  onMount(() => {
    readColours();
    resize();
    const media =
      typeof window.matchMedia === 'function'
        ? window.matchMedia('(prefers-color-scheme: dark)')
        : null;
    const repaint = () => {
      readColours();
      draw();
    };
    media?.addEventListener('change', repaint);
    window.addEventListener('resize', resize);
    canvas?.addEventListener('wheel', wheel, { passive: false });
    return () => {
      media?.removeEventListener('change', repaint);
      window.removeEventListener('resize', resize);
      canvas?.removeEventListener('wheel', wheel);
    };
  });

  // Redraw when the map, the choice, the found books or the theme change.
  $effect(() => {
    void [map, selected, found, theme.preference];
    readColours();
    draw();
  });
</script>

<div class="frame">
  <p class="visually-hidden">
    A map of your library by meaning. The list of areas beside it says the same in words.
  </p>
  <canvas
    bind:this={canvas}
    aria-hidden="true"
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointerleave={leave}
  ></canvas>
  <div class="tip" bind:this={tip} hidden={!tipText} aria-hidden="true">
    {#if tipText}
      <strong>{tipText.title}</strong>
      <span class="muted">{tipText.area}</span>
    {/if}
  </div>
</div>

<style>
  .frame {
    position: relative;
    height: 100%;
    min-height: 420px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    overflow: hidden;
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
    cursor: grab;
    touch-action: none;
  }

  canvas:active {
    cursor: grabbing;
  }

  .tip {
    position: absolute;
    left: 0;
    top: 0;
    max-width: 260px;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: 0 2px 10px rgb(0 0 0 / 15%);
    font-size: 13px;
    pointer-events: none;
  }

  .tip[hidden] {
    display: none;
  }

  .tip strong {
    display: block;
  }
</style>
