import { test } from 'node:test';
import assert from 'node:assert/strict';
import { reader, mbits, boxIssues } from './box.js';

const values = {
  'ha:box-disk/temperature': 56,
  'ha:box-cpu/temperature': 60,
  'ha:box-fan/value': 1900,
  'ha:box-raid/value': false,
  'ha:box-free/value': 55,
};
const read = reader((device, key) => values[`${device}/${key}`] ?? null);
const t = (key, p) => `${key} ${JSON.stringify(p)}`;

const box = {
  name: 'Box',
  degraded: 'ha:box-raid/value',
  free: 'ha:box-free/value',
  temperatures: [
    { label: 'Disque', point: 'ha:box-disk/temperature', warn: 50, alert: 55 },
    { label: 'CPU', point: 'ha:box-cpu/temperature', warn: 80, alert: 90 },
  ],
  fans: [{ label: 'Ventilateur', point: 'ha:box-fan/value', warn: 1800 }],
};

test('a point reads through its device and key', () => {
  assert.equal(read('ha:box-cpu/temperature'), 60);
  assert.equal(read('nothing'), null);
  assert.equal(read(undefined), null);
});

test('rates become Mbit/s', () => {
  assert.equal(mbits(1000, 'kB/s'), 8);
  assert.equal(mbits(7.03), 0.1);
  assert.equal(mbits(50, 'Mbit/s'), 50);
  assert.equal(mbits(null), null);
});

test('a hot disk and a fast fan need a look; the rest is fine', () => {
  const issues = boxIssues(box, read, t);
  assert.equal(issues.length, 2);
  assert.match(issues[0], /alerte_chaud.*Disque.*56/);
  assert.match(issues[1], /alerte_ventilateur.*1900/);
});

test('a degraded array and a full disk are said; no box, nothing', () => {
  values['ha:box-raid/value'] = true;
  values['ha:box-free/value'] = 5;
  const issues = boxIssues(box, read, t);
  assert.ok(issues.some((i) => i.includes('alerte_raid')));
  assert.ok(issues.some((i) => i.includes('alerte_plein') && i.includes('95')));
  assert.deepEqual(boxIssues(null, read, t), []);
});
