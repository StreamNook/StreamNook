import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { PageLink } from '../../types/profileView';
import { ProfileNameplate } from './ProfileNameplate';

const link: PageLink = {
  href: 'https://streamnook.app/u/tyler',
  url: 'https://streamnook.app/tyler',
  prefix: 'streamnook.app/',
  name: 'tyler',
  accent: '#22D3EE',
};

// React escapes the apostrophe in an attribute; compare the plain text.
const html = (el: React.ReactElement) => renderToStaticMarkup(el).replace(/&#x27;/g, "'");

describe('ProfileNameplate', () => {
  it('shows the prefix, the name in the accent, and both controls with their labels', () => {
    const out = html(<ProfileNameplate slot="shown" link={link} size="phone" />);
    expect(out).toContain('href="https://streamnook.app/u/tyler"');
    expect(out).toContain("aria-label=\"Open tyler's StreamNook page\"");
    expect(out).toContain('aria-label="Copy link"');
    expect(out).toContain('>streamnook.app/<');
    expect(out).toContain('>tyler<');
    expect(out).toContain('color:#22D3EE');
    expect(out).not.toContain('inert');
  });

  it('is 34 high on the phone and 32 on the desktop, over the fixed floor', () => {
    const phone = html(<ProfileNameplate slot="shown" link={link} size="phone" />);
    const desktop = html(<ProfileNameplate slot="shown" link={link} size="desktop" />);
    expect(phone).toContain('height:34px');
    expect(desktop).toContain('height:32px');
    expect(phone).toContain('background:rgba(14,14,18,0.92)');
    expect(phone).toContain('width:34px');
    expect(phone).not.toContain('width:214px');
  });

  it('holds a visible, empty plate 214 wide and 34 high on the phone while reserved', () => {
    const out = html(<ProfileNameplate slot="reserved" link={null} size="phone" />);
    expect(out).not.toContain('streamnook.app');
    expect(out).not.toContain('<a ');
    expect(out).not.toContain('<button');
    expect(out).toContain('aria-hidden="true"');
    expect(out).toContain('inert=""');
    expect(out).toContain('height:34px;width:214px');
    expect(out).toContain('background:rgba(14,14,18,0.92)');
    expect(out).toContain('inset 0 0 0 1px rgba(255,255,255,0.10)');
    expect(out).toContain('height:42px');
    expect(out).toContain('opacity:1');
  });

  it('holds the same width, 32 high, on the desktop', () => {
    const out = html(<ProfileNameplate slot="reserved" link={null} size="desktop" />);
    expect(out).toContain('height:32px;width:214px');
    expect(out).toContain('height:40px');
  });

  it('collapses to nothing when hidden, and never prints the link', () => {
    const out = html(<ProfileNameplate slot="hidden" link={link} size="phone" />);
    expect(out).toContain('height:0');
    expect(out).toContain('opacity:0');
    expect(out).not.toContain('href=');
  });

  it('keeps the name from wrapping: the prefix is the second, reversible item', () => {
    const out = html(<ProfileNameplate slot="shown" link={link} size="desktop" />);
    expect(out.indexOf('>tyler<')).toBeLessThan(out.indexOf('>streamnook.app/<'));
    expect(out).toContain('flex-row-reverse');
    expect(out).toContain('text-ellipsis');
  });
});
