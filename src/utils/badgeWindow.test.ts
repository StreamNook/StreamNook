// Run with: npm test
// Node strips the types natively, so no test runner is needed.
//
// Display formatting only. Window parsing and classification moved to Rust
// (src-tauri/src/services/badge_window.rs), where their cases are tested.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { decodeHtmlEntities, formatBadgeDateInfo } from './badgeWindow.ts';

const at = (iso: string) => new Date(iso).getTime();

// Asserted by property, not against a literal string: the output is
// locale- and timezone-dependent, so a fixed expectation would only pass in the
// zone it was written in.
test('date_info: an ISO stamp is rendered as local wall-clock text', () => {
  const out = formatBadgeDateInfo('2026-07-24T07:00:00Z');
  assert.ok(!out.includes('T'), `still machine-shaped: ${out}`);
  assert.ok(!out.includes('Z'), `still carries a zone suffix: ${out}`);
  assert.ok(out.length > 0);
});

test('date_info: a stamp with no zone suffix is read as UTC, not as local', () => {
  assert.equal(
    formatBadgeDateInfo('2026-07-24T07:00'),
    formatBadgeDateInfo('2026-07-24T07:00:00Z')
  );
});

test('date_info: prose windows are already readable and pass through', () => {
  assert.equal(formatBadgeDateInfo('Dec 1-12'), 'Dec 1-12');
  assert.equal(formatBadgeDateInfo(''), '');
  assert.equal(formatBadgeDateInfo(undefined), '');
});

test('date_info: the year appears only when it is not the current one', () => {
  const stamp = '2026-07-24T07:00:00Z';
  assert.ok(!formatBadgeDateInfo(stamp, at('2026-11-01T00:00:00Z')).includes('2026'));
  // Mid-year on purpose: a Jan 1 UTC instant is still the previous year in any
  // zone behind UTC, which would make this assertion pass or fail by locale.
  assert.ok(formatBadgeDateInfo(stamp, at('2027-06-01T00:00:00Z')).includes('2026'));
});

test('decodes the entities that scraped copy arrives with', () => {
  assert.equal(decodeHtmlEntities('Dec 6 &ndash; Dec 7'), 'Dec 6 – Dec 7');
  assert.equal(decodeHtmlEntities('Chaos Orb &#8220;PoE2&#8221;'), 'Chaos Orb “PoE2”');
});
