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

import DropsChatBar, { barLine, drawerAfter, DropsChatBarView, DropsPeekView, PREVIEW_LINE, showsHere } from './DropsChatBar';
import DropsDrawer, { rewardRows } from './DropsDrawer';
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
    // A link out to the game's page, said with the open-a-page arrow.
    expect(connect).toMatch(/>Connect<svg/);
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
    expect(barLine(view({ lead, more: 2 }))).toMatchObject({ name: 'Hoodie', status: '30m to watch', percent: 75, more: 2, ready: false, connectUrl: null });
  });
  it("carries the campaign's link when the lead needs the game account linked", () => {
    const row = { ...tier, id: 'b', lead: true, connect: true, status: 'Connect', bar: 100, current_minutes: 60 };
    const campaign = { ...notStarted, id: 'c1', started: true, link_url: 'https://game.example/link', drops: [row] };
    expect(barLine(view({ lead: { ...lead, status: 'Connect' }, campaigns: [campaign] }))?.connectUrl).toBe('https://game.example/link');
    const claimed = { ...campaign, drops: [{ ...row, claimed: true }] };
    expect(barLine(view({ lead, campaigns: [claimed] }))?.connectUrl).toBeNull();
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

describe('DropsChatBarView (the lip)', () => {
  const lip = (over: Partial<typeof PREVIEW_LINE> = {}) =>
    renderToStaticMarkup(<DropsChatBarView line={{ ...PREVIEW_LINE, ...over }} open={false} />);
  it('is a slim glazed lip with an inset track, never an edge strip', () => {
    const html = lip();
    expect(html).toContain('chrome-glaze drops-bar drops-lip');
    expect(html).toContain('h-6');
    // Square at the bottom, where it meets the message box.
    expect(html).toContain('border-radius:10px 10px 0 0');
    // The glass warms with the reward's progress.
    expect(html).toContain('--drops-heat:0.72');
    expect(html).toContain('width:72%');
    expect(html).not.toContain('liquid-glass-panel drops-bar');
    expect(html).not.toMatch(/absolute left-0 right-0 bottom-0/);
  });
  it('keeps only the time at rest and nothing about the other rewards', () => {
    const html = lip({ more: 2, moreArt: ['a.png', 'b.png'] });
    expect(html).toContain('<span class="font-semibold text-textPrimary">12m</span></span>');
    expect(html).not.toContain('to watch');
    expect(html).not.toContain('drops-stack');
  });
  it('turns green and says Ready when the reward is ready, with Claim left to the card and drawer', () => {
    const html = lip({ ready: true, percent: 100 });
    expect(html).not.toContain('<button');
    expect(html).not.toContain('Claim');
    expect(html).toContain('text-success">Ready</span>');
    expect(html).toMatch(/drops-lip[^"]*is-ready/);
    expect(html).toContain('drops-bar-fill block h-full rounded-full is-ready');
  });
});

describe('DropsPeekView (the hover card)', () => {
  const peek = (over: Partial<typeof PREVIEW_LINE> = {}, onClaim?: () => void) =>
    renderToStaticMarkup(<DropsPeekView line={{ ...PREVIEW_LINE, ...over }} claiming={false} onClaim={onClaim} />);
  it('says the reward in full with its time', () => {
    const html = peek();
    expect(html).toContain('chrome-glaze drops-bar drops-peek');
    expect(html).toContain('<span class="font-semibold text-textPrimary">12m</span> to watch');
    expect(html).toContain('2 more on this channel');
    expect(html).toContain('Click for all');
  });
  it('stacks the other rewards and counts only the ones beyond the pictures', () => {
    const two = peek({ more: 2, moreArt: ['a.png', 'b.png'] });
    expect(two.match(/<img/g)?.length).toBe(2);
    expect(two).not.toMatch(/>\+\d/);
    expect(peek({ more: 5, moreArt: ['a.png', 'b.png'] })).toContain('+3');
    expect(peek({ more: 1, moreArt: ['a.png'] })).toContain('1 more on this channel');
    const none = peek({ more: 0 });
    expect(none).not.toContain('drops-stack');
    expect(none).not.toContain('more on this channel');
  });
  it('offers Claim when the reward is ready', () => {
    expect(peek({ ready: true, percent: 100 }, () => {})).toContain('>Claim</button>');
  });
  it('offers Connect when Twitch would not grant the reward until the account is linked', () => {
    const html = peek({ status: 'Connect', connectUrl: 'https://game.example/link' });
    expect(html).toMatch(/>Connect<svg[^]*?<\/svg><\/button>/);
    expect(html).not.toContain('>Claim</button>');
    expect(peek({ status: 'Connect', connectUrl: null })).not.toContain('<button');
  });
  it('is square at the bottom like the lip it grows out of', () => {
    expect(peek()).toContain('border-radius:14px 14px 0 0');
  });
});

describe('the hover card wiring', () => {
  it('shows only for a mouse, after a beat, and never over the open drawer', () => {
    const src = readFileSync(resolve(__dirname, 'DropsChatBar.tsx'), 'utf8').replace(/\r\n/g, '\n');
    expect(src).toContain("if (e.pointerType !== 'mouse') return;");
    expect(src).toContain('{peek && !open && (');
    expect(src).toContain('if (next) setPeek(false);');
  });
  it('shows an arrow on the lip and a hand only on the card that takes the click', () => {
    const lip = renderToStaticMarkup(<DropsChatBarView line={PREVIEW_LINE} open={false} />);
    expect(lip).toContain('cursor-default');
    expect(lip).not.toContain('cursor-pointer');
    expect(renderToStaticMarkup(<DropsPeekView line={PREVIEW_LINE} claiming={false} />)).toContain('cursor-pointer');
  });
  it('appears at once under Motion Off and Performance mode', () => {
    const src = readFileSync(resolve(__dirname, 'DropsChatBar.tsx'), 'utf8').replace(/\r\n/g, '\n');
    expect(src).toContain("const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';");
    expect(src).toContain('const instant = motionOff || performanceQuiet;');
    expect(src).toContain('transition: instant ? INSTANT : PEEK_IN');
    expect(src).toContain('instant={instant}');
    const drawer = readFileSync(resolve(__dirname, 'DropsDrawer.tsx'), 'utf8');
    expect(drawer).toContain('transition: instant ? { duration: 0 }');
    const css = readFileSync(resolve(__dirname, '../../styles/globals.css'), 'utf8');
    expect(css).toContain('html[data-sn-perf] :is(.drops-bar-fill, .drops-tile, .chrome-glaze.drops-bar)');
  });
  it('closes the drawer on Escape wherever focus is, since the card it opened from goes away', () => {
    const src = readFileSync(resolve(__dirname, 'DropsChatBar.tsx'), 'utf8').replace(/\r\n/g, '\n');
    expect(src).toContain("window.addEventListener('keydown', onKey, true);");
    expect(src).toContain("window.removeEventListener('keydown', onKey, true);");
  });
  it('sits in the column above the message box, never over the message list', () => {
    const src = readFileSync(resolve(__dirname, 'DropsChatBar.tsx'), 'utf8').replace(/\r\n/g, '\n');
    expect(src).toContain('className="relative z-[55] -mb-px flex-shrink-0 px-2 pt-1.5"');
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

  it('lays several rewards out as glazed tiles in rows of three, the next one ringed', () => {
    const out = html([campaign('a', [drop('1', { lead: true, status: '12m to watch', bar: 80 }), drop('2'), drop('3', { claimed: true })])]);
    expect(out).toContain('grid gap-2 grid-cols-3');
    expect(out.match(/glaze-inset drops-tile /g)?.length).toBe(3);
    expect(out).toContain('glaze-inset drops-tile flex min-w-0 flex-col gap-1.5 p-2 is-next');
    expect(out).toContain('is-claimed');
    expect(out).toContain('1 of 3');
    expect(out).toContain('src="box.jpg"');
  });
  it('keeps every row full: two wider tiles for two, a bar for one left over', () => {
    const n = (count: number) => Array.from({ length: count }, (_, k) => drop(String(k + 1), { lead: k === 0 }));
    const two = html([campaign('a', n(2))]);
    expect(two).toContain('grid gap-2 grid-cols-2');
    expect(two).not.toContain('grid-cols-3');
    expect(two).toContain('aspect-ratio:16 / 10');
    const four = html([campaign('a', n(4))]);
    expect(four.match(/grid gap-2 grid-cols-3/g)?.length).toBe(1);
    expect(four).not.toContain('grid-cols-2');
    expect(four.match(/glaze-inset drops-tile flex items-center/g)?.length).toBe(1);
    const five = html([campaign('a', n(5))]);
    expect(five).toContain('grid gap-2 grid-cols-3');
    expect(five).toContain('grid gap-2 grid-cols-2');
    const seven = html([campaign('a', n(7))]);
    expect(seven.match(/grid gap-2 grid-cols-3/g)?.length).toBe(2);
    expect(seven.match(/glaze-inset drops-tile flex items-center/g)?.length).toBe(1);
  });
  it('splits rewards into rows of three with the rest last', () => {
    expect(rewardRows([1, 2, 3, 4]).map((r) => r.length)).toEqual([3, 1]);
    expect(rewardRows([1, 2, 3, 4, 5]).map((r) => r.length)).toEqual([3, 2]);
    expect(rewardRows([1, 2]).map((r) => r.length)).toEqual([2]);
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
