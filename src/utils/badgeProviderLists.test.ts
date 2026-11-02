// Every badge provider Rust knows must be named in each webview list that
// enumerates providers, so adding a provider on one side cannot leave a list
// (settings, the gallery, the profile card, the overlay) without it.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(__dirname, '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

function rustProviderKeys(): string[] {
  const src = read('src-tauri/src/services/badge_service.rs');
  const start = src.indexOf('pub fn as_key');
  const end = src.indexOf('\n        }', start);
  const body = src.slice(start, end);
  return [...body.matchAll(/=> "([a-z0-9-]+)"/g)].map((m) => m[1]).filter((k) => k !== 'twitch');
}

const LISTS = [
  'src/services/badgeService.ts',
  'src/utils/badgeSpotlight.ts',
  'src/components/ChatClientsGallery.tsx',
  'src/components/UserProfileCard.tsx',
  'src/components/settings/ChatSettings.tsx',
  'src/components/overlay/overlayConfig.ts',
];

describe('badge provider lists', () => {
  const keys = rustProviderKeys();

  it('reads the new providers from Rust', () => {
    expect(keys).toContain('limerino');
    expect(keys).toContain('jilchat');
  });

  for (const file of LISTS) {
    it(`${file} names every provider`, () => {
      const text = read(file);
      for (const key of keys) expect(text, `${file} is missing '${key}'`).toContain(`'${key}'`);
    });
  }
});
