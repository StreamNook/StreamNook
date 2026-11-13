// The automations page's and the chat dot's pure helpers.
import { describe, expect, it } from 'vitest';
import type { AutomationAccountRef } from '../types';
import {
  actedText,
  agoText,
  applyEntry,
  blankRule,
  chatLabel,
  dotStateFor,
  duplicateRule,
  errorAt,
  errorsUnder,
  eventSample,
  groupActivity,
  moveBeforeId,
  previewSample,
  sameRuleBody,
  secondsText,
  spanText,
  splitNames,
  withTrigger,
  type AutomationActivityEntry,
} from './automations';

const me: AutomationAccountRef = { provider: 'twitch', id: '100', login: 'me', name: 'Me' };

type DotStatus = NonNullable<Parameters<typeof dotStateFor>[0]>;
const status = (over: Partial<DotStatus> = {}): DotStatus => ({
  run: 'on',
  live_chats: ['twitch:xqc'],
  live_any: false,
  outage: false,
  ...over,
});

describe('dotStateFor', () => {
  it('shows only while run is On and a Live rule covers the chat', () => {
    expect(dotStateFor(null, ['twitch:xqc'])).toBeNull();
    expect(dotStateFor(status({ run: 'test' }), ['twitch:xqc'])).toBeNull();
    expect(dotStateFor(status(), ['twitch:xqc'])).toBe('on');
    expect(dotStateFor(status(), ['twitch:lirik'])).toBeNull();
    expect(dotStateFor(status(), [])).toBeNull();
  });

  it('compares keys through makeKey', () => {
    expect(dotStateFor(status(), ['XQC'])).toBe('on');
    expect(dotStateFor(status({ live_chats: ['xqc'] }), ['twitch:xqc'])).toBe('on');
    expect(dotStateFor(status(), ['kick:xqc'])).toBeNull();
  });

  it('covers a combined chat when any member room is covered', () => {
    expect(dotStateFor(status(), ['kick:xqc', 'twitch:xqc'])).toBe('on');
  });

  it('covers every chat when a rule answers anywhere, and goes hollow while StreamNook cannot be reached', () => {
    expect(dotStateFor(status({ live_chats: [], live_any: true }), ['kick:someone'])).toBe('on');
    expect(dotStateFor(status({ outage: true }), ['twitch:xqc'])).toBe('waiting');
  });
});

describe('moveBeforeId', () => {
  const ids = ['a', 'b', 'c', 'd'];
  it('names the rule the moved one lands before, or null for the end', () => {
    expect(moveBeforeId(ids, 2, -1)).toEqual({ beforeId: 'b' });
    expect(moveBeforeId(ids, 1, 1)).toEqual({ beforeId: 'd' });
    expect(moveBeforeId(ids, 2, 1)).toEqual({ beforeId: null });
  });
  it('moves nothing past either end', () => {
    expect(moveBeforeId(ids, 0, -1)).toBeNull();
    expect(moveBeforeId(ids, 3, 1)).toBeNull();
  });
});

const entry = (id: number, kind: AutomationActivityEntry['kind'], at = 1): AutomationActivityEntry => ({
  id,
  at_ms: at,
  kind,
  first_at_ms: at,
  rule_id: 'r-1',
  rule_name: 'Away reply',
  channel: 'twitch:xqc',
  text: 'Replied to @x in #xqc',
  count: 1,
  replaces: 0,
});

describe('Activity', () => {
  it('splits today from earlier by local midnight', () => {
    const now = new Date(2026, 10, 12, 15, 0).getTime();
    const morning = new Date(2026, 10, 12, 9, 0).getTime();
    const lastNight = new Date(2026, 10, 11, 23, 30).getTime();
    const { today, earlier } = groupActivity([entry(2, 'acted', morning), entry(1, 'test', lastNight)], now);
    expect(today.map((e) => e.id)).toEqual([2]);
    expect(earlier.map((e) => e.id)).toEqual([1]);
  });

  it('puts a live entry on top, leaves out what the filter hides, and moves a folded skip up', () => {
    expect(applyEntry([entry(1, 'acted')], entry(2, 'skipped'), 'acted').map((e) => e.id)).toEqual([1]);
    // A skip that folds into an older twin comes back with a new id and names the id it replaced.
    const folded = applyEntry([entry(1, 'acted'), entry(2, 'skipped')], { ...entry(5, 'skipped'), count: 3, replaces: 2 }, 'all');
    expect(folded.map((e) => [e.id, e.count])).toEqual([
      [5, 3],
      [1, 1],
    ]);
    expect(applyEntry([], entry(3, 'notice'), 'all')).toHaveLength(1);
    expect(applyEntry([], entry(3, 'notice'), 'test')).toHaveLength(0);
  });

  it('names a chat by its login', () => {
    expect(chatLabel('twitch:xqc')).toBe('#xqc');
    expect(chatLabel('lirik')).toBe('#lirik');
  });
});

describe('field errors', () => {
  const errors = [
    { path: 'name', message: 'Give the rule a name' },
    { path: 'actions[0].texts[1].text', message: 'Write the message' },
    { path: 'actions[1]', message: 'Badge rules can post a message or notify you, not reply' },
  ];
  it('finds the error at a field and every error inside a part', () => {
    expect(errorAt(errors, 'name')).toBe('Give the rule a name');
    expect(errorAt(errors, 'actions[0]')).toBeNull();
    expect(errorsUnder(errors, 'actions[0]').map((e) => e.path)).toEqual(['actions[0].texts[1].text']);
    expect(errorsUnder(errors, 'actions[1]')).toHaveLength(1);
  });
});

describe('rule drafts', () => {
  it('copies a rule as a new one in Test with fresh ids, leaving the original alone', () => {
    const rule = { ...blankRule(me), id: 'r-000000000001', name: 'Away reply', live: true, created_ms: 5, updated_ms: 9 };
    const copy = duplicateRule(rule);
    expect(copy).toMatchObject({ id: '', name: 'Away reply copy', live: false, enabled: true, created_ms: 0, updated_ms: 0 });
    expect(copy.actions[0].id).not.toBe(rule.actions[0].id);
    expect(copy.actions[0].id).toMatch(/^a-[0-9a-f]{8}$/);
    const [first] = copy.actions;
    if (first.kind !== 'reply') throw new Error('expected a reply');
    expect(first.texts[0].id).toMatch(/^l-[0-9a-f]{8}$/);
    expect(rule.name).toBe('Away reply');
  });

  it('makes a badge rule what Rust requires, and gives the chat cooldown back after', () => {
    const badge = withTrigger(blankRule(me), 'badge_drop');
    expect(badge.only_if.channels).toEqual({ kind: 'own' });
    expect(badge.only_if.only_live).toBe(true);
    expect(badge.limits.rule_cooldown_seconds).toBe(0);
    expect(badge.actions.map((a) => a.kind)).toEqual(['send']);
    expect(withTrigger(badge, 'mention').limits.rule_cooldown_seconds).toBe(30);
  });

  it('ignores the switch, Test or Live and the stamps when telling whether a rule changed', () => {
    const rule = blankRule(me);
    expect(sameRuleBody(rule, { ...rule, enabled: false, live: true, updated_ms: 7 })).toBe(true);
    expect(sameRuleBody(rule, { ...rule, name: 'Other' })).toBe(false);
  });

  it('reads names typed as one line', () => {
    expect(splitNames('@Foo, bar  baz,foo')).toEqual(['foo', 'bar', 'baz']);
  });
});

describe('previewSample', () => {
  it('makes each trigger act', () => {
    const mention = previewSample(blankRule(me));
    expect(mention.text).toContain('@me');
    expect(mention.channel).toBe('me');
    expect(mention.draft?.when.kind).toBe('mention');

    const raid = previewSample({
      ...blankRule(me),
      when: { kind: 'channel_event', events: ['raid'], min_months: 0, min_gifts: 0, min_viewers: 500, min_bits: 0 },
    });
    expect(raid.event).toMatchObject({ kind: 'raid', viewers: 500 });

    expect(previewSample(withTrigger(blankRule(me), 'first_message')).first_message).toBe(true);

    const base = blankRule(me);
    const mods = previewSample({ ...base, only_if: { ...base.only_if, roles: ['moderator', 'vip'] } });
    expect(mods.roles).toEqual(['moderator']);
  });

  it("gives Try it's event its one number", () => {
    expect(eventSample('cheer', 300)).toEqual({ kind: 'cheer', months: 0, gifts: 0, viewers: 0, bits: 300 });
    expect(eventSample('resub', 12).months).toBe(12);
  });
});

describe('time words', () => {
  const now = new Date(2026, 10, 12, 15, 0).getTime();
  it('says how long ago', () => {
    expect(agoText(now - 30_000, now)).toBe('just now');
    expect(agoText(now - 3 * 60_000, now)).toBe('3 min ago');
    expect(agoText(now - 2 * 3_600_000, now)).toBe('2 h ago');
    expect(agoText(now - 26 * 3_600_000, now)).toBe('yesterday');
  });
  it('says how long, a gap and a record', () => {
    expect(spanText(now - 20_000, now)).toBe('less than a minute');
    expect(spanText(now - 12 * 60_000, now)).toBe('12 minutes');
    expect(secondsText(30)).toBe('30 s');
    expect(secondsText(90)).toBe('1.5 min');
    expect(secondsText(600)).toBe('10 min');
    expect(actedText(0, null, now)).toBe("Hasn't acted yet");
    expect(actedText(1, now - 120_000, now)).toBe('Acted once, last 2 min ago');
    expect(actedText(12, now - 120_000, now)).toBe('Acted 12 times, last 2 min ago');
  });
});
