import { test } from 'node:test';
import assert from 'node:assert/strict';
import { prefsOf, index, sumBetween, rowsOf } from './moli-hass.js';

const H = 3_600_000;
const D = 24 * H;
const report = (starts, kwh) => ({ buckets: starts.map((s) => ({ start: s, meters: { a: { kwh } } })) });

test('the meters become HA energy sources', () => {
  const p = prefsOf({
    price_now: 0.2,
    meters: [
      { id: 'achat', role: 'total' },
      { id: 'injection', role: 'export' },
      { id: 'toit', role: 'solar' },
      { id: 'in', role: 'charge' },
      { id: 'out', role: 'discharge' },
      { id: 'frigo', role: 'appliance' },
    ],
  });
  assert.deepEqual(p.energy_sources, [
    { type: 'grid', stat_energy_from: 'achat', stat_energy_to: 'injection', number_energy_price: 0.2 },
    { type: 'solar', stat_energy_from: 'toit' },
    { type: 'battery', stat_energy_to: 'in', stat_energy_from: 'out' },
  ]);
});

test('a period adds whole months, then days, then hours', () => {
  // Two « months » of 3 days, days, and hours, all starting at 0.
  const series = {
    month: index(report([0, 3 * D], 100)),
    day: index(report([0, D, 2 * D, 3 * D, 4 * D, 5 * D], 10)),
    hour: index(report(Array.from({ length: 6 * 24 }, (_, i) => i * H), 1)),
  };
  // A whole month (100) + one day (10) + 5 hours (5).
  assert.equal(sumBetween(series, 'a', 0, 4 * D + 5 * H), 115);
  // Starting mid-day: hours up to the next day, then days.
  assert.equal(sumBetween(series, 'a', 20 * H, 2 * D), 4 + 10);
  assert.equal(sumBetween(series, 'nothing', 0, D), null);
});

test('rows per meter from a start', () => {
  const ix = index(report([0, D, 2 * D], 3));
  assert.deepEqual(rowsOf(ix, ['a', 'b'], D), { a: [{ start: D, change: 3 }, { start: 2 * D, change: 3 }], b: [] });
});
