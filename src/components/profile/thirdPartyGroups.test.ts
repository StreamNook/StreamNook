// The desktop card groups chat-client badges by provider in a hand list
// (UserProfileCard.tsx THIRD_PARTY_PROVIDER_GROUPS). Rust names the order
// (badge_service.rs THIRD_PARTY_ORDER) and the groups' titles (profile_view.rs
// PROVIDER_GROUPS) and builds the phones' groups from them, so the lists must
// agree in members, order and labels.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = resolve(__dirname, '..', '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

function desktopGroups(): [string, string][] {
  const src = read('src/components/UserProfileCard.tsx');
  const start = src.indexOf('const THIRD_PARTY_PROVIDER_GROUPS');
  const end = src.indexOf('];', start);
  expect(start).toBeGreaterThan(-1);
  return [...src.slice(start, end).matchAll(/key:\s*'([a-z0-9-]+)',\s*label:\s*'([^']+)'/g)].map((m) => [m[1], m[2]]);
}

function rustOrder(): string[] {
  const src = read('src-tauri/src/services/badge_service.rs');
  const m = /pub const THIRD_PARTY_ORDER:\s*\[&str;\s*\d+\]\s*=\s*\[([^\]]*)\]/.exec(src);
  expect(m, 'THIRD_PARTY_ORDER not found').not.toBeNull();
  return [...m![1].matchAll(/"([a-z0-9-]+)"/g)].map((x) => x[1]);
}

function rustGroups(): [string, string][] {
  const src = read('src-tauri/src/services/profile_view.rs');
  const m = /pub const PROVIDER_GROUPS:\s*\[\(&str,\s*&str\);\s*\d+\]\s*=\s*\[([\s\S]*?)\];/.exec(src);
  expect(m, 'PROVIDER_GROUPS not found in profile_view.rs').not.toBeNull();
  return [...m![1].matchAll(/\(\s*"([a-z0-9-]+)",\s*"([^"]+)"\s*\)/g)].map((x) => [x[1], x[2]]);
}

describe('chat-client badge groups', () => {
  it('reads all three lists', () => {
    expect(desktopGroups().length).toBeGreaterThan(5);
    expect(rustOrder().length).toBeGreaterThan(5);
    expect(rustGroups().length).toBeGreaterThan(5);
  });

  it('desktop group keys equal Rust THIRD_PARTY_ORDER, in order', () => {
    expect(desktopGroups().map(([key]) => key)).toEqual(rustOrder());
  });

  it('Rust PROVIDER_GROUPS names the same providers in the same order', () => {
    expect(rustGroups().map(([key]) => key)).toEqual(rustOrder());
  });

  it('desktop labels equal Rust PROVIDER_GROUPS labels', () => {
    expect(desktopGroups()).toEqual(rustGroups());
  });
});
