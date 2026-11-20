<script>
  import { value, reachable, act, num, pending } from '../lib/home.svelte.js';
  import { INVERTER_MODES, RISKY_MODES, ESS_MODES, rangeOf, limitOf, sliderStart, snap, flowOf } from '../lib/victron-battery.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** The home battery of a Victron GX, and the hand on it: the inverter's
   *  mode, the ESS mode, the minimum charge, the grid setpoint and the
   *  charge / discharge ceilings. Taking the house off the grid or off
   *  power asks twice. Every order goes through the hub (approval, mirror). */
  let { battery = null, inverter = null, settings = null } = $props();

  const bat = $derived(battery?.id);
  const inv = $derived(inverter?.id);
  const ess = $derived(settings?.id);
  const can = (dev, key) => !!dev && reachable(dev.id) && dev.points.some((p) => p.key === key && p.access?.write);
  const has = (dev, key) => !!dev?.points?.some((p) => p.key === key);

  const soc = $derived(bat ? value(bat, 'soc') : null);
  const power = $derived(bat ? value(bat, 'power') : null);
  const flow = $derived(flowOf(power));
  const minSoc = $derived(ess ? value(ess, 'minimum_soc') : null);
  const invMode = $derived(inv ? value(inv, 'mode') : null);
  const invState = $derived(inv ? value(inv, 'state') : null);
  const essMode = $derived(ess ? value(ess, 'ess_mode') : null);

  // Sliders show where the finger is; the order leaves on release.
  let drag = $state({});
  const socRange = $derived(rangeOf(settings, 'minimum_soc', { min: 10, max: 100, step: 5 }));
  const limitRange = $derived(rangeOf(settings, 'max_charge_power', { min: -1, max: 10000, step: 100 }));
  const ceiling = $derived({ min: 0, max: limitRange.max, step: limitRange.step });
  const gridRange = $derived(rangeOf(settings, 'grid_setpoint', { min: -5000, max: 5000, step: 10 }));
  const shownMin = $derived(drag.minimum_soc ?? (typeof minSoc === 'number' ? minSoc : socRange.min));
  const fill = (v, r) => `${((v - r.min) / Math.max(1, r.max - r.min)) * 100}%`;

  const busy = (dev, key) => !!dev && pending(dev.id, key);
  const send = (dev, key, val, label) => act(`${dev.id}/${key}`, val, label);

  // Asked twice: the mode waiting for its second tap.
  let confirm = $state(null);
  let confirmTimer;
  function setInverter(next) {
    if (next === invMode) return;
    if (RISKY_MODES.has(next) && confirm !== next) {
      confirm = next;
      clearTimeout(confirmTimer);
      confirmTimer = setTimeout(() => (confirm = null), 6000);
      return;
    }
    confirm = null;
    send(inverter, 'mode', next, t('recharge.act_onduleur', { mode: t(`recharge.onduleur.${next}`) }));
  }

  function setEss(next) {
    if (next !== essMode) send(settings, 'ess_mode', next, t('recharge.act_ess', { mode: t(`recharge.ess.${next}`) }));
  }

  function release(key, val, label) {
    drag = { ...drag, [key]: undefined };
    send(settings, key, val, label);
  }

  // The grid setpoint: typed, then sent.
  let gridDraft = $state(null);
  const gridNow = $derived(ess ? value(ess, 'grid_setpoint') : null);
  const gridShown = $derived(gridDraft ?? (typeof gridNow === 'number' ? gridNow : 0));
  function nudge(by) {
    gridDraft = snap(gridShown + by, gridRange);
  }
  function sendGrid() {
    const w = snap(Number(gridDraft), gridRange);
    gridDraft = null;
    if (w !== null && w !== gridNow) send(settings, 'grid_setpoint', w, t('recharge.act_consigne', { w: num(w, 0) }));
  }

  const LIMITS = ['max_charge_power', 'max_discharge_power'];
  const limit = (key) => (ess ? limitOf(value(ess, key)) : undefined);
  function toggleLimit(key) {
    const now = limit(key);
    const next = now === null ? sliderStart(null, ceiling) / 2 : -1;
    send(settings, key, next, t(`recharge.act_${key}`, { w: next < 0 ? t('recharge.sans_limite') : `${num(next, 0)} W` }));
  }
</script>

<section class="card batterie">
  <div class="head">
    <span class="logo" class:on={flow === 'charge'}><Icon name={typeof soc === 'number' && soc < 20 ? 'battery-20' : 'battery'} size={22} /></span>
    <div class="title">
      <h2>{t('recharge.batterie_titre')}</h2>
      {#if inverter?.native_name}<span class="muted small">{inverter.native_name}</span>{/if}
    </div>
    {#if flow}
      <span class="chip {flow === 'charge' ? 'good' : flow === 'discharge' ? 'warm' : ''}">
        {t(`recharge.flux.${flow}`)}{flow !== 'idle' ? ` · ${num(Math.abs(power) / 1000, 2)} kW` : ''}
      </span>
    {/if}
  </div>

  {#if typeof soc === 'number'}
    <div class="soc">
      <div class="soc-head">
        <strong class="num">{num(soc, 0)}<small>%</small></strong>
        {#if typeof minSoc === 'number'}<span class="muted small">{t('recharge.minimum_a', { pct: num(minSoc, 0) })}</span>{/if}
      </div>
      <div class="bar" role="img" aria-label={t('recharge.soc_aria', { pct: num(soc, 0) })}>
        <i style:width="{Math.min(100, Math.max(0, soc))}%" class:low={typeof minSoc === 'number' && soc <= minSoc}></i>
        {#if typeof minSoc === 'number'}<b class="mark" style:left="{minSoc}%" aria-hidden="true"></b>{/if}
      </div>
    </div>
  {/if}

  {#if can(inverter, 'mode')}
    <div class="block">
      <div class="block-head">
        <span class="label">{t('recharge.onduleur_titre')}</span>
        {#if invState}<span class="muted small">{t(`recharge.etat.${invState}`)}</span>{/if}
      </div>
      <div class="segmented four" role="radiogroup" aria-label={t('recharge.onduleur_titre')} class:busy={busy(inverter, 'mode')}>
        {#each INVERTER_MODES as m (m)}
          <button role="radio" aria-checked={invMode === m} class:active={invMode === m} class:confirm={confirm === m} onclick={() => setInverter(m)}>
            {confirm === m ? t('recharge.confirmer') : t(`recharge.onduleur.${m}`)}
          </button>
        {/each}
      </div>
      {#if confirm}
        <p class="warn small"><Icon name="alert" size={14} />{t(`recharge.attention.${confirm}`)}</p>
      {/if}
    </div>
  {/if}

  {#if can(settings, 'ess_mode')}
    <div class="block">
      <span class="label">{t('recharge.ess_titre')}</span>
      <div class="segmented" role="radiogroup" aria-label={t('recharge.ess_titre')} class:busy={busy(settings, 'ess_mode')}>
        {#each ESS_MODES as m (m)}
          <button role="radio" aria-checked={essMode === m} class:active={essMode === m} onclick={() => setEss(m)}>{t(`recharge.ess.${m}`)}</button>
        {/each}
      </div>
      {#if essMode}<p class="muted small help">{t(`recharge.ess_aide.${essMode}`)}</p>{/if}
    </div>
  {/if}

  {#if can(settings, 'minimum_soc')}
    <div class="block">
      <label class="block-head" for="soc-min-{ess}">
        <span class="label">{t('recharge.soc_min')}</span>
        <strong class="num">{shownMin}<small>%</small></strong>
      </label>
      <input
        id="soc-min-{ess}"
        type="range"
        min={socRange.min}
        max={socRange.max}
        step={socRange.step}
        value={shownMin}
        class:busy={busy(settings, 'minimum_soc')}
        style:--fill={fill(shownMin, socRange)}
        oninput={(e) => (drag = { ...drag, minimum_soc: Number(e.currentTarget.value) })}
        onchange={(e) => {
          const pct = Number(e.currentTarget.value);
          if (pct !== minSoc) release('minimum_soc', pct, t('recharge.act_soc_min', { pct }));
          else drag = { ...drag, minimum_soc: undefined };
        }}
      />
      <p class="muted small help">{t('recharge.soc_min_aide')}</p>
    </div>
  {/if}

  {#if can(settings, 'grid_setpoint')}
    <div class="block">
      <label class="block-head" for="grid-{ess}">
        <span class="label">{t('recharge.consigne_reseau')}</span>
      </label>
      <div class="stepper" class:busy={busy(settings, 'grid_setpoint')}>
        <button aria-label={t('recharge.moins', { w: 10 })} onclick={() => nudge(-10)}>−</button>
        <input id="grid-{ess}" type="number" inputmode="numeric" min={gridRange.min} max={gridRange.max} step={gridRange.step} value={gridShown}
          oninput={(e) => (gridDraft = e.currentTarget.value === '' ? null : Number(e.currentTarget.value))}
          onkeydown={(e) => e.key === 'Enter' && gridDraft !== null && sendGrid()} />
        <span class="unit">W</span>
        <button aria-label={t('recharge.plus', { w: 10 })} onclick={() => nudge(10)}>+</button>
        {#if gridDraft !== null && gridDraft !== gridNow}
          <button class="apply" onclick={sendGrid}>{t('recharge.appliquer')}</button>
        {/if}
      </div>
      <p class="muted small help">{t('recharge.consigne_aide')}</p>
    </div>
  {/if}

  {#each LIMITS as key (key)}
    {#if can(settings, key)}
      {@const lim = limit(key)}
      {@const shown = drag[key] ?? sliderStart(lim, ceiling)}
      <div class="block">
        <div class="block-head">
          <label class="label" for="{key}-{ess}">{t(`recharge.${key}`)}</label>
          <strong class="num">{lim === null && drag[key] === undefined ? t('recharge.sans_limite') : `${num(shown / 1000, 1)}`}{#if !(lim === null && drag[key] === undefined)}<small>kW</small>{/if}</strong>
        </div>
        <input
          id="{key}-{ess}"
          type="range"
          min={ceiling.min}
          max={ceiling.max}
          step={ceiling.step}
          value={shown}
          disabled={lim === null}
          class:busy={busy(settings, key)}
          style:--fill={fill(shown, ceiling)}
          oninput={(e) => (drag = { ...drag, [key]: Number(e.currentTarget.value) })}
          onchange={(e) => {
            const w = Number(e.currentTarget.value);
            if (w !== lim) release(key, w, t(`recharge.act_${key}`, { w: `${num(w, 0)} W` }));
            else drag = { ...drag, [key]: undefined };
          }}
        />
        <label class="switch small">
          <input type="checkbox" checked={lim === null} onchange={() => toggleLimit(key)} disabled={busy(settings, key)} />
          {t('recharge.sans_limite')}
        </label>
      </div>
    {/if}
  {/each}

  {#if !has(settings, 'ess_mode') && settings}
    <p class="muted small">{t('recharge.pilote_ancien')}</p>
  {/if}
</section>

<style>
  .batterie {
    display: grid;
    gap: 16px;
    max-width: 560px;
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

  .soc {
    display: grid;
    gap: 8px;
  }

  .soc-head,
  .block-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .soc-head strong {
    font-size: 30px;
    font-weight: 600;
  }

  .block-head strong {
    font-size: 20px;
    font-weight: 600;
  }

  .soc-head small,
  .block-head small {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
    margin-left: 3px;
  }

  .bar {
    position: relative;
    height: 10px;
    border-radius: 999px;
    background: var(--surface-3);
  }

  .bar i {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--good);
    transition: width 0.6s ease;
  }

  .bar i.low {
    background: var(--warm);
  }

  .bar .mark {
    position: absolute;
    top: -4px;
    bottom: -4px;
    width: 2px;
    margin-left: -1px;
    background: var(--ink);
    border-radius: 1px;
  }

  .block {
    display: grid;
    gap: 8px;
    padding: 14px 16px;
    border-radius: 14px;
    background: var(--surface-2);
  }

  .label {
    font-weight: 600;
  }

  .help {
    margin: 0;
  }

  .segmented {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 1fr;
    gap: 4px;
    padding: 4px;
    border-radius: 12px;
    background: var(--surface);
  }

  .segmented button {
    min-width: 0;
    border: 1px solid transparent;
    background: transparent;
    color: var(--ink-2);
    padding: 9px 6px;
    border-radius: 9px;
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s ease, transform 0.1s ease;
  }

  .segmented button:active {
    transform: scale(0.97);
  }

  .segmented button.active {
    background: var(--cool-soft);
    border-color: var(--cool);
    color: var(--cool);
  }

  .segmented button.confirm {
    background: var(--alert);
    color: var(--surface);
  }

  .busy {
    animation: wait 1s ease-in-out infinite;
  }

  @keyframes wait {
    50% {
      opacity: 0.5;
    }
  }

  .warn {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    color: var(--alert);
    font-weight: 600;
  }

  input[type='range'] {
    width: 100%;
    accent-color: var(--cool);
  }

  input[type='range']:disabled {
    accent-color: var(--ink-3);
    opacity: 0.5;
  }

  .stepper {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .stepper button {
    min-width: 40px;
    height: 38px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 18px;
    font-weight: 600;
    cursor: pointer;
  }

  .stepper input {
    width: 90px;
    height: 38px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 16px;
    text-align: right;
  }

  .stepper .unit {
    color: var(--ink-3);
  }

  .stepper .apply {
    padding: 0 14px;
    border-color: transparent;
    background: var(--cool);
    color: #fff;
    font-size: 14px;
  }

  .switch {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }

  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--cool);
    outline-offset: 2px;
  }

  @media (max-width: 420px) {
    .segmented.four {
      grid-auto-flow: row;
      grid-template-columns: 1fr 1fr;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .busy {
      animation: none;
    }

    .bar i,
    .segmented button {
      transition: none;
    }
  }
  /* A wall panel: the controls two by two, their explanations left out, so
     the card holds on the screen. Same query as PANEL in lib/layout.js. */
  @media (min-width: 761px) and (max-height: 960px) and (orientation: landscape) {
    .batterie {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
      align-items: start;
      gap: 10px 12px;
      max-width: none;
    }

    .batterie > :not(.block) {
      grid-column: 1 / -1;
    }

    .head {
      padding-bottom: 10px;
    }

    .soc-head strong {
      font-size: 26px;
    }

    .block {
      align-self: stretch;
      align-content: start;
      gap: 6px;
      padding: 10px 12px;
    }

    .help {
      display: none;
    }

    .segmented.four {
      grid-auto-flow: row;
      grid-template-columns: 1fr 1fr;
    }

    .segmented button {
      padding: 7px 4px;
      line-height: 1.2;
      white-space: normal;
    }
  }
</style>
