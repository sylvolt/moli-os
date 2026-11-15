<script>
  import { device, value, reachable, nameOf, act, num } from '../lib/home.svelte.js';
  import { statusOf, plugged, MODES, hoursMinutes, currentRange } from '../lib/evcharger.js';
  import { watts } from '../lib/energy-live.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** One EV charging station: what it does now (state, power, current), the
   *  session (energy, time), and the hand on it: mode, charging allowed,
   *  charging current. Stopping a charge in progress asks twice. Every
   *  order goes through the hub like any other (approval, mirror house). */
  let { id } = $props();

  const d = $derived(device(id));
  const v = (k) => value(id, k);
  const ok = $derived(reachable(id));
  const can = (key) => ok && d?.points.some((p) => p.key === key && p.access?.write);
  const status = $derived(statusOf(v('status')));
  const charging = $derived(v('status') === 'charging');
  const power = $derived(typeof v('power') === 'number' ? v('power') : null);
  const mode = $derived(v('mode'));
  const enabled = $derived(v('charging_enabled') === true);
  const range = $derived(currentRange(d));

  // The slider shows where the finger is; the order leaves on release.
  let dragging = $state(null);
  const shownCurrent = $derived(dragging ?? (typeof v('set_current') === 'number' ? v('set_current') : range.min));

  function setMode(next) {
    if (next !== mode) act(`${id}/mode`, next, t('borne.act_mode', { nom: nameOf(id), mode: t(`borne.mode.${next}`) }));
  }

  let confirm = $state(false);
  let confirmTimer;
  function toggleCharging() {
    if (enabled && charging && !confirm) {
      confirm = true;
      clearTimeout(confirmTimer);
      confirmTimer = setTimeout(() => (confirm = false), 5000);
      return;
    }
    confirm = false;
    act(`${id}/charging_enabled`, !enabled, t(enabled ? 'borne.act_arreter' : 'borne.act_autoriser', { nom: nameOf(id) }));
  }

  function setCurrent(event) {
    const amps = Number(event.currentTarget.value);
    dragging = null;
    if (amps !== v('set_current')) act(`${id}/set_current`, amps, t('borne.act_courant', { nom: nameOf(id), a: amps }));
  }
</script>

<section class="card borne" data-status={status.key ?? 'unknown'}>
  <div class="head">
    <span class="logo" class:on={charging}><Icon name="ev-station" size={22} /></span>
    <div class="title">
      <h2>{nameOf(id)}</h2>
      {#if d?.model}<span class="muted">{d.model}</span>{/if}
    </div>
    <span class="chip {status.tone}">
      {ok ? (status.key ? t(`borne.etat.${status.key}`) : t('borne.etat.inconnu')) : t('borne.hors_ligne')}
    </span>
  </div>

  <div class="now">
    <div class="figure">
      <strong class="big num">{charging || (power ?? 0) > 0 ? watts(power) : '—'}</strong>
      <span class="muted small">{t('borne.puissance')}</span>
    </div>
    <div class="figure">
      <strong class="num">{typeof v('current') === 'number' ? `${num(v('current'), 1)} A` : '—'}</strong>
      <span class="muted small">{t('borne.courant_actuel')}</span>
    </div>
    <div class="figure">
      <strong class="num">{typeof v('session_energy') === 'number' ? `${num(v('session_energy'), 2)} kWh` : '—'}</strong>
      <span class="muted small">{t('borne.session')}{plugged(v('status')) && hoursMinutes(v('charging_time')) ? ` · ${hoursMinutes(v('charging_time'))}` : ''}</span>
    </div>
  </div>

  {#if can('mode')}
    <div class="row">
      <span class="label">{t('borne.mode_titre')}</span>
      <div class="segmented" role="radiogroup" aria-label={t('borne.mode_titre')}>
        {#each MODES as m (m)}
          <button role="radio" aria-checked={mode === m} class:active={mode === m} onclick={() => setMode(m)}>{t(`borne.mode.${m}`)}</button>
        {/each}
      </div>
    </div>
    {#if mode === 'auto'}
      <p class="hint muted small">{t('borne.aide_auto')}</p>
    {/if}
  {/if}

  {#if can('charging_enabled')}
    <div class="row">
      <span class="label">{t('borne.charge_autorisee')}</span>
      <button class="act" class:lit={enabled} class:confirm onclick={toggleCharging} aria-pressed={enabled}>
        <Icon name={enabled ? 'pause' : 'play'} size={18} />
        {confirm ? t('borne.confirmer_arret') : enabled ? t('borne.arreter') : t('borne.autoriser')}
      </button>
    </div>
  {/if}

  {#if can('set_current')}
    <label class="row slider" for="courant-{id}">
      <span class="label">{t('borne.courant_consigne')}</span>
      <span class="num value">{shownCurrent} A</span>
      <input
        id="courant-{id}"
        type="range"
        min={range.min}
        max={range.max}
        step={range.step}
        value={shownCurrent}
        oninput={(e) => (dragging = Number(e.currentTarget.value))}
        onchange={setCurrent}
      />
    </label>
  {/if}

  {#if typeof v('energy') === 'number'}
    <p class="totals muted small">{t('borne.total', { kwh: num(v('energy'), 0) })}</p>
  {/if}
</section>

<style>
  .borne {
    display: grid;
    gap: 16px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .logo {
    display: grid;
    place-items: center;
    width: 42px;
    height: 42px;
    border-radius: 12px;
    background: var(--surface-2);
    color: var(--ink-3);
  }

  .logo.on {
    background: var(--warm-soft);
    color: var(--warm-ink);
  }

  .title {
    flex: 1;
    min-width: 0;
    display: grid;
  }

  .title h2 {
    margin: 0;
    font-size: 17px;
  }

  .now {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 110px), 1fr));
    gap: 12px;
  }

  .figure {
    display: grid;
    gap: 2px;
  }

  .figure strong {
    font-size: 20px;
    font-weight: 600;
  }

  .figure .big {
    font-size: 28px;
  }

  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 12px;
  }

  .label {
    flex: 1;
    min-width: 140px;
    font-weight: 600;
  }

  .segmented {
    display: inline-flex;
    border-radius: 999px;
    background: var(--surface-2);
    padding: 3px;
    gap: 2px;
  }

  .segmented button {
    border: 0;
    background: transparent;
    color: var(--ink-2);
    padding: 7px 14px;
    border-radius: 999px;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .segmented button.active {
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow);
  }

  .act {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 0;
    border-radius: 999px;
    padding: 8px 16px;
    background: var(--surface-2);
    color: var(--ink);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .act.lit {
    background: var(--good-soft);
  }

  .act.confirm {
    background: var(--alert-soft);
    color: var(--alert);
  }

  .slider input {
    flex-basis: 100%;
    accent-color: var(--warm);
  }

  .value {
    font-weight: 600;
  }

  .hint,
  .totals {
    margin: 0;
  }

  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--cool);
    outline-offset: 2px;
  }
</style>
