<script>
  import { onMount } from 'svelte';
  import { home, hub, value, device, isOn, clock, num, relative, lastSeen, roomList, lampsOf, nameOf, doorOpen, leak, lowBattery, deviceKind, hidden, act, pending, roomOf, protectedRoom } from '../lib/home.svelte.js';
  import { isPrinter, phase } from '../lib/printers.js';
  import { isValve } from '../lib/capabilities.js';
  import { t, locale } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import LightTile from '../ui/LightTile.svelte';
  import ClimateCard from '../ui/ClimateCard.svelte';
  import ValveCard from '../ui/ValveCard.svelte';
  import VacuumCard from '../ui/VacuumCard.svelte';
  import EnergyCard from '../ui/EnergyCard.svelte';
  import EnergyFlow from '../ui/EnergyFlow.svelte';
  import HistoryChart from '../ui/HistoryChart.svelte';
  import CameraTile from '../ui/CameraTile.svelte';
  import LiveVideo from '../ui/LiveVideo.svelte';
  import AmbiancePicker from '../ui/AmbiancePicker.svelte';
  import ScenePicker from '../ui/ScenePicker.svelte';
  import Routines from '../ui/Routines.svelte';
  import SkyHero from '../ui/SkyHero.svelte';
  import { PANEL } from '../lib/layout.js';

  const cfg = $derived(home.config ?? {});
  const out = $derived(cfg.outdoor ?? {});
  // `outdoor.temperature` : a point (« device/key ») measured outside (a heat
  // pump's outdoor probe), shown first on the Dehors card.
  const outTemp = $derived.by(() => {
    const point = out.temperature;
    if (typeof point !== 'string' || !point.includes('/')) return null;
    const i = point.indexOf('/');
    return { value: value(point.slice(0, i), point.slice(i + 1)) };
  });
  // `favorites.switches` : the on/off things worth a button here (plugs, a
  // gate relay…), `{ id, name, icon, point }`.
  const switches = $derived(cfg.favorites?.switches ?? []);
  // A robot goes through every room: it belongs to none.
  const robots = $derived(Object.values(hub.devices).filter((d) => deviceKind(d) === 'vacuum' && !hidden(d.id)));

  const lights = $derived(cfg.favorites?.lights ?? []);
  const litCount = $derived(lights.filter((l) => [l.id, ...(l.also ?? [])].some((i) => isOn(i))).length);

  // Ambiances, room by room (the last room chosen is remembered here).
  const moodRooms = $derived(roomList().map((r) => ({ name: r.name, lamps: lampsOf(r) })).filter((r) => r.lamps.length));
  let moodPick = $state((() => {
    try {
      return localStorage.getItem('moli.mood-room');
    } catch {
      return null;
    }
  })());
  const mood = $derived(moodRooms.find((r) => r.name === moodPick) ?? moodRooms.find((r) => r.name === 'Salon') ?? moodRooms[0]);
  function pickRoom(name) {
    moodPick = name;
    try {
      localStorage.setItem('moli.mood-room', name);
    } catch {
      /* remembered for this visit only */
    }
  }

  // The house's mode (a helper, « Mode Maison »): one tap, like Home
  // Assistant's mode buttons.
  const modeDevice = $derived(Object.values(hub.devices).find((d) => d.id.endsWith(':mode_maison')));
  const modes = $derived(modeDevice?.points.find((p) => p.key === 'state')?.kind.values ?? []);
  const mode = $derived(modeDevice ? value(modeDevice.id, 'state') : null);
  const MODE_ICON = { Normal: 'home', Nuit: 'moon', Absent: 'lock', Vacances: 'palm' };
  /** A mode's name in the house's language (an unknown one shows as it is). */
  function modeName(m) {
    const key = `maison.accueil.mode.${String(m).toLowerCase()}`;
    const word = t(key);
    return word === key ? m : word;
  }
  function setMode(m) {
    if (m !== mode) act(`${modeDevice.id}/state`, m, t('maison.accueil.mode_action', { mode: modeName(m) }));
  }

  // The speakers that talk (announcements): one tap silences them all, for
  // four hours at most (they speak again by themselves).
  const talkers = $derived(Object.values(hub.devices).filter((d) => d.points.some((p) => p.key === 'quiet' && p.access?.write)));
  const quiet = $derived(talkers.some((d) => value(d.id, 'quiet') === true));
  const quietUntil = $derived(Math.max(0, ...talkers.map((d) => Number(value(d.id, 'quiet_until') ?? 0))));
  function toggleQuiet() {
    for (const d of talkers) act(`${d.id}/quiet`, !quiet, quiet ? t('maison.accueil.annonces_retablies') : t('maison.accueil.annonces_coupees'));
  }

  // What deserves a word right now. Silence when all is well.
  const notices = $derived.by(() => {
    const list = [];
    // Smoke and water from any sensor that knows it, not only the listed ones.
    const all = Object.values(hub.devices);
    for (const d of all) {
      if (value(d.id, 'smoke') === true) list.push({ tone: 'alert', icon: 'fire', text: t('maison.accueil.fumee', { name: nameOf(d.id) }), href: '#/pieces' });
      if (leak(d.id)) list.push({ tone: 'alert', icon: 'water', text: t('maison.accueil.fuite', { name: nameOf(d.id) }), href: '#/pieces' });
    }
    if (hub.approvals?.length) list.push({ tone: 'cool', icon: 'lock', text: t('maison.accueil.demandes', { count: hub.approvals.length }), href: '#/atelier' });
    for (const door of cfg.doors ?? []) {
      const d = device(door.id);
      if (!d) continue;
      if (doorOpen(door.id) === true) list.push({ tone: 'warm', icon: 'door-open', text: t('maison.accueil.porte_ouverte', { name: door.name }), href: '#/pieces' });
    }
    const bell = (cfg.favorites?.cameras ?? [])[0]?.id;
    if (bell && value(bell, 'doorbell') === true) list.push({ tone: 'alert', icon: 'bell', text: t('maison.accueil.sonnette'), href: '#/cameras' });
    if (bell && value(bell, 'person') === true) list.push({ tone: 'warm', icon: 'account', text: t('maison.accueil.quelquun'), href: '#/cameras' });
    for (const p of Object.values(hub.devices).filter(isPrinter)) {
      if (p.online === false) continue;
      const st = phase(value(p.id, 'state'));
      const left = value(p.id, 'remaining');
      const pct = Math.round(Number(value(p.id, 'progress') ?? 0));
      if (st === 'printing') {
        const text =
          typeof left === 'number'
            ? t('maison.accueil.impression_fin', { name: nameOf(p.id), pct, time: clock(home.now + left * 60_000) })
            : t('maison.accueil.impression', { name: nameOf(p.id), pct });
        list.push({ tone: 'cool', icon: 'printer3d', text, href: '#/impression' });
      } else if (st === 'paused') {
        list.push({ tone: 'warm', icon: 'printer3d', text: t('maison.accueil.impression_pause', { name: nameOf(p.id), pct }), href: '#/impression' });
      } else if (st === 'error') {
        list.push({ tone: 'alert', icon: 'printer3d', text: t('maison.accueil.impression_erreur', { name: nameOf(p.id) }), href: '#/impression' });
      }
    }
    if (value('energie:maison', 'over_budget') === true) {
      const bill = value('energie:maison', 'bill_projection');
      list.push({ tone: 'warm', icon: 'bolt', text: t('maison.accueil.facture', { bill: num(bill) }), href: '#/energie' });
    }
    // A dead battery takes its sensor offline: count it all the same.
    const flat = all.filter((d) => lowBattery(d.id));
    if (flat.length) {
      const text =
        flat.length <= 3
          ? t('maison.accueil.piles_liste', { count: flat.length, names: flat.map((d) => nameOf(d.id)).join(', ') })
          : t('maison.accueil.piles', { count: flat.length });
      list.push({ tone: 'cool', icon: 'alert', text, href: '#/pieces' });
    }
    if (out.pool && value(out.pool, 'action_required') === true) list.push({ tone: 'cool', icon: 'pool', text: t('maison.accueil.piscine_oeil'), href: '#/dehors' });
    return list;
  });

  const tv = $derived(cfg.salon?.tv);
  const speaker = $derived(cfg.salon?.speaker);
  const tvOn = $derived(value(tv, 'power') === true);
  const playing = $derived(value(speaker, 'playing') === true);
  const aqi = $derived(value(out.air, 'aqi'));
  const aqiWord = $derived(
    aqi == null
      ? '—'
      : aqi <= 20
        ? t('maison.air.tres_bon')
        : aqi <= 40
          ? t('maison.air.bon')
          : aqi <= 60
            ? t('maison.air.moyen')
            : aqi <= 80
              ? t('maison.air.mediocre')
              : t('maison.air.mauvais'),
  );
  const cams = $derived(cfg.favorites?.cameras ?? []);
  const bell = $derived(cams[0]);

  // The card shows the door; arrows go through the other favorite cameras,
  // and it comes back to the door by itself after a while untouched.
  const BACK_TO_DOOR = 120_000;
  let camIdx = $state(0);
  const shown = $derived(cams[camIdx] ?? bell);
  // The camera shown plays live (WebRTC, straight from the camera) where
  // this browser reaches it: at home, never for a protected room (its live
  // view needs a person, as on the cameras page). Elsewhere, or when the
  // live view fails, its images every 15 s.
  const atHome = $derived(!['access', 'outside'].includes(hub.session?.via));
  // Away from home too where the house opened its cameras' WebRTC port
  // (home.json `live_outside`).
  const liveHere = $derived(atHome || cfg.live_outside === true);
  let noLive = $state(null);
  // Live only while someone looks: the page shown, and a touch, a click, a
  // key or a scroll in the last 2 minutes (a wall tablet left alone, a
  // forgotten tab, decode nothing). Back to images then; any gesture, or the
  // screen waking, plays it again.
  const LIVE_IDLE = 2 * 60_000;
  let pageShown = $state(typeof document === 'undefined' || document.visibilityState === 'visible');
  let idle = $state(false);
  let idleTimer;
  let lastGesture = 0;
  function gesture() {
    const now = Date.now();
    if (!idle && now - lastGesture < 2000) return; // pointer moves come by the dozen
    lastGesture = now;
    idle = false;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (idle = true), LIVE_IDLE);
  }
  // A tablet on its side (lib/layout.js): the door goes in the glance column.
  let panel = $state(false);
  onMount(() => {
    const q = window.matchMedia(PANEL);
    const change = () => (panel = q.matches);
    change();
    q.addEventListener('change', change);
    return () => q.removeEventListener('change', change);
  });
  onMount(() => {
    const events = ['pointerdown', 'pointermove', 'keydown', 'wheel', 'touchstart', 'scroll'];
    const onVisibility = () => {
      pageShown = document.visibilityState === 'visible';
      if (pageShown) gesture();
    };
    for (const e of events) window.addEventListener(e, gesture, { passive: true, capture: true });
    document.addEventListener('visibilitychange', onVisibility);
    gesture();
    return () => {
      clearTimeout(idleTimer);
      for (const e of events) window.removeEventListener(e, gesture, { capture: true });
      document.removeEventListener('visibilitychange', onVisibility);
    };
  });
  const liveShown = $derived(
    !!shown &&
      liveHere &&
      pageShown &&
      !idle &&
      !!hub.devices[shown.id]?.live &&
      !protectedRoom(roomOf(hub.devices[shown.id]) ?? '') &&
      noLive !== shown.id,
  );
  const step = (by) => (camIdx = (camIdx + by + cams.length) % cams.length);
  $effect(() => {
    if (camIdx === 0) return;
    const timer = setTimeout(() => (camIdx = 0), BACK_TO_DOOR);
    return () => clearTimeout(timer);
  });

  // The last real passage before the camera shown (the history knows; the
  // live value is refreshed every few seconds whatever happens). An answer
  // for a camera no longer shown is dropped.
  let passage = $state(null);
  const seen = (id) => lastSeen(id).then((ts) => shown?.id === id && (passage = ts));
  onMount(() => {
    const t = setInterval(() => shown && seen(shown.id), 120_000);
    return () => clearInterval(t);
  });
  $effect(() => {
    passage = null;
    if (shown) seen(shown.id);
  });
</script>

{#snippet doorbell()}
      <section class="card doorbell">
        <div class="card-head">
          {#if camIdx === 0}
            <h2><Icon name="bell" size={18} />{t('maison.accueil.devant_porte')}</h2>
          {:else}
            <h2><Icon name="cctv" size={18} />{shown.name}</h2>
          {/if}
          <a class="more" href="#/cameras" aria-label={t('maison.accueil.toutes_cameras')}><Icon name="arrow-top-right" size={18} /></a>
        </div>
        <div class="cam-carousel">
          <!-- One tile per camera: a load still running for the previous one
               must not hold back the new one. -->
          {#key shown.id}
            {#if liveShown}
              <div class="cam-live"><LiveVideo id={shown.id} onfail={() => (noLive = shown.id)} /></div>
            {:else}
              <CameraTile id={shown.id} name={shown.name} refresh={15} />
            {/if}
          {/key}
          {#if cams.length > 1}
            <button class="cam-step prev" onclick={() => step(-1)} aria-label={t('maison.accueil.camera_precedente', { nom: cams[(camIdx - 1 + cams.length) % cams.length].name })}>
              <Icon name="chevron-left" size={22} />
            </button>
            <button class="cam-step next" onclick={() => step(1)} aria-label={t('maison.accueil.camera_suivante', { nom: cams[(camIdx + 1) % cams.length].name })}>
              <Icon name="chevron-right" size={22} />
            </button>
            <span class="cam-dots" aria-hidden="true">
              {#each cams as c, i (c.id)}<span class:on={i === camIdx}></span>{/each}
            </span>
          {/if}
        </div>
        <p class="muted small">{passage ? t('maison.accueil.dernier_passage', { when: relative(passage, home.now) }) : t('maison.accueil.personne')}</p>
      </section>
{/snippet}

<div class="accueil">
  <!-- The glance: the hour, the weather, the mode, what to know. Above the
       cards, or beside them on a tablet on its side. -->
  <div class="glance">
  <SkyHero />

  {#if modes.length}
    <div class="modes" role="radiogroup" aria-label={t('maison.accueil.mode_maison')}>
      {#each modes as m (m)}
        <button role="radio" aria-checked={m === mode} class:on={m === mode} disabled={pending(modeDevice.id, 'state')} onclick={() => setMode(m)}>
          <Icon name={MODE_ICON[m] ?? 'home'} size={16} />{modeName(m)}
        </button>
      {/each}
      {#if talkers.length}
        <span class="sep" aria-hidden="true"></span>
        <button class="hush" class:on={quiet} aria-pressed={quiet} onclick={toggleQuiet} title={quiet ? t('maison.accueil.sonos_reparle') : t('maison.accueil.sonos_coupe')}>
          <Icon name={quiet ? 'volume-off' : 'speaker'} size={16} />{quiet && quietUntil ? t('maison.accueil.sonos_muette_jusqua', { time: new Date(quietUntil).toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' }) }) : t('maison.accueil.sonos_muette')}
        </button>
      {/if}
    </div>
  {/if}

  <section class="notices" aria-label={t('maison.accueil.a_savoir')}>
    {#each notices as n (n.text)}
      <a class="chip {n.tone}" href={n.href}><Icon name={n.icon} size={16} />{n.text}</a>
    {:else}
      <span class="chip good"><Icon name="leaf" size={16} />{t('maison.accueil.calme')}</span>
    {/each}
  </section>
  <!-- A wall panel: the door, always in view under the weather. -->
  {#if bell && panel}{@render doorbell()}{/if}
  </div>

  <div class="grid">
    {#if switches.length}
      <section class="card switches">
        <div class="card-head"><h2><Icon name="socket" size={18} />{t('maison.accueil.appareils')}</h2></div>
        <div class="tiles">
          {#each switches as s (s.id + (s.point ?? ''))}
            <LightTile id={s.id} name={s.name} icon={s.icon ?? 'socket'} point={s.point ?? null} />
          {/each}
        </div>
      </section>
    {/if}

    {#if lights.length || mood}
    <section class="card lights">
      <div class="card-head">
        <h2><Icon name="light" size={18} />{t('maison.accueil.lumieres')} <span class="count">{litCount ? t('maison.accueil.allumees', { count: litCount }) : t('maison.accueil.tout_eteint')}</span></h2>
        <a class="more" href="#/pieces" aria-label={t('maison.accueil.toutes_pieces')}><Icon name="arrow-top-right" size={18} /></a>
      </div>
      <div class="tiles">
        {#each lights as l (l.id)}
          <LightTile id={l.id} name={l.name} icon={l.icon} also={l.also ?? []} />
        {/each}
      </div>
      <ScenePicker />
      {#if mood}
        <div class="moods">
          <div class="moods-head">
            <span class="moods-title"><Icon name="palette" size={16} />{t('maison.accueil.ambiance')}</span>
            <div class="rooms" role="group" aria-label={t('maison.accueil.piece')}>
              {#each moodRooms as r (r.name)}
                <button class:on={r.name === mood.name} onclick={() => pickRoom(r.name)} aria-pressed={r.name === mood.name}>{r.name}</button>
              {/each}
            </div>
          </div>
          <AmbiancePicker lights={mood.lamps} label={mood.name} />
        </div>
      {/if}
    </section>
    {/if}

    <div class="side">
      <EnergyCard />
      <EnergyFlow home />
      {#each cfg.charts ?? [] as c, i (i)}
        <HistoryChart title={c.title} minutes={c.minutes ?? 60} points={c.points ?? []} />
      {/each}
      <Routines />
    </div>

    {#if (cfg.favorites?.climate ?? []).length}
      <section class="card climate">
        <div class="card-head"><h2><Icon name="air-conditioner" size={18} />{t('maison.accueil.clim')}</h2></div>
        <div class="clims">
          {#each cfg.favorites?.climate ?? [] as c (c.id)}
            {#if isValve(device(c.id))}<ValveCard id={c.id} name={c.name} />{:else}<ClimateCard id={c.id} name={c.name} />{/if}
          {/each}
        </div>
      </section>
    {/if}

    {#each robots as robot (robot.id)}
      <div class="robot-slot"><VacuumCard id={robot.id} /></div>
    {/each}

    {#if bell && !panel}{@render doorbell()}{/if}

    {#if cfg.salon?.tv}
    <a class="card remote" href="#/salon">
      <span class="tv-object" aria-hidden="true">
        <span class="screen" class:lit={tvOn}></span>
        <span class="stand"></span>
      </span>
      <span class="remote-text">
        <strong>{t('maison.accueil.telecommande')}</strong>
        <span class="muted">{tvOn ? t('maison.accueil.tele_allumee') : t('maison.accueil.tele_veille')}{playing ? ` · ${t('maison.accueil.son_joue')}` : ''}</span>
      </span>
      <span class="go"><Icon name="arrow-top-right" size={20} /></span>
    </a>
    {/if}

    <a class="card outside" href="#/dehors">
      <div class="card-head"><h2><Icon name="flower-outline" size={18} />{t('maison.accueil.dehors')}</h2><span class="more" aria-hidden="true"><Icon name="arrow-top-right" size={18} /></span></div>
      <div class="facts">
        {#if outTemp}
          <div><small>{t('maison.accueil.temperature')}</small><b class="num">{outTemp.value != null ? `${num(outTemp.value, 1)}°` : '—'}</b></div>
        {/if}
        {#if out.air}
          <div><small>{t('maison.accueil.air')}</small><b>{aqiWord}</b></div>
          <div><small>UV</small><b class="num">{num(value(out.air, 'uv_index'), 0)}</b></div>
        {/if}
        <div><small>{t('maison.accueil.humidite')}</small><b class="num">{value(out.weather, 'humidity') != null ? `${num(value(out.weather, 'humidity'))} %` : '—'}</b></div>
        {#if out.pool}
          <div><small>{t('maison.accueil.piscine')}</small><b class="num">{value(out.pool, 'temperature') != null ? `${num(value(out.pool, 'temperature'), 1)}°` : '—'}</b></div>
        {/if}
      </div>
    </a>
  </div>
</div>

<style>
  .accueil {
    display: grid;
    gap: 22px;
  }

  .glance {
    display: contents;
  }

  .modes {
    display: flex;
    gap: 8px;
    overflow-x: auto;
    scrollbar-width: none;
    min-width: 0;
  }

  .modes button {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 0;
    border-radius: 999px;
    padding: 8px 14px;
    font: inherit;
    font-size: 14px;
    font-weight: 650;
    background: var(--surface-2);
    color: var(--ink-3);
    cursor: pointer;
  }

  .modes button.on {
    background: var(--warm-soft);
    color: var(--warm-ink);
  }

  .modes .sep {
    flex: none;
    width: 1px;
    margin: 6px 4px;
    background: var(--line, color-mix(in srgb, var(--ink) 14%, transparent));
  }

  .modes button.hush.on {
    background: color-mix(in srgb, var(--alert) 16%, var(--surface-2));
    color: var(--alert);
  }

  .notices {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }

  .notices a {
    text-decoration: none;
  }

  .notices .chip {
    padding: 9px 14px;
    font-size: 14px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: 20px;
    align-items: start;
  }

  .lights,
  .switches {
    grid-column: span 8;
  }

  .switches .tiles {
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  }

  .side {
    grid-column: span 4;
  }

  .robot-slot {
    grid-column: span 5;
    display: grid;
  }

  .climate {
    grid-column: span 7;
  }

  .doorbell {
    grid-column: span 5;
    grid-row: span 2;
  }

  .cam-live {
    border-radius: var(--r-md, 14px);
    overflow: hidden;
    aspect-ratio: 16 / 9;
    background: #000;
  }

  .cam-carousel {
    position: relative;
  }

  .cam-step {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: 0;
    border-radius: 50%;
    background: rgb(0 0 0 / 0.45);
    color: #fff;
    cursor: pointer;
  }

  .cam-step:hover,
  .cam-step:focus-visible {
    background: rgb(0 0 0 / 0.65);
  }

  .cam-step.prev {
    left: 10px;
  }

  .cam-step.next {
    right: 10px;
  }

  .cam-dots {
    position: absolute;
    bottom: 14px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    gap: 6px;
    pointer-events: none;
  }

  .cam-dots span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: rgb(255 255 255 / 0.5);
  }

  .cam-dots span.on {
    background: #fff;
  }

  .remote {
    grid-column: span 3;
  }

  .outside {
    grid-column: span 4;
  }

  .moods {
    grid-template-columns: minmax(0, 1fr);
    margin-top: 18px;
    padding-top: 16px;
    border-top: 1px solid var(--line);
    display: grid;
    gap: 12px;
  }

  .moods-head {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px 14px;
    flex-wrap: wrap;
  }

  .moods-title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 700;
    font-size: 14px;
  }

  .rooms {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: none;
    min-width: 0;
    flex: 1;
  }

  .rooms button {
    flex: none;
    border: 0;
    border-radius: 999px;
    padding: 6px 12px;
    font: inherit;
    font-size: 13px;
    font-weight: 650;
    background: var(--surface-2);
    color: var(--ink-3);
    cursor: pointer;
  }

  .rooms button.on {
    background: var(--warm-soft);
    color: var(--warm-ink);
  }

  .count {
    font-weight: 550;
    color: var(--ink-3);
    margin-left: 4px;
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
  }

  .clims {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .small {
    font-size: 13px;
    margin-top: 10px;
  }

  .remote {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 14px;
    color: inherit;
    text-decoration: none;
    min-height: 168px;
    align-content: space-between;
    transition: transform 0.2s var(--ease);
  }

  .remote:hover,
  .outside:hover {
    transform: translateY(-2px);
  }

  .tv-object {
    grid-column: 1 / -1;
    display: grid;
    justify-items: center;
  }

  .screen {
    width: 96px;
    height: 56px;
    border-radius: 8px;
    background: #171c28;
    box-shadow: inset 0 0 0 3px #2a3142;
    transition: background 0.4s;
  }

  .screen.lit {
    background: linear-gradient(135deg, #5fb4f0, #8f8cf5 55%, #f0a53a);
    box-shadow: inset 0 0 0 3px #2a3142, 0 8px 26px -6px #8f8cf5aa;
  }

  .stand {
    width: 30px;
    height: 6px;
    border-radius: 0 0 6px 6px;
    background: #2a3142;
  }

  .remote-text {
    display: grid;
  }

  .remote-text strong {
    font-size: 18px;
  }

  .remote-text span {
    font-size: 13px;
  }

  .go {
    align-self: end;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--surface-2);
    display: grid;
    place-items: center;
    color: var(--ink-2);
  }

  .outside {
    color: inherit;
    text-decoration: none;
    transition: transform 0.2s var(--ease);
  }

  .facts {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(0, 1fr));
    gap: 8px;
  }

  .facts small {
    display: block;
    font-size: 11px;
    color: var(--ink-3);
    font-weight: 650;
  }

  .facts b {
    font-size: 17px;
    font-weight: 650;
  }

  /* Without the air-conditioning card the twelve-column grid no longer tiles:
     a row is as tall as its tallest card (the energy side, the doorbell over
     two rows) and leaves empty space under the short ones. On a wide screen
     the cards then stack in columns, which leaves no hole whichever cards
     the house has. */
  @media (min-width: 1181px) {
    .grid:not(:has(.climate)) {
      display: block;
      columns: 420px 3;
      column-gap: 20px;
    }

    .grid:not(:has(.climate)) > * {
      width: 100%;
      margin: 0 0 20px;
      break-inside: avoid;
    }

    /* Out of a grid, a card that is a link turns inline again and loses its
       background. Block it, with no specificity, so a card that declares its
       own display (the remote) keeps it. */
    :where(.grid:not(:has(.climate))) > :where(a) {
      display: block;
    }
  }

  @media (max-width: 1180px) {
    .lights,
    .switches,
    .climate {
      grid-column: span 12;
    }

    .side,
    .doorbell {
      grid-column: span 6;
      grid-row: auto;
    }

    .remote,
    .outside {
      grid-column: span 6;
    }
  }

  @media (max-width: 760px) {
    .grid > * {
      grid-column: span 12 !important;
    }

    .tiles {
      grid-template-columns: 1fr 1fr;
    }

    .clims {
      grid-template-columns: 1fr;
    }
  }
  /* A tablet on its side (a wall panel: about 1280 × 800): the glance becomes
     a column that stays in view, the cards go two by two beside it. Same
     query as PANEL in lib/layout.js. */
  @media (min-width: 761px) and (max-height: 960px) and (orientation: landscape) {
    .accueil {
      grid-template-columns: minmax(250px, 290px) minmax(0, 1fr);
      align-items: start;
      gap: 20px 24px;
    }

    .glance {
      display: grid;
      grid-template-columns: minmax(0, 1fr);
      align-content: start;
      gap: 16px;
      position: sticky;
      top: 12px;
      max-height: calc(100dvh - 24px);
      overflow-y: auto;
      scrollbar-width: none;
    }

    .modes {
      flex-wrap: wrap;
    }

    .glance .doorbell {
      grid-column: auto;
      grid-row: auto;
    }

    .accueil .grid {
      display: block;
      columns: 300px 2;
      column-gap: 18px;
    }

    .accueil .grid > * {
      width: 100%;
      margin: 0 0 18px;
      break-inside: avoid;
    }

    :where(.accueil .grid) > :where(a) {
      display: block;
    }

    /* The energy column, one block, would leave the other column half
       empty: its cards share the two columns like the others. */
    .accueil .side {
      display: contents;
    }

    .accueil .side > :global(*) {
      margin: 0 0 18px;
      break-inside: avoid;
    }
  }
</style>
