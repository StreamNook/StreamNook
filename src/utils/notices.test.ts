import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// A small notice switched off (Settings > Notifications > Small notices)
// shows nowhere; errors never carry a notice and always show.

const { emit } = vi.hoisted(() => ({ emit: vi.fn(async (_event: string, _payload?: unknown) => {}) }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit }));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from '../stores/AppStore';
import { NOTICE_IDS, noticeHidden, noticeKey } from './notices';
import type { LiveNotificationSettings, Settings } from '../types';

const rust = JSON.parse(
  readFileSync(new URL('../../src-tauri/src/services/notice_types.json', import.meta.url), 'utf8'),
) as { notices: { id: string; setting: string }[] };

const withPrefs = (prefs: Partial<LiveNotificationSettings>) =>
  useAppStore.setState({
    toasts: [],
    settings: { live_notifications: { enabled: true, use_toast: true, use_dynamic_island: true, ...prefs } } as unknown as Settings,
  });

const messages = () => useAppStore.getState().toasts.map((t) => t.message);

beforeEach(() => emit.mockClear());

describe('small notices', () => {
  it('lists the same ids as Rust, each writing live_notifications.notice_<id>', () => {
    expect(rust.notices.map((n) => n.id)).toEqual([...NOTICE_IDS]);
    for (const n of rust.notices) expect(n.setting).toBe(`live_notifications.${String(noticeKey(n.id as never))}`);
  });

  it('a switch is on until it is turned off', () => {
    expect(noticeHidden(undefined, 'copied')).toBe(false);
    expect(noticeHidden({}, 'copied')).toBe(false);
    expect(noticeHidden({ notice_copied: true }, 'copied')).toBe(false);
    expect(noticeHidden({ notice_copied: false }, 'copied')).toBe(true);
    expect(noticeHidden({ notice_copied: false }, undefined)).toBe(false);
  });

  it('a notice switched off shows no toast and no island entry', () => {
    withPrefs({ notice_welcome_back: false });
    useAppStore.getState().addToast('Welcome back, Brandon!', 'success', undefined, { notice: 'welcome_back' });
    expect(messages()).toEqual([]);
    expect(emit).not.toHaveBeenCalled();
  });

  it('only that notice: another one and an untagged toast still show', () => {
    withPrefs({ notice_copied: false });
    useAppStore.getState().addToast('Quality changed to 720p60', 'success', undefined, { notice: 'quality_changed' });
    useAppStore.getState().addToast('Pinned message', 'success');
    expect(messages()).toEqual(['Quality changed to 720p60', 'Pinned message']);
  });

  it('an error always shows, even with a notice named', () => {
    withPrefs({ notice_copied: false });
    useAppStore.getState().addToast('Could not copy the name', 'error', undefined, { notice: 'copied' });
    expect(messages()).toEqual(['Could not copy the name']);
  });
});
