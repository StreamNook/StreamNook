// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { MotionGlobalConfig } from 'framer-motion';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
vi.mock('../utils/platform', () => ({
  IS_MOBILE: false,
  IS_ANDROID: false,
  IS_MAC: false,
  IS_LINUX: false,
  IS_WEBKITGTK: false,
  IS_MAIN_WINDOW: true,
  isMainWindow: () => true,
}));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));
// The modules SEC-11's sharp art put in the body's import graph (the same four the slot test mocks).
vi.mock('./badge/SharpBadgeImg', () => ({
  SharpBadgeImg: ({ src, alt }: { src: string; alt: string }) => <img src={src} alt={alt} />,
}));
vi.mock('../services/streamnookArt', () => ({
  getStreamnookArtVersion: () => 0,
  subscribeStreamnookArt: () => () => {},
  streamnookArtCopyOf: () => null,
  streamnookArtSrc: (url: string) => url,
  requestStreamnookArt: () => {},
}));
vi.mock('../services/assetCacheQueue', () => ({
  onAssetsCached: () => () => {},
  requestAssetCaching: () => {},
  forgetAssetRequests: () => {},
  setAssetCacheBurst: () => {},
}));
vi.mock('../utils/cacheFileUrl', () => ({
  ensureCacheFileServer: async () => {},
  awaitCacheFileServer: async () => {},
  waitsForCacheFileServer: () => false,
  isDiskCopyUrl: () => false,
  cacheFileUrl: (url: string) => url,
}));
vi.mock('./drops/ChannelPickerModal', () => ({
  default: () => <div data-testid="channel-picker" />,
}));

import { invoke } from '@tauri-apps/api/core';
import BadgeDetailOverlay from './BadgeDetailOverlay';
import type { SelectedBadge } from './BadgeDetailSlot';
import { BADGE_LOAD_FAILED_TEXT, BADGE_NOT_FOUND_TEXT } from '../utils/badgeLinkOpen';

const badge = {
  id: '2',
  image_url_1x: 'https://static.example/2/1',
  image_url_2x: 'https://static.example/2/2',
  image_url_4x: 'https://static.example/2/3',
  title: 'Clover 2',
  description: 'Earned by watching.',
  click_action: null,
  click_url: null,
};
const shown: NonNullable<SelectedBadge> = { kind: 'badge', badge, setId: 'clover' };
const link = (
  state: 'loading' | 'missing' | 'failed',
  read = 1,
  retry = false,
  key = 'clover/2',
): NonNullable<SelectedBadge> => ({
  kind: 'link',
  key,
  state,
  read,
  ...(retry ? { retry: true as const } : {}),
});

let host: HTMLElement;
let root: Root;
const onClose = vi.fn();
const onBack = vi.fn();
const onRetry = vi.fn();
const q = (selector: string) => host.querySelector(selector);
const text = () => host.textContent ?? '';

async function show(selected: NonNullable<SelectedBadge>) {
  await act(async () => {
    root.render(<BadgeDetailOverlay selected={selected} onClose={onClose} onBack={onBack} onRetry={onRetry} />);
  });
}
const click = (el: Element | null) => act(async () => (el as HTMLElement).click());

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(() => new Promise(() => {}));
  onClose.mockReset();
  onBack.mockReset();
  onRetry.mockReset();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('BadgeDetailOverlay with a badge', () => {
  // Fails if the title row stayed in the frame as a fixed child, or moved out of the header between Back and Close.
  it('draws the title between Back and Close in the header, and the art in the scroll area', async () => {
    await show(shown);
    const header = q('.border-b') as HTMLElement;
    const back = header.querySelector('button[aria-label="Back to badges"]') as HTMLElement;
    const close = header.querySelector('button[aria-label="Close"]') as HTMLElement;
    const title = header.querySelector('h2') as HTMLElement;
    expect(title.textContent).toBe('Clover 2');
    expect(back.compareDocumentPosition(title) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(title.compareDocumentPosition(close) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(header.textContent).toContain('Twitch Chat Badge');
    expect(q('.custom-scrollbar img[alt="Clover 2"]')).not.toBeNull();
  });

  it('Back, Close and the backdrop call their handlers', async () => {
    await show(shown);
    await click(q('button[aria-label="Back to badges"]'));
    expect(onBack).toHaveBeenCalledTimes(1);
    await click(q('button[aria-label="Close"]'));
    expect(onClose).toHaveBeenCalledTimes(1);
    await click(q('.absolute.inset-0'));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  // Fails if the channel picker is drawn inside the card: the card's transform would make its fixed
  // box relative to the card instead of the window.
  it('draws the channel picker beside the card, not inside it', async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_badge_drop_campaign') {
        return {
          id: 'c1',
          name: 'Campaign',
          game_name: 'Game',
          game_id: '1',
          is_acl_based: true,
          allowed_channels: [1, 2, 3, 4].map((n) => ({ id: `${n}`, login: `chan${n}` })),
          time_based_drops: [],
        };
      }
      return new Promise(() => {});
    });
    await show(shown);
    const opener = () => [...host.querySelectorAll('button')].find((b) => b.textContent?.includes('Participating channels'));
    for (let i = 0; i < 50 && !opener(); i += 1) {
      await act(async () => {
        await new Promise((r) => setTimeout(r, 5));
      });
    }
    expect(opener()).toBeDefined();
    await click(opener() ?? null);
    const picker = q('[data-testid="channel-picker"]');
    expect(picker).not.toBeNull();
    expect(picker?.closest('.liquid-glass-panel')).toBeNull();
    expect(picker?.parentElement).toBe(q('.liquid-glass-panel')?.parentElement);
  });
});

describe('BadgeDetailOverlay with a link placeholder', () => {
  it('loading: a skeleton title and the Twitch Chat Badge line, no h2, a hidden "Loading badge"', async () => {
    await show(link('loading'));
    expect(q('h2')).toBeNull();
    expect(q('.animate-pulse[aria-hidden="true"]')).not.toBeNull();
    expect(q('.sr-only')?.textContent).toBe('Loading badge');
    expect(q('.border-b')?.textContent).toContain('Twitch Chat Badge');
    expect(host.querySelectorAll('.custom-scrollbar .animate-pulse')).toHaveLength(3);
  });

  it('missing: the not-found words and no button in the body', async () => {
    await show(link('missing'));
    expect(text()).toContain(BADGE_NOT_FOUND_TEXT);
    expect(q('.custom-scrollbar button')).toBeNull();
    expect(q('.animate-pulse')).toBeNull();
  });

  it('failed: the could-not-load words and one Try again that asks the owner to read again', async () => {
    await show(link('failed'));
    expect(text()).toContain(BADGE_LOAD_FAILED_TEXT);
    expect(text()).not.toContain(BADGE_NOT_FOUND_TEXT);
    await click(q('.custom-scrollbar button'));
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(onRetry).toHaveBeenCalledWith('clover/2');
  });

  // Fails if the body swaps the failed line for the loading body while the retry runs: the button would unmount.
  it('Try again stays mounted, aria-disabled and focused while the retry runs, and a second press is ignored', async () => {
    await show(link('failed', 1));
    const button = q('.custom-scrollbar button') as HTMLButtonElement;
    expect(button.getAttribute('aria-disabled')).toBe('false');
    await act(async () => button.focus());
    await click(button);
    await show(link('loading', 2, true));
    expect(q('.custom-scrollbar button')).toBe(button);
    expect(button.getAttribute('aria-disabled')).toBe('true');
    expect(document.activeElement).toBe(button);
    expect(text()).not.toContain('Loading badge');
    await click(button);
    expect(onRetry).toHaveBeenCalledTimes(1);
    await show(link('failed', 2, true));
    expect(q('.custom-scrollbar button')).toBe(button);
    expect(button.getAttribute('aria-disabled')).toBe('false');
    expect(document.activeElement).toBe(button);
  });

  // Fails if a Try again press is remembered by the frame instead of the slot: the next link's
  // placeholder, drawn in the same mounted frame, would show the failed line while it loads.
  it('a new link after a Try again shows its own loading body', async () => {
    await show(link('failed', 1));
    await click(q('.custom-scrollbar button'));
    await show(link('loading', 2, true));
    await show(link('failed', 2, true));
    await show(link('loading', 3, false, 'other/1'));
    expect(text()).toContain('Loading badge');
    expect(text()).not.toContain(BADGE_LOAD_FAILED_TEXT);
    expect(q('.custom-scrollbar button')).toBeNull();
  });

  // Fails the same way for the same badge linked again: a repeat link is a fresh read, not a retry.
  it('a repeat link of the same badge after a Try again shows its own loading body', async () => {
    await show(link('failed', 1));
    await click(q('.custom-scrollbar button'));
    await show(link('loading', 2, true));
    await show(link('loading', 3));
    expect(text()).toContain('Loading badge');
    expect(text()).not.toContain(BADGE_LOAD_FAILED_TEXT);
  });

  // Fails if the frame is rebuilt when the badge arrives: the card would be a new node and the box would flash.
  it('keeps the same card and backdrop when the badge arrives', async () => {
    await show(link('loading'));
    const card = q('.liquid-glass-panel');
    const backdrop = card?.parentElement;
    await show(shown);
    expect(q('.liquid-glass-panel')).toBe(card);
    expect(q('.liquid-glass-panel')?.parentElement).toBe(backdrop);
    expect(q('h2')?.textContent).toBe('Clover 2');
    expect(q('.sr-only')).toBeNull();
  });
});
