import { describe, expect, it } from 'vitest';
import {
  onePerCategory,
  pickRoleBadge,
  toggleLoadoutBadge,
  visibleChatterBadges,
  wearCategory,
} from './chatterBadges';
import type { ThirdPartyBadge } from '../services/badgeService';

const tp = (provider: string): ThirdPartyBadge => ({
  id: provider,
  title: provider,
  imageUrl: `https://x/${provider}.png`,
  image1x: '',
  image2x: '',
  image4x: '',
  provider,
});

const twitch = [{ key: 'moderator/1' }, { key: 'subscriber/12' }];
const seventv = { id: '7tv-badge' };
const third = [tp('bttv'), tp('ffz'), tp('chatterino')];

describe('visibleChatterBadges', () => {
  it('passes everything through by default, keeping references', () => {
    const out = visibleChatterBadges(undefined, twitch, seventv, third);
    expect(out.twitch).toBe(twitch);
    expect(out.seventv).toBe(seventv);
    expect(out.thirdParty).toBe(third);
    expect(out.streamNook).toBe(true);
  });

  it('hides Twitch badges only when badges are off', () => {
    const out = visibleChatterBadges({ show_badges: false }, twitch, seventv, third);
    expect(out.twitch).toEqual([]);
    expect(out.seventv).toBe(seventv);
    expect(out.thirdParty).toBe(third);
  });

  it('turning third-party badges off hides 7TV, the others and StreamNook', () => {
    const out = visibleChatterBadges({ show_third_party_badges: false }, twitch, seventv, third);
    expect(out.twitch).toBe(twitch);
    expect(out.seventv).toBeNull();
    expect(out.thirdParty).toEqual([]);
    expect(out.streamNook).toBe(false);
  });

  it('hides single providers by name, case-insensitively on the badge side', () => {
    const out = visibleChatterBadges(
      { hidden_badge_providers: ['7tv', 'ffz', 'streamnook'] },
      twitch,
      seventv,
      [tp('BTTV'), tp('FFZ'), tp('chatterino')],
    );
    expect(out.seventv).toBeNull();
    expect(out.thirdParty.map((b) => b.provider)).toEqual(['BTTV', 'chatterino']);
    expect(out.streamNook).toBe(false);
  });

  it('treats a missing 7TV badge as none', () => {
    expect(visibleChatterBadges(undefined, twitch, undefined, third).seventv).toBeNull();
  });
});

describe('onePerCategory', () => {
  const b = (provider: string, id: string, category?: string) => ({ provider, id, category });

  it('keeps one per provider for single-category providers, and providers stack', () => {
    const list = [b('ffz', 'ffz-3', 'ffz-slot-5'), b('dankchat', 'x', 'dankchat'), b('dankchat', 'y', 'dankchat'), b('chatty', 'c', 'chatty')];
    expect(onePerCategory(list).map((x) => x.id)).toEqual(['ffz-3', 'x', 'c']);
  });

  it('lets a Homies holder wear one custom, one role and one staff badge', () => {
    const list = [
      b('homies', 'homies-custom', 'homies-custom'),
      b('homies', 'homies-Homies Supporter', 'homies'),
      b('homies', 'homies-Homies Developer', 'homies'),
      b('homies', 'homies-Homies Mod', 'homies-staff'),
    ];
    expect(onePerCategory(list).map((x) => x.id)).toEqual(['homies-custom', 'homies-Homies Supporter', 'homies-Homies Mod']);
  });

  it('keeps one per FFZ slot', () => {
    const list = [b('ffz', 'ffz-3', 'ffz-slot-5'), b('ffz', 'ffz-1', 'ffz-slot-5'), b('ffz', 'ffz-9', 'ffz-slot-6')];
    expect(onePerCategory(list).map((x) => x.id)).toEqual(['ffz-3', 'ffz-9']);
  });

  it('without a category, the Homies custom badge still counts apart', () => {
    const list = [b('homies', 'homies-custom'), b('homies', 'homies-Homies Supporter'), b('homies', 'homies-Homies Founder')];
    expect(onePerCategory(list).map((x) => x.id)).toEqual(['homies-custom', 'homies-Homies Supporter']);
  });

  it('treats BTTV and bttv as one, so Pro yields to the badge before it', () => {
    expect(onePerCategory([b('bttv', 'bttv-Translator', 'bttv'), b('BTTV', 'bttv-pro')]).map((x) => x.id)).toEqual(['bttv-Translator']);
    expect(wearCategory(b('BTTV', 'bttv-pro'))).toBe('bttv:bttv');
  });

  it('returns the same array when nothing is dropped', () => {
    const list = [b('ffz', 'a'), b('chatty', 'c')];
    expect(onePerCategory(list)).toBe(list);
  });
});

describe('toggleLoadoutBadge', () => {
  const categories: Record<string, string> = {
    'homies:homies-custom': 'homies:homies-custom',
    'homies:homies-Homies Supporter': 'homies:homies',
    'homies:homies-Homies Founder': 'homies:homies',
    'homies:homies-Homies Mod': 'homies:homies-staff',
  };
  const categoryOf = (k: string) => categories[k] ?? k.split(':')[0];

  it('a Homies custom badge and a role badge can both be on', () => {
    expect(toggleLoadoutBadge(['homies:homies-Homies Supporter'], 'homies:homies-custom', categoryOf)).toEqual([
      'homies:homies-Homies Supporter',
      'homies:homies-custom',
    ]);
  });

  it('switching a role on switches off the other role of that list only', () => {
    const before = ['twitch:abc/1', 'homies:homies-Homies Supporter', 'homies:homies-Homies Mod', 'ffz:ffz-3'];
    expect(toggleLoadoutBadge(before, 'homies:homies-Homies Founder', categoryOf)).toEqual([
      'twitch:abc/1',
      'homies:homies-Homies Mod',
      'ffz:ffz-3',
      'homies:homies-Homies Founder',
    ]);
  });

  it('switching a chosen badge off removes just that key', () => {
    expect(toggleLoadoutBadge(['ffz:ffz-3', 'dankchat:dankchat-x'], 'ffz:ffz-3')).toEqual(['dankchat:dankchat-x']);
  });

  it('without a list, DankChat badges and BTTV Pro still switch within their provider', () => {
    expect(toggleLoadoutBadge(['dankchat:dankchat-DankChat Supporter'], 'dankchat:dankchat-DankChat Contributor')).toEqual([
      'dankchat:dankchat-DankChat Contributor',
    ]);
    expect(toggleLoadoutBadge(['bttv:bttv-Translator'], 'BTTV:bttv-pro')).toEqual(['BTTV:bttv-pro']);
  });
});

describe('Homies custom badges toggle on their own', () => {
  const custom = { ...tp('homies'), id: 'homies-custom', title: 'usVesper Badge', category: 'homies-custom' };
  // A member's resolved bundle names the badge by its loadout key and may carry no category.
  const memberCustom = { ...tp('homies'), id: 'homies:homies-custom', title: 'Robits Badge' };
  const role = { ...tp('homies'), id: 'homies-Homies Supporter', title: 'Homies Supporter', category: 'homies' };
  const list = [tp('ffz'), custom, role];

  it('hiding Homies custom keeps the role badge', () => {
    const out = visibleChatterBadges({ hidden_badge_providers: ['homies-custom'] }, twitch, seventv, list);
    expect(out.thirdParty.map((b) => b.title)).toEqual(['ffz', 'Homies Supporter']);
    const member = visibleChatterBadges({ hidden_badge_providers: ['homies-custom'] }, twitch, seventv, [memberCustom, role]);
    expect(member.thirdParty.map((b) => b.title)).toEqual(['Homies Supporter']);
  });

  it('hiding Homies keeps the custom badge', () => {
    const out = visibleChatterBadges({ hidden_badge_providers: ['homies'] }, twitch, seventv, list);
    expect(out.thirdParty.map((b) => b.title)).toEqual(['ffz', 'usVesper Badge']);
  });

  it('hiding both hides every Homies badge', () => {
    const out = visibleChatterBadges({ hidden_badge_providers: ['homies', 'homies-custom'] }, twitch, seventv, list);
    expect(out.thirdParty.map((b) => b.title)).toEqual(['ffz']);
  });
});

describe('pickRoleBadge', () => {
  const b = (...keys: string[]) => keys.map((key) => ({ key }));

  it('picks broadcaster over a subscription', () => {
    expect(pickRoleBadge(b('broadcaster/1', 'subscriber/12'))?.key).toBe('broadcaster/1');
  });

  it('picks moderator over VIP and a subscription, whatever the order', () => {
    expect(pickRoleBadge(b('subscriber/12', 'vip/1', 'moderator/1'))?.key).toBe('moderator/1');
  });

  it('picks staff over moderator', () => {
    expect(pickRoleBadge(b('moderator/1', 'staff/1'))?.key).toBe('staff/1');
  });

  it('picks none for a subscriber with earned and global badges', () => {
    expect(pickRoleBadge(b('subscriber/12', 'bits/1000', 'premium/1'))).toBeNull();
  });

  it('picks none when the chatter wears only third-party badges', () => {
    // Third-party badges never enter the Twitch list, so it arrives empty.
    const look = visibleChatterBadges(undefined, [], seventv, third);
    expect(look.thirdParty.length).toBeGreaterThan(0);
    expect(pickRoleBadge(look.twitch)).toBeNull();
  });
});
