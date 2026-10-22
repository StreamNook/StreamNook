// @vitest-environment node
// Run with: npm test
//
// Entrances: rows that arrive together enter one after another, catch-up rows
// (history, restored chat, a reconnect's refill) appear without one, and the
// stagger never delays the retract fade.

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { OverlayChat, entranceDelays } from './OverlayChat';
import { SAMPLE_MESSAGES, type OverlayMessage } from './sampleMessages';
import { DEFAULT_OVERLAY_STYLE } from './overlayConfig';

const template = SAMPLE_MESSAGES.find(
  (m) => (m.provider ?? 'twitch') === 'twitch' && !m.metadata?.system_message && !m.tags?.['system-msg'],
)!;

const row = (id: string, backfill = false): OverlayMessage => ({
  ...template,
  id,
  content: `hello ${id}`,
  segments: [{ type: 'text', content: `hello ${id}` }],
  metadata: { ...template.metadata!, from_backfill: backfill || undefined },
});

const render = (messages: OverlayMessage[]) =>
  renderToStaticMarkup(<OverlayChat messages={messages} style={{ ...DEFAULT_OVERLAY_STYLE, entrance: 'fade' }} />);

/** The opening tag of each row, in order. */
const rowTags = (html: string) => html.match(/<div class="sn-ov-row[^>]*>/g) ?? [];

describe('entrances', () => {
  it('rows that arrive together enter 30 ms apart', () => {
    const tags = rowTags(render([row('a'), row('b'), row('c')]));
    expect(tags).toHaveLength(3);
    expect(tags.every((t) => t.includes('sn-ov-fade'))).toBe(true);
    expect(tags[0]).not.toContain('--sn-ov-enter-delay');
    expect(tags[1]).toContain('--sn-ov-enter-delay:30ms');
    expect(tags[2]).toContain('--sn-ov-enter-delay:60ms');
  });

  it('a catch-up row appears without an entrance', () => {
    const tags = rowTags(render([row('old', true), row('new')]));
    expect(tags[0]).not.toContain('sn-ov-fade');
    expect(tags[1]).toContain('sn-ov-fade');
  });

  it('the stagger is the entrance delay only, never the retract fade', () => {
    const html = render([row('a')]);
    expect(html).toMatch(/\.sn-ov-fade \{ animation: snOvFade 240ms ease-in-out var\(--sn-ov-enter-delay, 0ms\) backwards;/);
    expect(html).toMatch(/\.sn-ov-out \{ animation: snOvOut 260ms ease both;/);
  });

  it('a row seen a while ago does not enter again, and the stagger is capped', () => {
    const now = 10_000;
    const list = Array.from({ length: 12 }, (_, i) => row(`r${i}`));
    const seen = new Map(list.map((m) => [m.id, now]));
    const arrival = new Map(list.map((m, i) => [m.id, i]));
    seen.set('r0', now - 5_000);
    const delays = entranceDelays(list, seen, arrival, now);
    expect(delays.has('r0')).toBe(false);
    expect(delays.get('r1')).toBe(30);
    expect(delays.get('r11')).toBe(240);
  });
});
