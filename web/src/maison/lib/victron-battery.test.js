import { test } from 'node:test';
import assert from 'node:assert/strict';
import { installations, isInverter, rangeOf, limitOf, sliderStart, snap, flowOf } from './victron-battery.js';

const p = (key, kind = { type: 'numeric' }, write = false) => ({ key, kind, access: { read: true, write } });
const battery = { id: 'victron:battery_512', instance: 'victron', points: [p('soc'), p('power'), p('voltage')] };
const inverter = {
  id: 'victron:vebus_288',
  instance: 'victron',
  points: [p('state'), p('mode', { type: 'enum', values: ['charger_only', 'inverter_only', 'on', 'off'] }, true)],
};
const settings = { id: 'victron:settings_0', instance: 'victron', points: [p('minimum_soc', { type: 'numeric', min: 10, max: 100, step: 5 }, true)] };
const station = { id: 'victron:evcharger_40', instance: 'victron', points: [p('status'), p('mode', { type: 'enum', values: ['manual', 'auto'] }, true)] };

test('one installation per GX, with its battery, inverter and settings', () => {
  const list = installations([station, battery, settings, inverter]);
  assert.equal(list.length, 1);
  assert.equal(list[0].battery.id, battery.id);
  assert.equal(list[0].inverter.id, inverter.id);
  assert.equal(list[0].settings.id, settings.id);
  assert.equal(isInverter(station), false, "a station's mode is not an inverter's");
  // A battery alone has nothing to steer.
  assert.deepEqual(installations([battery]), []);
});

test('ranges, ceilings and snapping', () => {
  assert.deepEqual(rangeOf(settings, 'minimum_soc', { min: 0, max: 1, step: 1 }), { min: 10, max: 100, step: 5 });
  assert.deepEqual(rangeOf(settings, 'nothing', { min: 0, max: 10000, step: 100 }), { min: 0, max: 10000, step: 100 });
  assert.equal(limitOf(-1), null);
  assert.equal(limitOf(2500), 2500);
  assert.equal(limitOf(null), undefined);
  const r = { min: 0, max: 10000, step: 100 };
  assert.equal(sliderStart(null, r), 10000);
  assert.equal(sliderStart(2500, r), 2500);
  assert.equal(snap(2549, r), 2500);
  assert.equal(snap(20000, r), 10000);
  assert.equal(snap(-37, { min: -5000, max: 5000, step: 10 }), -40);
  assert.equal(snap(Number.NaN, r), null);
});

test('the battery charges, gives back or rests', () => {
  assert.equal(flowOf(1200), 'charge');
  assert.equal(flowOf(-800), 'discharge');
  assert.equal(flowOf(12), 'idle');
  assert.equal(flowOf(null), null);
});
