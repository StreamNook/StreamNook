// Every link a song carries is drawn with the right logo: the names Rust can
// give a link (services.rs SERVICES) each have a mark here, and each mark
// draws its own artwork rather than SocialLogo's fallback globe.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';
import { SocialLogo } from '../ui/SocialLogo';
import { markOf, SERVICE_MARK } from './serviceMarks';

const root = resolve(__dirname, '..', '..', '..');

/** The service names in Rust's SERVICES table. */
function rustServiceNames(): string[] {
  const src = readFileSync(resolve(root, 'src-tauri/src/services/song_id/services.rs'), 'utf8');
  const start = src.indexOf('pub const SERVICES');
  if (start < 0) throw new Error('SERVICES not found in services.rs');
  const table = src.slice(start, src.indexOf('];', start));
  return [...table.matchAll(/name:\s*"([^"]+)"/g)].map((m) => m[1]);
}

const draw = (platform: string) => renderToStaticMarkup(<SocialLogo platform={platform} size={16} />);

describe('music service marks', () => {
  it('has a mark for every service Rust names, and for nothing else', () => {
    const names = rustServiceNames();
    expect(names.length).toBeGreaterThanOrEqual(11);
    expect(Object.keys(SERVICE_MARK).sort()).toEqual([...names].sort());
  });

  it('draws each mark from its own artwork, not the fallback globe', () => {
    const globe = draw('not-a-platform');
    const seen = new Map<string, string>();
    for (const [name, mark] of Object.entries(SERVICE_MARK)) {
      const svg = draw(mark);
      expect(svg, `${name} -> ${mark}`).not.toBe(globe);
      if (mark !== 'music') {
        // Two services never share one logo by accident.
        expect(seen.get(svg), `${name} draws ${seen.get(svg)}'s logo`).toBeUndefined();
        seen.set(svg, name);
      }
    }
    expect(draw('music')).toBe(draw(markOf('Deezer')));
    expect(markOf('Something new')).toBe('music');
  });

  it('draws the brand colors the services use', () => {
    expect(draw('spotify')).toContain('#1ED760');
    expect(draw('applemusic')).toContain('#FA243C');
    expect(draw('youtubemusic')).toContain('#FF0000');
    expect(draw('shazam')).toContain('#0088FF');
    expect(draw('soundcloud')).toContain('#FF5500');
    expect(draw('beatport')).toContain('#01FF95');
  });
});
