import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isEvCharger, statusOf, plugged, hoursMinutes, currentRange, fullScaleKw, gaugeFraction } from './evcharger.js';

const station = (extra = {}) => ({
  points: [
    { key: 'status' },
    { key: 'charging_enabled' },
    { key: 'set_current', kind: { min: 6, max: 16, step: 1 }, ...extra },
  ],
});

test('a station is told by its points', () => {
  assert.equal(isEvCharger(station()), true);
  assert.equal(isEvCharger({ points: [{ key: 'status' }] }), false);
  assert.equal(isEvCharger(null), false);
});

test('each state has a word and a tone, faults stand out', () => {
  assert.deepEqual(statusOf('charging'), { key: 'charging', tone: 'warm' });
  assert.deepEqual(statusOf('waiting_for_sun'), { key: 'waiting_for_sun', tone: 'cool' });
  assert.deepEqual(statusOf('overheating'), { key: 'overheating', tone: 'alert' });
  assert.deepEqual(statusOf('reserved15'), { key: null, tone: '' });
  assert.deepEqual(statusOf(null), { key: null, tone: '' });
  assert.equal(plugged('disconnected'), false);
  assert.equal(plugged('charged'), true);
});

test('times and the current range', () => {
  assert.equal(hoursMinutes(28), '0 min');
  assert.equal(hoursMinutes(3900), '1 h 05');
  assert.equal(hoursMinutes(-1), null);
  assert.deepEqual(currentRange(station()), { min: 6, max: 16, step: 1 });
  assert.deepEqual(currentRange({ points: [] }), { min: 6, max: 32, step: 1 });
});

test('the gauge: scale from the most current, fraction of it', () => {
  assert.equal(fullScaleKw({ max: 32 }), 7.4);
  assert.equal(fullScaleKw({ max: 16 }), 3.7);
  assert.equal(gaugeFraction(4600, 7.4).toFixed(3), '0.622');
  assert.equal(gaugeFraction(9000, 7.4), 1);
  assert.equal(gaugeFraction(-5, 7.4), 0);
  assert.equal(gaugeFraction(null, 7.4), 0);
});
