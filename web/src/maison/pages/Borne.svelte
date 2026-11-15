<script>
  import { hub } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import { isEvCharger } from '../lib/evcharger.js';
  import EvChargerCard from '../ui/EvChargerCard.svelte';

  const stations = $derived(
    Object.values(hub.devices)
      .filter(isEvCharger)
      .sort((a, b) => (b.online === true) - (a.online === true) || a.id.localeCompare(b.id)),
  );
</script>

<div class="borne-page">
  <header>
    <h1 class="page-title">{t('borne.titre')}</h1>
    <p class="page-sub">{t('borne.sous_titre')}</p>
  </header>

  {#if stations.length}
    <div class="grid">
      {#each stations as s (s.id)}
        <EvChargerCard id={s.id} />
      {/each}
    </div>
  {:else}
    <p class="muted">{t('borne.aucune')}</p>
  {/if}
</div>

<style>
  .borne-page {
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
</style>
