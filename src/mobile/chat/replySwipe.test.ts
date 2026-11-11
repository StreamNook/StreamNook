import { describe, expect, it } from 'vitest';
import cases from './replySwipeCases.json';
import {
  REPLY_ARM,
  REPLY_DISARM,
  REPLY_DRAG_START,
  REPLY_ENGAGE,
  REPLY_HINT,
  REPLY_HINT_SHORT,
  REPLY_HINT_SHOWS,
  REPLY_ICON_FROM,
  REPLY_REACH,
  REPLY_RESISTANCE,
  SNAP_BACK_MS,
  canReplyTo,
  composerTakesText,
  notSentYet,
  replyAfterSend,
  replyArmed,
  replyHintWords,
  replyProgress,
  replyRowOf,
  replyShown,
  replyStep,
  snapBackMs,
  type ReplyDrag,
} from './replySwipe';
import { GRAB_ENGAGE, GRAB_RATIO, GRAB_ZONE } from './swipeSteps';

const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;

describe('the reply swipe follows the shared table', () => {
  it('uses the table numbers and words', () => {
    const n = cases.numbers;
    expect([REPLY_ENGAGE, REPLY_ARM, REPLY_DISARM, REPLY_REACH, REPLY_RESISTANCE, REPLY_ICON_FROM, SNAP_BACK_MS, REPLY_HINT_SHOWS]).toEqual([
      n.engage, n.arm, n.disarm, n.reach, n.resistance, n.iconFrom, n.snapBackMs, n.hintShows,
    ]);
    expect([GRAB_ZONE, GRAB_ENGAGE, GRAB_RATIO]).toEqual([n.grabZone, n.grabEngage, n.grabRatio]);
    expect([REPLY_HINT, REPLY_HINT_SHORT]).toEqual([cases.words.hint, cases.words.hintShort]);
  });

  it('moves the row, arms and fades the arrow in by the table', () => {
    for (const c of cases.shown) expect(near(replyShown(c.pull), c.shown), `shown ${c.pull}`).toBe(true);
    for (const c of cases.armed) expect(replyArmed(c.pull, c.was), `armed ${c.pull} ${c.was}`).toBe(c.armed);
    for (const c of cases.progress) expect(near(replyProgress(c.pull), c.progress), `progress ${c.pull}`).toBe(true);
  });

  it('slides back only when Motion keeps movement', () => {
    for (const c of cases.snapBack) expect(snapBackMs(c.motion)).toBe(c.ms);
  });

  it('runs whole swipes the way the phones do', () => {
    for (const s of cases.sequence) {
      let drag: ReplyDrag = REPLY_DRAG_START;
      const changes: boolean[] = [];
      for (const pull of s.pulls) {
        const step = replyStep(drag, pull);
        changes.push(step.armedChanged);
        drag = step.drag;
      }
      expect(changes, s.name).toEqual(s.armedChanges);
      expect(drag.armed, s.name).toBe(s.replies);
    }
  });
});

describe('what can be replied to', () => {
  it('follows the one rule for every route', () => {
    for (const c of cases.target) {
      const row = { id: c.id, username: c.username, system: c.system, systemMsg: c.systemMsg, held: c.held, provider: c.provider };
      const composer = { readOnly: c.readOnly, blocked: c.blocked, warning: c.warning, kickSignedOut: c.kickSignedOut };
      expect(canReplyTo(row, composer), c.id).toBe(c.want);
    }
  });

  it('counts emote-only as a room a reply can start in', () => {
    expect(composerTakesText({ readOnly: false, blocked: 'emote_only', warning: false, kickSignedOut: false })).toBe(true);
    expect(composerTakesText({ readOnly: false, blocked: 'subs_only', warning: false, kickSignedOut: false })).toBe(false);
  });

  it('reads a live line the way the rule needs it', () => {
    const tags = (entries: [string, string][]) => new Map(entries);
    const twitchRoom = { readOnly: false, blocked: null, warning: false, kickSignedOut: false };
    // A plain line takes the room's platform; a line that names its own keeps it.
    expect(replyRowOf('m1', { username: 'zoox', tags: tags([]) }, undefined, 'twitch')).toEqual({
      id: 'm1', username: 'zoox', system: false, systemMsg: false, held: false, provider: 'twitch',
    });
    expect(replyRowOf('m2', { username: 'zoox', tags: tags([]) }, 'youtube', 'twitch').provider).toBe('youtube');
    // A sub notice, the room's own notice and a nameless line cannot be replied to.
    const sub = replyRowOf('m3', { username: 'zoox', tags: tags([['system-msg', 'zoox subscribed']]) }, undefined, 'twitch');
    expect(sub.systemMsg).toBe(true);
    expect(canReplyTo(sub, twitchRoom)).toBe(false);
    expect(canReplyTo(replyRowOf('m4', { username: 'System', tags: tags([]) }, undefined, 'twitch'), twitchRoom)).toBe(false);
    expect(canReplyTo(replyRowOf('m5', { username: null, tags: tags([]) }, undefined, 'twitch'), twitchRoom)).toBe(false);
    expect(canReplyTo(replyRowOf('m6', { username: 'zoox', tags: tags([]) }, 'youtube', 'twitch'), twitchRoom)).toBe(false);
    expect(canReplyTo(replyRowOf('m7', { username: 'zoox', tags: tags([]) }, undefined, 'kick'), twitchRoom)).toBe(true);
  });

  it('a finished send clears only the reply it went out with', () => {
    for (const c of cases.afterSend) expect(replyAfterSend(c.draft, c.sent), `${c.draft} ${c.sent}`).toBe(c.clear);
  });
});

describe('the paused hint fits its column', () => {
  // A fake measurer: 6 px a character.
  const measure = (t: string) => t.length * 6;
  it('says the long words when they fit, the short ones otherwise', () => {
    expect(replyHintWords(REPLY_HINT.length * 6, measure)).toBe(REPLY_HINT);
    expect(replyHintWords(REPLY_HINT.length * 6 - 1, measure)).toBe(REPLY_HINT_SHORT);
    expect(replyHintWords(0, measure)).toBe(REPLY_HINT);
  });

  it('says right with Left-handed swipes, from the shared table', () => {
    expect(replyHintWords(999, measure, true)).toBe(cases.words.hintFlipped);
    expect(replyHintWords(cases.words.hintFlipped.length * 6 - 1, measure, true)).toBe(cases.words.hintShortFlipped);
  });
});

describe('a line Twitch has not echoed yet', () => {
  it('is known by its local id, so the swipe offers no Delete on it', () => {
    expect(notSentYet('local-1f2e')).toBe(true);
    expect(notSentYet('kick-local-9')).toBe(true);
    expect(notSentYet('b0c4e7d2-1a2b-4c3d-9e8f-0a1b2c3d4e5f')).toBe(false);
  });
});
