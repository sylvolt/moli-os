// A small stand-in for Home Assistant's `hass` object, for a Lovelace card
// that only reads energy statistics (the house's own « Performance énergie »
// card, kept as it is). It answers the three calls that card makes from
// Moli's energy history (/api/energy, /api/energy/series), which holds the
// same hourly kWh (HA's statistics were imported into it).
//
//   energy/get_prefs                 → the meters as HA's Energy dashboard sources
//   recorder/statistic_during_period → one meter's kWh over [start, end)
//   recorder/statistics_during_period→ one row per month or per day, per meter
//
// Ids are Moli's meter ids (`reseau-achat`, `solaire-jardin`…).

const HOUR = 3_600_000;
const COUNTS = { hour: 8784, day: 3660, month: 600 };

/** HA's Energy preferences, from Moli's meters. */
export function prefsOf(summary) {
  const meters = summary?.meters ?? [];
  const ids = (role) => meters.filter((m) => m.role === role).map((m) => m.id);
  const buying = ids('grid').length ? ids('grid') : ids('total');
  const price = summary?.price_now ?? summary?.house_price ?? null;
  const sources = [];
  const exports = ids('export');
  buying.forEach((id, i) =>
    sources.push({
      type: 'grid',
      stat_energy_from: id,
      stat_energy_to: i === 0 ? (exports[0] ?? null) : null,
      number_energy_price: price,
    }),
  );
  for (const id of ids('solar')) sources.push({ type: 'solar', stat_energy_from: id });
  const charge = ids('charge');
  const discharge = ids('discharge');
  if (charge.length || discharge.length) {
    sources.push({ type: 'battery', stat_energy_to: charge[0] ?? null, stat_energy_from: discharge[0] ?? null });
  }
  return { energy_sources: sources, device_consumption: [] };
}

/** `{ <meter>: Map(start ms → kWh) }` and the sorted bucket starts. */
export function index(report) {
  const starts = (report?.buckets ?? []).map((b) => b.start).sort((a, b) => a - b);
  const byMeter = {};
  for (const b of report?.buckets ?? []) {
    for (const [id, amount] of Object.entries(b.meters ?? {})) {
      (byMeter[id] ??= new Map()).set(b.start, amount?.kwh ?? 0);
    }
  }
  return { starts, byMeter };
}

/** The bucket start after `start` in a sorted list (`null` at the end). */
function nextStart(starts, start) {
  let lo = 0;
  let hi = starts.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (starts[mid] <= start) lo = mid + 1;
    else hi = mid;
  }
  return lo < starts.length ? starts[lo] : null;
}

/** kWh of one meter over [start, end): whole months where they fit, then
 *  whole days, then hours (buckets start on the house's own boundaries).
 *  `null` when nothing was recorded at all. */
export function sumBetween(series, id, start, end) {
  const { month, day, hour } = series;
  let at = start;
  let total = 0;
  let seen = false;
  const take = (ix, s) => {
    const v = ix.byMeter[id]?.get(s);
    if (v != null) {
      total += v;
      seen = true;
    }
  };
  let guard = 0;
  while (at < end && guard++ < 20000) {
    const mNext = month.starts.includes(at) ? nextStart(month.starts, at) : null;
    if (mNext != null && mNext <= end) {
      take(month, at);
      at = mNext;
      continue;
    }
    const dNext = day.starts.includes(at) ? nextStart(day.starts, at) : null;
    if (dNext != null && dNext <= end) {
      take(day, at);
      at = dNext;
      continue;
    }
    take(hour, at);
    at += HOUR;
  }
  return seen ? total : null;
}

/** HA's `statistics_during_period` rows from one of Moli's reports. */
export function rowsOf(ix, ids, from) {
  const out = {};
  for (const id of ids) {
    const rows = [];
    for (const s of ix.starts) {
      if (s < from) continue;
      const kwh = ix.byMeter[id]?.get(s);
      if (kwh != null) rows.push({ start: s, change: kwh });
    }
    out[id] = rows;
  }
  return out;
}

/** The stand-in `hass`. `dark` says the page's theme. */
export async function moliHass({ dark = false } = {}) {
  const get = async (url) => {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${url}: ${res.status}`);
    return res.json();
  };
  const summary = await get('/api/energy');
  const series = {};
  const load = (step) =>
    (series[step] ??= get(`/api/energy/series?step=${step}&count=${COUNTS[step]}`).then(index));
  const all = async () => ({ month: await load('month'), day: await load('day'), hour: await load('hour') });

  return {
    config: { time_zone: summary.timezone },
    themes: { darkMode: dark },
    language: 'fr',
    async callWS(msg) {
      switch (msg.type) {
        case 'energy/get_prefs':
          return prefsOf(summary);
        case 'recorder/statistic_during_period': {
          const start = Date.parse(msg.fixed_period.start_time);
          const end = Date.parse(msg.fixed_period.end_time);
          return { change: sumBetween(await all(), msg.statistic_id, start, end) };
        }
        case 'recorder/statistics_during_period': {
          const step = msg.period === 'month' ? 'month' : msg.period === 'day' ? 'day' : 'hour';
          return rowsOf(await load(step), msg.statistic_ids ?? [], Date.parse(msg.start_time));
        }
        default:
          throw new Error(`not available here: ${msg.type}`);
      }
    },
  };
}
