// @vitest-environment node
// Run with: npm test
//
// 7TV emote set rows, the custom moderator and VIP art switch, the system rows
// the live preview leaves out, and the live preview's channel holds.

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';


import { OverlayChat } from './OverlayChat';
import { SAMPLE_MESSAGES, type OverlayMessage } from './sampleMessages';
import {
  DEFAULT_OVERLAY_STYLE,
  badgeWithBackdrop,
  clampOverlayStyle,
  ffzRoomRoleArt,
  pngDimensions,
  seventvChangesFromDispatch,
  type OverlayStyle,
  type SeventvEmoteChange,
} from './overlayConfig';
import { createSourceHolds } from './liveSourceHolds';
import { isSystemRow, overlayTwitchBadgeUrl, systemRowForOverlay } from './liveOverlayRows';
import type { BackendChatMessage } from '../../services/twitchChat';
import { seventvEmoteChanges } from '../../services/seventvEmoteChanges';

const style = (over: Partial<OverlayStyle> = {}): OverlayStyle => ({ ...DEFAULT_OVERLAY_STYLE, ...over });

const changeRow = (change: SeventvEmoteChange, id = 'row-7tv'): OverlayMessage => ({
  id,
  user_id: '',
  username: '',
  display_name: '',
  color: '',
  badges: [],
  timestamp: new Date(0).toISOString(),
  content: '',
  provider: 'twitch',
  channel: 'harborlight',
  emotes: [],
  layout: { height: 0, width: 0 },
  tags: {},
  seventvEmoteChange: change,
});

const removed: SeventvEmoteChange = {
  action: 'removed',
  actor: 'kathi',
  name: 'Banana',
  emoteUrl: 'https://cdn.7tv.app/emote/01ABC/3x.webp',
};

describe('7TV emote set changes', () => {
  it('reads adds, removes and renames from an EventAPI dispatch, skipping flag-only updates', () => {
    const changes = seventvChangesFromDispatch({
      id: 'set',
      actor: { display_name: 'kathi', username: 'kathi' },
      pushed: [{ key: 'emotes', index: 0, value: { id: 'add1', name: 'BananaVari' } }],
      pulled: [{ key: 'emotes', index: 1, old_value: { id: 'rm1', name: 'Banana' } }],
      updated: [
        { key: 'emotes', value: { id: 'rn1', name: 'NewName' }, old_value: { id: 'rn1', name: 'OldName' } },
        { key: 'emotes', value: { id: 'fl1', name: 'Same', flags: 1 }, old_value: { id: 'fl1', name: 'Same' } },
      ],
    });
    expect(changes).toEqual([
      { action: 'added', actor: 'kathi', name: 'BananaVari', emoteUrl: 'https://cdn.7tv.app/emote/add1/3x.webp' },
      { action: 'removed', actor: 'kathi', name: 'Banana', emoteUrl: 'https://cdn.7tv.app/emote/rm1/3x.webp' },
      { action: 'renamed', actor: 'kathi', name: 'NewName', oldName: 'OldName', emoteUrl: 'https://cdn.7tv.app/emote/rn1/3x.webp' },
    ]);
    expect(seventvChangesFromDispatch(null)).toEqual([]);
    expect(seventvChangesFromDispatch({ pushed: [{ key: 'emotes', value: { id: 'x', name: 'Y' } }] })[0].actor).toBe('Someone');
  });

  it("takes the app's live update with each emote's image", () => {
    const changes = seventvEmoteChanges(
      {
        channel: 'harborlight',
        channel_id: '1',
        actor_name: 'kathi',
        added: ['BananaVari'],
        removed: ['Banana'],
        renamed: [{ old: 'Old', new: 'New' }],
        composed: {
          added: [
            { id: 'add1', name: 'BananaVari', url: 'x', provider: '7tv' },
            { id: 'rn1', name: 'New', url: 'x', provider: '7tv' },
          ],
          removed: [{ id: 'rm1', name: 'Banana' }],
        },
      },
      [{ id: 'old-banana', name: 'Banana', url: 'x', provider: '7tv' }],
    );
    expect(changes).toEqual([
      { action: 'added', actor: 'kathi', name: 'BananaVari', emoteUrl: 'https://cdn.7tv.app/emote/add1/3x.webp' },
      // The set as it was names the removed emote first.
      { action: 'removed', actor: 'kathi', name: 'Banana', emoteUrl: 'https://cdn.7tv.app/emote/old-banana/3x.webp' },
      { action: 'renamed', actor: 'kathi', name: 'New', oldName: 'Old', emoteUrl: 'https://cdn.7tv.app/emote/rn1/3x.webp' },
    ]);
    // Kick and YouTube updates carry no composed delta: no image, still a row.
    expect(
      seventvEmoteChanges({ channel: 'x', channel_id: '1', actor_name: '', added: ['A'], removed: [], renamed: [] }, []),
    ).toEqual([{ action: 'added', actor: 'Someone', name: 'A' }]);
  });

  it('draws a change as a 7TV event with the action, who did it, the emote and its fate', () => {
    const html = renderToStaticMarkup(<OverlayChat messages={[changeRow(removed)]} style={style()} />);
    expect(html).toContain('data-ov-7tv="removed"');
    expect(html).toContain('Removed Emote');
    expect(html).toContain('kathi');
    expect(html).toContain('https://cdn.7tv.app/emote/01ABC/3x.webp');
    expect(html).toMatch(/Banana<\/span> was removed from the set/);
    // Never a chat line signed by "Twitch".
    expect(html).not.toContain('Twitch:');
  });

  it('renders added and renamed rows in their own words', () => {
    const added = renderToStaticMarkup(<OverlayChat messages={[changeRow({ action: 'added', actor: 'kathi', name: 'BananaVari' })]} style={style()} />);
    expect(added).toContain('Added Emote');
    expect(added).toMatch(/BananaVari<\/span> was added to the set/);
    const renamed = renderToStaticMarkup(
      <OverlayChat messages={[changeRow({ action: 'renamed', actor: 'kathi', name: 'New', oldName: 'Old' })]} style={style()} />,
    );
    expect(renamed).toContain('Renamed Emote');
    expect(renamed).toMatch(/Old<\/span> is now <span[^>]*>New<\/span>/);
  });

  it('follows the event style and its fill', () => {
    const outline = renderToStaticMarkup(
      <OverlayChat messages={[changeRow(removed)]} style={style({ eventStyle: 'outline', eventFill: true, eventAnimation: 'sheen' })} />,
    );
    expect(outline).toContain('sn-ft-anim-ring sn-ft-t-sheen');
    expect(outline).toContain('0.09em solid color-mix(in srgb, #29b6f6 60%, transparent)');
    expect(outline).toContain('#29b6f6 8%');
    const wash = renderToStaticMarkup(<OverlayChat messages={[changeRow(removed)]} style={style({ eventStyle: 'streamnook' })} />);
    expect(wash).toContain('class="sn-ev-seventv"');
  });

  it('hides the rows when the switch is off, and older configs show them', () => {
    const off = renderToStaticMarkup(<OverlayChat messages={[changeRow(removed)]} style={style({ seventvEmoteChanges: false })} />);
    expect(off).not.toContain('data-ov-7tv');
    const legacy = { ...DEFAULT_OVERLAY_STYLE } as Partial<OverlayStyle>;
    delete legacy.seventvEmoteChanges;
    delete legacy.ffzCustomRoleBadges;
    const clamped = clampOverlayStyle(legacy as OverlayStyle);
    expect(clamped.seventvEmoteChanges).toBe(true);
    expect(clamped.ffzCustomRoleBadges).toBe(false);
  });
});

describe('system rows in the live preview', () => {
  const system = (extra: Record<string, unknown> = {}): BackendChatMessage =>
    ({
      id: 'sys-1',
      username: 'System',
      display_name: 'Twitch',
      content: 'kathi removed the emote Banana',
      tags: new Map([['user-id', 'tw-system']]),
      timestamp: String(Date.now()),
      badges: [],
      ...extra,
    }) as unknown as BackendChatMessage;

  it('turns a noted 7TV change into its overlay row and drops every other system line', () => {
    expect(isSystemRow(system())).toBe(true);
    expect(systemRowForOverlay(system(), 'twitch')).toBeNull();
    const row = systemRowForOverlay(system({ seventvEmoteChange: removed }), 'kick');
    expect(row?.seventvEmoteChange).toEqual(removed);
    expect(row?.provider).toBe('kick');
    expect(row?.display_name).toBe('');
    const html = renderToStaticMarkup(<OverlayChat messages={[row!]} style={style({ sources: ['kick'] })} />);
    expect(html).toContain('Removed Emote');
    expect(html).not.toContain('Twitch');
  });

  it('knows a chatter from a system row', () => {
    const chatter = SAMPLE_MESSAGES.find((m) => (m.provider ?? 'twitch') === 'twitch')!;
    expect(isSystemRow(chatter)).toBe(false);
    expect(isSystemRow({ ...chatter, user_id: 'tw-system' })).toBe(true);
  });
});

describe('custom moderator and VIP art', () => {
  const ffzMod = 'data:image/svg+xml;base64,ffzmod';
  const twitchMod = 'https://static-cdn.jtvnw.net/badges/v1/3267646d-33f0-4b17-b3df-f923a41db1d0/3';
  const channelSetWithFfz = { image_url_4x: ffzMod };
  const globalMod = () => ({ image_url_4x: twitchMod });

  it('off shows Twitch art even where the chat channel set carries the custom art', () => {
    expect(overlayTwitchBadgeUrl('moderator', channelSetWithFfz, undefined, globalMod)).toBe(twitchMod);
    expect(overlayTwitchBadgeUrl('vip', { image_url_4x: 'https://cdn.frankerfacez.com/room-badge/vip/id/1/v/x/4' }, undefined, () => ({ image_url_4x: 'https://static-cdn.jtvnw.net/badges/v1/vip/3' }))).toBe(
      'https://static-cdn.jtvnw.net/badges/v1/vip/3',
    );
    // Never no badge: without a global copy the channel's own image stays.
    expect(overlayTwitchBadgeUrl('moderator', channelSetWithFfz, undefined, () => null)).toBe(ffzMod);
  });

  it('on shows the room art, and Twitch art for a role the room does not replace', () => {
    const art = { moderator: 'data:image/svg+xml;base64,room', vip: null };
    expect(overlayTwitchBadgeUrl('moderator', { image_url_4x: twitchMod }, art, globalMod)).toBe('data:image/svg+xml;base64,room');
    expect(overlayTwitchBadgeUrl('vip', { image_url_4x: 'https://static-cdn.jtvnw.net/badges/v1/vip/3' }, art, globalMod)).toBe(
      'https://static-cdn.jtvnw.net/badges/v1/vip/3',
    );
    expect(overlayTwitchBadgeUrl('subscriber', { image_url_4x: 'sub' }, art, globalMod)).toBe('sub');
  });

  it('reads the largest art from an FFZ room payload', () => {
    expect(
      ffzRoomRoleArt({
        room: {
          mod_urls: { '1': 'https://cdn.frankerfacez.com/room-badge/mod/id/1/v/a/1', '4': 'https://cdn.frankerfacez.com/room-badge/mod/id/1/v/a/4' },
          vip_badge: { '1': 'https://cdn.frankerfacez.com/room-badge/vip/id/1/v/b/1' },
        },
      }),
    ).toEqual({
      moderator: 'https://cdn.frankerfacez.com/room-badge/mod/id/1/v/a/4',
      vip: 'https://cdn.frankerfacez.com/room-badge/vip/id/1/v/b/4',
    });
    expect(ffzRoomRoleArt({ room: { mod_urls: null, vip_badge: null } })).toEqual({ moderator: null, vip: null });
    expect(ffzRoomRoleArt(null)).toEqual({ moderator: null, vip: null });
  });

  it('paints the moderator glyph on its green square the way the app does', () => {
    const png = new Uint8Array(32);
    png.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 72, 0, 0, 0, 72]);
    expect(pngDimensions(png)).toEqual({ w: 72, h: 72 });
    const uri = badgeWithBackdrop(png, '#34AE0A')!;
    expect(uri.startsWith('data:image/svg+xml;base64,')).toBe(true);
    const svg = atob(uri.slice('data:image/svg+xml;base64,'.length));
    expect(svg).toContain('rx="8" fill="#34AE0A"');
    expect(svg).toContain('href="data:image/png;base64,');
    expect(badgeWithBackdrop(new Uint8Array(4), '#34AE0A')).toBeNull();
    expect(badgeWithBackdrop(png, 'green')).toBeNull();
  });

  it('a glyph add-on badge draws on its backdrop', () => {
    const chatter = SAMPLE_MESSAGES.find((m) => (m.provider ?? 'twitch') === 'twitch')!;
    const html = renderToStaticMarkup(
      <OverlayChat
        messages={[{ ...chatter, id: 'ap', extraBadges: [{ url: 'https://example.test/ap.png', title: 'FFZ:AP', source: 'chatty', backdrop: '#123456' }] }]}
        style={style()}
      />,
    );
    expect(html).toContain('background-color:#123456');
  });
});

describe('live preview channel holds', () => {
  const ops = () => {
    const log: string[] = [];
    return { log, holds: createSourceHolds({ acquire: (k, id) => log.push(`+${k}:${id}`), release: (k) => log.push(`-${k}`) }) };
  };

  it('a source dropped before its id resolves is neither released nor acquired late', () => {
    const { log, holds } = ops();
    let resolve: (id: string) => void = () => {};
    holds.want(['twitch:zoox'], (k) => {
      resolve = (id) => holds.take(k, id);
    });
    holds.dropAll(); // switched to Sample before Helix answered
    resolve('2830977');
    expect(log).toEqual([]);
    expect(holds.held.size).toBe(0);
  });

  it('a double mount takes one reference and gives back exactly that one', () => {
    const { log, holds } = ops();
    const pending: Array<() => void> = [];
    const start = (k: string) => pending.push(() => holds.take(k, '1'));
    holds.want(['twitch:a'], start);
    holds.dropAll();
    holds.want(['twitch:a'], start);
    for (const p of pending) p();
    expect(log).toEqual(['+twitch:a:1']);
    holds.want([], start);
    expect(log).toEqual(['+twitch:a:1', '-twitch:a']);
  });

  it('keeps unchanged sources and swaps only what changed', () => {
    const { log, holds } = ops();
    const start = (k: string) => holds.take(k, null);
    holds.want(['kick:a', 'kick:b'], start);
    holds.want(['kick:b', 'kick:c'], start);
    expect(log).toEqual(['+kick:a:null', '+kick:b:null', '+kick:c:null', '-kick:a']);
  });
});
