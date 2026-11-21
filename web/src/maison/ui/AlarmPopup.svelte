<script>
  import { home, hub, value, since } from '../lib/home.svelte.js';
  import { isAlarm } from '../lib/alarm.js';
  import { wallSend } from '../lib/native-wall.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';
  import LiveVideo from './LiveVideo.svelte';
  import CameraTile from './CameraTile.svelte';
  import LightTile from './LightTile.svelte';

  /** The alarm went off: the camera that sees where, live, and what to do
   *  about it at once (open the gate to whoever is expected, light the way),
   *  over any page. From `home.json`:
   *
   *    "alarm_popup": {
   *      "modes": ["home"],                 // the alarm's modes it opens in (default: all)
   *      "camera": "frigate:eye",           // the camera shown live
   *      "tiles": [{ "id": "…", "name": "Portail", "icon": "gate", "also": [] }]
   *    }
   *
   *  Closed by hand, or by itself two minutes after the alarm calmed down.
   *  On a wall tablet, the screen is woken and kept awake while it shows. */
  const cfg = $derived(home.config?.alarm_popup ?? null);
  const alarm = $derived(Object.values(hub.devices).find(isAlarm) ?? null);
  const state = $derived(alarm ? value(alarm.id, 'state') : null);
  const mode = $derived(alarm ? value(alarm.id, 'mode') : null);
  const cause = $derived(alarm ? value(alarm.id, 'cause') : null);
  // When the alarm went off: one popup per time, not again once closed.
  const went = $derived(alarm && state === 'triggered' ? since(alarm.id, 'state') : null);

  const CALM_CLOSE_MS = 120_000;
  const AWAKE_EVERY_MS = 30_000;

  let shownFor = $state(null);
  let closedFor = $state(null);
  let noLive = $state(false);

  $effect(() => {
    if (!cfg || went == null || went === closedFor) return;
    const modes = cfg.modes ?? null;
    if (modes && !modes.includes(mode)) return;
    if (shownFor !== went) {
      shownFor = went;
      noLive = false;
    }
  });

  const open = $derived(shownFor != null && shownFor !== closedFor);

  function close() {
    closedFor = shownFor;
  }

  // The wall tablet's screen on, and kept on, while the popup shows.
  $effect(() => {
    if (!open) return;
    wallSend('activity');
    const timer = setInterval(() => wallSend('activity'), AWAKE_EVERY_MS);
    return () => clearInterval(timer);
  });

  // Closed by itself two minutes after the alarm calmed down.
  $effect(() => {
    if (!open || state === 'triggered') return;
    const timer = setTimeout(close, CALM_CLOSE_MS);
    return () => clearTimeout(timer);
  });

  const camera = $derived(cfg?.camera ?? null);
  const tiles = $derived(cfg?.tiles ?? []);
</script>

{#if open}
  <div class="backdrop" role="presentation"></div>
  <div class="popup" role="alertdialog" aria-modal="true" aria-labelledby="alarm-popup-title">
    <header>
      <div>
        <h2 id="alarm-popup-title"><Icon name="alert" size={22} />{t('maison.alarme_fenetre.titre')}</h2>
        {#if cause}<p class="cause">{cause}</p>{/if}
        {#if state !== 'triggered'}<p class="calm">{t('maison.alarme_fenetre.calmee')}</p>{/if}
      </div>
      <button class="close" onclick={close} aria-label={t('maison.alarme_fenetre.fermer')}><Icon name="close" size={22} /></button>
    </header>
    {#if camera}
      <div class="camera">
        {#if noLive}
          <CameraTile id={camera} name="" refresh={5} />
        {:else}
          <LiveVideo id={camera} onfail={() => (noLive = true)} />
        {/if}
      </div>
    {/if}
    {#if tiles.length}
      <div class="tiles">
        {#each tiles as tile (tile.id)}
          <LightTile id={tile.id} name={tile.name} icon={tile.icon ?? 'socket'} point={tile.point ?? null} also={tile.also ?? []} />
        {/each}
      </div>
    {/if}
    <button class="done" onclick={close}>{t('maison.alarme_fenetre.fermer')}</button>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
    background: rgb(0 0 0 / 55%);
    backdrop-filter: blur(4px);
  }

  .popup {
    position: fixed;
    z-index: 91;
    inset: max(16px, env(safe-area-inset-top)) 16px max(16px, env(safe-area-inset-bottom));
    margin: auto;
    max-width: 960px;
    max-height: calc(100dvh - 32px);
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 18px;
    border-radius: var(--r-xl);
    background: var(--surface);
    color: var(--ink);
    border: 2px solid var(--alert);
    box-shadow: var(--shadow-lift);
  }

  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    color: var(--alert);
    font-size: 1.3rem;
  }

  .cause {
    margin: 4px 0 0;
    font-weight: 600;
  }

  .calm {
    margin: 2px 0 0;
    color: var(--ink);
    opacity: 0.75;
    font-size: 0.9rem;
  }

  .close {
    border: 0;
    background: transparent;
    color: inherit;
    padding: 6px;
    cursor: pointer;
  }

  .camera {
    border-radius: var(--r-lg);
    overflow: hidden;
    aspect-ratio: 16 / 9;
    background: #000;
  }

  .camera :global(video),
  .camera :global(img) {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 12px;
  }

  .done {
    align-self: flex-end;
    padding: 12px 26px;
    border: 0;
    border-radius: 999px;
    background: var(--alert);
    color: #fff;
    font: inherit;
    font-weight: 650;
    cursor: pointer;
  }
</style>
