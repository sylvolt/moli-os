// The home battery of a Victron GX (the `victron` driver): its BMS, its
// inverter/charger and its ESS settings are three devices of one driver
// instance. Pure helpers for the « Recharge et batterie » page.

const keys = (device) => new Set((device?.points ?? []).map((p) => p.key));
const pointOf = (device, key) => device?.points?.find((p) => p.key === key);

/** The ESS settings: the minimum charge, and the mode when the GX has it. */
export const isEssSettings = (device) => keys(device).has('minimum_soc');

/** An inverter/charger whose mode can be read (on, charger only…). */
export function isInverter(device) {
  const values = pointOf(device, 'mode')?.kind?.values ?? [];
  return values.includes('charger_only') && values.includes('inverter_only');
}

/** A battery (its BMS or monitor). */
export function isBattery(device) {
  const k = keys(device);
  return k.has('soc') && k.has('power') && k.has('voltage');
}

/** One installation per driver instance: `{ instance, battery, inverter,
 *  settings }`, kept when it has something to steer. */
export function installations(devices) {
  const by = new Map();
  for (const d of devices ?? []) {
    const role = isEssSettings(d) ? 'settings' : isInverter(d) ? 'inverter' : isBattery(d) ? 'battery' : null;
    if (!role) continue;
    const key = d.instance ?? '';
    const one = by.get(key) ?? { instance: key, battery: null, inverter: null, settings: null };
    if (!one[role] || d.id < one[role].id) one[role] = d;
    by.set(key, one);
  }
  return [...by.values()].filter((i) => i.settings || i.inverter).sort((a, b) => a.instance.localeCompare(b.instance));
}

/** The inverter modes, in the order shown (`off` last, it cuts the output). */
export const INVERTER_MODES = ['on', 'charger_only', 'inverter_only', 'off'];
/** Modes that take the house off the grid or off power: asked twice. */
export const RISKY_MODES = new Set(['inverter_only', 'off']);
/** The ESS modes, in the order shown. */
export const ESS_MODES = ['optimized_batterylife', 'optimized', 'keep_charged'];

/** A point's writable range, with defaults. */
export function rangeOf(device, key, fallback) {
  const kind = pointOf(device, key)?.kind ?? {};
  const pick = (v, d) => (Number.isFinite(v) ? v : d);
  return {
    min: pick(kind.min, fallback.min),
    max: pick(kind.max, fallback.max),
    step: Number.isFinite(kind.step) && kind.step > 0 ? kind.step : fallback.step,
  };
}

/** A power ceiling as the GX says it: `null` for « no limit » (-1, or any
 *  negative), watts otherwise, `undefined` when unknown. */
export function limitOf(raw) {
  if (typeof raw !== 'number' || !Number.isFinite(raw)) return undefined;
  return raw < 0 ? null : raw;
}

/** Where a slider of a ceiling starts: the ceiling, else the top of its range. */
export const sliderStart = (limit, range) => (typeof limit === 'number' ? Math.min(range.max, Math.max(0, limit)) : range.max);

/** A value snapped to its range and step. */
export function snap(value, range) {
  if (!Number.isFinite(value)) return null;
  const n = Math.round((value - range.min) / range.step) * range.step + range.min;
  return Math.min(range.max, Math.max(range.min, Math.round(n * 1000) / 1000));
}

/** What the battery does: `charge`, `discharge` or `idle` (under 30 W). */
export function flowOf(powerW) {
  if (typeof powerW !== 'number' || !Number.isFinite(powerW)) return null;
  if (powerW > 30) return 'charge';
  if (powerW < -30) return 'discharge';
  return 'idle';
}
