// The internet box on the Infra page, from points the house already reads
// (through Home Assistant or any driver), named in home.json:
//
//   "infra": { "box": {
//     "name": "Freebox",
//     "down": "<point>", "up": "<point>", "rate_unit": "kB/s",   // or "Mbit/s"
//     "free": "<point>",          // free space, in percent
//     "degraded": "<point>",      // true: the disk array is degraded
//     "wifi": "<point>",          // true: Wi-Fi on
//     "temperatures": [{ "label": "Disque", "point": "<point>", "warn": 50, "alert": 55 }],
//     "fans": [{ "label": "Ventilateur", "point": "<point>", "warn": 1800 }]
//   } }

/** `<device>/<key>` → its value, through `value(device, key)`. */
export function reader(value) {
  return (point) => {
    if (typeof point !== 'string') return null;
    const i = point.lastIndexOf('/');
    return i > 0 ? value(point.slice(0, i), point.slice(i + 1)) : null;
  };
}

/** A rate in the box's unit → Mbit/s (one decimal). */
export function mbits(rate, unit = 'kB/s') {
  if (typeof rate !== 'number') return null;
  const factor = unit === 'Mbit/s' ? 1 : unit === 'kbit/s' ? 0.001 : unit === 'MB/s' ? 8 : 0.008;
  return Math.round(rate * factor * 10) / 10;
}

/** What needs a look on the box: a hot temperature, a fast fan, a degraded
 *  array, a nearly full disk. `read(point)` gives a point's value. */
export function boxIssues(box, read, t) {
  if (!box) return [];
  const name = box.name ?? 'Box';
  const out = [];
  for (const s of box.temperatures ?? []) {
    const x = read(s.point);
    if (typeof x === 'number' && typeof s.alert === 'number' && x >= s.alert) {
      out.push(t('systeme.infra.box.alerte_chaud', { name, label: s.label, temp: Math.round(x) }));
    }
  }
  for (const f of box.fans ?? []) {
    const x = read(f.point);
    if (typeof x === 'number' && typeof f.warn === 'number' && x >= f.warn) {
      out.push(t('systeme.infra.box.alerte_ventilateur', { name, label: f.label, rpm: Math.round(x) }));
    }
  }
  if (read(box.degraded) === true) out.push(t('systeme.infra.box.alerte_raid', { name }));
  const free = read(box.free);
  if (typeof free === 'number' && free <= 10) out.push(t('systeme.infra.box.alerte_plein', { name, pct: Math.round(100 - free) }));
  return out;
}
