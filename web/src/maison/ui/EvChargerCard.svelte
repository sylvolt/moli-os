<script>
  import { device, value, reachable, nameOf, act, num } from '../lib/home.svelte.js';
  import { statusOf, plugged, hoursMinutes, currentRange, fullScaleKw, gaugeFraction } from '../lib/evcharger.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** One EV charging station, read at a glance: the power on a gauge with
   *  the measured current and the set one around it, the charging current
   *  right below (locked while the sun drives it), the mode, the state, the
   *  session, and one large start / stop. Stopping a charge in progress asks
   *  twice. Every order goes through the hub (approval, mirror house). */
  let { id } = $props();

  const d = $derived(device(id));
  const v = (k) => value(id, k);
  const ok = $derived(reachable(id));
  const can = (key) => ok && d?.points.some((p) => p.key === key && p.access?.write);
  const status = $derived(statusOf(v('status')));
  const charging = $derived(v('status') === 'charging');
  const power = $derived(typeof v('power') === 'number' ? Math.max(0, v('power')) : null);
  const mode = $derived(v('mode'));
  const auto = $derived(mode === 'auto');
  const enabled = $derived(v('charging_enabled') === true);
  const range = $derived(currentRange(d));
  const scale = $derived(fullScaleKw(range));
  const fraction = $derived(gaugeFraction(power, scale));

  // The slider shows where the finger is; the order leaves on release.
  let dragging = $state(null);
  const setPoint = $derived(typeof v('set_current') === 'number' ? v('set_current') : null);
  const shownCurrent = $derived(dragging ?? setPoint ?? range.min);
  const sliderFill = $derived(((shownCurrent - range.min) / Math.max(1, range.max - range.min)) * 100);

  // Auto and Manuel, as on the station's screen; « Programmé » only when the
  // station is in it (set from its schedule, not from here).
  const modes = $derived(mode === 'scheduled' ? ['auto', 'manual', 'scheduled'] : ['auto', 'manual']);

  function setMode(next) {
    if (next !== mode) act(`${id}/mode`, next, t('borne.act_mode', { nom: nameOf(id), mode: t(`borne.mode.${next}`) }));
  }

  let confirm = $state(false);
  let confirmTimer;
  function startStop() {
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
    if (amps !== setPoint) act(`${id}/set_current`, amps, t('borne.act_courant', { nom: nameOf(id), a: amps }));
  }

  // The gauge: 240° from bottom left to bottom right, through the top.
  const R = 78;
  const at = (deg) => [100 + R * Math.cos((deg * Math.PI) / 180), 100 + R * Math.sin((deg * Math.PI) / 180)];
  const [sx, sy] = at(150);
  const [ex, ey] = at(30);
  const ARC = `M ${sx.toFixed(1)} ${sy.toFixed(1)} A ${R} ${R} 0 1 1 ${ex.toFixed(1)} ${ey.toFixed(1)}`;
</script>

<section class="card borne" class:off={!ok} data-status={status.key ?? 'unknown'}>
  <div class="head">
    <span class="logo" class:on={charging}><Icon name="ev-station" size={22} /></span>
    <div class="title">
      <h2>{nameOf(id)}</h2>
      {#if d?.model}<span class="muted small">{d.model}</span>{/if}
    </div>
    <span class="chip {status.tone}">
      <i class="dot" aria-hidden="true"></i>
      {ok ? (status.key ? t(`borne.etat.${status.key}`) : t('borne.etat.inconnu')) : t('borne.hors_ligne')}
    </span>
  </div>

  <div class="dial">
    <div class="side">
      <span class="muted small">{t('borne.courant_actuel')}</span>
      <strong class="num">{typeof v('current') === 'number' ? num(v('current'), 1) : '—'}<small>A</small></strong>
    </div>
    <svg viewBox="0 0 200 170" role="img" aria-label={t('borne.jauge', { kw: power == null ? '—' : num(power / 1000, 2), max: num(scale, 1) })}>
      <path class="track" d={ARC} pathLength="100" />
      {#if fraction > 0}
        <path class="fill" class:live={charging} d={ARC} pathLength="100" stroke-dasharray="{(fraction * 100).toFixed(2)} 100" />
      {/if}
      <text class="cap" x="100" y="78">{t('borne.puissance')}</text>
      <text class="kw" x="100" y="112">{power == null ? '—' : num(power / 1000, 2)}<tspan class="unit" dx="4">kW</tspan></text>
      <text class="end" x={sx.toFixed(1)} y={(sy + 22).toFixed(1)}>0</text>
      <text class="end" x={ex.toFixed(1)} y={(ey + 22).toFixed(1)}>{num(scale, 1)}</text>
    </svg>
    <div class="side">
      <span class="muted small">{t('borne.consigne')}</span>
      <strong class="num">{setPoint == null ? '—' : num(setPoint, 0)}<small>A</small></strong>
    </div>
  </div>

  {#if can('set_current')}
    <div class="current" class:locked={auto}>
      <label class="current-head" for="courant-{id}">
        <span class="label">{t('borne.courant_consigne')}</span>
        <strong class="num">{shownCurrent}<small>A</small></strong>
      </label>
      <input
        id="courant-{id}"
        type="range"
        min={range.min}
        max={range.max}
        step={range.step}
        value={shownCurrent}
        disabled={auto}
        style:--fill="{sliderFill}%"
        oninput={(e) => (dragging = Number(e.currentTarget.value))}
        onchange={setCurrent}
      />
      <div class="ends muted small"><span>{range.min} A</span><span>{range.max} A</span></div>
      {#if auto}
        <p class="lock muted small"><Icon name="lock" size={14} />{t('borne.aide_auto')}</p>
      {/if}
    </div>
  {/if}

  {#if can('mode')}
    <div class="mode">
      <span class="label">{t('borne.mode_titre')}</span>
      <div class="segmented" role="radiogroup" aria-label={t('borne.mode_titre')}>
        {#each modes as m (m)}
          <button role="radio" aria-checked={mode === m} class:active={mode === m} disabled={m === 'scheduled'} onclick={() => setMode(m)}>{t(`borne.mode.${m}`)}</button>
        {/each}
      </div>
    </div>
  {/if}

  <div class="status">
    <span class="muted"><Icon name="info" size={16} />{t('borne.statut')}</span>
    <strong>{ok ? (status.key ? t(`borne.etat.${status.key}`) : t('borne.etat.inconnu')) : t('borne.hors_ligne')}</strong>
  </div>

  <div class="session muted small">
    <span>{t('borne.session')} : <b class="num">{typeof v('session_energy') === 'number' ? `${num(v('session_energy'), 2)} kWh` : '—'}</b>{plugged(v('status')) && hoursMinutes(v('charging_time')) ? ` · ${hoursMinutes(v('charging_time'))}` : ''}</span>
    {#if typeof v('energy') === 'number'}<span>{t('borne.total', { kwh: num(v('energy'), 0) })}</span>{/if}
  </div>

  {#if can('charging_enabled')}
    <button class="startstop" class:stop={enabled} class:confirm onclick={startStop} aria-pressed={enabled}>
      <Icon name={enabled ? 'stop' : 'play'} size={18} />
      {confirm ? t('borne.confirmer_arret') : enabled ? t('borne.arreter_charge') : t('borne.demarrer_charge')}
    </button>
  {/if}
</section>

<style>
  .borne {
    display: grid;
    gap: 18px;
    max-width: 560px;
  }

  .borne.off {
    opacity: 0.7;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--line);
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
    background: var(--good-soft);
    color: var(--good);
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

  .chip .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: currentColor;
    margin-right: 6px;
    vertical-align: 1px;
  }

  .dial {
    display: grid;
    grid-template-columns: 1fr minmax(160px, 200px) 1fr;
    align-items: center;
    gap: 8px;
  }

  .side {
    display: grid;
    justify-items: center;
    gap: 4px;
    text-align: center;
  }

  .side strong,
  .current-head strong {
    font-size: 24px;
    font-weight: 600;
  }

  .side small,
  .current-head small {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
    margin-left: 3px;
  }

  svg {
    width: 100%;
    height: auto;
    display: block;
  }

  .track,
  .fill {
    fill: none;
    stroke-width: 14;
    stroke-linecap: round;
  }

  .track {
    stroke: var(--ink-3);
    opacity: 0.2;
  }

  .fill {
    stroke: var(--ink-3);
    transition: stroke-dasharray 0.6s ease;
  }

  .fill.live {
    stroke: var(--good);
  }

  text {
    text-anchor: middle;
    fill: var(--ink);
  }

  .cap {
    font-size: 12px;
    fill: var(--ink-3);
  }

  .kw {
    font-size: 34px;
    font-weight: 600;
  }

  .unit {
    font-size: 13px;
    font-weight: 500;
    fill: var(--ink-3);
  }

  .end {
    font-size: 11px;
    fill: var(--ink-3);
  }

  .current {
    display: grid;
    gap: 6px;
    padding: 14px 16px;
    border-radius: 14px;
    background: var(--surface-2);
  }

  .current-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .label {
    font-weight: 600;
  }

  .current input {
    width: 100%;
    accent-color: var(--cool);
  }

  .current.locked input {
    accent-color: var(--ink-3);
    cursor: not-allowed;
  }

  .ends {
    display: flex;
    justify-content: space-between;
  }

  .lock {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 2px 0 0;
  }

  .mode {
    display: grid;
    gap: 8px;
  }

  .segmented {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 1fr;
    gap: 4px;
    padding: 4px;
    border-radius: 14px;
    background: var(--surface-2);
  }

  .segmented button {
    border: 1px solid transparent;
    background: transparent;
    color: var(--ink-2);
    padding: 11px 14px;
    border-radius: 10px;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .segmented button.active {
    background: var(--cool-soft);
    border-color: var(--cool);
    color: var(--cool);
  }

  .segmented button:disabled {
    cursor: default;
  }

  .status {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    padding: 14px 16px;
    border-radius: 14px;
    border: 1px solid var(--line);
  }

  .status span {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .session {
    display: flex;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 4px 16px;
  }

  .session b {
    color: var(--ink);
  }

  .startstop {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    width: 100%;
    padding: 14px;
    border-radius: 14px;
    border: 1px solid var(--good);
    background: var(--good-soft);
    color: var(--good);
    font: inherit;
    font-size: 15px;
    font-weight: 650;
    cursor: pointer;
  }

  .startstop.stop {
    border-color: var(--alert);
    background: var(--alert-soft);
    color: var(--alert);
  }

  .startstop.confirm {
    background: var(--alert);
    color: var(--surface);
  }

  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--cool);
    outline-offset: 2px;
  }

  @media (max-width: 420px) {
    .dial {
      grid-template-columns: 1fr 1fr;
    }

    .dial svg {
      grid-column: 1 / -1;
      grid-row: 1;
      max-width: 220px;
      margin: 0 auto;
    }
  }
</style>
