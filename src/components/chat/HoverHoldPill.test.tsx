// @vitest-environment node
// The hover hold's pill: what it says, when the list lets it show, and that
// it never takes the pointer or talks to screen readers.
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import { HoverHoldPill } from './HoverHoldPill';

const render = (count: number, shown: boolean) => renderToStaticMarkup(createElement(HoverHoldPill, { count, shown }));

describe('hover hold pill', () => {
  it('names the hold and what it keeps back, capped to one line', () => {
    expect(render(3, true)).toContain('Paused on Hover (3 new)');
    expect(render(150, true)).toContain('Paused on Hover (99+ new)');
  });

  it('is due only when the list says so', () => {
    expect(render(3, false)).not.toContain('data-due');
    expect(render(3, true)).toContain('data-due="true"');
  });

  it('never takes the pointer', () => {
    const html = render(3, true);
    expect(html).toContain('pointer-events-none');
    expect(html).not.toContain('<button');
  });

  it('stays out of screen readers', () => {
    expect(render(3, true)).toContain('aria-hidden="true"');
  });
});
