<script>
  import { value, act, nameOf, since, relative, home } from '../lib/home.svelte.js';
  import { canWrite } from '../lib/capabilities.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** A robotic mower (Navimow): what it does in words, its battery, and the
   *  orders its points allow (mow or stop, pause or resume, back to its
   *  dock). Each button shows only where its point can be written. */
  let { id } = $props();

  const state = $derived(value(id, 'state'));
  const battery = $derived(value(id, 'battery'));
  const mowing = $derived(value(id, 'mow') === true);
  const paused = $derived(value(id, 'pause') === true);
  const fault = $derived(value(id, 'fault') === true);
  const changed = $derived(since(id, 'state'));
  const tone = $derived(fault ? 'alert' : state === 'mowing' ? 'good' : state === 'offline' ? '' : 'cool');
  const word = $derived(t(`maison.tondeuse.etat.${state ?? 'unknown'}`));
</script>

<section class="card mower">
  <div class="card-head">
    <h2><Icon name="robot-mower" size={18} />{nameOf(id, t('maison.tondeuse.titre'))}</h2>
    <span class="chip {tone}">{word}</span>
  </div>

  <div class="facts">
    <div>
      <small>{t('maison.tondeuse.batterie')}</small>
      <b class="num">{typeof battery === 'number' ? `${Math.round(battery)} %` : '—'}</b>
      {#if typeof battery === 'number'}<span class="track" class:low={battery < 20}><i style:width="{battery}%"></i></span>{/if}
    </div>
    {#if changed}
      <div>
        <small>{t('maison.tondeuse.depuis')}</small>
        <b>{relative(changed, home.now)}</b>
      </div>
    {/if}
  </div>

  {#if fault}
    <p class="problem"><Icon name="warning" size={16} />{t('maison.tondeuse.probleme', { detail: value(id, 'detail') || '—' })}</p>
  {/if}

  <div class="actions">
    {#if canWrite(id, 'mow')}
      {#if !mowing}
        <button class="main" onclick={() => act(`${id}/mow`, true, t('maison.tondeuse.tondre'))}><Icon name="play" size={18} />{t('maison.tondeuse.tondre')}</button>
      {:else if canWrite(id, 'pause') && !paused}
        <button class="main" onclick={() => act(`${id}/pause`, true, t('maison.tondeuse.pause'))}><Icon name="pause" size={18} />{t('maison.tondeuse.pause')}</button>
      {:else if canWrite(id, 'pause') && paused}
        <button class="main" onclick={() => act(`${id}/pause`, false, t('maison.tondeuse.reprendre'))}><Icon name="play" size={18} />{t('maison.tondeuse.reprendre')}</button>
      {/if}
      {#if mowing}
        <button onclick={() => act(`${id}/mow`, false, t('maison.tondeuse.arreter'))}><Icon name="stop" size={18} />{t('maison.tondeuse.arreter')}</button>
      {/if}
    {/if}
    {#if canWrite(id, 'dock') && state !== 'docked' && state !== 'returning'}
      <button onclick={() => act(`${id}/dock`, true, t('maison.tondeuse.retour_base'))}><Icon name="home" size={18} />{t('maison.tondeuse.retour_base')}</button>
    {/if}
  </div>
</section>

<style>
  .mower {
    display: grid;
    gap: 14px;
  }

  .facts {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 12px;
  }

  .facts div {
    display: grid;
    gap: 4px;
  }

  .facts small {
    color: var(--ink-2);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .facts b {
    font-size: 20px;
  }

  .track {
    height: 6px;
    border-radius: 999px;
    background: var(--surface-3);
    overflow: hidden;
  }

  .track i {
    display: block;
    height: 100%;
    background: var(--good);
  }

  .track.low i {
    background: var(--warm);
  }

  .problem {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    color: var(--alert);
    font-weight: 600;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .actions button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 44px;
    padding: 0 16px;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .actions button.main {
    border-color: transparent;
    background: var(--good);
    color: #fff;
  }
</style>
