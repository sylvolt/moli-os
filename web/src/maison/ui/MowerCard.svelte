<script>
  import { value, act, nameOf, since, relative, home, device } from '../lib/home.svelte.js';
  import { canWrite } from '../lib/capabilities.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** A robotic mower (Navimow): what it does in words, its battery, and the
   *  orders its points allow (mow or stop, pause or resume, back to its
   *  dock). Each button shows only where its point can be written. */
  let { id } = $props();

  const dev = $derived(device(id));

  const state = $derived(value(id, 'state'));
  const battery = $derived(value(id, 'battery'));
  const mowing = $derived(value(id, 'mow') === true);
  const paused = $derived(value(id, 'pause') === true);
  const fault = $derived(value(id, 'fault') === true);
  const changed = $derived(since(id, 'state'));
  const tone = $derived(fault ? 'alert' : state === 'mowing' ? 'good' : state === 'offline' ? '' : 'cool');
  const word = $derived(t(`maison.tondeuse.etat.${state ?? 'unknown'}`));

  // Where the last order is: on its way (Moli waits for Navimow's answer),
  // accepted and waiting for the mower to move, or refused.
  let sending = $state(null);
  let step = $state(null); // { kind: 'accepted' | 'slow' | 'refused', from, at }

  async function order(key, val, label) {
    if (sending) return;
    sending = key;
    step = null;
    const from = state;
    const res = await act(`${id}/${key}`, val, label);
    sending = null;
    if (res === 'ok') step = { kind: 'accepted', from, at: Date.now() };
    else if (res === 'error') step = { kind: 'refused', from, at: Date.now() };
  }

  // The mower moved: the order is done. Too long without a move: say so.
  $effect(() => {
    if (step?.kind === 'accepted' && state !== step.from) step = null;
  });
  $effect(() => {
    const s = step;
    if (!s) return;
    const wait = s.kind === 'accepted' ? 90_000 : 8_000;
    const timer = setTimeout(() => {
      if (step !== s) return;
      step = s.kind === 'accepted' ? { ...s, kind: 'slow' } : null;
    }, wait);
    return () => clearTimeout(timer);
  });
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
    {#snippet button(key, val, label, icon, main = false)}
      <button class:main disabled={sending !== null} aria-busy={sending === key} onclick={() => order(key, val, label)}>
        {#if sending === key}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name={icon} size={16} />{/if}
        <span class="label">{label}</span>
      </button>
    {/snippet}
    {#if canWrite(dev, 'mow')}
      {#if !mowing}
        {@render button('mow', true, t('maison.tondeuse.tondre'), 'play', true)}
      {:else if canWrite(dev, 'pause') && !paused}
        {@render button('pause', true, t('maison.tondeuse.pause'), 'pause', true)}
      {:else if canWrite(dev, 'pause') && paused}
        {@render button('pause', false, t('maison.tondeuse.reprendre'), 'play', true)}
      {/if}
      {#if mowing}
        {@render button('mow', false, t('maison.tondeuse.arreter'), 'stop')}
      {/if}
    {/if}
    {#if canWrite(dev, 'dock') && state !== 'docked' && state !== 'returning'}
      {@render button('dock', true, t('maison.tondeuse.retour_base'), 'home')}
    {/if}
  </div>

  <p class="step {step?.kind ?? ''}" role="status" aria-live="polite" hidden={!sending && !step}>
    {#if sending}
      <span class="spinner" aria-hidden="true"></span>{t('maison.tondeuse.envoi')}
    {:else if step?.kind === 'accepted'}
      <span class="dot" aria-hidden="true"></span>{t('maison.tondeuse.accepte')}
    {:else if step?.kind === 'slow'}
      <Icon name="warning" size={14} />{t('maison.tondeuse.lent')}
    {:else if step?.kind === 'refused'}
      <Icon name="warning" size={14} />{t('maison.tondeuse.refuse')}
    {/if}
  </p>
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
    gap: 6px;
  }

  .actions button {
    flex: 1 1 auto;
    min-width: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    min-height: 38px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
    transition: transform 120ms ease, opacity 160ms ease;
  }

  .actions button .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .actions button:active:not(:disabled) {
    transform: scale(0.96);
  }

  .actions button:disabled {
    cursor: default;
    opacity: 0.55;
  }

  .actions button[aria-busy='true'] {
    opacity: 1;
  }

  .actions button.main {
    border-color: transparent;
    background: var(--good);
    color: #fff;
  }

  .step {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 18px;
    margin: -6px 0 0;
    color: var(--ink-2);
    font-size: 13px;
  }

  .step[hidden] {
    display: none;
  }

  .step.accepted {
    color: var(--good);
  }

  .step.slow,
  .step.refused {
    color: var(--alert);
  }

  .spinner {
    flex: none;
    width: 14px;
    height: 14px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 800ms linear infinite;
  }

  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: currentColor;
    animation: pulse 1.2s ease-in-out infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @keyframes pulse {
    50% {
      opacity: 0.25;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner,
    .dot {
      animation: none;
    }

    .actions button {
      transition: none;
    }
  }
</style>
