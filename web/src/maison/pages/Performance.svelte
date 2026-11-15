<script>
  import { onMount } from 'svelte';
  import { home } from '../lib/home.svelte.js';
  import { moliHass } from '../lib/moli-hass.js';
  import { t } from '../../lib/i18n.svelte.js';
  import '../vendor/energy-perf-card.js';

  /** « Performance énergie » : the house's own Lovelace card (vendor/,
   *  unchanged), fed by Moli's energy history through a stand-in `hass`.
   *  Same layout as on Home Assistant. Its settings (`solar_names`,
   *  `investments`, `years`, `price`) come from `home.json` → `energy_perf`. */
  const LAYOUT = [
    { view: 'month', wide: false },
    { view: 'year', wide: false },
    { view: 'cumulative', wide: true },
    { view: 'payback', wide: true },
    { view: 'monthly', wide: true },
    { view: 'yearly', wide: true },
  ];

  let slots = $state([]);
  let failed = $state(null);
  let root;

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
      <section class="card slot" class:wide={item.wide} bind:this={slots[i]}></section>
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

  @media (max-width: 760px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
