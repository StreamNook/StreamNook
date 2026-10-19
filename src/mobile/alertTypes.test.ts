// The alert types Rust lists (src-tauri/src/services/alert_types.json) post
// under Android channels this shell creates, and write live_notifications
// switches; the Notifications page and the setup step draw them as listed.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { NOTIFY_CHANNEL } from './notifications';
import { alertKey, alertOn, type AlertType } from './alertTypes';
import { settingIconFor } from './screens/settingsIcons';

const list = JSON.parse(
  readFileSync(resolve(__dirname, '../../src-tauri/src/services/alert_types.json'), 'utf8'),
) as { types: Array<AlertType & { platforms: string[] }> };

const android = list.types.filter((t) => t.platforms.includes('android'));

describe('alert types', () => {
  it('post every Android type under a channel the shell creates', () => {
    const channels = Object.values(NOTIFY_CHANNEL) as string[];
    expect(android.filter((t) => !channels.includes(t.channel)).map((t) => t.id)).toEqual([]);
  });

  it('write a live_notifications switch', () => {
    expect(list.types.filter((t) => alertKey(t) === null).map((t) => t.id)).toEqual([]);
  });

  it('give every phone row its glyph', () => {
    const phones = list.types.filter((t) => t.platforms.includes('android') || t.platforms.includes('ios'));
    expect(phones.filter((t) => settingIconFor('Notify me about', t.title) === null).map((t) => t.title)).toEqual([]);
  });

  it('read a switch that was never saved as on', () => {
    const live = android.find((t) => t.id === 'live')!;
    expect(alertOn(live, undefined)).toBe(true);
    expect(alertOn(live, { show_live_notifications: false })).toBe(false);
  });
});
