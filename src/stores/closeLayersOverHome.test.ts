import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// Every layer drawn above Home that the store holds is closed through its own
// closer (some also record state), and App's overlay block cannot gain a layer
// that this list does not classify.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(),
}));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from './AppStore';
import { useBadgeSpotlightStore } from './badgeSpotlightStore';
import type { BadgeRef } from '../utils/badgeSpotlight';

const get = () => useAppStore.getState();
const clipInfo = { id: 'c1', title: 'A clip' } as never;
const aBadge: BadgeRef = { provider: 'twitch', id: 'clover/1', title: 'Clover', art: [] };

/** One entry per closer: how to open the layer, the closer's key and the call it must get. */
const layers: Array<{
  name: string;
  open: Record<string, unknown>;
  closer: string;
  args: unknown[];
}> = [
  { name: 'Settings', open: { isSettingsOpen: true }, closer: 'closeSettings', args: [] },
  { name: 'Badges', open: { showBadgesOverlay: true }, closer: 'setShowBadgesOverlay', args: [false] },
  { name: 'Whispers', open: { showWhispersOverlay: true }, closer: 'setShowWhispersOverlay', args: [false] },
  { name: 'Emote sets', open: { showEmoteSetsOverlay: true }, closer: 'setShowEmoteSetsOverlay', args: [false] },
  { name: 'Drops', open: { showDropsOverlay: true }, closer: 'setShowDropsOverlay', args: [false] },
  { name: 'Live streams', open: { showLiveStreamsOverlay: true }, closer: 'setShowLiveStreamsOverlay', args: [false] },
  { name: 'Marketplace', open: { showMarketplaceOverlay: true }, closer: 'setShowMarketplaceOverlay', args: [false] },
  { name: 'Dashboard', open: { showDashboardOverlay: true }, closer: 'setShowDashboardOverlay', args: [false] },
  { name: 'Profile viewer', open: { profileViewerUserId: '42' }, closer: 'closeProfileViewer', args: [] },
  { name: 'Songs', open: { songsPanel: { focus: null } }, closer: 'closeSongs', args: [] },
  { name: 'Emote spotlight', open: { emoteSpotlight: { id: 'e', name: 'Kappa' } }, closer: 'setEmoteSpotlight', args: [null] },
  { name: 'Command palette', open: { isCommandPaletteOpen: true }, closer: 'closeCommandPalette', args: [] },
  { name: 'Profile modal', open: { profileModalUser: { user_login: 'chan' } }, closer: 'setProfileModalUser', args: [null] },
  { name: 'Clip modal', open: { clipModal: { url: 'https://clips/c1', info: clipInfo } }, closer: 'closeClipModal', args: [] },
  { name: 'VOD modal', open: { vodModal: { url: 'https://vods/v1', info: clipInfo } }, closer: 'closeVodModal', args: [] },
];

const closed: Record<string, unknown> = {
  isSettingsOpen: false,
  showBadgesOverlay: false,
  showWhispersOverlay: false,
  showEmoteSetsOverlay: false,
  showDropsOverlay: false,
  showLiveStreamsOverlay: false,
  showMarketplaceOverlay: false,
  showDashboardOverlay: false,
  profileViewerUserId: null,
  songsPanel: null,
  emoteSpotlight: null,
  isCommandPaletteOpen: false,
  profileModalUser: null,
  clipModal: null,
  vodModal: null,
};

const realClosers = Object.fromEntries(layers.map((l) => [l.closer, (get() as never)[l.closer]]));

beforeEach(() => {
  useAppStore.setState({ ...realClosers, ...closed, isHomeActive: false });
  useBadgeSpotlightStore.setState({ badge: null, fromBadges: false });
});

describe('closeLayersOverHome', () => {
  it.each(layers)('$name goes through its own closer, once', ({ open, closer, args }) => {
    // Fails against the old store: the action does not exist.
    const spy = vi.fn();
    useAppStore.setState({ ...open, [closer]: spy });
    get().closeLayersOverHome();
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith(...args);
  });

  it('calls no closer for a layer that is already closed', () => {
    const spies = layers.map((l) => {
      const spy = vi.fn();
      useAppStore.setState({ [l.closer]: spy });
      return spy;
    });
    get().closeLayersOverHome();
    for (const spy of spies) expect(spy).not.toHaveBeenCalled();
  });

  it('closes every layer for real, and the state each closer records goes with it', () => {
    useAppStore.setState({
      isSettingsOpen: true,
      settingsInitialTab: 'Moderation' as never,
      showWhispersOverlay: true,
      whisperTargetUser: { id: '1', login: 'chan', display_name: 'Chan' },
      showEmoteSetsOverlay: true,
      emoteSpotlight: { id: 'e', name: 'Kappa' },
      profileViewerUserId: '42',
      profileViewerPreview: { hiddenSections: [], profileTheme: 't', badgeRevision: 0 },
      showBadgesOverlay: true,
      badgesOverlayInitialTarget: { tab: 'twitch-badges' },
    });
    get().closeLayersOverHome();
    const s = get();
    expect(s.isSettingsOpen).toBe(false);
    expect(s.settingsInitialTab).toBeNull();
    expect(s.showWhispersOverlay).toBe(false);
    expect(s.whisperTargetUser).toBeNull();
    expect(s.showEmoteSetsOverlay).toBe(false);
    expect(s.emoteSpotlight).toBeNull();
    expect(s.profileViewerUserId).toBeNull();
    expect(s.profileViewerPreview).toBeNull();
    expect(s.showBadgesOverlay).toBe(false);
    expect(s.badgesOverlayInitialTarget).toBeNull();
  });

  it('closes the badge spotlight through its own store', () => {
    useBadgeSpotlightStore.setState({ badge: aBadge, fromBadges: true });
    get().closeLayersOverHome();
    expect(useBadgeSpotlightStore.getState().badge).toBeNull();
    expect(useBadgeSpotlightStore.getState().fromBadges).toBe(false);
  });

  it('leaves Home and the stream alone', () => {
    useAppStore.setState({ isHomeActive: false, streamUrl: 'http://localhost/live.m3u8', showDropsOverlay: true });
    get().closeLayersOverHome();
    expect(get().isHomeActive).toBe(false);
    expect(get().streamUrl).toBe('http://localhost/live.m3u8');
  });

  it('writes no flag by hand where a closer exists (source pin)', () => {
    const text = readFileSync(resolve(__dirname, 'AppStore.ts'), 'utf8').replace(/\r\n/g, '\n');
    const start = text.lastIndexOf('\n  closeLayersOverHome: () => {');
    const end = text.indexOf('\n  },\n', start);
    expect(start).toBeGreaterThan(-1);
    const body = text.slice(start, end);
    expect(body).not.toMatch(/\bset\(/);
    // One call per layer the table lists, the spotlight's through its own store.
    for (const l of layers) expect(body).toContain(`.${l.closer}(`);
    expect(body).toContain('useBadgeSpotlightStore.getState()');
  });
});

// App mounts the layers the store cannot close (changelog, the badge detail
// slot) and the ones a link must keep. A new root in that block has to be
// classified here, so a layer cannot be added without deciding what a link does
// to it.
describe('App overlay block', () => {
  type Kind = 'closed' | 'kept' | 'not-a-layer';
  const ROOTS: Record<string, Kind> = {
    SettingsDialog: 'closed',
    OwnProfilePreview: 'closed',
    DropsOverlay: 'closed',
    MarketplaceOverlay: 'closed',
    DropProgressController: 'not-a-layer',
    EmoteSetsOverlay: 'closed',
    EmoteSpotlight: 'closed',
    BadgeSpotlightHost: 'closed',
    SearchProfileModal: 'closed',
    BadgeDetailSlot: 'closed',
    ChangelogOverlay: 'closed',
    WhispersWidget: 'closed',
    SongsPanelHost: 'closed',
    PluginRuntimeBridge: 'not-a-layer',
    PluginUiHost: 'not-a-layer',
    PluginUpdatesChecker: 'not-a-layer',
    PluginOverlayOutlet: 'not-a-layer',
    SetupWizard: 'kept',
    AnnouncementsBanner: 'not-a-layer',
    ToastManager: 'not-a-layer',
    DeviceLoginOverlay: 'kept',
    EntitlementUnlockNote: 'not-a-layer',
    CommandPalette: 'closed',
    InputContextMenuHost: 'not-a-layer',
    ClipModal: 'closed',
    ClipEditor: 'kept',
    TwitchOverlay: 'kept',
  };
  const WRAPPERS = new Set(['Suspense', 'AnimatePresence', 'ErrorBoundary']);

  /** The roots of the block after the boot overlay's LoadingWidget, found by text. */
  function roots(): string[] {
    const app = readFileSync(resolve(__dirname, '..', 'App.tsx'), 'utf8').replace(/\r\n/g, '\n');
    const loading = app.indexOf('<LoadingWidget fullScreen={false} message="Loading StreamNook" />');
    expect(loading).toBeGreaterThan(-1);
    const startAt = app.indexOf('</AnimatePresence>', loading) + '</AnimatePresence>'.length;
    const last = app.indexOf('<TwitchOverlay />', startAt);
    expect(last).toBeGreaterThan(startAt);
    const block = app.slice(startAt, last + '<TwitchOverlay />'.length);
    const starts = [...block.matchAll(/^ {6}(?:<(?!\/)|\{)/gm)].map((m) => m.index as number);
    expect(starts.length).toBeGreaterThan(10);
    const names: string[] = [];
    starts.forEach((from, i) => {
      const root = block.slice(from, starts[i + 1] ?? block.length);
      if (root.trimStart().startsWith('{/*')) return;
      const tags = [...root.matchAll(/<([A-Za-z][\w.]*)/g)].map((m) => m[1]);
      names.push(tags.find((t) => !WRAPPERS.has(t)) ?? tags[0] ?? root.trim().slice(0, 40));
    });
    return names;
  }

  it('lists every root the block mounts, each classified', () => {
    // Fails the moment App.tsx mounts a root that is not in ROOTS.
    const found = roots();
    expect(found.filter((n) => !(n in ROOTS))).toEqual([]);
  });

  it('mounts the badge detail slot inside the block', () => {
    // The slot holds both badge mounts, so neither can be a root of its own.
    expect(roots()).toContain('BadgeDetailSlot');
  });
});
