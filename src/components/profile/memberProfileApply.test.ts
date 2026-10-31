import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  noLive,
  planFirstPaint,
  planNamePaint,
  planPaintTheme,
  planPrefs,
  sevenTvBadgesStep,
  sevenTvCountsStep,
  type FirstPaint,
  type FirstPaintCard,
  type LivePaint,
  type PrefsRead,
} from './memberProfileApply';

const card = (over: Partial<FirstPaintCard> = {}): FirstPaintCard => ({
  profileTheme: 'dispersion',
  hiddenSections: ['roast'],
  memberNumber: 42,
  cosmeticSlug: 'streamnook-penrose',
  namePaint: { backgroundColor: '#9146FF', color: 'transparent' },
  seventvBadge: { id: '01KYQHQ7FVRR3X4S0F2KG74207', name: 'XDX', tooltip: 'XDX' },
  wornBadges: {
    twitch: { src: 'https://static-cdn.jtvnw.net/badges/v1/x/3', title: 'Twitch Recap 2025' },
    thirdParty: [{ key: 'ffz:ffz-3', provider: 'ffz', title: 'FFZ Supporter', src: 'https://cdn.frankerfacez.com/badge/3/4/rounded' }],
    bttvPro: { src: 'https://cdn.betterttv.net/badges/pro/x.webp', title: 'BTTV Pro' },
  },
  counts: { paints: 12, badges: 7, sn: 3 },
  ...over,
});

const snap = (over: Partial<FirstPaintCard> = {}): FirstPaint => ({
  identity: { login: 'golden_member', displayName: 'Golden_Member', avatar: 'https://static-cdn.jtvnw.net/a.png' },
  card: card(over),
});

const prefs = (over: Partial<PrefsRead> = {}): PrefsRead => ({ ok: true, profileTheme: 'tier', hiddenSections: [], ...over });

describe('planFirstPaint', () => {
  it('writes nothing for no answer', () => {
    expect(planFirstPaint(null, noLive(), false)).toEqual({ endLoading: false });
  });

  it('applies the whole card when nothing live has landed', () => {
    const plan = planFirstPaint(snap(), noLive(), false);
    expect(plan.info?.login).toBe('golden_member');
    expect(plan.counts).toEqual({ paints: 12, badges: 7, sn: 3 });
    expect(plan.namePaint).toEqual({ backgroundColor: '#9146FF', color: 'transparent' });
    expect(plan.hiddenSections).toEqual(['roast']);
    expect(plan.wornBadges?.seventv?.id).toBe('01KYQHQ7FVRR3X4S0F2KG74207');
    expect(plan.wornBadges?.thirdParty?.[0].provider).toBe('ffz');
    expect(plan.theme).toEqual({ profileTheme: 'dispersion', namePaint: plan.namePaint });
    expect(plan.endLoading).toBe(true);
  });

  it('with no row, paints the identity alone and ends loading', () => {
    const plan = planFirstPaint({ identity: snap().identity, card: null }, noLive(), false);
    expect(plan.info?.displayName).toBe('Golden_Member');
    expect(plan.counts).toBeUndefined();
    expect(plan.namePaint).toBeUndefined();
    expect(plan.hiddenSections).toBeUndefined();
    expect(plan.wornBadges).toBeUndefined();
    expect(plan.theme).toBeUndefined();
    expect(plan.endLoading).toBe(true);
  });

  it('leaves every field a live source already set alone', () => {
    const all = { info: true, counts: true, sn: true, paint: true, hidden: true, badges: true, otherBadges: true, theme: true };
    expect(planFirstPaint(snap(), all, false)).toEqual({ endLoading: false });
    expect(planFirstPaint(snap(), { ...noLive(), info: true }, false).info).toBeUndefined();
    expect(planFirstPaint(snap(), { ...noLive(), counts: true }, false).counts).toBeUndefined();
    expect(planFirstPaint(snap(), { ...noLive(), paint: true }, false).namePaint).toBeUndefined();
    expect(planFirstPaint(snap(), { ...noLive(), hidden: true }, false).hiddenSections).toBeUndefined();
    expect(planFirstPaint(snap(), { ...noLive(), badges: true }, false).wornBadges).toBeUndefined();
    expect(planFirstPaint(snap(), { ...noLive(), theme: true }, false).theme).toBeUndefined();
  });

  it('keeps the live preview badge row', () => {
    expect(planFirstPaint(snap(), noLive(), true).wornBadges).toBeUndefined();
  });

  it('a null paint is a real answer: the name has none', () => {
    expect(planFirstPaint(snap({ namePaint: null }), noLive(), false).namePaint).toBeNull();
  });
});

describe('planPrefs', () => {
  it('a failed read sets nothing and marks nothing live', () => {
    for (const failed of [null, prefs({ ok: false }), { profileTheme: 'tier' } as unknown as PrefsRead]) {
      expect(planPrefs(failed)).toEqual({ live: {} });
    }
  });

  it('a failed read leaves the card to hide what the member hides', () => {
    const live = noLive();
    Object.assign(live, planPrefs(prefs({ ok: false })).live);
    const plan = planFirstPaint(snap(), live, false);
    expect(plan.hiddenSections).toEqual(['roast']);
    expect(plan.theme?.profileTheme).toBe('dispersion');
  });

  it('an answered read wins over a card that lands later', () => {
    const live = noLive();
    const plan = planPrefs(prefs({ profileTheme: 'void', hiddenSections: ['twitch'] }));
    Object.assign(live, plan.live);
    expect(plan.hiddenSections).toEqual(['twitch']);
    expect(plan.theme).toEqual({ profileTheme: 'void' });
    const late = planFirstPaint(snap(), live, false);
    expect(late.hiddenSections).toBeUndefined();
    expect(late.theme).toBeUndefined();
  });

  it('answers a member with no saved row as the tier theme, live', () => {
    const plan = planPrefs(prefs());
    expect(plan.live).toEqual({ hidden: true, theme: true });
    expect(plan.hiddenSections).toEqual([]);
    expect(plan.theme).toEqual({ profileTheme: 'tier' });
  });

  it('the paint theme waits for the paint: hidden is live, the theme is not', () => {
    const plan = planPrefs(prefs({ profileTheme: 'paint', hiddenSections: ['roast'] }));
    expect(plan.live).toEqual({ hidden: true });
    expect(plan.theme).toBeUndefined();
  });
});

describe('planPaintTheme', () => {
  const paint = (over: Partial<LivePaint> = {}): LivePaint => ({ answered: true, hasPaint: true, ...over });

  it('draws the paint theme when the member wears a paint', () => {
    expect(planPaintTheme(prefs({ profileTheme: 'paint' }), paint())).toEqual({ live: { theme: true }, theme: 'paint' });
  });

  it('with an answered read and no paint, clears the theme and marks it live', () => {
    expect(planPaintTheme(prefs({ profileTheme: 'paint' }), paint({ hasPaint: false }))).toEqual({
      live: { theme: true },
      theme: null,
    });
  });

  it('after a failed 7TV read, leaves the theme alone and unmarked', () => {
    const live = noLive();
    const plan = planPaintTheme(prefs({ profileTheme: 'paint' }), { answered: false, hasPaint: false });
    expect(plan).toBeNull();
    Object.assign(live, plan?.live);
    expect(live.theme).toBe(false);
    // The first-paint card's paint aura still applies.
    expect(planFirstPaint(snap({ profileTheme: 'paint' }), live, false).theme?.profileTheme).toBe('paint');
  });

  it('does nothing when the prefs failed or name another theme', () => {
    expect(planPaintTheme(prefs({ ok: false, profileTheme: 'paint' }), paint({ hasPaint: false }))).toBeNull();
    expect(planPaintTheme(null, paint())).toBeNull();
    expect(planPaintTheme(prefs({ profileTheme: 'void' }), paint())).toBeNull();
  });

  it('a cleared paint theme blocks the stale card theme', () => {
    const live = noLive();
    Object.assign(live, planPaintTheme(prefs({ profileTheme: 'paint' }), paint({ hasPaint: false }))?.live);
    expect(planFirstPaint(snap({ profileTheme: 'paint' }), live, false).theme).toBeUndefined();
  });
});

describe('the live 7TV read steps', () => {
  const failedCounts = { paints: 0, badges: 0, sn: 5 };
  const onScreen = { paints: 12, badges: 7, sn: 3 };
  const cardBadge = { id: '01KYQHQ7FVRR3X4S0F2KG74207', name: 'XDX', tooltip: 'XDX' };
  type Row = {
    seventv: typeof cardBadge | null;
    twitch: { src: string; title: string } | null;
    thirdParty: unknown[];
    bttvPro: { src: string; title: string } | null;
  };
  const liveRow: Row = {
    seventv: null,
    twitch: { src: 'https://static-cdn.jtvnw.net/badges/v1/new/3', title: 'New' },
    thirdParty: [],
    bttvPro: null,
  };

  // What the hook does with a first-paint plan: merge its partial fields.
  const merge = <T extends object>(prev: T, part: Partial<T> | undefined): T => (part ? { ...prev, ...part } : prev);

  it('a failed inventory read keeps the 7TV counts on screen and marks only the StreamNook count live', () => {
    const step = sevenTvCountsStep({ answered: false, counts: failedCounts });
    expect(step.marks).toEqual({ sn: true });
    expect(step.apply(onScreen)).toEqual({ paints: 12, badges: 7, sn: 5 });
    // With no card yet the StreamNook count still shows.
    expect(step.apply({ paints: 0, badges: 0, sn: 0 })).toEqual({ paints: 0, badges: 0, sn: 5 });
  });

  it('an answered read sets its counts, even zero, and marks them all live', () => {
    const step = sevenTvCountsStep({ answered: true, counts: failedCounts });
    expect(step.marks).toEqual({ counts: true, sn: true });
    expect(step.apply(onScreen)).toEqual(failedCounts);
  });

  it('a failed read keeps the 7TV badge on screen and marks only the other badges live', () => {
    const step = sevenTvBadgesStep({ answered: false, row: liveRow });
    expect(step.marks).toEqual({ otherBadges: true });
    const before = { seventv: cardBadge, twitch: null, thirdParty: [], bttvPro: null };
    expect(step.apply(before)).toEqual({ ...liveRow, seventv: cardBadge });
  });

  it('an answered read replaces the whole row, a missing 7TV badge included, and marks it live', () => {
    const step = sevenTvBadgesStep({ answered: true, row: liveRow });
    expect(step.marks).toEqual({ badges: true, otherBadges: true });
    expect(step.apply({ seventv: cardBadge, twitch: null, thirdParty: [], bttvPro: null })).toEqual(liveRow);
  });

  it('failed read first, card second: the card fills only the 7TV parts', () => {
    const live = noLive();
    const counts = sevenTvCountsStep({ answered: false, counts: failedCounts });
    const badges = sevenTvBadgesStep({ answered: false, row: liveRow });
    Object.assign(live, counts.marks, badges.marks);
    const countsOnScreen = counts.apply({ paints: 0, badges: 0, sn: 0 });
    const rowOnScreen = badges.apply({ seventv: null, twitch: null, thirdParty: [], bttvPro: null });

    const plan = planFirstPaint(snap(), live, false);
    expect(merge(countsOnScreen, plan.counts)).toEqual({ paints: 12, badges: 7, sn: 5 });
    expect(merge(rowOnScreen, plan.wornBadges as Partial<Row>)).toEqual({ ...liveRow, seventv: cardBadge });
  });

  it('card first, failed read second: the card stays where the read did not answer', () => {
    const live = noLive();
    const plan = planFirstPaint(snap(), live, false);
    const cardCounts = merge({ paints: 0, badges: 0, sn: 0 }, plan.counts);
    const cardRow = merge({ seventv: null, twitch: null, thirdParty: [], bttvPro: null } as Row, plan.wornBadges as Partial<Row>);
    expect(sevenTvCountsStep({ answered: false, counts: failedCounts }).apply(cardCounts)).toEqual({ paints: 12, badges: 7, sn: 5 });
    expect(sevenTvBadgesStep({ answered: false, row: liveRow }).apply(cardRow)).toEqual({ ...liveRow, seventv: cardBadge });
  });

  it('the card hook takes its answers from the profile and applies these steps as they are', () => {
    const hook = readFileSync(resolve(__dirname, 'memberProfile.ts'), 'utf8');
    expect(hook.match(/live\.(counts|badges|sn|otherBadges) = true/g) ?? []).toHaveLength(0);
    expect(hook).not.toContain('isUserCosmeticsHardFailed');
    expect(hook).toContain('const paintAnswered = profile.wornAnswered === true;');
    expect(hook).toContain('answered: profile.inventoryAnswered === true,');
    expect(hook).toContain('Object.assign(live, countsStep.marks);');
    expect(hook).toContain('setCounts(countsStep.apply);');
    expect(hook).toContain('Object.assign(live, badgesStep.marks);');
    expect(hook).toContain('setWornBadges(badgesStep.apply);');
    // The Settings preview re-resolve uses the same step.
    expect(hook).toContain('sevenTvBadgesStep<typeof wornBadges>({ answered: profile.wornAnswered === true, row: wb });');
    // A first paint is merged, never set whole.
    expect(hook).toContain('setCounts((prev) => ({ ...prev, ...cardCounts }));');
    expect(hook).toContain('setWornBadges((prev) => ({ ...prev, ...cardBadges }));');
  });
});

describe('planNamePaint', () => {
  const style = { backgroundColor: '#9146FF', color: 'transparent' };

  it('keeps the first-paint name paint after a failed 7TV read', () => {
    const plan = planNamePaint({ answered: false, paintStyle: null });
    expect(plan).toEqual({ set: false });
    // Nothing marks the paint live, so the card's paint still applies.
    const live = noLive();
    expect(planFirstPaint(snap(), live, false).namePaint).toEqual({ backgroundColor: '#9146FF', color: 'transparent' });
  });

  it('an answered read with no paint clears the name paint', () => {
    expect(planNamePaint({ answered: true, paintStyle: null })).toEqual({ set: true, value: null });
  });

  it('an answered read with a paint sets it', () => {
    expect(planNamePaint({ answered: true, paintStyle: style })).toEqual({ set: true, value: style });
  });
});
