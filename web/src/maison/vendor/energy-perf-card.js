/*
 * Carte « Performance énergie » — v1.4.0
 *
 * Compare la production solaire et les achats EDF mois par mois et année par
 * année, à partir des statistiques long terme du recorder. Les sources sont lues
 * dans la configuration du tableau Énergie (réseau, solaire, batterie) : rien à
 * déclarer à la main, une source ajoutée là-bas apparaît ici.
 *
 * Vues (clé `view`) :
 *   month   — mois en cours à date, comparé à la même période de l'an dernier
 *   year    — depuis le 1er janvier à date, idem
 *   monthly — histogramme mois par mois sur plusieurs années + tableau des écarts
 *   yearly  — totaux et ratios par année
 *   cumulative — cumul depuis le 1er janvier, une courbe par année, avec l'écart
 *                à la même date, l'avance ou le retard en jours et une projection
 *   payback — rentabilité : économies cumulées face aux achats déclarés dans
 *             `investments`, avec la date de retour sur investissement estimée
 *
 * Options :
 *   title        titre de la carte (défaut selon la vue)
 *   years        nombre d'années comparées dans la vue monthly (défaut 3)
 *   metric       indicateur affiché au départ dans les vues monthly et cumulative
 *                (solar, import, export, conso, autonomie, savings)
 *   price        prix du kWh acheté en €/kWh (défaut : prix fixe du tableau Énergie)
 *   solar_names  libellés des sources solaires { statistic_id: "Libellé" }
 *   ignore       statistiques du tableau Énergie à ne pas compter, par exemple un
 *                second compteur réseau qui mesure la même chose (Linky + Victron) :
 *                [ "linky:14284080922909" ]. Vaut pour toutes les cartes de la page.
 *   investments  (vue payback) achats de l'installation, en euros :
 *                [{ date: "2024-03-26", amount: 2136.72, label: "Panneaux" }, …]
 *
 * « À date » : la période de l'an dernier s'arrête au même jour et à la même
 * heure qu'aujourd'hui, pour comparer ce qui est comparable.
 *
 * Jours et mois suivent le fuseau du serveur HA, quel que soit celui de
 * l'appareil qui affiche la carte.
 */

const CARD_VERSION = "1.4.0";
const CARD_TAG = "energy-perf-card";

const VIEWS = ["month", "year", "monthly", "yearly", "cumulative", "payback"];
const CACHE_MS = 10 * 60 * 1000;

const MONTHS = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
const MONTHS_SHORT = ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."];

// better : sens dans lequel l'écart est une bonne nouvelle (null = neutre).
const METRICS = {
  solar: { label: "Production solaire", tab: "Production", unit: "kWh", better: "up" },
  import: { label: "Achat EDF", tab: "Achat EDF", unit: "kWh", better: "down" },
  export: { label: "Injection réseau", tab: "Injection", unit: "kWh", better: null },
  conso: { label: "Consommation maison", tab: "Conso maison", unit: "kWh", better: null },
  autonomie: {
    label: "Autonomie", tab: "Autonomie", unit: "%", better: "up",
    hint: "Part de la consommation de la maison qui n'a pas été achetée à EDF",
  },
  autoconso: {
    label: "Autoconsommation", tab: "Autoconso", unit: "%", better: "up",
    hint: "Part de la production solaire consommée sur place (non injectée)",
  },
  savings: {
    label: "Économies solaires", tab: "Économies", unit: "€", better: "up",
    hint: "Énergie consommée sans l'acheter à EDF, valorisée au prix du kWh",
  },
};

// Rampe bleue à une teinte, de l'année la plus ancienne à l'année en cours
// (validée : monotone, écarts de luminance visibles, contraste ≥ 2:1 sur la carte).
const YEAR_RAMP = {
  light: ["#86b6ef", "#3987e5", "#184f95"],
  dark: ["#184f95", "#3987e5", "#9ec5f4"],
};

const nf0 = new Intl.NumberFormat("fr-FR", { maximumFractionDigits: 0 });
const nf1 = new Intl.NumberFormat("fr-FR", { minimumFractionDigits: 1, maximumFractionDigits: 1 });
const nf2 = new Intl.NumberFormat("fr-FR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

const esc = (s) =>
  String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

const num = (v) => (Math.abs(v) < 10 && v !== 0 ? nf1.format(v) : nf0.format(v));

function fmtValue(v, unit, withUnit = true) {
  if (v == null || !Number.isFinite(v)) return "—";
  const n = unit === "kWh" ? num(v) : nf0.format(v);
  if (!withUnit) return n;
  return unit === "%" ? `${n} %` : `${n} ${unit}`;
}

const monthKey = (y, m) => `${y}-${String(m + 1).padStart(2, "0")}`;

const DAY_MS = 86400000;
const daysInYear = (y) => Math.round((new Date(y + 1, 0, 1) - new Date(y, 0, 1)) / DAY_MS);
const dayOfYear = (d) =>
  Math.round((new Date(d.getFullYear(), d.getMonth(), d.getDate()) - new Date(d.getFullYear(), 0, 1)) / DAY_MS);

// Le recorder découpe jours et mois dans le fuseau du serveur HA. Les dates de
// la carte (fin de période, débuts de mois…) sont des dates « murales » : des
// Date locales dont les champs portent l'heure du serveur, converties en
// instants seulement pour interroger le recorder. Sans ça, un appareil réglé
// sur un autre fuseau (tablette en UTC) décale chaque mois d'un cran.
const tzFormats = new Map();
function wallParts(date, tz) {
  let f = tzFormats.get(tz);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone: tz, hourCycle: "h23",
      year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric",
    });
    tzFormats.set(tz, f);
  }
  const p = {};
  for (const { type, value } of f.formatToParts(date)) p[type] = +value;
  p.hour %= 24; // anciens moteurs : minuit = « 24 »
  return p;
}

// Instant -> date murale du fuseau tz.
function toWall(date, tz) {
  const p = wallParts(date, tz);
  return new Date(p.year, p.month - 1, p.day, p.hour, p.minute);
}

// Date murale du fuseau tz -> instant.
function fromWall(wall, tz) {
  const utc = Date.UTC(wall.getFullYear(), wall.getMonth(), wall.getDate(), wall.getHours(), wall.getMinutes());
  const offset = (t) => {
    const p = wallParts(new Date(t), tz);
    return Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute) - t;
  };
  return new Date(utc - offset(utc - offset(utc)));
}

const signed = (v, unit) => `${v > 0 ? "+" : v < 0 ? "−" : ""}${fmtValue(Math.abs(v), unit, false)}`;
const unitSuffix = (unit) => (unit === "%" ? " %" : ` ${unit}`);

// Date d'achat : « 2024-03-26 », « 26/03/2024 » ou objet Date (YAML).
function parseDay(v) {
  if (v instanceof Date) return new Date(v.getUTCFullYear(), v.getUTCMonth(), v.getUTCDate());
  const s = String(v ?? "").trim();
  let m = s.match(/^(\d{4})-(\d{1,2})-(\d{1,2})/);
  if (m) return new Date(+m[1], +m[2] - 1, +m[3]);
  m = s.match(/^(\d{1,2})\/(\d{1,2})\/(\d{4})$/);
  if (m) return new Date(+m[3], +m[2] - 1, +m[1]);
  return null;
}

// Montant : nombre, ou texte à la française (« 2 136,72 € »).
function parseAmount(v) {
  if (typeof v === "number") return Number.isFinite(v) ? v : null;
  const n = parseFloat(String(v ?? "").replace(/[\s€]/g, "").replace(",", "."));
  return Number.isFinite(n) ? n : null;
}

function parseInvestments(list) {
  if (!Array.isArray(list) || !list.length) {
    throw new Error("Vue « payback » : liste « investments » attendue (date, amount, label).");
  }
  return list
    .map((it, i) => {
      const date = parseDay(it?.date);
      const amount = parseAmount(it?.amount);
      if (!date || amount == null) {
        throw new Error(`Investissement n° ${i + 1} : « date » (AAAA-MM-JJ) et « amount » (en €) attendus.`);
      }
      return { date, amount, label: it.label ? String(it.label) : "" };
    })
    .sort((a, b) => a.date - b.date);
}

const fmtDay = (d) => `${d.getDate()} ${MONTHS_SHORT[d.getMonth()]} ${d.getFullYear()}`;

function tileHtml({ label, value, unit = "", delta = "", extra = "", hint = "", text = false }) {
  return `<div class="tile">
    <div class="t-label"${hint ? ` title="${esc(hint)}"` : ""}>${esc(label)}</div>
    <div class="t-value${text ? " txt" : ""}">${value}<span class="u">${unit}</span></div>
    <div class="t-delta">${delta}</div>${extra ? `<div class="t-extra">${extra}</div>` : ""}
  </div>`;
}

// Même instant un an plus tôt ; le 29 février retombe sur le 28.
function minusYear(d) {
  const y = d.getFullYear() - 1;
  const last = new Date(y, d.getMonth() + 1, 0).getDate();
  return new Date(y, d.getMonth(), Math.min(d.getDate(), last), d.getHours(), d.getMinutes());
}

// ---------------------------------------------------------------------------
// Données
// ---------------------------------------------------------------------------

// Statistiques exclues (option `ignore`), communes à toutes les cartes de la page
// puisque les données sont chargées une seule fois pour toutes.
const IGNORED = new Set();

function readSources(prefs) {
  const s = { solar: [], import: [], export: [], batIn: [], batOut: [], price: null };
  for (const e of prefs?.energy_sources || []) {
    if (e.type === "grid") {
      if (e.stat_energy_from) s.import.push(e.stat_energy_from);
      if (e.stat_energy_to) s.export.push(e.stat_energy_to);
      if (s.price == null && e.number_energy_price != null) s.price = e.number_energy_price;
      // Ancien format : listes de flux par source réseau.
      for (const f of e.flow_from || []) {
        s.import.push(f.stat_energy_from);
        if (s.price == null && f.number_energy_price != null) s.price = f.number_energy_price;
      }
      for (const f of e.flow_to || []) s.export.push(f.stat_energy_to);
    } else if (e.type === "solar" && e.stat_energy_from) {
      s.solar.push(e.stat_energy_from);
    } else if (e.type === "battery") {
      if (e.stat_energy_from) s.batOut.push(e.stat_energy_from);
      if (e.stat_energy_to) s.batIn.push(e.stat_energy_to);
    }
  }
  for (const role of ["solar", "import", "export", "batIn", "batOut"]) s[role] = s[role].filter((id) => !IGNORED.has(id));
  return s;
}

const ROLES = ["solar", "import", "export", "batIn", "batOut"];

const emptyTotals = () => ({ solar: 0, import: 0, export: 0, batIn: 0, batOut: 0, bySource: {}, hasData: false });

function addChange(totals, src, id, change) {
  if (change == null || !Number.isFinite(change)) return;
  for (const role of ROLES) {
    if (!src[role].includes(id)) continue;
    totals[role] += change;
    if (role === "solar") totals.bySource[id] = (totals.bySource[id] || 0) + change;
    if (role === "solar" || role === "import") totals.hasData = true;
  }
}

async function periodTotals(hass, src, ids, tz, start, end) {
  const totals = emptyTotals();
  if (end <= start) return totals;
  const results = await Promise.all(
    ids.map((id) =>
      hass.callWS({
        type: "recorder/statistic_during_period",
        statistic_id: id,
        fixed_period: { start_time: fromWall(start, tz).toISOString(), end_time: fromWall(end, tz).toISOString() },
        types: ["change"],
        units: { energy: "kWh" },
      })
    )
  );
  ids.forEach((id, i) => addChange(totals, src, id, results[i]?.change));
  return totals;
}

async function fetchData(hass) {
  const prefs = await hass.callWS({ type: "energy/get_prefs" });
  const src = readSources(prefs);
  const ids = [...new Set(ROLES.flatMap((r) => src[r]))];
  if (!src.solar.length && !src.import.length) {
    throw new Error("Aucune source réseau ni solaire dans la configuration du tableau Énergie.");
  }

  const tz = hass.config?.time_zone || Intl.DateTimeFormat().resolvedOptions().timeZone;

  // Bornes « à date » arrondies à l'heure : au-delà de 10 jours, le recorder ne
  // garde que des statistiques horaires, l'an dernier ne peut pas être plus fin.
  const end = toWall(new Date(), tz);
  end.setMinutes(0, 0, 0);
  const y = end.getFullYear();
  const monthStart = new Date(y, end.getMonth(), 1);
  const yearStart = new Date(y, 0, 1);

  const [stats, monthCur, monthPrev, ytdCur, ytdPrev] = await Promise.all([
    hass.callWS({
      type: "recorder/statistics_during_period",
      start_time: fromWall(new Date(y - 20, 0, 1), tz).toISOString(),
      statistic_ids: ids,
      period: "month",
      types: ["change"],
      units: { energy: "kWh" },
    }),
    periodTotals(hass, src, ids, tz, monthStart, end),
    periodTotals(hass, src, ids, tz, minusYear(monthStart), minusYear(end)),
    periodTotals(hass, src, ids, tz, yearStart, end),
    periodTotals(hass, src, ids, tz, minusYear(yearStart), minusYear(end)),
  ]);

  const months = new Map();
  for (const id of ids) {
    for (const row of stats[id] || []) {
      const d = toWall(new Date(row.start), tz);
      const key = monthKey(d.getFullYear(), d.getMonth());
      if (!months.has(key)) months.set(key, emptyTotals());
      addChange(months.get(key), src, id, row.change);
    }
  }

  let first = null;
  for (const [key, t] of months) if (t.hasData && (!first || key < first)) first = key;

  return {
    src,
    tz,
    end,
    months,
    first: first ? { year: +first.slice(0, 4), month: +first.slice(5) - 1 } : null,
    toDate: { month: { cur: monthCur, prev: monthPrev }, ytd: { cur: ytdCur, prev: ytdPrev } },
  };
}

// Dernières données reçues, recopiées dans le navigateur : une carte recréée
// ou une page rechargée les affiche aussitôt, puis se met à jour.
const STORE_KEY = "energy-perf-card:last";
let last = restoreLast();

function restoreLast() {
  try {
    const s = JSON.parse(localStorage.getItem(STORE_KEY));
    if (!s?.data?.months) return null;
    return {
      data: { ...s.data, end: new Date(s.data.end), months: new Map(s.data.months) },
      daily: s.daily ? new Map(s.daily) : null,
    };
  } catch {
    return null;
  }
}

function saveLast() {
  try {
    const { data, daily } = last;
    localStorage.setItem(
      STORE_KEY,
      JSON.stringify({
        data: { ...data, end: data.end.getTime(), months: [...data.months] },
        daily: daily ? [...daily] : null,
      })
    );
  } catch {
    // Stockage plein ou interdit : on se passe du cache.
  }
}

// Un seul chargement partagé par toutes les cartes de la page.
let shared = null;
function getData(hass) {
  if (!shared || Date.now() - shared.at > CACHE_MS) {
    const entry = { at: Date.now() };
    entry.promise = fetchData(hass).then(
      (data) => {
        last = { data, daily: last?.daily ?? null };
        saveLast();
        return data;
      },
      (e) => {
        if (shared === entry) shared = null;
        throw e;
      }
    );
    shared = entry;
  }
  return shared.promise;
}

// Totaux jour par jour des dernières années, pour les courbes cumulées.
async function fetchDaily(hass, data) {
  const { src, tz, end } = data;
  const ids = [...new Set(ROLES.flatMap((r) => src[r]))];
  const stats = await hass.callWS({
    type: "recorder/statistics_during_period",
    start_time: fromWall(new Date(end.getFullYear() - YEAR_RAMP.light.length + 1, 0, 1), tz).toISOString(),
    statistic_ids: ids,
    period: "day",
    types: ["change"],
    units: { energy: "kWh" },
  });
  const days = new Map(); // année -> totaux indexés par jour de l'année
  for (const id of ids) {
    for (const row of stats[id] || []) {
      const d = toWall(new Date(row.start), tz);
      if (!days.has(d.getFullYear())) days.set(d.getFullYear(), []);
      const arr = days.get(d.getFullYear());
      const i = dayOfYear(d);
      arr[i] ??= emptyTotals();
      addChange(arr[i], src, id, row.change);
    }
  }
  return days;
}

// Rechargé en même temps que les données mensuelles dont il dépend.
let sharedDaily = null;
function getDaily(hass, data) {
  if (!sharedDaily || sharedDaily.data !== data) {
    const entry = { data };
    entry.promise = fetchDaily(hass, data).then(
      (daily) => {
        if (last?.data === data) {
          last.daily = daily;
          saveLast();
        }
        return daily;
      },
      (e) => {
        if (sharedDaily === entry) sharedDaily = null;
        throw e;
      }
    );
    sharedDaily = entry;
  }
  return sharedDaily.promise;
}

// Valeur d'une journée pour un indicateur additif (kWh ou €).
function dailyValue(t, key, price) {
  if (!t || !t.hasData) return null;
  const conso = t.import + t.solar + t.batOut - t.export - t.batIn;
  if (key === "conso") return conso;
  if (key === "savings") return price == null ? null : (conso - t.import) * price;
  return t[key];
}

// Cumul jour par jour depuis le 1er janvier ; null après le dernier jour connu.
function cumulate(days, n, key, price) {
  const out = new Array(n).fill(null);
  if (!days) return out;
  let last = -1;
  for (let i = 0; i < n; i++) if (dailyValue(days[i], key, price) != null) last = i;
  let sum = 0;
  for (let i = 0; i <= last; i++) {
    sum += dailyValue(days[i], key, price) ?? 0;
    out[i] = sum;
  }
  return out;
}

function derive(t, price) {
  if (!t || !t.hasData) return null;
  const conso = t.import + t.solar + t.batOut - t.export - t.batIn;
  const local = conso - t.import;
  return {
    solar: t.solar,
    import: t.import,
    export: t.export,
    conso,
    autonomie: conso > 0 ? Math.min(Math.max(local / conso, 0), 1) * 100 : null,
    autoconso: t.solar > 0 ? Math.min(Math.max((t.solar - t.export) / t.solar, 0), 1) * 100 : null,
    savings: price != null ? Math.max(local, 0) * price : null,
    cost: price != null ? t.import * price : null,
    bySource: t.bySource,
  };
}

function sumTotals(list) {
  const out = emptyTotals();
  for (const t of list) {
    if (!t) continue;
    for (const r of ROLES) out[r] += t[r];
    for (const [id, v] of Object.entries(t.bySource)) out.bySource[id] = (out.bySource[id] || 0) + v;
    out.hasData = out.hasData || t.hasData;
  }
  return out;
}

// Écart entre deux valeurs d'un indicateur : % pour les kWh, points pour les
// ratios, euros pour les montants. tone = good | bad | neutral.
function delta(metricKey, cur, prev) {
  if (cur == null || prev == null) return null;
  const m = METRICS[metricKey];
  let value;
  let text;
  if (m.unit === "%") {
    value = cur - prev;
    const a = Math.abs(value);
    text = `${nf0.format(a)} pt${Math.round(a) >= 2 ? "s" : ""}`;
  } else if (m.unit === "€") {
    value = cur - prev;
    text = `${nf0.format(Math.abs(value))} €`;
  } else {
    if (prev <= 0) return null;
    value = (cur / prev - 1) * 100;
    text = `${nf0.format(Math.abs(value))} %`;
  }
  const flat = Math.abs(value) < 0.5;
  const up = value > 0;
  let tone = "neutral";
  if (!flat && m.better) tone = (m.better === "up") === up ? "good" : "bad";
  const arrow = flat ? "=" : up ? "▲" : "▼";
  const sign = flat ? "" : up ? "+" : "−";
  return { value, tone, html: `<span class="d ${tone}">${arrow} ${sign}${text}</span>` };
}

const niceStep = (raw) => {
  const p = 10 ** Math.floor(Math.log10(raw));
  const f = raw / p;
  return (f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10) * p;
};

function barPath(x, y, w, h) {
  if (h <= 0.5) return "";
  const r = Math.min(4, w / 2, h);
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
}

// ---------------------------------------------------------------------------
// Carte
// ---------------------------------------------------------------------------

const STYLE = `
  :host {
    --epc-good: #006300;
    --epc-bad: #c03030;
    --epc-hover: rgba(0, 0, 0, 0.05);
    --epc-save: #2a78d6;
    --epc-track: #cde2fb;
    --epc-cost: #898781;
    display: block;
  }
  :host([dark]) {
    --epc-good: #2fbf2f;
    --epc-bad: #ec7a7a;
    --epc-hover: rgba(255, 255, 255, 0.06);
    --epc-save: #3987e5;
    --epc-track: rgba(57, 135, 229, 0.25);
  }
  ha-card { padding: 16px; overflow: hidden; }
  .head { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 4px 12px; margin-bottom: 12px; }
  .title { font-size: 18px; font-weight: 500; color: var(--primary-text-color); }
  .sub { font-size: 13px; color: var(--secondary-text-color); }
  .msg { color: var(--secondary-text-color); font-size: 14px; padding: 8px 0; }
  .d { font-weight: 600; white-space: nowrap; font-variant-numeric: tabular-nums; }
  .d.good { color: var(--epc-good); }
  .d.bad { color: var(--epc-bad); }
  .d.neutral { color: var(--secondary-text-color); }
  .muted { color: var(--secondary-text-color); }

  /* Tuiles */
  .tiles { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 8px; }
  .tile { border: 1px solid var(--divider-color); border-radius: 12px; padding: 10px 12px; min-width: 0; }
  .t-label { font-size: 13px; color: var(--secondary-text-color); }
  .t-value { font-size: 26px; font-weight: 600; color: var(--primary-text-color); line-height: 1.25; margin-top: 2px; white-space: nowrap; }
  .t-value.txt { font-size: 18px; line-height: 1.8; }
  .t-value .u { font-size: 15px; font-weight: 500; color: var(--secondary-text-color); }
  .t-delta { font-size: 13px; margin-top: 2px; display: flex; flex-wrap: wrap; gap: 0 6px; }
  .t-extra { font-size: 12px; color: var(--secondary-text-color); margin-top: 2px; }
  [title] { cursor: help; }

  /* Onglets */
  .tabs { display: flex; flex-wrap: wrap; gap: 4px; }
  .tabs button {
    font: inherit; font-size: 13px; padding: 4px 10px; border-radius: 16px; cursor: pointer;
    border: 1px solid var(--divider-color); background: none; color: var(--secondary-text-color);
  }
  .tabs button[aria-pressed="true"] {
    border-color: var(--primary-color); color: var(--primary-text-color);
    background: color-mix(in srgb, var(--primary-color) 18%, transparent);
  }

  /* Légende + graphique */
  .legend { display: flex; flex-wrap: wrap; gap: 4px 14px; font-size: 13px; color: var(--secondary-text-color); margin: 4px 0 6px; }
  .legend span { display: inline-flex; align-items: center; gap: 6px; }
  .sw { width: 12px; height: 12px; border-radius: 3px; display: inline-block; }
  .sw.mark { height: 2px; border-radius: 1px; background: var(--primary-text-color); }
  .chart { position: relative; }
  .chart svg { display: block; overflow: visible; }
  .chart text { font-size: 11px; fill: var(--secondary-text-color); font-variant-numeric: tabular-nums; }
  .chart text.cur { fill: var(--primary-text-color); font-weight: 600; }
  .grid { stroke: var(--divider-color); stroke-width: 1; shape-rendering: crispEdges; }
  .hit { fill: transparent; cursor: pointer; }
  .hit.on { fill: var(--epc-hover); }
  .marker { stroke: var(--primary-text-color); stroke-width: 2; stroke-linecap: round; }
  .line { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
  .dot { stroke: var(--ha-card-background, var(--card-background-color, #fff)); stroke-width: 2; }
  .xhair { stroke: var(--secondary-text-color); stroke-width: 1; shape-rendering: crispEdges; }
  .ov { fill: transparent; cursor: crosshair; }
  .chart text.endlbl { fill: var(--primary-text-color); font-size: 12px; font-weight: 600; }
  .cum-tiles { margin-bottom: 12px; }
  .meter { height: 6px; border-radius: 3px; background: var(--epc-track); margin-top: 6px; overflow: hidden; }
  .meter i { display: block; height: 100%; border-radius: 3px; background: var(--epc-save); }
  .pb-save { stroke: var(--epc-save); }
  .pb-proj { stroke: var(--epc-save); stroke-dasharray: 5 4; }
  .pb-area { fill: var(--epc-save); opacity: 0.12; }
  .pb-cost { stroke: var(--epc-cost); }
  .pb-dot-save { fill: var(--epc-save); }
  .pb-dot-cost { fill: var(--epc-cost); }
  .sw.save { background: var(--epc-save); }
  .sw.cost { background: var(--epc-cost); }
  .sw.dash { height: 0; border-radius: 0; background: none; border-top: 2px dashed var(--epc-save); }
  .yr { stroke: var(--divider-color); stroke-width: 1; shape-rendering: crispEdges; }
  details.inv { margin-top: 10px; font-size: 13px; color: var(--secondary-text-color); }
  details.inv summary { cursor: pointer; }
  details.inv table { margin-top: 6px; }
  .tip {
    position: absolute; pointer-events: none; z-index: 2; min-width: 170px; max-width: 260px;
    background: var(--card-background-color, var(--ha-card-background, #fff)); color: var(--primary-text-color);
    border: 1px solid var(--divider-color); border-radius: 10px; padding: 8px 10px; font-size: 13px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25);
  }
  .tip .h { font-weight: 600; margin-bottom: 4px; }
  .tip .r { display: grid; grid-template-columns: 12px auto 1fr; gap: 6px; align-items: center; }
  .tip .r b { font-weight: 500; text-align: right; font-variant-numeric: tabular-nums; }
  .tip .s { font-size: 12px; color: var(--secondary-text-color); margin: 0 0 3px 18px; }
  .tip .f { border-top: 1px solid var(--divider-color); margin-top: 6px; padding-top: 4px; }

  /* Tableaux */
  .scroll { overflow-x: auto; max-width: 100%; margin-top: 12px; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; font-variant-numeric: tabular-nums; }
  th { font-weight: 500; color: var(--secondary-text-color); text-align: right; padding: 6px; border-bottom: 1px solid var(--divider-color); white-space: nowrap; }
  td { text-align: right; padding: 5px 6px; border-bottom: 1px solid var(--divider-color); color: var(--primary-text-color); white-space: nowrap; }
  th:first-child, td:first-child { text-align: left; padding-left: 0; }
  tr:last-child td { border-bottom: none; }
  tr.now td { background: var(--epc-hover); }
  tr.total td { font-weight: 600; border-top: 1px solid var(--divider-color); }
  tr.ref td { color: var(--secondary-text-color); font-size: 12px; }
  td .note { font-weight: 400; font-size: 11px; color: var(--secondary-text-color); }
  td .d { font-size: 12px; }
  .cell-sub { display: block; font-size: 12px; }
  .foot { font-size: 12px; color: var(--secondary-text-color); margin-top: 10px; }
`;

class EnergyPerfCard extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this._data = last?.data ?? null;
    this._daily = last?.daily ?? null;
    this._error = null;
    this._loading = false;
    this._loadedAt = 0;
    this._width = 0;
    this._dark = null;
  }

  static getStubConfig() {
    return { view: "monthly" };
  }

  setConfig(config) {
    if (!config || !VIEWS.includes(config.view)) {
      throw new Error(`Option « view » attendue : ${VIEWS.join(", ")}`);
    }
    if (config.metric && !METRICS[config.metric]) {
      throw new Error(`Option « metric » inconnue : ${config.metric}`);
    }
    if (config.ignore != null && !Array.isArray(config.ignore)) {
      throw new Error("Option « ignore » : liste d'identifiants de statistiques attendue.");
    }
    for (const id of config.ignore || []) {
      if (!IGNORED.has(id)) {
        IGNORED.add(id);
        shared = null; // données déjà chargées sans cette exclusion : à recharger
      }
    }
    this._invest = config.view === "payback" ? parseInvestments(config.investments) : [];
    this._config = { years: 3, metric: "solar", ...config };
    this._metric = this._config.metric;
    this._render();
  }

  set hass(hass) {
    this._hass = hass;
    const dark = !!hass.themes?.darkMode;
    if (dark !== this._dark) {
      this._dark = dark;
      this.toggleAttribute("dark", dark);
      this._render();
    }
    if (!this._loading && Date.now() - this._loadedAt > CACHE_MS) this._load();
  }

  connectedCallback() {
    if (["monthly", "cumulative", "payback"].includes(this._config?.view) && !this._ro) {
      this._ro = new ResizeObserver((entries) => {
        const w = Math.floor(entries[0].contentRect.width);
        if (w && w !== this._width) {
          this._width = w;
          this._renderChart();
        }
      });
    }
    this._observe();
  }

  disconnectedCallback() {
    this._ro?.disconnect();
  }

  getCardSize() {
    return { month: 4, year: 4, monthly: 12, yearly: 5, cumulative: 9, payback: 9 }[this._config?.view] || 4;
  }

  getGridOptions() {
    return { columns: "full", rows: "auto" };
  }

  async _load() {
    this._loading = true;
    this._loadedAt = Date.now();
    try {
      this._data = await getData(this._hass);
      if (this._config.view === "cumulative") this._daily = await getDaily(this._hass, this._data);
      this._error = null;
    } catch (e) {
      this._error = e?.message || String(e);
    }
    this._loading = false;
    this._render();
  }

  get _price() {
    const p = this._config.price ?? this._data?.src.price;
    return p == null ? null : Number(p);
  }

  _sourceName(id) {
    return this._config.solar_names?.[id] || this._hass?.states[id]?.attributes.friendly_name || id;
  }

  _observe() {
    const box = this.shadowRoot.querySelector(".chart");
    if (this._ro && box) {
      this._ro.disconnect();
      this._ro.observe(box);
    }
  }

  _render() {
    if (!this._config) return;
    const v = this._config.view;
    let body;
    if (this._error) body = `<div class="msg">Impossible de lire les statistiques : ${esc(this._error)}</div>`;
    else if (!this._data || (v === "cumulative" && !this._daily)) body = `<div class="msg">Chargement des statistiques…</div>`;
    else if (v === "month" || v === "year") body = this._summaryHtml(v);
    else if (v === "monthly") body = this._monthlyHtml();
    else if (v === "cumulative") body = this._cumulativeHtml();
    else if (v === "payback") body = this._paybackHtml();
    else body = this._yearlyHtml();
    this.shadowRoot.innerHTML = `<style>${STYLE}</style><ha-card>${body}</ha-card>`;
    this._bind();
    this._observe();
    this._renderChart();
  }

  // --- Bilans à date --------------------------------------------------------

  _summaryHtml(view) {
    const { end, toDate } = this._data;
    const price = this._price;
    const block = view === "month" ? toDate.month : toDate.ytd;
    const cur = derive(block.cur, price);
    const prev = derive(block.prev, price);
    const y = end.getFullYear();
    const monthName = MONTHS[end.getMonth()];
    const title =
      this._config.title || (view === "month" ? `${monthName[0].toUpperCase()}${monthName.slice(1)} à date` : `${y} à date`);
    const until = `${end.getDate()} ${monthName} ${end.getHours()} h`;
    const sub = `jusqu'au ${until}, comparé à la même période de ${y - 1}`;

    if (!cur) {
      return `<div class="head"><div class="title">${esc(title)}</div></div>
        <div class="msg">Pas encore de données pour cette période.</div>`;
    }

    const keys = ["solar", "import", "autonomie", "savings", "conso", "export"].filter(
      (k) => k !== "savings" || price != null
    );
    const tiles = keys
      .map((k) => {
        const m = METRICS[k];
        const d = delta(k, cur[k], prev?.[k]);
        const prevTxt = prev?.[k] != null ? `<span class="muted">${y - 1} : ${fmtValue(prev[k], m.unit)}</span>` : "";
        let extra = "";
        if (k === "import" && cur.cost != null) extra = `≈ ${fmtValue(cur.cost, "€")} à ${nf2.format(price)} €/kWh`;
        if (k === "solar" && Object.keys(cur.bySource).length > 1) extra = this._sourcesText(cur.bySource);
        return tileHtml({
          label: m.label,
          value: fmtValue(cur[k], m.unit, false),
          unit: unitSuffix(m.unit),
          delta: `${d ? d.html : ""}${prevTxt}`,
          extra,
          hint: m.hint,
        });
      })
      .join("");

    return `<div class="head"><div class="title">${esc(title)}</div><div class="sub">${esc(sub)}</div></div>
      <div class="tiles">${tiles}</div>`;
  }

  _sourcesText(bySource) {
    return this._data.src.solar
      .filter((id) => bySource[id] != null)
      .map((id) => `${esc(this._sourceName(id))} ${fmtValue(bySource[id], "kWh", false)}`)
      .join(" · ");
  }

  // --- Mois par mois --------------------------------------------------------

  _years() {
    const yNow = this._data.end.getFullYear();
    const first = this._data.first?.year ?? yNow;
    const n = Math.max(1, Math.min(this._config.years, YEAR_RAMP.light.length));
    const years = [];
    for (let y = yNow - n + 1; y <= yNow; y++) if (y >= first) years.push(y);
    return years;
  }

  _yearColor(year) {
    const ramp = YEAR_RAMP[this._dark ? "dark" : "light"];
    const offset = this._data.end.getFullYear() - year;
    return ramp[ramp.length - 1 - offset] || ramp[0];
  }

  _fullYear(y) {
    return derive(sumTotals([...Array(12).keys()].map((mo) => this._data.months.get(monthKey(y, mo)))), this._price);
  }

  _partialYear(y) {
    const f = this._data.first;
    return !!f && f.year === y && f.month > 0;
  }

  _complete(y) {
    return y < this._data.end.getFullYear() && !this._partialYear(y);
  }

  _monthValue(year, month, key) {
    return derive(this._data.months.get(monthKey(year, month)), this._price)?.[key] ?? null;
  }

  _metricKeys() {
    return ["solar", "import", "export", "conso", "autonomie", "savings"].filter(
      (k) => k !== "savings" || this._price != null
    );
  }

  _monthlyHtml() {
    const key = this._metricKeys().includes(this._metric) ? this._metric : "solar";
    this._metric = key;
    const m = METRICS[key];
    const years = this._years();
    const { end, toDate } = this._data;
    const yNow = end.getFullYear();
    const mNow = end.getMonth();
    const prevToDate = derive(toDate.month.prev, this._price)?.[key];

    const tabs = this._metricKeys()
      .map((k) => `<button data-metric="${k}" aria-pressed="${k === key}">${esc(METRICS[k].tab)}</button>`)
      .join("");
    const legend =
      years.map((y) => `<span><i class="sw" style="background:${this._yearColor(y)}"></i>${y}</span>`).join("") +
      (years.includes(yNow - 1) && prevToDate != null
        ? `<span><i class="sw mark"></i>${yNow - 1} à la même date</span>`
        : "");

    const head = `<th>Mois <span class="note">(${m.unit})</span></th>${years.map((y) => `<th>${y}</th>`).join("")}${
      years.length > 1 ? `<th>Écart ${yNow}/${yNow - 1}</th>` : ""
    }`;
    const rows = [];
    for (let mo = 0; mo < 12; mo++) {
      const isNow = mo === mNow;
      const cells = years.map((y) => {
        return `<td>${fmtValue(this._monthValue(y, mo, key), m.unit, false)}</td>`;
      });
      let d = null;
      if (years.length > 1 && (mo < mNow || isNow)) {
        const cur = this._monthValue(yNow, mo, key);
        const prev = isNow ? prevToDate : this._monthValue(yNow - 1, mo, key);
        d = delta(key, cur, prev);
      }
      const dCell = years.length > 1 ? `<td>${d ? d.html : ""}${isNow && d ? ` <span class="note">à date</span>` : ""}</td>` : "";
      const name = MONTHS[mo][0].toUpperCase() + MONTHS[mo].slice(1);
      rows.push(`<tr class="${isNow ? "now" : ""}"><td>${name}</td>${cells.join("")}${dCell}</tr>`);
    }

    // Totaux : années complètes, année en cours à date.
    const totalCells = years.map((y) => {
      const t = this._fullYear(y);
      return `<td>${fmtValue(t?.[key], m.unit, false)}${y === yNow ? "" : this._partialNote(y)}</td>`;
    });
    let totalDelta = "";
    if (years.length > 1) {
      const d = delta(key, derive(toDate.ytd.cur, this._price)?.[key], derive(toDate.ytd.prev, this._price)?.[key]);
      totalDelta = `<td>${d ? `${d.html} <span class="note">à date</span>` : ""}</td>`;
    }
    rows.push(`<tr class="total"><td>Total</td>${totalCells.join("")}${totalDelta}</tr>`);

    const hint = m.hint ? `<div class="foot">${esc(m.label)} : ${esc(m.hint[0].toLowerCase() + m.hint.slice(1))}.</div>` : "";
    return `<div class="head"><div class="title">${esc(this._config.title || "Mois par mois")}</div>
        <div class="tabs" role="group" aria-label="Indicateur">${tabs}</div></div>
      <div class="legend">${legend}</div>
      <div class="chart"><div class="tip" hidden></div></div>
      <div class="scroll"><table><thead><tr>${head}</tr></thead><tbody>${rows.join("")}</tbody></table></div>
      ${hint}`;
  }

  _partialNote(year) {
    const f = this._data.first;
    if (!f || f.year !== year || f.month === 0) return "";
    return ` <span class="note">depuis ${MONTHS_SHORT[f.month]}</span>`;
  }

  _renderChart() {
    if (this._config?.view === "monthly") this._renderMonthlyChart();
    else if (this._config?.view === "cumulative") this._renderCumChart();
    else if (this._config?.view === "payback") this._renderPaybackChart();
  }

  _renderMonthlyChart() {
    if (!this._data) return;
    const box = this.shadowRoot.querySelector(".chart");
    if (!box || !this._width) return;
    box.querySelector("svg")?.remove();

    const key = this._metric;
    const m = METRICS[key];
    const years = this._years();
    const { end, toDate } = this._data;
    const yNow = end.getFullYear();
    const mNow = end.getMonth();
    const prevToDate = derive(toDate.month.prev, this._price)?.[key];

    const W = this._width;
    const H = 244;
    const L = 46;
    const R = 4;
    const T = 24;
    const B = 22;
    const plotW = W - L - R;
    const plotH = H - T - B;

    let max = 0;
    for (const y of years) for (let mo = 0; mo < 12; mo++) max = Math.max(max, this._monthValue(y, mo, key) ?? 0);
    if (prevToDate != null) max = Math.max(max, prevToDate);
    let step;
    let top;
    if (m.unit === "%") {
      step = 25;
      top = 100;
    } else {
      step = niceStep(Math.max(max, 1) / 4);
      top = Math.ceil(Math.max(max, 1) / step) * step;
    }
    const yPx = (v) => T + plotH - (Math.max(v, 0) / top) * plotH;

    const parts = [];
    for (let v = 0; v <= top + 1e-9; v += step) {
      const py = Math.round(yPx(v)) + 0.5;
      parts.push(`<line class="grid" x1="${L}" x2="${W - R}" y1="${py}" y2="${py}"/>`);
      parts.push(`<text x="${L - 6}" y="${py + 4}" text-anchor="end">${fmtValue(v, m.unit, m.unit === "%")}</text>`);
    }
    if (m.unit !== "%") parts.push(`<text x="${L - 6}" y="10" text-anchor="end">${m.unit}</text>`);

    const groupW = plotW / 12;
    const n = years.length;
    const gap = 2;
    const barW = Math.max(3, Math.min(24, (groupW * 0.78 - gap * (n - 1)) / n));
    const inner = n * barW + (n - 1) * gap;
    const narrow = groupW < 34;

    for (let mo = 0; mo < 12; mo++) {
      const gx = L + mo * groupW;
      parts.push(`<rect class="hit" data-month="${mo}" x="${gx}" y="${T}" width="${groupW}" height="${plotH + B}"/>`);
      years.forEach((y, i) => {
        const v = this._monthValue(y, mo, key);
        if (v == null) return;
        const x = gx + (groupW - inner) / 2 + i * (barW + gap);
        const top = yPx(v);
        parts.push(`<path d="${barPath(x, top, barW, T + plotH - top)}" fill="${this._yearColor(y)}" pointer-events="none"/>`);
        if (mo === mNow && y === yNow - 1 && prevToDate != null) {
          const my = yPx(prevToDate);
          parts.push(`<line class="marker" x1="${x - 2}" x2="${x + barW + 2}" y1="${my}" y2="${my}" pointer-events="none"/>`);
        }
      });
      const label = narrow ? MONTHS[mo][0].toUpperCase() : MONTHS_SHORT[mo];
      parts.push(
        `<text class="${mo === mNow ? "cur" : ""}" x="${gx + groupW / 2}" y="${H - 6}" text-anchor="middle" pointer-events="none">${label}</text>`
      );
    }

    box.insertAdjacentHTML(
      "afterbegin",
      `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="${esc(m.label)} par mois">${parts.join("")}</svg>`
    );

    const tip = box.querySelector(".tip");
    const show = (rect) => {
      box.querySelectorAll(".hit.on").forEach((r) => r.classList.remove("on"));
      rect.classList.add("on");
      const mo = +rect.dataset.month;
      tip.innerHTML = this._tipHtml(mo, years, key);
      tip.hidden = false;
      const gx = L + mo * groupW;
      const tw = tip.offsetWidth;
      let left = gx + groupW + 8;
      if (left + tw > W) left = gx - tw - 8;
      tip.style.left = `${Math.max(0, left)}px`;
      tip.style.top = `${T}px`;
    };
    box.querySelectorAll(".hit").forEach((rect) => {
      rect.addEventListener("pointerenter", () => show(rect));
      rect.addEventListener("pointerdown", () => show(rect));
    });
    box.querySelector("svg").addEventListener("pointerleave", () => {
      tip.hidden = true;
      box.querySelectorAll(".hit.on").forEach((r) => r.classList.remove("on"));
    });
  }

  _tipHtml(mo, years, key) {
    const m = METRICS[key];
    const { end, toDate } = this._data;
    const yNow = end.getFullYear();
    const isNow = mo === end.getMonth();
    const prevToDate = derive(toDate.month.prev, this._price);
    const lines = [...years].reverse().map((y) => {
      const t = derive(this._data.months.get(monthKey(y, mo)), this._price);
      let note = "";
      if (isNow && y === yNow && t) note = " (en cours)";
      let html = `<div class="r"><i class="sw" style="background:${this._yearColor(y)}"></i><span>${y}${note}</span><b>${fmtValue(t?.[key], m.unit)}</b></div>`;
      if (isNow && y === yNow - 1 && prevToDate) {
        html += `<div class="s">à la même date : ${fmtValue(prevToDate[key], m.unit)}</div>`;
      }
      if (key === "solar" && t && Object.keys(t.bySource).length > 1) {
        html += `<div class="s">${this._sourcesText(t.bySource)}</div>`;
      }
      return html;
    });
    let foot = "";
    if (years.length > 1) {
      const cur = this._monthValue(yNow, mo, key);
      const prev = isNow ? prevToDate?.[key] : this._monthValue(yNow - 1, mo, key);
      const d = delta(key, cur, prev);
      if (d) foot = `<div class="f">${yNow} vs ${yNow - 1}${isNow ? " à date" : ""} : ${d.html}</div>`;
    }
    const name = MONTHS[mo][0].toUpperCase() + MONTHS[mo].slice(1);
    return `<div class="h">${name}</div>${lines.join("")}${foot}`;
  }

  // --- Cumul depuis le 1er janvier -----------------------------------------

  _cumKeys() {
    return ["solar", "import", "export", "conso", "savings"].filter((k) => k !== "savings" || this._price != null);
  }

  // Une courbe par année complète ou en cours (une année démarrée en cours de
  // route fausserait la comparaison des cumuls).
  _cumSeries(key) {
    return this._years()
      .filter((y) => !this._partialYear(y))
      .map((y) => {
        const n = daysInYear(y);
        return { year: y, n, values: cumulate(this._daily.get(y), n, key, this._price) };
      });
  }

  // Avance ou retard en jours : à quelle date l'an dernier avait-il atteint le
  // cumul d'aujourd'hui ? N'a de sens que pour un indicateur où « plus » est mieux.
  _aheadTile(key, cur) {
    const { end } = this._data;
    const yPrev = end.getFullYear() - 1;
    if (!this._daily.has(yPrev) || this._partialYear(yPrev)) return "";
    const prev = cumulate(this._daily.get(yPrev), daysInYear(yPrev), key, this._price);
    const j = prev.findIndex((v) => v != null && v >= cur);
    if (j < 0) {
      if (!this._complete(yPrev)) return "";
      return tileHtml({
        label: `Avance sur ${yPrev}`,
        value: "Année dépassée",
        text: true,
        delta: `<span class="d good">▲ plus que tout ${yPrev}</span>`,
      });
    }
    const days = j - dayOfYear(end);
    const date = new Date(yPrev, 0, 1 + j);
    const when = `<span class="muted">${yPrev} y était le ${date.getDate()} ${MONTHS_SHORT[date.getMonth()]}</span>`;
    if (days === 0) return tileHtml({ label: `Rythme de ${yPrev}`, value: "À égalité", text: true, delta: when });
    const ahead = days > 0;
    return tileHtml({
      label: ahead ? `Avance sur ${yPrev}` : `Retard sur ${yPrev}`,
      value: nf0.format(Math.abs(days)),
      unit: Math.abs(days) > 1 ? " jours" : " jour",
      delta: `<span><span class="d ${ahead ? "good" : "bad"}">${ahead ? "▲" : "▼"}</span> ${when}</span>`,
    });
  }

  _cumulativeHtml() {
    const key = this._cumKeys().includes(this._metric) ? this._metric : "solar";
    this._metric = key;
    const m = METRICS[key];
    const price = this._price;
    const { end, toDate } = this._data;
    const yNow = end.getFullYear();
    const yPrev = yNow - 1;
    const cur = derive(toDate.ytd.cur, price)?.[key] ?? null;
    const prev = derive(toDate.ytd.prev, price)?.[key] ?? null;
    const prevFull = this._complete(yPrev) ? this._fullYear(yPrev)?.[key] ?? null : null;

    const tabs = this._cumKeys()
      .map((k) => `<button data-metric="${k}" aria-pressed="${k === key}">${esc(METRICS[k].tab)}</button>`)
      .join("");
    const legend = this._cumSeries(key)
      .map((s) => `<span><i class="sw" style="background:${this._yearColor(s.year)}"></i>${s.year}</span>`)
      .join("");

    const tiles = [];
    if (cur != null) {
      tiles.push(
        tileHtml({
          label: `Cumul ${yNow}`,
          value: fmtValue(cur, m.unit, false),
          unit: unitSuffix(m.unit),
          delta: `<span class="muted">au ${end.getDate()} ${MONTHS[end.getMonth()]}, ${end.getHours()} h</span>`,
        })
      );
    }
    if (cur != null && prev != null) {
      const d = delta(key, cur, prev);
      tiles.push(
        tileHtml({
          label: `Écart avec ${yPrev}`,
          value: signed(cur - prev, m.unit),
          unit: unitSuffix(m.unit),
          delta: `${d ? d.html : ""}<span class="muted">${yPrev} à la même date : ${fmtValue(prev, m.unit)}</span>`,
        })
      );
    }
    if (cur != null && m.better === "up") tiles.push(this._aheadTile(key, cur));
    if (cur != null && prev != null && prevFull != null) {
      const proj = cur + (prevFull - prev);
      const d = delta(key, proj, prevFull);
      tiles.push(
        tileHtml({
          label: `Projection fin ${yNow}`,
          value: fmtValue(proj, m.unit, false),
          unit: unitSuffix(m.unit),
          delta: `${d ? d.html : ""}<span class="muted">${yPrev} : ${fmtValue(prevFull, m.unit)}</span>`,
          extra: `si la fin d'année ressemble à ${yPrev}`,
        })
      );
    }

    return `<div class="head"><div class="title">${esc(this._config.title || "Cumul depuis le 1er janvier")}</div>
        <div class="tabs" role="group" aria-label="Indicateur">${tabs}</div></div>
      <div class="tiles cum-tiles">${tiles.join("")}</div>
      <div class="legend">${legend}</div>
      <div class="chart"><div class="tip" hidden></div></div>`;
  }

  _renderCumChart() {
    if (!this._data || !this._daily) return;
    const box = this.shadowRoot.querySelector(".chart");
    if (!box || !this._width) return;
    box.querySelector("svg")?.remove();

    const key = this._metric;
    const m = METRICS[key];
    const series = this._cumSeries(key);
    if (!series.length) return;
    const { end } = this._data;
    const yNow = end.getFullYear();
    const nRef = daysInYear(yNow);
    const today = dayOfYear(end);

    const W = this._width;
    const H = 260;
    const L = 52;
    const R = 8;
    const T = 24;
    const B = 22;
    const plotW = W - L - R;
    const plotH = H - T - B;

    let max = 0;
    for (const s of series) for (const v of s.values) if (v != null) max = Math.max(max, v);
    const step = niceStep(Math.max(max, 1) / 4);
    const top = Math.ceil(Math.max(max, 1) / step) * step;
    const yPx = (v) => T + plotH - (Math.max(v, 0) / top) * plotH;
    // Point de fin de journée i ; l'axe va du 1er janvier 0 h au 31 décembre 24 h.
    const xPx = (i, n) => L + ((i + 1) / n) * plotW;
    const color = (y) => this._yearColor(y);

    const parts = [];
    for (let v = 0; v <= top + 1e-9; v += step) {
      const py = Math.round(yPx(v)) + 0.5;
      parts.push(`<line class="grid" x1="${L}" x2="${W - R}" y1="${py}" y2="${py}"/>`);
      parts.push(`<text x="${L - 6}" y="${py + 4}" text-anchor="end">${fmtValue(v, m.unit, false)}</text>`);
    }
    parts.push(`<text x="${L - 6}" y="10" text-anchor="end">${m.unit}</text>`);
    const narrow = plotW / 12 < 34;
    for (let mo = 0; mo < 12; mo++) {
      const x = xPx(dayOfYear(new Date(yNow, mo, 15)), nRef);
      const label = narrow ? MONTHS[mo][0].toUpperCase() : MONTHS_SHORT[mo];
      parts.push(`<text class="${mo === end.getMonth() ? "cur" : ""}" x="${x}" y="${H - 6}" text-anchor="middle">${label}</text>`);
    }

    for (const s of series) {
      let d = `M${L},${yPx(0)}`;
      s.values.forEach((v, i) => {
        if (v != null) d += `L${xPx(i, s.n).toFixed(1)},${yPx(v).toFixed(1)}`;
      });
      parts.push(`<path class="line" d="${d}" stroke="${color(s.year)}"/>`);
    }

    // Repères « aujourd'hui » : fin de la courbe de l'année et l'an dernier au même jour.
    const curS = series.find((s) => s.year === yNow);
    const prevS = series.find((s) => s.year === yNow - 1);
    const prevAt = prevS?.values[Math.min(today, prevS.n - 1)] ?? null;
    if (prevAt != null) {
      parts.push(`<circle class="dot" cx="${xPx(Math.min(today, prevS.n - 1), prevS.n)}" cy="${yPx(prevAt)}" r="4" fill="${color(yNow - 1)}"/>`);
    }
    const last = curS ? curS.values.findLastIndex((v) => v != null) : -1;
    if (last >= 0) {
      const v = curS.values[last];
      const x = xPx(last, curS.n);
      const y = yPx(v);
      parts.push(`<circle class="dot" cx="${x}" cy="${y}" r="4" fill="${color(yNow)}"/>`);
      // Étiquette du côté opposé à la courbe de l'an dernier pour ne pas la chevaucher.
      const below = prevAt != null && prevAt > v;
      const anchor = x > W - 90 ? "end" : "start";
      const lx = anchor === "end" ? x - 8 : x + 8;
      parts.push(`<text class="endlbl" x="${lx}" y="${below ? y + 18 : y - 10}" text-anchor="${anchor}">${fmtValue(v, m.unit)}</text>`);
    }

    parts.push(`<line class="xhair" x1="0" x2="0" y1="${T}" y2="${T + plotH}" visibility="hidden"/>`);
    parts.push(`<g class="hdots"></g>`);
    parts.push(`<rect class="ov" x="${L}" y="${T}" width="${plotW}" height="${plotH}"/>`);

    box.insertAdjacentHTML(
      "afterbegin",
      `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="${esc(m.label)} cumulée depuis le 1er janvier">${parts.join("")}</svg>`
    );

    const svg = box.querySelector("svg");
    const xh = svg.querySelector(".xhair");
    const hd = svg.querySelector(".hdots");
    const tip = box.querySelector(".tip");
    const move = (ev) => {
      const f = Math.min(Math.max((ev.clientX - svg.getBoundingClientRect().left - L) / plotW, 0), 1);
      const iRef = Math.min(nRef - 1, Math.max(0, Math.ceil(f * nRef) - 1));
      const x = xPx(iRef, nRef);
      xh.setAttribute("x1", x);
      xh.setAttribute("x2", x);
      xh.setAttribute("visibility", "visible");
      const vals = {};
      let dots = "";
      const rows = [...series].reverse().map((s) => {
        const i = Math.min(s.n - 1, Math.max(0, Math.ceil(f * s.n) - 1));
        const v = s.values[i];
        vals[s.year] = v;
        if (v != null) dots += `<circle class="dot" cx="${xPx(i, s.n)}" cy="${yPx(v)}" r="4" fill="${color(s.year)}"/>`;
        return `<div class="r"><i class="sw" style="background:${color(s.year)}"></i><span>${s.year}</span><b>${fmtValue(v, m.unit)}</b></div>`;
      });
      hd.innerHTML = dots;
      let foot = "";
      const a = vals[yNow];
      const b = vals[yNow - 1];
      const d = delta(key, a, b);
      if (d) foot = `<div class="f">${yNow} vs ${yNow - 1} : ${d.html} <span class="muted">(${signed(a - b, m.unit)} ${m.unit})</span></div>`;
      const date = new Date(yNow, 0, 1 + iRef);
      tip.innerHTML = `<div class="h">Cumul au ${date.getDate()} ${MONTHS[date.getMonth()]}</div>${rows.join("")}${foot}`;
      tip.hidden = false;
      const tw = tip.offsetWidth;
      let left = x + 12;
      if (left + tw > W) left = x - tw - 12;
      tip.style.left = `${Math.max(0, left)}px`;
      tip.style.top = `${T}px`;
    };
    const ov = svg.querySelector(".ov");
    ov.addEventListener("pointermove", move);
    ov.addEventListener("pointerdown", move);
    ov.addEventListener("pointerleave", () => {
      tip.hidden = true;
      xh.setAttribute("visibility", "hidden");
      hd.innerHTML = "";
    });
  }

  // --- Rentabilité ----------------------------------------------------------

  // Économies mois par mois depuis le début des statistiques (mois en cours à date).
  _savingsByMonth() {
    const { first, end } = this._data;
    const out = [];
    if (!first) return out;
    const last = end.getFullYear() * 12 + end.getMonth();
    for (let t = first.year * 12 + first.month; t <= last; t++) {
      const y = Math.floor(t / 12);
      const mo = t % 12;
      out.push({ y, mo, v: this._monthValue(y, mo, "savings") ?? 0 });
    }
    return out;
  }

  // Économies attendues pour un mois calendaire : sa dernière occurrence complète,
  // donc l'installation telle qu'elle est aujourd'hui (panneaux ajoutés compris).
  _typicalSavings(mo) {
    const { first, end } = this._data;
    const from = mo < end.getMonth() ? end.getFullYear() : end.getFullYear() - 1;
    for (let y = from; y >= (first?.year ?? end.getFullYear()); y--) {
      if (first && y === first.year && mo <= first.month) continue;
      const v = this._monthValue(y, mo, "savings");
      if (v != null) return v;
    }
    return null;
  }

  // Ce que l'an dernier a économisé entre la même date et la fin du mois.
  _restOfMonth() {
    const { end, toDate } = this._data;
    const full = this._monthValue(end.getFullYear() - 1, end.getMonth(), "savings");
    const toDatePrev = derive(toDate.month.prev, this._price)?.savings;
    return full != null && toDatePrev != null ? Math.max(full - toDatePrev, 0) : 0;
  }

  // Projection mois par mois jusqu'à ce que les économies couvrent l'investissement,
  // en supposant que chaque mois à venir ressemble au même mois des 12 derniers mois.
  _projection(invested, saved) {
    const { end } = this._data;
    let t = end.getFullYear() * 12 + end.getMonth();
    let cum = saved + this._restOfMonth();
    const points = [{ t, cum }];
    let paid = cum >= invested ? { t, frac: 1 } : null;
    for (let k = 0; k < 120 && !paid; k++) {
      t++;
      const v = this._typicalSavings(t % 12);
      if (v == null) break;
      const before = cum;
      cum += v;
      points.push({ t, cum });
      if (cum >= invested) paid = { t, frac: v > 0 ? (invested - before) / v : 1 };
    }
    return { points, paid };
  }

  _paybackHtml() {
    const title = this._config.title || "Rentabilité de l'installation";
    const price = this._price;
    if (price == null) {
      return `<div class="head"><div class="title">${esc(title)}</div></div>
        <div class="msg">Prix du kWh inconnu : renseigner l'option « price » ou un prix fixe dans le tableau Énergie.</div>`;
    }
    const { end, first } = this._data;
    const inv = this._invest.filter((i) => i.date <= end);
    const invested = inv.reduce((a, i) => a + i.amount, 0);
    const byMonth = this._savingsByMonth();
    const saved = byMonth.reduce((a, r) => a + r.v, 0);
    const proj = this._projection(invested, saved);
    this._pb = { inv, invested, byMonth, saved, proj };

    const nowT = end.getFullYear() * 12 + end.getMonth();
    const tiles = [];
    tiles.push(
      tileHtml({
        label: "Investi",
        value: fmtValue(invested, "€", false),
        unit: unitSuffix("€"),
        delta: inv.length
          ? `<span class="muted">${inv.length} achat${inv.length > 1 ? "s" : ""}, du ${fmtDay(inv[0].date)} au ${fmtDay(inv[inv.length - 1].date)}</span>`
          : "",
      })
    );
    tiles.push(
      tileHtml({
        label: "Économisé",
        value: fmtValue(saved, "€", false),
        unit: unitSuffix("€"),
        delta: first ? `<span class="muted">depuis ${MONTHS[first.month]} ${first.year} (début des statistiques)</span>` : "",
        hint: METRICS.savings.hint,
      })
    );
    const pct = invested > 0 ? (saved / invested) * 100 : null;
    const rest = invested - saved;
    tiles.push(
      tileHtml({
        label: "Rentabilisé",
        value: fmtValue(pct, "%", false),
        unit: unitSuffix("%"),
        delta: `<span class="muted">${rest > 0 ? `reste ${fmtValue(rest, "€")}` : `bénéfice ${fmtValue(-rest, "€")}`}</span>`,
        extra: pct != null ? `<div class="meter"><i style="width:${Math.min(pct, 100).toFixed(1)}%"></i></div>` : "",
      })
    );

    if (rest <= 0) {
      let cum = 0;
      const hit = byMonth.find((r) => (cum += r.v) >= invested);
      tiles.push(
        tileHtml({
          label: "Retour sur investissement",
          value: hit ? `${MONTHS[hit.mo]} ${hit.y}` : "Atteint",
          text: true,
          delta: `<span class="d good">▲ installation remboursée</span>`,
        })
      );
    } else if (proj.paid) {
      const months = proj.paid.t - nowT;
      const when = months < 24 ? `dans ≈ ${months} mois` : `dans ≈ ${nf1.format(months / 12)} ans`;
      tiles.push(
        tileHtml({
          label: "Retour sur investissement",
          value: `≈ ${MONTHS[proj.paid.t % 12]} ${Math.floor(proj.paid.t / 12)}`,
          text: true,
          delta: `<span class="muted">${when}</span>`,
          extra: "au rythme des 12 derniers mois",
        })
      );
    } else {
      tiles.push(tileHtml({ label: "Retour sur investissement", value: "Au-delà de 10 ans", text: true }));
    }

    // Gain des 12 derniers mois, du même jour l'an dernier à aujourd'hui.
    if (byMonth.length >= 13) {
      const last12 =
        byMonth.slice(-12).reduce((a, r) => a + r.v, 0) + this._restOfMonth();
      tiles.push(
        tileHtml({
          label: "Gain sur 12 mois",
          value: fmtValue(last12, "€", false),
          unit: unitSuffix("€"),
          delta: invested > 0 ? `<span class="muted">soit ${nf0.format((last12 / invested) * 100)} % de l'investissement par an</span>` : "",
        })
      );
    }

    const legend = `<span><i class="sw save"></i>Économies cumulées</span><span><i class="sw dash"></i>Projection</span><span><i class="sw cost"></i>Investissement cumulé</span>`;
    const invRows = inv
      .map((i) => `<tr><td>${fmtDay(i.date)}</td><td style="text-align:left">${esc(i.label)}</td><td>${nf2.format(i.amount)} €</td></tr>`)
      .join("");
    const future = this._invest.length - inv.length;

    return `<div class="head"><div class="title">${esc(title)}</div>
        <div class="sub">économies valorisées à ${nf2.format(price)} €/kWh</div></div>
      <div class="tiles cum-tiles">${tiles.join("")}</div>
      <div class="legend">${legend}</div>
      <div class="chart"><div class="tip" hidden></div></div>
      <details class="inv"><summary>${inv.length} achat${inv.length > 1 ? "s" : ""} pris en compte${
        future ? ` (${future} daté${future > 1 ? "s" : ""} dans le futur, ignoré${future > 1 ? "s" : ""})` : ""
      }</summary>
        <div class="scroll"><table><tbody>${invRows}</tbody></table></div></details>
      <div class="foot">Économies = énergie consommée sans l'acheter à EDF × prix du kWh. La production d'avant le début des statistiques n'est pas comptée.</div>`;
  }

  _renderPaybackChart() {
    if (!this._data || !this._pb) return;
    const box = this.shadowRoot.querySelector(".chart");
    if (!box || !this._width) return;
    box.querySelector("svg")?.remove();

    const { inv, invested, byMonth, proj } = this._pb;
    const { end } = this._data;
    if (!byMonth.length) return;

    // Axe des temps en mois (t = année × 12 + mois), fraction comprise.
    const tOfDate = (d) => d.getFullYear() * 12 + d.getMonth() + (d.getDate() - 1) / new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
    const nowT = tOfDate(end) + end.getHours() / 24 / 30;
    const firstT = byMonth[0].y * 12 + byMonth[0].mo;
    const t0 = Math.min(firstT, ...inv.map((i) => tOfDate(i.date)));
    const lastProjT = proj.points[proj.points.length - 1].t + 1;
    const t1 = Math.max(Math.ceil(nowT) + 1, lastProjT + 1);

    const W = this._width;
    const H = 260;
    const L = 52;
    const R = 8;
    const T = 24;
    const B = 22;
    const plotW = W - L - R;
    const plotH = H - T - B;
    const max = Math.max(invested, ...proj.points.map((p) => p.cum), 1);
    const step = niceStep(max / 4);
    const top = Math.ceil(max / step) * step;
    const yPx = (v) => T + plotH - (Math.max(v, 0) / top) * plotH;
    const xPx = (t) => L + ((t - t0) / (t1 - t0)) * plotW;

    const parts = [];
    for (let v = 0; v <= top + 1e-9; v += step) {
      const py = Math.round(yPx(v)) + 0.5;
      parts.push(`<line class="grid" x1="${L}" x2="${W - R}" y1="${py}" y2="${py}"/>`);
      parts.push(`<text x="${L - 6}" y="${py + 4}" text-anchor="end">${fmtValue(v, "€", false)}</text>`);
    }
    parts.push(`<text x="${L - 6}" y="10" text-anchor="end">€</text>`);
    for (let y = Math.ceil(t0 / 12); y * 12 <= t1; y++) {
      const x = Math.round(xPx(y * 12)) + 0.5;
      if (x > L + 1) parts.push(`<line class="yr" x1="${x}" x2="${x}" y1="${T}" y2="${T + plotH}"/>`);
      if (x < W - R - 30) parts.push(`<text x="${x + 4}" y="${H - 6}">${y}</text>`);
    }

    // Investissement cumulé : une marche par achat.
    let cost = 0;
    let d = `M${xPx(t0)},${yPx(0)}`;
    for (const i of inv) {
      const x = xPx(tOfDate(i.date)).toFixed(1);
      d += `H${x}`;
      cost += i.amount;
      d += `V${yPx(cost).toFixed(1)}`;
    }
    d += `H${xPx(t1)}`;
    parts.push(`<path class="line pb-cost" d="${d}"/>`);

    // Économies réelles (fin de chaque mois, mois en cours à date).
    let cum = 0;
    const pts = [[xPx(firstT), yPx(0)]];
    byMonth.forEach((r, k) => {
      cum += r.v;
      const t = k === byMonth.length - 1 ? nowT : r.y * 12 + r.mo + 1;
      pts.push([xPx(t), yPx(cum)]);
    });
    const line = pts.map((p, k) => `${k ? "L" : "M"}${p[0].toFixed(1)},${p[1].toFixed(1)}`).join("");
    const [lx, ly] = pts[pts.length - 1];
    parts.push(`<path class="pb-area" d="${line}L${lx.toFixed(1)},${yPx(0)}L${pts[0][0].toFixed(1)},${yPx(0)}Z"/>`);
    parts.push(`<path class="line pb-save" d="${line}"/>`);

    // Projection, coupée au point de retour sur investissement.
    let pd = `M${lx.toFixed(1)},${ly.toFixed(1)}`;
    let breakEven = null;
    proj.points.forEach((p, k) => {
      if (breakEven) return;
      if (proj.paid && p.t === proj.paid.t && cum < invested) {
        // Le premier point couvre la fin du mois en cours, les suivants un mois entier.
        const t = k === 0 ? nowT + (p.t + 1 - nowT) * proj.paid.frac : p.t + proj.paid.frac;
        breakEven = [xPx(t), yPx(invested)];
        pd += `L${breakEven[0].toFixed(1)},${breakEven[1].toFixed(1)}`;
      } else {
        pd += `L${xPx(p.t + 1).toFixed(1)},${yPx(p.cum).toFixed(1)}`;
      }
    });
    if (cum < invested) parts.push(`<path class="line pb-proj" d="${pd}"/>`);

    parts.push(`<circle class="dot pb-dot-save" cx="${lx}" cy="${ly}" r="4"/>`);
    parts.push(`<text class="endlbl" x="${lx + 8}" y="${ly + 16}">${fmtValue(cum, "€")}</text>`);
    if (breakEven) {
      const label = `≈ ${MONTHS_SHORT[proj.paid.t % 12]} ${Math.floor(proj.paid.t / 12)}`;
      const anchor = breakEven[0] > W - 80 ? "end" : "middle";
      parts.push(`<circle class="dot pb-dot-save" cx="${breakEven[0]}" cy="${breakEven[1]}" r="4"/>`);
      parts.push(`<text class="endlbl" x="${breakEven[0]}" y="${breakEven[1] - 10}" text-anchor="${anchor}">${label}</text>`);
    }

    parts.push(`<line class="xhair" x1="0" x2="0" y1="${T}" y2="${T + plotH}" visibility="hidden"/>`);
    parts.push(`<rect class="ov" x="${L}" y="${T}" width="${plotW}" height="${plotH}"/>`);
    box.insertAdjacentHTML(
      "afterbegin",
      `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="Économies cumulées face à l'investissement">${parts.join("")}</svg>`
    );

    // Survol : valeurs à la fin du mois pointé (réelles ou projetées).
    const realCum = new Map();
    let acc = 0;
    for (const r of byMonth) realCum.set(r.y * 12 + r.mo, (acc += r.v));
    const projCum = new Map(proj.points.map((p) => [p.t, p.cum]));
    const nowMonth = end.getFullYear() * 12 + end.getMonth();
    const svg = box.querySelector("svg");
    const xh = svg.querySelector(".xhair");
    const tip = box.querySelector(".tip");
    const move = (ev) => {
      const f = Math.min(Math.max((ev.clientX - svg.getBoundingClientRect().left - L) / plotW, 0), 1);
      const t = Math.min(Math.floor(t0 + f * (t1 - t0)), t1 - 1);
      const x = xPx(Math.min(t + 1, t1));
      xh.setAttribute("x1", x);
      xh.setAttribute("x2", x);
      xh.setAttribute("visibility", "visible");
      const isProj = t > nowMonth;
      const s = t <= nowMonth ? realCum.get(t) : projCum.get(t);
      const monthEnd = new Date(Math.floor(t / 12), (t % 12) + 1, 0);
      const c = inv.filter((i) => i.date <= monthEnd).reduce((a, i) => a + i.amount, 0);
      const bal = s != null ? s - c : null;
      tip.innerHTML = `<div class="h">${MONTHS[t % 12][0].toUpperCase()}${MONTHS[t % 12].slice(1)} ${Math.floor(t / 12)}${
        t === nowMonth ? " (à date)" : isProj ? " (projection)" : ""
      }</div>
        <div class="r"><i class="sw save"></i><span>Économies cumulées</span><b>${fmtValue(s, "€")}</b></div>
        <div class="r"><i class="sw cost"></i><span>Investi</span><b>${fmtValue(c, "€")}</b></div>
        ${bal != null ? `<div class="f">Solde : <span class="d ${bal >= 0 ? "good" : "neutral"}">${signed(bal, "€")} €</span></div>` : ""}`;
      tip.hidden = false;
      const tw = tip.offsetWidth;
      let left = x + 12;
      if (left + tw > W) left = x - tw - 12;
      tip.style.left = `${Math.max(0, left)}px`;
      tip.style.top = `${T}px`;
    };
    const ov = svg.querySelector(".ov");
    ov.addEventListener("pointermove", move);
    ov.addEventListener("pointerdown", move);
    ov.addEventListener("pointerleave", () => {
      tip.hidden = true;
      xh.setAttribute("visibility", "hidden");
    });
  }

  _bind() {
    this.shadowRoot.querySelectorAll(".tabs button").forEach((b) =>
      b.addEventListener("click", () => {
        this._metric = b.dataset.metric;
        this._render();
      })
    );
  }

  // --- Année par année ------------------------------------------------------

  _yearlyHtml() {
    const { end, toDate, first } = this._data;
    const price = this._price;
    const yNow = end.getFullYear();
    const keys = ["solar", "import", "export", "conso", "autonomie", "autoconso", "savings"].filter(
      (k) => k !== "savings" || price != null
    );
    const firstYear = first?.year ?? yNow;

    const head = `<th>Année</th>${keys
      .map(
        (k) =>
          `<th${METRICS[k].hint ? ` title="${esc(METRICS[k].hint)}"` : ""}>${esc(METRICS[k].tab)}<span class="cell-sub">${METRICS[k].unit}</span></th>`
      )
      .join("")}`;
    const row = (label, cur, prev, cls = "") =>
      `<tr class="${cls}"><td>${label}</td>${keys
        .map((k) => {
          const d = prev ? delta(k, cur?.[k], prev[k]) : null;
          return `<td>${fmtValue(cur?.[k], METRICS[k].unit, false)}${d ? `<span class="cell-sub">${d.html}</span>` : ""}</td>`;
        })
        .join("")}</tr>`;

    const rows = [];
    const ytdPrev = derive(toDate.ytd.prev, price);
    rows.push(row(`${yNow} <span class="note">à date</span>`, derive(toDate.ytd.cur, price), ytdPrev));
    if (ytdPrev) rows.push(row(`${yNow - 1} à la même date`, ytdPrev, null, "ref"));
    for (let y = yNow - 1; y >= firstYear; y--) {
      const prev = this._complete(y - 1) ? this._fullYear(y - 1) : null;
      rows.push(row(`${y}${this._partialNote(y)}`, this._fullYear(y), prev));
    }

    return `<div class="head"><div class="title">${esc(this._config.title || "Année par année")}</div>
        <div class="sub">écarts par rapport à l'année précédente</div></div>
      <div class="scroll"><table><thead><tr>${head}</tr></thead><tbody>${rows.join("")}</tbody></table></div>
      <div class="foot">Autonomie : part de la consommation non achetée à EDF. Autoconso : part de la production consommée sur place.${
        price != null ? ` Économies valorisées à ${nf2.format(price)} €/kWh.` : ""
      }</div>`;
  }
}

if (!customElements.get(CARD_TAG)) {
  customElements.define(CARD_TAG, EnergyPerfCard);
  window.customCards = window.customCards || [];
  window.customCards.push({
    type: CARD_TAG,
    name: "Performance énergie",
    description: "Comparatifs mois/année de la production solaire et des achats EDF",
  });
  console.info(`%c ENERGY-PERF-CARD %c v${CARD_VERSION} `, "background:#184f95;color:#fff", "");
}
