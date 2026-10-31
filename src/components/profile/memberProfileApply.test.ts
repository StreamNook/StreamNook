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
    const all = {
      info: true, counts: true, sn: true, paint: true, hidden: true,
      seventvBadge: true, twitchBadge: true, thirdPartyBadges: true, bttvBadge: true, theme: true,
    };
    expect(planFirstPaint(snap(), all, false)).toEqual({ endLoading: false });
    expect(planFirstPaint(snap(), { ...noLive(), info: true }, false).info).toBeUndefined();
    expect(planFirstPaint(snap(), { ...noLive(), counts: true, sn: true }, false).counts).toBeUndefined();
    expect(planFirstPaint(snap(), { ...noLive(), counts: true }, false).counts).toEqual({ sn: 3 });
    expect(planFirstPaint(snap(), { ...noLive(), sn: true }, false).counts).toEqual({ paints: 12, badges: 7 });
    expect(planFirstPaint(snap(), { ...noLive(), paint: true }, false).namePaint).toBeUndefined();
    expect(planFirstPaint(snap(), { ...noLive(), hidden: true }, false).hiddenSections).toBeUndefined();
    const badgesLive = { seventvBadge: true, twitchBadge: true, thirdPartyBadges: true, bttvBadge: true };
    expect(planFirstPaint(snap(), { ...noLive(), ...badgesLive }, false).wornBadges).toBeUndefined();
    expect(Object.keys(planFirstPaint(snap(), { ...noLive(), seventvBadge: true }, false).wornBadges ?? {}).sort()).toEqual(['bttvPro', 'thirdParty', 'twitch']);
    expect(Object.keys(planFirstPaint(snap(), { ...noLive(), ...badgesLive, seventvBadge: false }, false).wornBadges ?? {})).toEqual(['seventv']);
    expect(Object.keys(planFirstPaint(snap(), { ...noLive(), ...badgesLive, twitchBadge: false }, false).wornBadges ?? {})).toEqual(['twitch']);
    expect(Object.keys(planFirstPaint(snap(), { ...noLive(), ...badgesLive, thirdPartyBadges: false }, false).wornBadges ?? {})).toEqual(['thirdParty']);
    expect(Object.keys(planFirstPaint(snap(), { ...noLive(), ...badgesLive, bttvBadge: false }, false).wornBadges ?? {})).toEqual(['bttvPro']);
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

  const emptyRow: Row = { seventv: null, twitch: null, thirdParty: [], bttvPro: null };
  const cardRow: Row = {
    seventv: cardBadge,
    twitch: { src: 'https://static-cdn.jtvnw.net/badges/v1/x/3', title: 'Twitch Recap 2025' },
    thirdParty: [{ key: 'ffz:ffz-3', provider: 'ffz', title: 'FFZ Supporter', src: 'https://cdn.frankerfacez.com/badge/3/4/rounded' }],
    bttvPro: { src: 'https://cdn.betterttv.net/badges/pro/x.webp', title: 'BTTV Pro' },
  };

  it('each count part is set and marked only by the read that answered it', () => {
    const neither = sevenTvCountsStep({ answered: false, snAnswered: false, counts: failedCounts });
    expect(neither.marks).toEqual({});
    expect(neither.apply(onScreen)).toEqual(onScreen);

    const snOnly = sevenTvCountsStep({ answered: false, snAnswered: true, counts: failedCounts });
    expect(snOnly.marks).toEqual({ sn: true });
    expect(snOnly.apply(onScreen)).toEqual({ paints: 12, badges: 7, sn: 5 });
    // With no card yet the StreamNook count still shows.
    expect(snOnly.apply({ paints: 0, badges: 0, sn: 0 })).toEqual({ paints: 0, badges: 0, sn: 5 });

    const sevenTvOnly = sevenTvCountsStep({ answered: true, snAnswered: false, counts: failedCounts });
    expect(sevenTvOnly.marks).toEqual({ counts: true });
    expect(sevenTvOnly.apply(onScreen)).toEqual({ paints: 0, badges: 0, sn: 3 });

    const both = sevenTvCountsStep({ answered: true, snAnswered: true, counts: failedCounts });
    expect(both.marks).toEqual({ counts: true, sn: true });
    expect(both.apply(onScreen)).toEqual(failedCounts);
  });

  const NONE = { twitch: false, thirdParty: false, bttv: false };
  const ALL = { twitch: true, thirdParty: true, bttv: true };

  it('each badge part is set and marked only by the read that answered it', () => {
    const neither = sevenTvBadgesStep({ answered: false, others: NONE, row: liveRow });
    expect(neither.marks).toEqual({});
    expect(neither.apply(cardRow)).toEqual(cardRow);

    const othersOnly = sevenTvBadgesStep({ answered: false, others: ALL, row: liveRow });
    expect(othersOnly.marks).toEqual({ twitchBadge: true, thirdPartyBadges: true, bttvBadge: true });
    expect(othersOnly.apply(cardRow)).toEqual({ ...liveRow, seventv: cardBadge });

    const sevenTvOnly = sevenTvBadgesStep({ answered: true, others: NONE, row: liveRow });
    expect(sevenTvOnly.marks).toEqual({ seventvBadge: true });
    // An answered "wears no 7TV badge" clears it; the card's other badges stay.
    expect(sevenTvOnly.apply(cardRow)).toEqual({ ...cardRow, seventv: null });

    const both = sevenTvBadgesStep({ answered: true, others: ALL, row: liveRow });
    expect(both.marks).toEqual({ seventvBadge: true, twitchBadge: true, thirdPartyBadges: true, bttvBadge: true });
    expect(both.apply(cardRow)).toEqual(liveRow);
  });

  it('Twitch failing alone keeps the answered third-party and BTTV badges', () => {
    const row: Row = { ...liveRow, thirdParty: [{ key: 'ffz:ffz-9', provider: 'ffz' }], twitch: null };
    const step = sevenTvBadgesStep({ answered: true, others: { twitch: false, thirdParty: true, bttv: true }, row });
    expect(step.marks).toEqual({ seventvBadge: true, thirdPartyBadges: true, bttvBadge: true });
    // With no card yet the answered parts show; Twitch stays empty, unmarked.
    expect(step.apply(emptyRow)).toEqual(row);
    // With the card on screen its Twitch badge stays.
    expect(step.apply(cardRow).twitch).toEqual(cardRow.twitch);
  });

  it("without Twitch's answer the loadout's copy fills an empty slot, unmarked, and a later card replaces it", () => {
    const fromKey = { src: 'https://static-cdn.jtvnw.net/badges/v1/key/3', title: 'Twitch' };
    const keyRow: Row = { ...liveRow, twitch: fromKey };
    const step = sevenTvBadgesStep({ answered: true, others: { twitch: false, thirdParty: true, bttv: true }, row: keyRow });
    expect(step.marks.twitchBadge).toBeUndefined();
    const shown = step.apply(emptyRow);
    expect(shown.twitch).toEqual(fromKey);
    const live = { ...noLive(), ...step.marks };
    expect(merge(shown, planFirstPaint(snap(), live, false).wornBadges as Partial<Row>).twitch).toEqual(cardRow.twitch);
  });

  it('reads that failed first, card second: the card fills every part that did not answer', () => {
    const live = noLive();
    // 7TV, the identity reads and the registry all failed or had not loaded.
    const counts = sevenTvCountsStep({ answered: false, snAnswered: false, counts: { paints: 0, badges: 0, sn: 0 } });
    const badges = sevenTvBadgesStep({ answered: false, others: NONE, row: emptyRow });
    Object.assign(live, counts.marks, badges.marks);
    const countsOnScreen = counts.apply({ paints: 0, badges: 0, sn: 0 });
    const rowOnScreen = badges.apply(emptyRow);

    const plan = planFirstPaint(snap(), live, false);
    expect(merge(countsOnScreen, plan.counts)).toEqual({ paints: 12, badges: 7, sn: 3 });
    expect(merge(rowOnScreen, plan.wornBadges as Partial<Row>)).toEqual(cardRow);
  });

  it('only 7TV failed, first; card second: the card fills only the 7TV parts', () => {
    const live = noLive();
    const counts = sevenTvCountsStep({ answered: false, snAnswered: true, counts: failedCounts });
    const badges = sevenTvBadgesStep({ answered: false, others: ALL, row: liveRow });
    Object.assign(live, counts.marks, badges.marks);
    const countsOnScreen = counts.apply({ paints: 0, badges: 0, sn: 0 });
    const rowOnScreen = badges.apply(emptyRow);

    const plan = planFirstPaint(snap(), live, false);
    expect(merge(countsOnScreen, plan.counts)).toEqual({ paints: 12, badges: 7, sn: 5 });
    expect(merge(rowOnScreen, plan.wornBadges as Partial<Row>)).toEqual({ ...liveRow, seventv: cardBadge });
  });

  it('card first, failed reads second: the card stays where a read did not answer', () => {
    const plan = planFirstPaint(snap(), noLive(), false);
    const cardCounts = merge({ paints: 0, badges: 0, sn: 0 }, plan.counts);
    const shownRow = merge(emptyRow, plan.wornBadges as Partial<Row>);
    expect(sevenTvCountsStep({ answered: false, snAnswered: false, counts: failedCounts }).apply(cardCounts)).toEqual(onScreen);
    expect(sevenTvBadgesStep({ answered: false, others: NONE, row: emptyRow }).apply(shownRow)).toEqual(cardRow);
  });

  it('a registry that loads after the card sets only the StreamNook count, and a later card keeps it', () => {
    const live = noLive();
    const beforeLoad = sevenTvCountsStep({ answered: false, snAnswered: false, counts: { paints: 0, badges: 0, sn: 0 } });
    Object.assign(live, beforeLoad.marks);
    let shown = beforeLoad.apply({ paints: 12, badges: 7, sn: 3 });
    expect(shown).toEqual({ paints: 12, badges: 7, sn: 3 });
    expect(live.sn).toBe(false);

    const loaded = sevenTvCountsStep({ answered: false, snAnswered: true, counts: { paints: 0, badges: 0, sn: 6 } });
    Object.assign(live, loaded.marks);
    shown = loaded.apply(shown);
    expect(shown).toEqual({ paints: 12, badges: 7, sn: 6 });
    expect(planFirstPaint(snap(), live, false).counts).toEqual({ paints: 12, badges: 7 });
  });

  it('the card hook takes each answer from its own read and applies these steps as they are', () => {
    const hook = readFileSync(resolve(__dirname, 'memberProfile.ts'), 'utf8');
    expect(hook.match(/live\.(counts|badges|seventvBadge|sn|twitchBadge|thirdPartyBadges|bttvBadge) = true/g) ?? []).toHaveLength(0);
    expect(hook).not.toContain('isUserCosmeticsHardFailed');
    expect(hook).toContain('const paintAnswered = profile.wornAnswered === true;');
    expect(hook).toContain('answered: profile.inventoryAnswered === true,');
    expect(hook).toContain('snAnswered: isCosmeticsRegistryLoaded(),');
    expect(hook).toContain('Object.assign(live, countsStep.marks);');
    expect(hook).toContain('setCounts(countsStep.apply);');
    expect(hook).toContain('others: answers,');
    expect(hook).toContain('Object.assign(live, badgesStep.marks);');
    expect(hook).toContain('setWornBadges(badgesStep.apply);');
    // resolveWornBadges: each part from its own read, failed identity reads not passed off as answers.
    expect(hook).toContain('twitch: liveTwitch !== undefined,');
    expect(hook).toContain('thirdParty: resolved !== null && !isIdentityFallback(resolved),');
    expect(hook).toContain('bttv: rawLoadout !== null && !isIdentityFallback(rawLoadout) && bttvAnswered,');
    expect(hook).toContain('bttvAnswered = url !== undefined;');
    // A registry that loads after the card opened sets the StreamNook count through the same step.
    expect(hook).toContain('const unsubscribeRegistry = subscribeCosmeticsVersion(() => {');
    expect(hook).toContain('snAnswered: isCosmeticsRegistryLoaded(),\n        counts: { paints: 0, badges: 0, sn: getOwnedCosmeticSlugs(userId).size },'.replace(/\n/g, hook.includes('\r\n') ? '\r\n' : '\n'));
    expect(hook).toContain('Object.assign(live, registryStep.marks);');
    expect(hook).toContain('setCounts(registryStep.apply);');
    expect(hook).toContain('unsubscribeRegistry();');
    // The Settings preview applies the member's own edit, keeping the 7TV badge on a failed read.
    expect(hook).toContain('others: PREVIEW_OTHERS,');
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
