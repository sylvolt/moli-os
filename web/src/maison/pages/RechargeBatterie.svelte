<script>
  import { hub } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import { isEvCharger } from '../lib/evcharger.js';
  import { installations } from '../lib/victron-battery.js';
  import EvChargerCard from '../ui/EvChargerCard.svelte';
  import BatteryCard from '../ui/BatteryCard.svelte';

  /** The car's charging stations and the home battery: both sit behind the
   *  same GX at home, and both are what the sun's surplus goes to. */
  const online = (a, b) => (b.online === true) - (a.online === true) || a.id.localeCompare(b.id);
  const stations = $derived(Object.values(hub.devices).filter(isEvCharger).sort(online));
  const batteries = $derived(installations(Object.values(hub.devices)));
</script>

<div class="recharge-page">
  <header>
    <h1 class="page-title">{t('recharge.titre')}</h1>
    <p class="page-sub">{t('recharge.sous_titre')}</p>
  </header>

  {#if stations.length || batteries.length}
    <div class="grid">
      {#each stations as s (s.id)}
        <EvChargerCard id={s.id} />
      {/each}
      {#each batteries as b (b.instance)}
        <BatteryCard battery={b.battery} inverter={b.inverter} settings={b.settings} />
      {/each}
    </div>
  {:else}
    <p class="muted">{t('recharge.rien')}</p>
  {/if}
</div>

<style>
  .recharge-page {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 20px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 440px), 1fr));
    gap: 20px;
    align-items: start;
  }
  /* A wall panel: the car narrower, the battery (two columns of controls)
     wider. Same query as PANEL in lib/layout.js. */
  @media (min-width: 761px) and (max-height: 960px) and (orientation: landscape) {
    .grid:has(> :nth-child(2)) {
      grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
    }
  }
</style>
