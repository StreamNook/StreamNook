// @vitest-environment node
import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';
import { SmileyStylePicker } from './SmileyStylePicker';
import type { SmileySetChoice, SmileyStyle } from './smileyStyle';

// The picker reads IS_MOBILE while it renders, so one test file draws both shells.
const shell = vi.hoisted(() => ({ mobile: false }));
vi.mock('../../utils/platform', () => ({
  get IS_MOBILE() {
    return shell.mobile;
  },
}));

const face = (id: string, text: string) => ({ id, text, url: `https://cdn/${id}.png` });

function choice(key: string, name: string, available: boolean): SmileySetChoice {
  return {
    key,
    name,
    available,
    faces: [face(`${key}-1`, ':)'), face(`${key}-2`, '<3'), face(`${key}-3`, 'O_o')],
    preview: [face(`${key}-p1`, ':)'), face(`${key}-p2`, ';)'), face(`${key}-p3`, '<3')],
  };
}

const view = (over: Partial<SmileyStyle> = {}): SmileyStyle => ({
  can_switch: false,
  current: 'robots',
  upgrade_url: 'https://www.twitch.tv/settings/prime',
  sets: [choice('robots', 'Robots', true), choice('purple', 'Purple', false), choice('monkeys', 'Monkeys', false)],
  ...over,
});

function render(opts: { mobile?: boolean; view?: SmileyStyle; pending?: string | null } = {}) {
  shell.mobile = opts.mobile ?? false;
  return renderToStaticMarkup(
    <SmileyStylePicker view={opts.view ?? view()} pending={opts.pending ?? null} onSelect={() => {}} />,
  );
}

const tiles = (html: string) => html.match(/<button[^>]*role="radio"[^>]*>/g) ?? [];
const attr = (tag: string, name: string) => tag.match(new RegExp(`${name}="([^"]*)"`))?.[1];

describe('SmileyStylePicker', () => {
  it('is a radio group with one tile per set, available sets first', () => {
    const html = render({
      view: view({
        sets: [choice('robots', 'Robots', true), choice('purple', 'Purple', false), choice('monkeys', 'Monkeys', true)],
      }),
    });
    expect(html).toContain('role="radiogroup"');
    expect(tiles(html).map((t) => attr(t, 'aria-label'))).toEqual([
      'Robots',
      'Monkeys',
      'Purple, opens Twitch Prime',
    ]);
  });

  it('rings only the current tile', () => {
    const t = tiles(render({ view: view({ current: 'robots' }) }));
    expect(t.map((x) => attr(x, 'aria-checked'))).toEqual(['true', 'false', 'false']);
    expect(t.map((x) => (attr(x, 'class') ?? '').split(/\s+/).includes('ring-accent'))).toEqual([true, false, false]);
  });

  it('moves the ring with the view', () => {
    const t = tiles(
      render({
        view: view({
          current: 'monkeys',
          can_switch: true,
          sets: [choice('robots', 'Robots', true), choice('purple', 'Purple', true), choice('monkeys', 'Monkeys', true)],
        }),
      }),
    );
    expect(t.map((x) => attr(x, 'aria-checked'))).toEqual(['false', 'false', 'true']);
  });

  it('dims and grays only the sets the account cannot pick', () => {
    const t = tiles(render());
    const dimmed = t.map((x) => {
      const cls = attr(x, 'class') ?? '';
      return cls.includes('opacity-40') && cls.includes('grayscale');
    });
    expect(dimmed).toEqual([false, true, true]);
  });

  it('shows three faces on each tile at 28 px on a desktop', () => {
    const html = render();
    const tileHtml = html.split('<button').slice(1).map((s) => s.split('</button>')[0]);
    expect(tileHtml).toHaveLength(3);
    for (const t of tileHtml) {
      expect(t.match(/<img/g)).toHaveLength(3);
      expect(t).toContain('width="28"');
    }
  });

  it('shows the faces at 32 px on a phone, with no tooltip', () => {
    const html = render({ mobile: true });
    expect(html).toContain('width="32"');
    expect(html).not.toContain('width="28"');
    for (const t of tiles(html)) expect(t).not.toContain('title=');
  });

  it('gives a desktop tile a tooltip with the same words as its name', () => {
    // The shared Tooltip draws on hover, so static markup cannot show it: pin
    // that each tile is wrapped in one carrying its name, off on the phone.
    const source = readFileSync(resolve(__dirname, 'SmileyStylePicker.tsx'), 'utf8');
    expect(source).toContain('<Tooltip key={set.key} content={label} disabled={phone}>');
    for (const t of tiles(render())) expect(t).not.toContain('title=');
  });

  it('names a set without a name plainly', () => {
    const html = render({
      view: view({ sets: [choice('robots', 'Robots', true), choice('set:77', '', false)] }),
    });
    expect(tiles(html).map((t) => attr(t, 'aria-label'))).toEqual(['Robots', 'Smiley set, opens Twitch Prime']);
  });

  it('draws every face of the current set under the tiles, each with its text', () => {
    const html = render({ view: view({ current: 'robots' }) });
    const strip = html.slice(html.indexOf('data-smiley-faces'));
    expect(strip).toContain('robots-1.png');
    expect(strip).toContain('robots-3.png');
    expect(strip).not.toContain('purple-1.png');
    expect(strip).toContain('>:)<');
    expect(strip).toContain('>&lt;3<');
    expect(strip).toContain('>O_o<');
    expect(html.indexOf('data-smiley-faces')).toBeGreaterThan(html.lastIndexOf('</button>'));
  });

  it('marks the group busy while a change is going out', () => {
    expect(render({ pending: 'monkeys' })).toContain('aria-busy="true"');
    expect(render()).toContain('aria-busy="false"');
  });
});
