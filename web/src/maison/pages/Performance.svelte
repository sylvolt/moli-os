<script>
  import { onMount } from 'svelte';
  import { home } from '../lib/home.svelte.js';
  import { moliHass } from '../lib/moli-hass.js';
  import { t } from '../../lib/i18n.svelte.js';
  import '../vendor/energy-perf-card.js';

  /** « Performance énergie » : the house's own Lovelace card (vendor/,
   *  now adapted to Moli: its « Moli : » style notes), fed by Moli's energy
   *  history through a stand-in `hass`.
   *  Same layout as on Home Assistant. Its settings (`solar_names`,
   *  `investments`, `years`, `price`) come from `home.json` → `energy_perf`. */
  // `shape`: the card's outline while its figures load (tiles in a row,
  // a chart's height, table rows), so the page does not jump when they come.
  const LAYOUT = [
    // Heights taken from the loaded cards on a wide screen.
    { view: 'month', wide: false, shape: { tiles: 6, cols: 3, tile: 145 } },
    { view: 'year', wide: false, shape: { tiles: 6, cols: 3, tile: 135 } },
    { view: 'cumulative', wide: true, shape: { tiles: 4, cols: 4, tile: 105, chart: 313 } },
    { view: 'payback', wide: true, shape: { tiles: 5, cols: 5, tile: 105, chart: 363 } },
    { view: 'monthly', wide: true, shape: { chart: 332, rows: 13, pitch: 29 } },
    { view: 'yearly', wide: true, shape: { rows: 7, pitch: 40 } },
  ];

  let slots = $state([]);
  let ready = $state(LAYOUT.map(() => false));
  let failed = $state(null);
  let root;

  // The card has its figures (or its own error) to show. It keeps the last
  // ones of the session: back on the page, it is ready at once.
  function loaded(card, view) {
    return !!card._error || (!!card._data && (view !== 'cumulative' || !!card._daily));
  }

  function isNight() {
    return root?.closest('.maison')?.dataset.theme === 'night';
  }

  async function feed(cards) {
    try {
      const hass = await moliHass({ dark: isNight() });
      for (const card of cards) card.hass = hass;
      failed = null;
    } catch (e) {
      failed = String(e?.message ?? e);
    }
  }

  // The cards are made once the house's layout is there (`home.json` holds
  // their settings): opened straight on this page, it comes a moment later.
  let made = false;
  $effect(() => {
    const config = home.config;
    if (made || !config || !slots.length || slots.some((slot) => !slot)) return;
    made = true;
    const settings = config.energy_perf ?? {};
    const cards = [];
    // Each card redraws its whole shadow root once its figures are in.
    const drawn = [];
    LAYOUT.forEach(({ view }, i) => {
      const card = document.createElement('energy-perf-card');
      const own = { view, ...settings };
      if (view !== 'payback') delete own.investments;
      else own.investments ??= [];
      try {
        card.setConfig(own);
      } catch (e) {
        failed = String(e?.message ?? e);
        return;
      }
      slots[i].appendChild(card);
      cards.push(card);
      ready[i] = loaded(card, view);
      if (ready[i]) return;
      const seen = new MutationObserver(() => {
        if (!loaded(card, view)) return;
        ready[i] = true;
        seen.disconnect();
      });
      seen.observe(card.shadowRoot, { childList: true });
      drawn.push(seen);
    });
    feed(cards);
    // The page follows the day / night theme like the rest of Moli.
    const maison = root.closest('.maison');
    const watch = new MutationObserver(() => feed(cards));
    if (maison) watch.observe(maison, { attributes: true, attributeFilter: ['data-theme'] });
    // Fresh figures every 10 minutes (the card keeps its own cache as long).
    const timer = setInterval(() => feed(cards), 10 * 60 * 1000);
    cleanup = () => {
      watch.disconnect();
      for (const seen of drawn) seen.disconnect();
      clearInterval(timer);
    };
  });
  let cleanup = () => {};
  onMount(() => () => cleanup());
</script>

<div class="perf" bind:this={root}>
  <header>
    <h1 class="page-title">{t('energie.performance.titre')}</h1>
  </header>
  {#if failed}<p class="muted">{failed}</p>{/if}
  <div class="grid">
    {#each LAYOUT as item, i (item.view)}
      {@const shape = item.shape}
      {@const waiting = !ready[i] && !failed}
      <section class="card slot" class:wide={item.wide} class:waiting aria-busy={waiting} bind:this={slots[i]}>
        {#if waiting}
          <div class="skeleton" aria-hidden="true">
            <div class="head">
              <span class="bone title"></span>
              <span class="bone sub"></span>
            </div>
            {#if shape.tiles}
              <div class="tiles" style:--cols={shape.cols} style:--tile="{shape.tile}px">
                {#each { length: shape.tiles } as _}
                  <div class="tile">
                    <span class="bone label"></span>
                    <span class="bone value"></span>
                    <span class="bone note"></span>
                    <span class="bone note short"></span>
                  </div>
                {/each}
              </div>
            {/if}
            {#if shape.chart}<span class="bone chart" style:height="{shape.chart}px"></span>{/if}
            {#if shape.rows}
              <div class="rows" style:--pitch="{shape.pitch}px">
                {#each { length: shape.rows } as _}<span class="bone row"></span>{/each}
              </div>
            {/if}
          </div>
        {/if}
      </section>
    {/each}
  </div>
</div>

<style>
  .perf {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 20px;
    /* The card speaks Home Assistant's colours: Moli's, day and night. */
    --primary-text-color: var(--ink);
    --secondary-text-color: var(--ink-3);
    --divider-color: var(--line);
    --primary-color: var(--cool);
    --card-background-color: var(--surface);
    --ha-card-background: var(--surface);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 20px;
    align-items: start;
  }

  .slot {
    min-width: 0;
  }

  /* The card's own frame (\`ha-card\`) is not an element outside Home
     Assistant: Moli's card gives the margins. */

  .slot.wide {
    grid-column: 1 / -1;
  }

  /* While its figures load, the card stays hidden behind its outline (it
     would only say « Chargement des statistiques… »), then fades in. */
  .slot.waiting > :global(energy-perf-card) {
    display: none;
  }

  .slot > :global(energy-perf-card) {
    display: block;
    animation: appear 0.25s ease-out;
  }

  @keyframes appear {
    from {
      opacity: 0;
    }
  }

  .skeleton {
    display: grid;
    gap: 16px;
  }

  .skeleton .head {
    display: grid;
    gap: 8px;
  }

  .bone {
    display: block;
    border-radius: 6px;
    background: var(--line);
    animation: breathe 1.4s ease-in-out infinite;
  }

  .bone.title {
    width: 38%;
    max-width: 220px;
    height: 18px;
  }

  .bone.sub {
    width: 62%;
    max-width: 380px;
    height: 12px;
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    gap: 8px;
  }

  .tile {
    display: grid;
    gap: 8px;
    align-content: start;
    min-height: var(--tile);
    padding: 12px;
    border: 1px solid var(--line);
    border-radius: 10px;
  }

  .bone.label {
    width: 60%;
    height: 11px;
  }

  .bone.value {
    width: 45%;
    height: 22px;
  }

  .bone.note {
    width: 75%;
    height: 10px;
  }

  .bone.note.short {
    width: 50%;
  }

  .bone.chart {
    width: 100%;
    border-radius: 10px;
  }

  .rows {
    display: grid;
    gap: calc(var(--pitch) - 14px);
  }

  .bone.row {
    height: 14px;
  }

  .bone.row:nth-child(even) {
    width: 92%;
  }

  @keyframes breathe {
    50% {
      opacity: 0.45;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .bone,
    .slot > :global(energy-perf-card) {
      animation: none;
    }
  }

  @media (max-width: 760px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }

    .tiles {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
