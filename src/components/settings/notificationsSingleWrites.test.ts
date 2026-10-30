// The Notifications page writes one field per change. A whole-group write
// from the page's copy of `live_notifications` would put back a mute list a
// bell or Copy from Twitch had just changed.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const page = readFileSync(resolve(__dirname, './NotificationsSettings.tsx'), 'utf8');

describe('Notifications page writes single paths', () => {
  it('sends every switch change as its own live_notifications path', () => {
    const start = page.indexOf('const updateLiveNotifications = ');
    expect(start).toBeGreaterThan(-1);
    const end = page.indexOf('\n  };', start);
    const body = page.slice(start, end);
    expect(body).toContain("setSettingsFields(fieldsIn('live_notifications', updates))");
    expect(body).not.toContain('updateSettings(');
  });

  it('never writes the live_notifications group whole', () => {
    expect(page).not.toMatch(/live_notifications:\s*\{/);
  });
});
