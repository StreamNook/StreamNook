import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { countLabel, rowsFor, type UnbanRequest } from './unbanRequests';

const row = (id: string, channel: string, name: string, text: string): UnbanRequest => ({
  id,
  channel,
  broadcaster_id: '1',
  user_id: id,
  user_login: name.toLowerCase(),
  user_name: name,
  text,
  created_at_ms: 0,
  age: '4 minutes ago',
});

describe('unban requests strip', () => {
  it('keeps only the pane channel rows', () => {
    const rows = [row('a', 'chan', 'Gus', 'hi'), row('b', 'other', 'Amy', 'yo')];
    expect(rowsFor(rows, 'Chan').map((r) => r.id)).toEqual(['a']);
  });

  it('names the count for the Tools tooltip', () => {
    expect(countLabel(1)).toBe('1 unban request');
    expect(countLabel(2)).toBe('2 unban requests');
  });
});

// SN-244: a strip above the message box stayed there for days in a channel
// that gets requests often. They live in Tools > Moderation instead.
describe('where unban requests live on desktop', () => {
  const widget = readFileSync(resolve(__dirname, '..', 'ChatWidget.tsx'), 'utf8');
  const menu = readFileSync(resolve(__dirname, 'ChannelToolsMenu.tsx'), 'utf8');

  it('nothing above the message box', () => {
    expect(widget).not.toContain('UnbanRequests');
  });

  it('a Moderation row with the count, opening its own page in the Tools menu', () => {
    expect(menu).toMatch(/useUnbanRequests\(\s*broadcasterId,\s*channelLogin,/);
    expect(menu).toMatch(/label="Unban requests"[\s\S]{0,80}badge=\{String\(unbanRows\.length\)\}/);
    expect(menu).toContain("setPage('unban')");
    expect(menu).toContain('<UnbanRequestsPage');
  });

  it('the phone composer menu counts them on its Moderation row', () => {
    const phone = readFileSync(resolve(__dirname, '..', '..', 'mobile', 'chat', 'ComposerMenuSheet.tsx'), 'utf8');
    expect(phone).toMatch(/useUnbanRequests\([\s\S]{0,120}open && !!onModeration/);
    expect(phone).toMatch(/value=\{unbanRows\.length > 0 [\s\S]{0,160}Moderation/);
  });
});
