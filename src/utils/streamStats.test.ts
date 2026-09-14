// Run with: npm test
//
// The uptime clock feeds both the chat header and the Compact View chips, so a
// mistake at the hour boundary or in the zero-padding is visible every second.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { formatViewerCount, formatUptimeClock } from './streamStats.ts';

const NOW = Date.parse('2026-08-20T12:00:00.000Z');
const startedAgo = (ms: number) => new Date(NOW - ms).toISOString();

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;

test('switches to H:MM:SS exactly at the first hour', () => {
  assert.equal(formatUptimeClock(startedAgo(59 * MINUTE + 59 * SECOND), NOW), '59:59');
  assert.equal(formatUptimeClock(startedAgo(HOUR), NOW), '1:00:00');
});

test('pads minutes and seconds to two digits', () => {
  assert.equal(formatUptimeClock(startedAgo(HOUR + 2 * MINUTE + 3 * SECOND), NOW), '1:02:03');
  assert.equal(formatUptimeClock(startedAgo(5 * MINUTE + 7 * SECOND), NOW), '5:07');
});

test('does not pad the leading unit', () => {
  assert.equal(formatUptimeClock(startedAgo(7 * SECOND), NOW), '0:07');
  assert.equal(formatUptimeClock(startedAgo(12 * HOUR + 34 * MINUTE + 56 * SECOND), NOW), '12:34:56');
});

test('returns empty for a missing or unusable start time', () => {
  assert.equal(formatUptimeClock(undefined, NOW), '');
  assert.equal(formatUptimeClock(null, NOW), '');
  assert.equal(formatUptimeClock('', NOW), '');
  assert.equal(formatUptimeClock('not a timestamp', NOW), '');
});

test('clamps a start time in the future instead of going negative', () => {
  assert.equal(formatUptimeClock(startedAgo(-5 * MINUTE), NOW), '0:00');
  assert.equal(formatUptimeClock(startedAgo(0), NOW), '0:00');
});

test('abbreviates viewer counts at the thousand and million boundaries', () => {
  assert.equal(formatViewerCount(0), '0');
  assert.equal(formatViewerCount(999), '999');
  assert.equal(formatViewerCount(1000), '1.0K');
  assert.equal(formatViewerCount(12300), '12.3K');
  assert.equal(formatViewerCount(999999), '1000.0K');
  assert.equal(formatViewerCount(1000000), '1.0M');
  assert.equal(formatViewerCount(1234567), '1.2M');
});
