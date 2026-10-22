import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ModerationTag } from './ModerationTag';
import { modTagTooltip, sameModerationContext, shortModName, strikeTagText } from './moderationTagText';
import type { ModerationContext } from '../../stores/chatConnectionStore';

const timeout: ModerationContext = { type: 'timeout', duration: 600 };
const named: ModerationContext = { ...timeout, by: 'kay_lee', reason: 'spoilers' };

function text(html: string): string {
  return html.replace(/<[^>]+>/g, '');
}

describe('the strikethrough bracket', () => {
  it('reads as before without a moderator', () => {
    expect(strikeTagText(timeout)).toBe('[timed out for 10m]');
    expect(strikeTagText({ type: 'timeout' })).toBe('[timed out]');
    expect(strikeTagText({ type: 'ban' })).toBe('[banned]');
    expect(strikeTagText({ type: 'deleted' })).toBe('[deleted by mod]');
  });

  it('names the moderator after the length', () => {
    expect(strikeTagText(named)).toBe('[timed out for 10m by kay_lee]');
    expect(strikeTagText({ type: 'ban', by: 'kay_lee' })).toBe('[banned by kay_lee]');
  });

  it('never shows the reason inline', () => {
    const html = renderToStaticMarkup(<ModerationTag context={named} style="strikethrough" />);
    expect(text(html)).toBe('[timed out for 10m by kay_lee]');
    expect(html).toContain('aria-label="Timed out for 10m by kay_lee. Reason: spoilers"');
  });
});

describe('the italic pill', () => {
  it('reads as before without a moderator', () => {
    const html = renderToStaticMarkup(<ModerationTag context={timeout} style="italic" />);
    expect(text(html)).toBe(' Timed out10m');
    expect(html).not.toContain('aria-label');
  });

  it('adds the moderator after the length, the reason only in the tooltip', () => {
    const html = renderToStaticMarkup(<ModerationTag context={named} style="italic" />);
    expect(text(html)).toBe(' Timed out10mkay_lee');
    expect(html).not.toContain('>spoilers<');
    expect((html.match(/sn-mod-tag__detail/g) ?? []).length).toBe(2);
  });

  it('cuts a long name and keeps it whole in the tooltip', () => {
    const long = 'a_very_long_moderator_name';
    const html = renderToStaticMarkup(<ModerationTag context={{ type: 'ban', by: long }} style="italic" />);
    expect(text(html)).toBe(` Banned${shortModName(long)}`);
    expect(shortModName(long)).toBe('a_very_long_m…');
    expect(modTagTooltip({ type: 'ban', by: long })).toBe(`By ${long}`);
  });

  it('has no tooltip when the pill already says everything', () => {
    expect(modTagTooltip({ type: 'ban', by: 'kay' })).toBeNull();
    expect(modTagTooltip(named)).toBe('Reason: spoilers');
  });
});

describe('the row memos', () => {
  it('repaint when the moderator or the reason arrives', () => {
    expect(sameModerationContext(timeout, { ...timeout })).toBe(true);
    expect(sameModerationContext(timeout, { ...timeout, by: 'kay_lee' })).toBe(false);
    expect(sameModerationContext({ ...timeout, by: 'kay_lee' }, named)).toBe(false);
    expect(sameModerationContext(null, undefined)).toBe(true);
    expect(sameModerationContext(null, timeout)).toBe(false);
  });
});
