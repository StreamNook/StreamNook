// @vitest-environment node
// The swipe dial's readout: the level as a percent, or Auto once the
// brightness dial has handed the screen back to the phone.
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import { DialIndicator, type DialReading } from './DialIndicator';

const render = (dial: DialReading | null) => renderToStaticMarkup(createElement(DialIndicator, { dial }));

describe('dial readout', () => {
  it('says Auto over an empty track, whatever the value holds', () => {
    const html = render({ axis: 'brightness', value: 0.6, auto: true });
    expect(html).toContain('>Auto<');
    expect(html).toContain('aria-label="Brightness Auto"');
    expect(html).toContain('width:0%');
    expect(html).not.toContain('>60%<');
  });

  it('prints the level while the dial holds one', () => {
    const html = render({ axis: 'brightness', value: 0.42 });
    expect(html).toContain('>42%<');
    expect(html).toContain('aria-label="Brightness 42%"');
    expect(html).toContain('width:42%');
    expect(html).not.toContain('>Auto<');
  });

  it('prints the volume the same way', () => {
    expect(render({ axis: 'volume', value: 0.7 })).toContain('aria-label="Volume 70%"');
  });

  it('renders nothing without a dial', () => {
    expect(render(null)).toBe('');
  });
});
