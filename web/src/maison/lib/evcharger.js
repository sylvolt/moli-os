// EV charging stations (a Victron EVCS through the `victron` driver, and any
// device that speaks the same points): pure helpers for the Borne page.

/** A charging station: its state, whether charging is allowed, its current. */
export function isEvCharger(device) {
  const keys = new Set((device?.points ?? []).map((p) => p.key));
  return keys.has('status') && keys.has('charging_enabled') && keys.has('set_current');
}

/** What the station says, as a word of the catalogue and a tone for the pill. */
const STATUS = {
  disconnected: '',
  connected: 'good',
  charging: 'warm',
  charged: 'good',
  waiting_for_sun: 'cool',
  waiting_for_rfid: 'cool',
  waiting_for_start: 'cool',
  low_soc: 'cool',
  charging_limit: 'cool',
  start_charging: 'warm',
  switching_to_3_phase: 'warm',
  switching_to_1_phase: 'warm',
};
const FAULTS = new Set([
  'ground_test_error',
  'welded_contacts_error',
  'cp_input_error',
  'residual_current',
  'undervoltage',
  'overvoltage',
  'overheating',
]);

/** `{ key, tone }`: `key` names the word (`borne.etat.<key>`), `null` when unknown. */
export function statusOf(raw) {
  const key = typeof raw === 'string' ? raw : null;
  if (!key) return { key: null, tone: '' };
  if (FAULTS.has(key)) return { key, tone: 'alert' };
  return { key: key in STATUS ? key : null, tone: STATUS[key] ?? '' };
}

/** A car is plugged in (the station knows it, whatever it does with it). */
export const plugged = (raw) => typeof raw === 'string' && raw !== 'disconnected';

/** The modes a station offers, in the order shown. */
export const MODES = ['manual', 'auto', 'scheduled'];

/** Seconds as « 1 h 05 » / « 12 min ». */
export function hoursMinutes(seconds) {
  if (typeof seconds !== 'number' || !Number.isFinite(seconds) || seconds < 0) return null;
  const minutes = Math.floor(seconds / 60);
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h ? `${h} h ${String(m).padStart(2, '0')}` : `${m} min`;
}

/** The current a slider may set: the point's bounds, 6 to 32 A by default. */
export function currentRange(device) {
  const kind = device?.points?.find((p) => p.key === 'set_current')?.kind ?? {};
  const min = Number.isFinite(kind.min) ? kind.min : 6;
  const max = Number.isFinite(kind.max) ? kind.max : 32;
  return { min, max, step: Number.isFinite(kind.step) && kind.step > 0 ? kind.step : 1 };
}
