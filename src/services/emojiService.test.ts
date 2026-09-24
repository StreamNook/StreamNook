import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { parseEmojisProxied, parseEmojisSync } from './emojiService';

// Brazil's flag: regional indicators B (1F1E7) and R (1F1F7).
const BRAZIL = '\u{1F1E7}\u{1F1F7}';
const USA = '\u{1F1FA}\u{1F1F8}';
const LONE_B = '\u{1F1E7}';

const emojis = (text: string) => parseEmojisSync(text).filter((s) => s.type === 'emoji');

describe('flag emoji', () => {
  it('is one emoji, imaged by its full sequence', () => {
    const found = emojis(`Live from ${BRAZIL} tonight`);
    expect(found).toHaveLength(1);
    expect(found[0].content).toBe(BRAZIL);
    expect(found[0].emojiUrl).toMatch(/\/1f1e7-1f1f7\.png$/);
  });

  it('splits two adjacent flags at the pair boundary', () => {
    expect(emojis(`${BRAZIL}${USA}`).map((s) => s.content)).toEqual([BRAZIL, USA]);
  });

  it('leaves a lone regional-indicator letter as text', () => {
    const segments = parseEmojisSync(`grade ${LONE_B} ok`);
    expect(segments.every((s) => s.type === 'text')).toBe(true);
  });

  it('still finds ordinary emoji beside a flag', () => {
    expect(emojis(`\u{1F600} ${BRAZIL}`).map((s) => s.content)).toEqual(['\u{1F600}', BRAZIL]);
  });
});

describe('proxied flags', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation(async (cmd: string, args?: { text?: string; codepoint?: string }) =>
      cmd === 'convert_emoji_shortcodes' ? args?.text : `data:image/png;base64,${args?.codepoint}`,
    );
  });

  it('asks the proxy for the whole flag, never for its letters', async () => {
    await parseEmojisProxied(`Stream ${BRAZIL} ${LONE_B}`);
    const requested = invoke.mock.calls.filter(([cmd]) => cmd === 'get_emoji_image').map(([, a]) => a.codepoint);
    expect(requested).toEqual(['1f1e7-1f1f7']);
  });
});
