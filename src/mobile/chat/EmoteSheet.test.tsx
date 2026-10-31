import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { EmoteSheetBody } from './EmoteSheetBody';
import type { EmoteCredit } from '../../services/emoteCredit';

const target = { provider: '7tv', id: '01G3WEGZN0000ET2J0MQP5YJ0G', name: 'RainTime', url: 'https://cdn.7tv.app/emote/x/1x.webp' };
const noop = () => {};
const body = (credit: EmoteCredit | null, done: boolean, stack = [target]) =>
  renderToStaticMarkup(
    <EmoteSheetBody target={target} stack={stack} credit={credit} done={done} onClose={noop} onSwitch={noop} onOpenTwitch={noop} onCopy={noop} />,
  );

const answer: EmoteCredit = {
  provider: '7tv',
  subtitle: 'Alias of xqcTours · 7TV channel emote',
  pageUrl: 'https://7tv.app/emotes/x',
  pageLabel: 'Open on 7TV',
  creators: [
    {
      role: 'uploader',
      name: 'Alpha',
      link: { kind: 'web', url: 'https://7tv.app/users/x', site: '7TV' },
      roleLabel: 'Made by',
      destination: '7TV profile',
    },
  ],
};

describe('the emote sheet', () => {
  it('shows the emote and holds the line under it while the lookup runs', () => {
    const html = body(null, false);
    expect(html).toContain('>RainTime<');
    expect(html).toContain('>Looking up who made it...<');
    expect(html).not.toContain('Alias of');
    expect(html).toContain('min-h-[18px]');
  });

  it("says Rust's words in order once the answer lands", () => {
    const html = body(answer, true);
    const order = ['>Alias of xqcTours · 7TV channel emote<', '>Made by<', '>Alpha<', '>7TV profile<', 'Open on 7TV', 'Copy name'].map((t) => html.indexOf(t));
    expect(order.every((i) => i >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(html).not.toContain('Looking up');
  });

  it('says nobody is listed only when the provider said so, and nothing for a failed lookup', () => {
    const nobody = body({ provider: 'bttv', subtitle: 'BetterTTV', emptyLine: "BetterTTV doesn't list who made this one." }, true);
    expect(nobody).toContain("doesn&#x27;t list who made this one.");
    const failed = body(null, true);
    expect(failed).not.toContain('list who made');
    expect(failed).toContain('Copy name');
  });

  it("lists a stack's emotes with the shown one marked", () => {
    const overlay = { ...target, id: '01G3WEGZN0000ET2J0MQP5YJ0H', name: 'Overlay' };
    const html = body(null, false, [target, overlay]);
    expect(html).toContain('aria-label="Emotes stacked here"');
    expect(html.match(/aria-selected="true"/g)?.length).toBe(1);
    expect(html).toContain('aria-label="Overlay"');
  });
});
