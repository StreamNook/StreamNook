// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { SwitchList } from './_primitives';

const ITEMS = [
  { id: 'a', label: 'Alpha' },
  { id: 'b', label: 'Beta' },
  { id: 'c', label: 'Gamma' },
];

const render = (disabled?: boolean) =>
  renderToStaticMarkup(<SwitchList items={ITEMS} isOn={(id) => id === 'b'} onToggle={() => {}} disabled={disabled} />);

describe('SwitchList', () => {
  it('draws one switch per item, lit only for the on one, labelled by the item', () => {
    const html = render();
    const switches = html.match(/<button[^>]*role="switch"[^>]*>/g) ?? [];
    expect(switches).toHaveLength(3);
    expect(switches.map((s) => s.match(/aria-checked="(\w+)"/)?.[1])).toEqual(['false', 'true', 'false']);
    expect(switches.map((s) => s.match(/aria-label="([^"]*)"/)?.[1])).toEqual(['Alpha', 'Beta', 'Gamma']);
    expect(html).not.toContain('aria-pressed');
  });

  it('dims and disables every switch when disabled', () => {
    const html = render(true);
    expect(html).toContain('opacity-50');
    const switches = html.match(/<button[^>]*role="switch"[^>]*>/g) ?? [];
    expect(switches).toHaveLength(3);
    for (const s of switches) expect(s).toContain('disabled=""');
  });
});
