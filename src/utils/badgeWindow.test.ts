// Run with: npm test
// Node strips the types natively, so no test runner is needed.
//
// Each case is a real window shape seen in production. The clock is fixed so
// classification is asserted against a known instant, not against today.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import {
  parseDateRange,
  deriveBadgeStatus,
  decodeHtmlEntities,
  formatBadgeDateInfo,
} from './badgeWindow.ts';

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

test('parses a full-month range with years', () => {
  const range = parseDateRange('Event duration: December 6, 2025 – December 7, 2025');
  assert.ok(range);
  assert.equal(range.start.getFullYear(), 2025);
  assert.equal(range.start.getMonth(), 11);
  assert.equal(range.start.getDate(), 6);
  assert.equal(range.end.getDate(), 7);
});

test('parses an abbreviated range that crosses into the next year', () => {
  const range = parseDateRange('Event duration: Dec 19 – Jan 01');
  assert.ok(range);
  assert.equal(range.end.getFullYear(), range.start.getFullYear() + 1);
});

test('parses an ISO range', () => {
  const range = parseDateRange('Event duration: 2026-07-08T13:00:00Z - 2026-08-24T13:00:00Z');
  assert.ok(range);
  assert.equal(range.start.toISOString(), '2026-07-08T13:00:00.000Z');
  assert.equal(range.end.toISOString(), '2026-08-24T13:00:00.000Z');
});

test('parses an AM/PM range', () => {
  const range = parseDateRange('December 4, 2025 at 7:00 AM – December 4, 2025 at 11:59 PM');
  assert.ok(range);
  assert.equal(range.start.getHours(), 7);
  assert.equal(range.end.getHours(), 23);
});

test('parses an abbreviated same-month range', () => {
  const range = parseDateRange('Event duration: Jul 23 - Jul 25');
  assert.ok(range);
  assert.equal(range.start.getMonth(), 6);
  assert.equal(range.start.getDate(), 23);
  assert.equal(range.end.getDate(), 25);
});

test('a campaign window beats anything written in the prose', () => {
  const status = deriveBadgeStatus(
    'Event duration: Jan 01 - Jan 02',
    { starts_utc: '2026-07-24T07:00:00Z', ends_utc: '2026-07-27T08:00:00Z' },
    at('2026-07-24T22:00:00Z')
  );
  assert.equal(status, 'available');
});

test('reclassifies a badge across its window with no new push', () => {
  const window = { starts_utc: '2026-07-24T07:00:00Z', ends_utc: '2026-07-27T08:00:00Z' };
  assert.equal(deriveBadgeStatus(null, window, at('2026-07-24T06:46:00Z')), 'coming-soon');
  assert.equal(deriveBadgeStatus(null, window, at('2026-07-24T22:51:00Z')), 'available');
  assert.equal(deriveBadgeStatus(null, window, at('2026-07-28T00:00:00Z')), 'expired');
});

test('an open-ended window is available once it has started', () => {
  assert.equal(
    deriveBadgeStatus(null, { starts_utc: '2026-07-25T17:00:00Z' }, at('2026-07-26T00:00:00Z')),
    'available'
  );
  assert.equal(
    deriveBadgeStatus(null, { starts_utc: '2026-07-25T17:00:00Z' }, at('2026-07-24T00:00:00Z')),
    'coming-soon'
  );
});

test('falls back to ISO stamps in the copy when there is no campaign', () => {
  const status = deriveBadgeStatus(
    'Event duration: 2026-07-08T13:00:00Z - 2026-08-24T13:00:00Z',
    null,
    at('2026-07-24T00:00:00Z')
  );
  assert.equal(status, 'available');
});

// Every string below is copied verbatim from a real badge's badgebase entry.
// Before these were handled, 24 badges rendered no window at all.
test('parses full month names, not just three-letter abbreviations', () => {
  for (const [text, label] of [
    ['during the campaign period (December 2 – December 13)', 'clip-the-halls'],
    ['Event time: July 24 – July 25', 'budz'],
    ['during the event: July 10 – July 12', 'dreamers'],
    ['Time window: June 24 – July 12, 2025', 'league-of-legends-msi-grey'],
  ] as const) {
    const range = parseDateRange(text);
    assert.ok(range, `failed to parse ${label}: ${text}`);
  }
});

test('parses "between X and Y" and "from X to Y" joiners', () => {
  const between = parseDateRange('share a clip from the category between May 29 and June 3');
  assert.ok(between);
  assert.equal(between.start.getMonth(), 4);
  assert.equal(between.end.getMonth(), 5);

  const from = parseDateRange('the campaign ran from February 27 to March 3');
  assert.ok(from);
  assert.equal(from.start.getMonth(), 1);
  assert.equal(from.end.getMonth(), 2);
});

test('parses ordinal days and same-month compact ranges', () => {
  const ordinal = parseDateRange('completed the survey from July 26th to July 28th, 2024');
  assert.ok(ordinal);
  assert.equal(ordinal.start.getFullYear(), 2024);
  assert.equal(ordinal.start.getDate(), 26);

  const compact = parseDateRange('the “Together For Good” campaign (December 3–15)');
  assert.ok(compact);
  assert.equal(compact.start.getDate(), 3);
  assert.equal(compact.end.getDate(), 15);

  const trailing = parseDateRange('Event window : June 28–29 This is the first time');
  assert.ok(trailing);
  assert.equal(trailing.end.getDate(), 29);
});

test('a leading non-month word does not block a later real range', () => {
  const range = parseDateRange('the campaign period 2 to June 3 was extended, running May 29 to June 3');
  assert.ok(range);
  assert.equal(range.start.getMonth(), 4);
});

test('a split campaign is not earnable in the gap between its runs', () => {
  // borderlands-4-ripper: two disjoint releases, months apart.
  const copy =
    'First Release: from 2025-06-21T15:00:00Z to 2025-06-22T00:00:00Z ' +
    'Second Release: from 2025-09-11T12:00:00Z to 2025-09-15T06:59:00Z';
  assert.equal(deriveBadgeStatus(copy, null, at('2025-06-21T18:00:00Z')), 'available');
  assert.equal(deriveBadgeStatus(copy, null, at('2025-07-15T00:00:00Z')), 'coming-soon');
  assert.equal(deriveBadgeStatus(copy, null, at('2025-09-12T00:00:00Z')), 'available');
  assert.equal(deriveBadgeStatus(copy, null, at('2025-10-01T00:00:00Z')), 'expired');
});

test('a permanent badge has no window and no status', () => {
  assert.equal(deriveBadgeStatus('Given to channel subscribers.', null), null);
  assert.equal(deriveBadgeStatus(null, null), null);
  assert.equal(deriveBadgeStatus(undefined, {}), null);
});

test('ignores a malformed campaign timestamp rather than inventing a status', () => {
  assert.equal(deriveBadgeStatus(null, { starts_utc: 'not a date' }, at('2026-07-24T00:00:00Z')), null);
});
