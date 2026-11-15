<script lang="ts">
  // Explore (Slice 4.6.1): the library as named areas on a map by meaning,
  // drawn and kept current in the background. The list of areas is the
  // map's legend and its text equivalent.
  import { onMount, tick } from 'svelte';
  import { AtlasApiError, toApiError } from '../lib/api/client';
  import type { ExploreArea } from '../lib/api/bindings/ExploreArea';
  import type { ExplorePoint } from '../lib/api/bindings/ExplorePoint';
  import type { ExploreSubArea } from '../lib/api/bindings/ExploreSubArea';
  import ExploreMap from '../lib/components/ExploreMap.svelte';
  import ProblemNotice from '../lib/components/ProblemNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { hueColour, isDark } from '../lib/exploreColours';
  import {
    areaChip,
    areaLabel,
    emptyText,
    foundText,
    heldText,
    statusLine,
  } from '../lib/exploreCopy';
  import { count } from '../lib/inventoryCopy';
  import { details } from '../lib/stores/details.svelte';
  import { explore } from '../lib/stores/explore.svelte';
  import { library } from '../lib/stores/library.svelte';
  import { theme } from '../lib/stores/theme.svelte';
  import { view } from '../lib/stores/view.svelte';

  let mapView = $state<ExploreMap | null>(null);
  let findText = $state(explore.findText);
  let renaming = $state<number | null>(null);
  let draft = $state('');
  let renameError = $state<AtlasApiError | null>(null);
  let swatches = $state<HTMLElement[]>([]);

  const status = $derived(explore.status);
  const map = $derived(explore.map);
  const empty = $derived(emptyText(status));
  const held = $derived(heldText(status?.held ?? null));
  const dark = $derived(isDark(theme.preference));
  const onMap = $derived(
    map && explore.found ? map.points.filter((p) => explore.found?.has(p.documentId)).length : 0,
  );

  onMount(() => {
    void explore.refresh();
  });

  // Each swatch's colour, through CSSOM (style attributes are blocked).
  $effect(() => {
    void dark;
    for (const el of swatches) {
      if (el?.dataset.hue) el.style.background = hueColour(Number(el.dataset.hue), dark);
    }
  });

  function open(point: ExplorePoint, trigger: HTMLElement | null) {
    void details.open(point.fileId, trigger);
  }

  function showInLibrary(a: ExploreArea | ExploreSubArea) {
    library.mode = 'files';
    library.setFilters({ area: { id: a.id, label: areaChip(a) } });
    view.go('library');
  }

  async function startRename(a: ExploreArea | ExploreSubArea) {
    renaming = a.id;
    draft = a.name ?? '';
    renameError = null;
    await tick();
    document.getElementById(`rename-${a.id}`)?.focus();
  }

  async function saveRename(id: number) {
    try {
      await explore.rename(id, draft);
      renaming = null;
    } catch (err) {
      renameError = err instanceof AtlasApiError ? err : toApiError('rename_area', err);
    }
  }

  function renameKeys(e: KeyboardEvent, id: number) {
    if (e.key === 'Enter') {
      e.preventDefault();
      void saveRename(id);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      renaming = null;
    }
  }
</script>

<div class="view">
  <ViewHeading>Explore</ViewHeading>

  {#if explore.error}
    <ProblemNotice
      title="Atlas couldn’t show the map"
      message={explore.error.message}
      recovery={explore.error.recovery}
    />
  {/if}

  {#if !status}
    <p class="muted" role="status">Loading the map…</p>
  {:else if empty}
    <div class="empty" data-testid="explore-empty">
      <p><strong>{empty.title}</strong></p>
      <p class="muted">{empty.body}</p>
      {#if status.state === 'no_model'}
        <button type="button" class="button" onclick={() => view.go('settings')}>
          Open Settings
        </button>
      {:else if status.state === 'not_drawn' && !status.building}
        <button type="button" class="button primary" onclick={() => void explore.redraw()}>
          Draw the map now
        </button>
      {/if}
      {#if held}<p class="muted small" role="status">{held}</p>{/if}
    </div>
  {:else if map}
    <div class="bar">
      <p class="muted small" role="status" data-testid="explore-status">{statusLine(status)}</p>
      <div class="actions">
        <button
          type="button"
          class="button quiet"
          disabled={status.building}
          title="Draw the map again from the books Atlas has now"
          onclick={() => void explore.redraw()}
        >
          Redraw
        </button>
        <button
          type="button"
          class="button quiet"
          disabled={status.building}
          title="Draw the map afresh: areas and places may change"
          onclick={() => void explore.redraw(true)}
        >
          Redraw from scratch
        </button>
      </div>
    </div>
    {#if held}<p class="muted small" role="status">{held}</p>{/if}

    <div class="layout">
      <section class="map" aria-label="The map">
        <form
          class="find"
          role="search"
          onsubmit={(e) => {
            e.preventDefault();
            void explore.find(findText);
          }}
        >
          <input
            type="search"
            aria-label="Find on the map"
            placeholder="Find on the map"
            bind:value={findText}
          />
          <button type="submit" class="button" disabled={explore.finding}>Find</button>
          {#if explore.found}
            <span class="muted small" data-testid="found-note"
              >{foundText(explore.foundCount, onMap)}</span
            >
            <button
              type="button"
              class="button quiet"
              onclick={() => {
                findText = '';
                explore.clearFound();
              }}>Clear</button
            >
          {/if}
        </form>
        <div class="canvas">
          <ExploreMap
            bind:this={mapView}
            {map}
            selected={explore.selected}
            found={explore.found}
            onselect={(id) => explore.select(id)}
            onopen={open}
          />
        </div>
        <div class="controls">
          <button
            type="button"
            class="button quiet"
            aria-label="Zoom in"
            onclick={() => mapView?.zoom(1.4)}>+</button
          >
          <button
            type="button"
            class="button quiet"
            aria-label="Zoom out"
            onclick={() => mapView?.zoom(1 / 1.4)}>−</button
          >
          <button type="button" class="button quiet" onclick={() => mapView?.showAll()}>
            Show everything
          </button>
          <span class="muted small"
            >Drag to move, scroll to zoom. Each dot is a book; choose one to see its details.</span
          >
        </div>
      </section>

      <section class="areas" aria-label="Areas">
        <h2 class="visually-hidden">Areas</h2>
        <p class="muted small">
          {count(map.areas.length, 'area')}. Choose one to light it up on the map.
        </p>
        <ul>
          {#each map.areas as area, i (area.id)}
            <li class="area" class:selected={explore.selected === area.id}>
              <div class="head">
                <span class="swatch" aria-hidden="true" data-hue={area.hue} bind:this={swatches[i]}
                ></span>
                <span class="number">{area.number}</span>
                {#if renaming === area.id}
                  <input
                    id={`rename-${area.id}`}
                    class="rename"
                    aria-label={`Name for area ${area.number}`}
                    placeholder={area.words.join(' · ')}
                    maxlength="60"
                    bind:value={draft}
                    onkeydown={(e) => renameKeys(e, area.id)}
                  />
                  <button
                    type="button"
                    class="button quiet small"
                    onclick={() => saveRename(area.id)}>Save</button
                  >
                {:else}
                  <button
                    type="button"
                    class="name"
                    aria-pressed={explore.selected === area.id}
                    onclick={() => explore.select(area.id)}
                  >
                    {areaLabel(area)}
                  </button>
                {/if}
                <span class="muted small">{area.size.toLocaleString('en-US')}</span>
              </div>
              {#if renaming === area.id && renameError}
                <p class="problem small" role="alert">{renameError.message}</p>
              {/if}
              {#if area.name && area.words.length}
                <p class="muted small">{area.words.join(' · ')}</p>
              {/if}
              {#if area.central.length}
                <p class="muted small central">{area.central.join(' · ')}</p>
              {/if}
              <div class="row-actions">
                {#if renaming !== area.id}
                  <button type="button" class="link" onclick={() => startRename(area)}
                    >Rename</button
                  >
                {/if}
                <button
                  type="button"
                  class="link"
                  aria-label={`Show area ${area.number} in the Library`}
                  onclick={() => showInLibrary(area)}>Show in Library</button
                >
              </div>
              {#if area.subs.length}
                <ul class="subs">
                  {#each area.subs as sub (sub.id)}
                    <li class:selected={explore.selected === sub.id}>
                      <button
                        type="button"
                        class="name small"
                        aria-pressed={explore.selected === sub.id}
                        onclick={() => explore.select(sub.id)}
                      >
                        {areaLabel(sub)}
                      </button>
                      <span class="muted small">{sub.size.toLocaleString('en-US')}</span>
                      <button
                        type="button"
                        class="link small"
                        aria-label={`Show ${areaLabel(sub)} in the Library`}
                        onclick={() => showInLibrary(sub)}>Library</button
                      >
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    </div>
  {:else}
    <p class="muted" role="status">Loading the map…</p>
  {/if}
</div>

<style>
  .view {
    display: grid;
    gap: var(--space-4);
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-3);
  }

  .actions,
  .controls,
  .find {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .find input {
    flex: 1;
    min-width: 200px;
    min-height: 36px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
  }

  .layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(260px, 340px);
    gap: var(--space-5);
    align-items: start;
  }

  @media (max-width: 900px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }

  .map {
    display: grid;
    gap: var(--space-3);
  }

  .canvas {
    height: min(70vh, 720px);
  }

  .areas ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }

  .areas {
    max-height: min(80vh, 820px);
    overflow: auto;
  }

  .area {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  .area.selected {
    border-color: var(--accent);
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .swatch {
    flex: none;
    width: 12px;
    height: 12px;
    border-radius: 4px;
    background: var(--text-muted);
  }

  .number {
    flex: none;
    min-width: 24px;
    padding: 0 6px;
    border: 1px solid var(--text-muted);
    border-radius: 999px;
    text-align: center;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .name {
    flex: 1;
    min-height: 32px;
    border: 0;
    background: none;
    color: var(--text);
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }

  .subs .name {
    font-weight: 400;
  }

  .rename {
    flex: 1;
    min-height: 32px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
  }

  .central {
    margin: var(--space-1) 0 0;
  }

  .row-actions {
    display: flex;
    gap: var(--space-3);
  }

  .link {
    min-height: 28px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text, var(--accent));
    cursor: pointer;
    font-size: 13px;
  }

  .subs {
    margin-top: var(--space-2) !important;
    padding-left: var(--space-5) !important;
  }

  .subs li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .subs li.selected .name {
    color: var(--accent);
    font-weight: 600;
  }

  .empty {
    display: grid;
    gap: var(--space-2);
    justify-items: start;
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
  }

  .problem {
    color: var(--text);
  }

  p {
    margin: 0;
  }

  .small {
    font-size: 13px;
  }
</style>
