// @vitest-environment node
//
// The drops bar shows only in the chat that is earning, says the lead reward
// (or what a campaign here will start on), keeps its fill inset clear of the
// corners, closes its drawer on anything but its own click, and is memoized
// and mounted above the message box with nothing but primitives.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../../stores/AppStore', () => ({ useAppStore: Object.assign(() => true, { getState: () => ({ addToast: vi.fn() }) }) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

vi.mock('../../utils/openExternal', () => ({ openExternal: vi.fn() }));

import DropsChatBar, { barLine, drawerAfter, DropsChatBarView, PREVIEW_LINE, showsHere } from './DropsChatBar';
import DropsDrawer from './DropsDrawer';
import { splitStatus } from './dropsStatusText';
import type { HereDrop, WatchDrops } from '../../stores/watchDropsStore';

const view = (over: Partial<WatchDrops> = {}): WatchDrops => ({
  channel_id: '111', login: 'Shroud', game_id: 'g1', game_name: 'Rust', lead: null, more: 0, more_art: [], campaigns: [],
  complete: false, stalled: false, stall_episode: 0, ...over,
});

describe('DropsDrawer', () => {
  const tier = (over: Partial<HereDrop>): HereDrop => ({
    id: 't', instance_id: 'i', name: 'Hoodie', image: '', claimed: false, ready: true, pending: false, connect: false,
    lead: false, status: 'Ready', detail: null, bar: 100, current_minutes: 60, required_minutes: 60, claim_label: 'Claim', ...over,
  });
  const drawer = (drop: HereDrop) =>
    renderToStaticMarkup(
      <DropsDrawer
        view={view({
          campaigns: [{
            id: 'c', name: 'Rust Drops', game_name: 'Rust', art: '', box_art: '', ends: '', claimed_label: '0 of 1 claimed', started: true,
            needs_link: drop.connect, link_url: drop.connect ? 'https://example.com/link' : null, cumulative_minutes: 60, drops: [drop],
          }],
        })}
        maxHeight={300}
        claimingId={null}
        onClaim={() => {}}
      />,
    );

  it('offers Claim only on a ready tier nothing is waiting on', () => {
    expect(drawer(tier({}))).toContain('>Claim<');
    // A claim already sent: never a second Claim.
    const pending = drawer(tier({ pending: true, status: 'Claiming' }));
    expect(pending).not.toContain('>Claim<');
    expect(pending).toMatch(/disabled=""[^>]*>Claiming</);
    // Twitch would not grant it until the game account is linked.
    const connect = drawer(tier({ connect: true, status: 'Connect' }));
    expect(connect).not.toContain('>Claim<');
    expect(connect).toContain('>Connect<');
  });
});

describe('showsHere', () => {
  it('matches ids when both are known', () => {
    expect(showsHere(view(), '111', 'anyone')).toBe(true);
    expect(showsHere(view(), '222', 'shroud')).toBe(false);
  });
  it('falls back to the login while the pane has no id yet', () => {
    expect(showsHere(view(), '', 'shroud')).toBe(true);
    expect(showsHere(view(), '', 'other')).toBe(false);
  });
  it('never shows for the empty view', () => {
    expect(showsHere(view({ channel_id: '', login: '' }), '', '')).toBe(false);
  });
});

describe('barLine', () => {
  const lead = {
    campaign_id: 'c1', drop_id: 'b', instance_id: null, name: 'Hoodie', image: '', trailing: '30m to watch',
    status: '30m to watch', line: '', percent: 75, complete: false, claim_label: 'Claim', spoken: 'Hoodie, 30 minutes left',
  };
  const tier = {
    id: 'n', instance_id: null, name: 'Jacket', image: '', claimed: false, ready: false, pending: false, connect: false,
    lead: false, status: 'Starts when you watch', detail: null, bar: 0, current_minutes: 0, required_minutes: 60, claim_label: 'Claim',
  };
  const notStarted = {
    id: 'c2', name: 'Later', game_name: 'Rust', art: 'art', box_art: '', ends: '', claimed_label: '', started: false, needs_link: false,
    link_url: null, cumulative_minutes: 0, drops: [tier],
  };
  it('says the lead', () => {
    expect(barLine(view({ lead, more: 2 }))).toMatchObject({ name: 'Hoodie', status: '30m to watch', percent: 75, more: 2, ready: false });
  });
  it('says what a campaign here will start on', () => {
    expect(barLine(view({ campaigns: [notStarted] }))).toMatchObject({ name: 'Jacket', status: 'Starts when you watch', percent: 0 });
    expect(barLine(view({ campaigns: [notStarted, { ...notStarted, id: 'c3' }] }))?.name).toBe('2 rewards to earn here');
  });
  it('shows no bar with nothing left here', () => {
    expect(barLine(view({ complete: true }))).toBeNull();
  });
});

describe('drawerAfter', () => {
  it('opens only on its own click and closes on everything else', () => {
    expect(drawerAfter(false, 'toggle')).toBe(true);
    expect(drawerAfter(true, 'toggle')).toBe(false);
    for (const e of ['outside', 'escape', 'picker', 'gone'] as const) {
      expect(drawerAfter(true, e)).toBe(false);
      expect(drawerAfter(false, e)).toBe(false);
    }
  });
});

describe('DropsChatBarView', () => {
  it('draws an inset track and the extra count, never an edge strip', () => {
    const html = renderToStaticMarkup(<DropsChatBarView line={PREVIEW_LINE} open={false} claiming={false} />);
    expect(html).toContain('ml-[34px]');
    expect(html).toContain('width:72%');
    expect(html).toContain('+2');
    expect(html).toContain('liquid-glass-panel drops-bar');
    expect(html).not.toMatch(/absolute left-0 right-0 bottom-0/);
  });
  it('draws the time strong and the rest quiet', () => {
    const html = renderToStaticMarkup(<DropsChatBarView line={PREVIEW_LINE} open={false} claiming={false} />);
    expect(html).toContain('<span class="font-semibold text-textPrimary">12m</span> to watch');
  });
  it('stacks the other rewards and counts only the ones beyond the pictures', () => {
    const two = renderToStaticMarkup(
      <DropsChatBarView line={{ ...PREVIEW_LINE, more: 2, moreArt: ['a.png', 'b.png'] }} open={false} claiming={false} />,
    );
    expect(two.match(/<img/g)?.length).toBe(2);
    expect(two).not.toContain('+');
    const five = renderToStaticMarkup(
      <DropsChatBarView line={{ ...PREVIEW_LINE, more: 5, moreArt: ['a.png', 'b.png'] }} open={false} claiming={false} />,
    );
    expect(five).toContain('+3');
    const none = renderToStaticMarkup(<DropsChatBarView line={{ ...PREVIEW_LINE, more: 0 }} open={false} claiming={false} />);
    expect(none).not.toContain('drops-stack');
  });
  it('offers a filled Claim and lights the rim when the reward is ready', () => {
    const html = renderToStaticMarkup(
      <DropsChatBarView line={{ ...PREVIEW_LINE, ready: true }} open={false} claiming={false} onClaim={() => {}} />,
    );
    expect(html).toContain('>Claim</button>');
    expect(html).toContain('drops-bar flex');
    expect(html).toMatch(/drops-bar[^"]*is-ready/);
    expect(html).toContain('drops-bar-fill block h-full rounded-full is-ready');
  });
});

describe('splitStatus', () => {
  it('takes a leading time and leaves words alone', () => {
    expect(splitStatus('12m to watch')).toEqual({ time: '12m', rest: ' to watch' });
    expect(splitStatus('2h 12m to watch')).toEqual({ time: '2h 12m', rest: ' to watch' });
    expect(splitStatus('1d 3h to watch')).toEqual({ time: '1d 3h', rest: ' to watch' });
    expect(splitStatus('5h')).toEqual({ time: '5h', rest: '' });
    expect(splitStatus('Ready')).toEqual({ time: '', rest: 'Ready' });
    expect(splitStatus('Starts when you watch')).toEqual({ time: '', rest: 'Starts when you watch' });
  });
});

describe('DropsDrawer layouts', () => {
  const drop = (id: string, over: Partial<HereDrop> = {}): HereDrop => ({
    id, instance_id: null, name: `Reward ${id}`, image: `${id}.png`, claimed: false, ready: false, pending: false, connect: false,
    lead: false, status: '2h 12m to watch', detail: null, bar: 27, current_minutes: 48, required_minutes: 180, claim_label: 'Claim', ...over,
  });
  const campaign = (id: string, drops: HereDrop[]) => ({
    id, name: `Campaign ${id}`, game_name: 'Once Human', art: 'art.png', box_art: 'box.jpg', ends: '8d left',
    claimed_label: '', started: true, needs_link: false, link_url: null, cumulative_minutes: 48, drops,
  });
  const html = (campaigns: ReturnType<typeof campaign>[]) =>
    renderToStaticMarkup(<DropsDrawer view={view({ campaigns })} maxHeight={400} claimingId={null} onClaim={() => {}} />);

  it('lays several rewards out as tiles in rows of three, the next one ringed', () => {
    const out = html([campaign('a', [drop('1', { lead: true, status: '12m to watch', bar: 80 }), drop('2'), drop('3', { claimed: true })])]);
    expect(out).toContain('grid grid-cols-3');
    expect(out.match(/drops-tile /g)?.length).toBe(3);
    expect(out).toContain('drops-tile flex min-w-0 flex-col gap-1.5 p-2 is-next');
    expect(out).toContain('is-claimed');
    expect(out).toContain('1 of 3');
    expect(out).toContain('src="box.jpg"');
  });
  it('draws a lone reward as one wide tile with its minutes', () => {
    const out = html([campaign('a', [drop('1', { lead: true, status: '12m to watch', bar: 80, current_minutes: 48, required_minutes: 60 })])]);
    expect(out).not.toContain('grid-cols-3');
    expect(out).toContain('48 of 60 min watched');
    expect(out).not.toContain('1 of 1');
  });
  it('gives each one-reward campaign its own wide tile with its campaign line', () => {
    const out = html([campaign('a', [drop('1', { lead: true })]), campaign('b', [drop('2')])]);
    expect(out).toContain('Drops on this channel');
    expect(out).toContain('2 left');
    expect(out).toContain('Campaign a · 8d left');
    expect(out).toContain('Campaign b · 8d left');
    expect(out).not.toContain('grid-cols-3');
  });
});

describe('mounting', () => {
  it('is memoized', () => {
    expect((DropsChatBar as unknown as { $$typeof: symbol }).$$typeof).toBe(Symbol.for('react.memo'));
  });
  it('sits between the AutoMod strip and the composer with primitives and a stable close', () => {
    const src = readFileSync(resolve(__dirname, '../ChatWidget.tsx'), 'latin1');
    const strip = src.indexOf('<AutomodQueueStrip channel={panelChannelKey} />');
    const bar = src.indexOf('<DropsChatBar');
    const input = src.indexOf('{/* Input container - static flex item at bottom');
    expect(strip).toBeGreaterThan(0);
    expect(bar).toBeGreaterThan(strip);
    expect(input).toBeGreaterThan(bar);
    expect(src.slice(bar, input)).toContain('onDrawerOpen={closeEmotePicker}');
    // The bar matches THIS widget's channel (a dock or MultiChat pane carries its
    // own), never the main window's stream for every pane.
    expect(src.slice(bar, input)).toContain('channelId={(channelOverride ? channelOverride.user_id : currentStream?.user_id) ??');
    expect(src.slice(bar, input)).toContain('login={(channelOverride ? channelOverride.user_login : currentStream?.user_login) ??');
    expect(src).toContain('const closeEmotePicker = useCallback(() => setShowEmotePicker(false), []);');
  });
});

describe('the Android card', () => {
  it('follows Drops progress, and polls nothing while it is off', () => {
    const src = readFileSync(resolve(__dirname, '../../mobile/watch/DropProgressBar.tsx'), 'utf8');
    expect(src).toContain("s.settings.chat_design?.show_drops_in_chat ?? true");
    expect(src).toContain('const twitch = showInChat && isTwitchStream(currentStream) && !!channelLogin;');
  });
});
