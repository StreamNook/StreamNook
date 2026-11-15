import { beforeEach, describe, expect, it, vi } from 'vitest';

// The badge spotlight lives in its own store, which the boot watch on the app
// store never sees. Opening its card while the app boots notes the action
// itself, so the link still waiting in Rust is dropped, as for any page.

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }));

const app = vi.hoisted(() => ({ state: { isBooting: true } }));
vi.mock('./AppStore', () => ({
  useAppStore: Object.assign(vi.fn(), { getState: () => app.state, setState: vi.fn(), subscribe: vi.fn() }),
}));

const win = vi.hoisted(() => ({ main: true }));
vi.mock('../utils/platform', async (original) => ({
  ...(await original<typeof import('../utils/platform')>()),
  IS_MAIN_WINDOW: true,
  isMainWindow: () => win.main,
}));

import { openBadgeSpotlight, useBadgeSpotlightStore } from './badgeSpotlightStore';
import { noteLinkStamp, noticeWorkSettled, resetNoticeReportForTest, runAsAutomatic } from '../utils/noticeReport';
import type { BadgeRef } from '../utils/badgeSpotlight';

const badge: BadgeRef = { provider: 'ffz', id: 'moderator', title: 'Moderator', art: [] };
const superseded = () => invokeMock.mock.calls.filter(([c]) => c === 'link_superseded');

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  resetNoticeReportForTest();
  win.main = true;
  app.state.isBooting = true;
  noteLinkStamp(4);
  useBadgeSpotlightStore.setState({ badge: null, fromBadges: false });
});

describe('openBadgeSpotlight while the app boots', () => {
  it('tells Rust the page acted, once, with the stamp it has heard, and opens the card', async () => {
    // Fails against the old store: opening the card notes nothing.
    openBadgeSpotlight(badge);
    await noticeWorkSettled();
    expect(superseded()).toHaveLength(1);
    expect(superseded()[0][1]).toEqual({ seen: 4 });
    expect(useBadgeSpotlightStore.getState().badge).toBe(badge);
  });

  it('tells Rust nothing once boot is over', async () => {
    app.state.isBooting = false;
    openBadgeSpotlight(badge);
    await noticeWorkSettled();
    expect(superseded()).toHaveLength(0);
    expect(useBadgeSpotlightStore.getState().badge).toBe(badge);
  });

  it('tells Rust nothing from a popout', async () => {
    win.main = false;
    openBadgeSpotlight(badge);
    await noticeWorkSettled();
    expect(superseded()).toHaveLength(0);
  });

  it('tells Rust nothing when the app opens it by itself', async () => {
    runAsAutomatic(() => openBadgeSpotlight(badge));
    await noticeWorkSettled();
    expect(superseded()).toHaveLength(0);
  });
});
