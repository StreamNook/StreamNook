import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { AutomationAccountRef, AutomationPlatform, AutomationRule } from '../types';
import {
  KICK_ACTION_KINDS,
  NOTIFY_ACTION_KINDS,
  PLATFORM_EVENT_KINDS,
  actionFits,
  actionOf,
  blankRule,
  canPost,
  eventsOn,
  providerOfKey,
  rulePlatform,
  withPlatforms,
} from './automations';

const owner: AutomationAccountRef = { provider: 'twitch', id: '100', login: 'me', name: 'Me' };
const kick = { provider: 'kick' as const, id: '5001', login: 'mykick' };
const rust = readFileSync(new URL('../../src-tauri/src/models/automations.rs', import.meta.url), 'utf8');

const words = (list: string): string[] => [...list.matchAll(/"([a-z_]+)"/g)].map((m) => m[1]).sort();

/** `GiftBomb` is `gift_bomb`, and TikTok's kinds keep their one-word prefix. */
const snake = (name: string): string =>
  name.startsWith('TikTok') ? `tiktok_${name.slice(6).toLowerCase()}` : name.replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase();

function postingRule(): AutomationRule {
  return blankRule(owner);
}

function quietRule(): AutomationRule {
  return { ...blankRule(owner), actions: [actionOf('notify')] };
}

describe('platforms', () => {
  it('a blank rule watches Twitch only and can post there and on Kick, not on YouTube or TikTok', () => {
    expect(blankRule(owner).only_if.platforms).toEqual(['twitch']);
    expect(rulePlatform(blankRule(owner))).toBe('twitch');
    expect(canPost('twitch') && canPost('kick')).toBe(true);
    expect(canPost('youtube') || canPost('tiktok')).toBe(false);
  });

  it('mirrors the action table Rust checks at save', () => {
    const kickList = /Platform::Kick => \{\s*matches!\(kind, ([^)]*)\)/.exec(rust)?.[1] ?? '';
    const notifyList = /Platform::Youtube \| Platform::Tiktok => matches!\(kind, ([^)]*)\)/.exec(rust)?.[1] ?? '';
    expect(words(kickList)).toEqual([...KICK_ACTION_KINDS].sort());
    expect(words(notifyList)).toEqual([...NOTIFY_ACTION_KINDS].sort());
    expect(actionFits('twitch', 'anything')).toBe(true);
    expect(actionFits('youtube', 'reply') || actionFits('tiktok', 'send')).toBe(false);
  });

  it('mirrors the events each platform has', () => {
    const found = new Map<string, string[]>();
    for (const m of rust.matchAll(/Platform::(Twitch|Kick|Youtube|Tiktok) => matches!\(self, ([^)]*)\)/g)) {
      found.set(
        m[1].toLowerCase(),
        m[2].split('|').map((n) => snake(n.trim())).sort(),
      );
    }
    for (const platform of ['twitch', 'kick', 'youtube', 'tiktok'] as AutomationPlatform[]) {
      expect(found.get(platform), platform).toEqual([...PLATFORM_EVENT_KINDS[platform]].sort());
    }
    expect(eventsOn(['twitch', 'kick'])).toEqual(PLATFORM_EVENT_KINDS.twitch);
    expect(eventsOn(['kick', 'youtube'])).toEqual(['sub', 'resub', 'gift', 'gift_bomb', 'super_chat', 'super_sticker', 'membership', 'member_gift']);
  });

  it('reads a chat key platform', () => {
    expect(providerOfKey('kick:alpha')).toBe('kick');
    expect(providerOfKey('YouTube:UCabc')).toBe('youtube');
    expect(providerOfKey('tiktok:x')).toBe('tiktok');
    expect(providerOfKey('xqc')).toBe('twitch');
    expect(providerOfKey('twitch:xqc')).toBe('twitch');
    expect(providerOfKey('')).toBe('twitch');
  });
});

describe('withPlatforms', () => {
  it('a rule that posts moves to Kick and sends as the connected Kick account, and back to its owner', () => {
    const onKick = withPlatforms(postingRule(), ['kick'], kick);
    expect(onKick.only_if.platforms).toEqual(['kick']);
    expect(onKick.send_as).toEqual({ provider: 'kick', id: '5001', login: 'mykick', name: 'mykick' });
    const back = withPlatforms(onKick, ['twitch'], kick);
    expect(back.send_as).toEqual(owner);
  });

  it('a rule that posts runs on one platform: the newest choice wins', () => {
    const next = withPlatforms(postingRule(), ['twitch', 'kick'], kick);
    expect(next.only_if.platforms).toEqual(['kick']);
  });

  it('a rule that only notifies may watch several platforms', () => {
    const next = withPlatforms(quietRule(), ['twitch', 'kick', 'youtube'], kick);
    expect(next.only_if.platforms).toEqual(['twitch', 'kick', 'youtube']);
    expect(next.actions.map((a) => a.kind)).toEqual(['notify']);
  });

  it('YouTube and TikTok keep only what they can do and never post', () => {
    for (const platform of ['youtube', 'tiktok'] as AutomationPlatform[]) {
      const next = withPlatforms(postingRule(), [platform], kick);
      expect(next.actions.map((a) => a.kind)).toEqual(['notify']);
    }
  });

  it('drops Only while live, Watching and the badge trigger when another platform joins', () => {
    const rule: AutomationRule = {
      ...quietRule(),
      only_if: { ...quietRule().only_if, only_live: true, channels: { kind: 'watching' } },
      when: { kind: 'badge_drop', on: 'both', link: { preset: 'streamnook', custom: '', if_not_ready: 'streamnook' }, batch_text: '' },
    };
    const next = withPlatforms(rule, ['twitch', 'kick'], kick);
    expect(next.only_if.only_live).toBe(false);
    expect(next.only_if.channels).toEqual({ kind: 'watching' });
    expect(next.when.kind).toBe('mention');
    const kickOnly = withPlatforms(rule, ['kick'], kick);
    expect(kickOnly.only_if.channels).toEqual({ kind: 'own' });
  });

  it('keeps only the events and chats the platforms have', () => {
    const rule: AutomationRule = {
      ...quietRule(),
      when: { kind: 'channel_event', events: ['raid', 'sub'], min_months: 0, min_gifts: 0, min_viewers: 0, min_bits: 0 },
      only_if: {
        ...quietRule().only_if,
        channels: {
          kind: 'these',
          list: [
            { id: 'c-1', key: 'kick:alpha', room_id: '', name: 'alpha' },
            { id: 'c-2', key: '', room_id: '1', name: 'lirik' },
          ],
        },
      },
    };
    const next = withPlatforms(rule, ['kick'], kick);
    expect(next.when.kind === 'channel_event' && next.when.events).toEqual(['sub']);
    expect(next.only_if.channels.kind === 'these' && next.only_if.channels.list.map((c) => c.id)).toEqual(['c-1']);
    const yt = withPlatforms(rule, ['youtube'], kick);
    expect(yt.when.kind === 'channel_event' && yt.when.events).toEqual(['super_chat']);
  });

  it('a Send to another chat on a platform the rule left is cleared', () => {
    const rule: AutomationRule = {
      ...postingRule(),
      actions: [{ kind: 'send', id: 'a-1', to: { id: 'c-1', key: '', room_id: '9', name: 'lirik' }, texts: [{ id: 'l-1', text: 'hi' }], pick: 'random' }],
    };
    const next = withPlatforms(rule, ['kick'], kick);
    expect(next.actions[0].kind === 'send' && next.actions[0].to).toBeNull();
  });

  it('with no choice the rule is a Twitch rule', () => {
    expect(withPlatforms(quietRule(), [], kick).only_if.platforms).toEqual(['twitch']);
  });
});
