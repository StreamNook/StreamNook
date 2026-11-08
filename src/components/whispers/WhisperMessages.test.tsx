import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));

import { WhisperMessages } from './WhisperMessages';
import type { StoredWhisperMessage, WhisperDelivery } from '../../types';

const base: StoredWhisperMessage = {
  id: 'm',
  from_user_id: '99999999',
  from_user_login: 'alpha',
  from_user_name: 'Alpha',
  to_user_id: '1000000000',
  to_user_login: 'me',
  to_user_name: 'Me',
  message: 'plain words',
  timestamp: 1000,
  is_sent: false,
  parts: [],
  deleted: false,
  edited_ms: null,
  origin: 'twitch',
  delivery: null,
  failure: null,
  cut_to: null,
};

const NOW = new Date(2026, 10, 7, 21, 41).getTime();
const MIN = 60_000;

const draw = (messages: StoredWhisperMessage[]) =>
  renderToStaticMarkup(
    <WhisperMessages messages={messages} nowMs={NOW} formatTime={() => '12:00'} instant onRetry={() => {}} onDiscard={() => {}} />,
  );

const count = (html: string, needle: string) => html.split(needle).length - 1;

describe('a conversation as drawn', () => {
  it('draws emotes from the parts and plain text without them', () => {
    const html = draw([
      { ...base, id: 'a', message: 'hi Kappa', parts: [{ type: 'text', text: 'hi ' }, { type: 'emote', name: 'Kappa', url: 'https://example.invalid/kappa.png' }] },
      { ...base, id: 'b', message: 'no emotes here' },
    ]);
    expect(html).toContain('alt="Kappa"');
    expect(html).toContain('src="https://example.invalid/kappa.png"');
    expect(html).toContain('no emotes here');
  });

  it("labels each of your sends by Rust's state, and reads an unknown one as Sent", () => {
    // Ten minutes apart: each send is its own run, so each shows its line.
    const states = ['sending', 'unknown', 'delivered', 'queued'] as unknown as WhisperDelivery[];
    const sends = states.map((d, i) => ({ ...base, id: `s-${i}`, is_sent: true, delivery: d, timestamp: NOW - (40 - i * 10) * MIN }));
    const html = draw(sends);
    expect(html).toContain('>Sending...<');
    expect(html).toContain('>Checking...<');
    expect(html).toContain('>Delivered<');
    expect(html).toContain('>Sent<');
  });

  it('shows where a send stands only under the newest of your run', () => {
    const html = draw([
      { ...base, id: 'a', is_sent: true, delivery: 'delivered', timestamp: NOW - 3 * MIN },
      { ...base, id: 'b', is_sent: true, delivery: 'delivered', timestamp: NOW - 2 * MIN },
      { ...base, id: 'c', is_sent: true, delivery: 'sending', timestamp: NOW - MIN },
    ]);
    expect(count(html, 'data-delivery=""')).toBe(1);
    expect(html).toContain('>Sending...<');
    expect(html).not.toContain('>Delivered<');
  });

  it('heads the day with its line and puts no names on bubbles', () => {
    const html = draw([
      { ...base, id: 'old', timestamp: NOW - 24 * 60 * MIN },
      { ...base, id: 'new', timestamp: NOW - 5 * MIN },
    ]);
    expect(html).toContain('>Yesterday<');
    expect(html).toContain('Today · ');
    expect(html).not.toContain('>Alpha<');
  });

  it('shows the reason, Retry and Discard only under a send that did not arrive', () => {
    const html = draw([
      { ...base, id: 'x', is_sent: true, delivery: 'not_delivered', failure: 'Twitch did not deliver this whisper.' },
      { ...base, id: 'y', is_sent: true, delivery: 'delivered' },
      { ...base, id: 'z' },
    ]);
    expect(html).toContain('Twitch did not deliver this whisper.');
    expect(count(html, '>Retry<')).toBe(1);
    expect(count(html, '>Discard<')).toBe(1);
  });

  it('says when a message was deleted, edited or cut', () => {
    const html = draw([
      { ...base, id: 'd', deleted: true },
      { ...base, id: 'e', edited_ms: 2000 },
      { ...base, id: 'c', is_sent: true, delivery: 'delivered', cut_to: 500 },
    ]);
    expect(html).toContain('Message deleted');
    expect(html).toContain('>Edited<');
    expect(html).toContain('Twitch cut this whisper to 500 characters.');
  });
});
