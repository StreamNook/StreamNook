// Mobile twin of src-tauri/tests/acl_parity.rs.
//
// The desktop test parses main.rs against permissions/app-commands.toml, but
// the phone registers its commands in src-tauri/src/lib.rs, and cargo test
// cannot run on the WSL build host (host-target cargo wants GTK). So the same
// invariant is checked here, in TypeScript, where `npm test` does run.
//
// Why it matters: once an app-level ACL manifest exists, Tauri denies every
// app command a capability does not allow, silently, for every webview. A
// command registered in lib.rs but listed in neither toml file is DENIED at
// runtime on the phone with no error anywhere a person would look.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(__dirname, '..', '..');
const read = (p: string) => readFileSync(resolve(root, p), 'utf8');

/** Command names registered in the mobile entry's generate_handler! block. */
function registered(): Set<string> {
  const src = read('src-tauri/src/lib.rs');
  const start = src.indexOf('generate_handler![');
  if (start < 0) throw new Error('generate_handler! block not found in lib.rs');
  const end = src.indexOf('])', start);
  const block = src.slice(start + 'generate_handler!['.length, end);
  const names = new Set<string>();
  for (const raw of block.split('\n')) {
    const line = raw.replace(/\/\/.*$/, '').trim();
    if (!line || line.startsWith('#[')) continue;
    // `commands::foo::bar,` or `bar,` or a last entry without the comma.
    const m = line.match(/^([A-Za-z_0-9:]+),?$/);
    if (!m) continue;
    names.add(m[1].split('::').pop()!);
  }
  return names;
}

/** Command names in a permission file's `allow = [...]` array. */
function allowed(file: string): Set<string> {
  const toml = read(file);
  const start = toml.indexOf('allow = [');
  if (start < 0) throw new Error(`allow array not found in ${file}`);
  const end = toml.indexOf(']', start);
  const body = toml.slice(start, end);
  return new Set([...body.matchAll(/"([a-z_0-9]+)"/g)].map((m) => m[1]));
}

describe('mobile ACL parity', () => {
  const reg = registered();
  const app = allowed('src-tauri/permissions/app-commands.toml');
  const mobile = allowed('src-tauri/permissions/mobile-commands.toml');

  it('parses a plausible registry', () => {
    expect(reg.size).toBeGreaterThan(300);
    expect(app.size).toBeGreaterThan(300);
  });

  it('lists no command in both permission files', () => {
    const both = [...mobile].filter((n) => app.has(n)).sort();
    expect(both, 'names in both app-commands.toml and mobile-commands.toml').toEqual([]);
  });

  it('allows every command lib.rs registers', () => {
    const missing = [...reg].filter((n) => !app.has(n) && !mobile.has(n)).sort();
    expect(
      missing,
      'registered in lib.rs but allowed by neither toml (DENIED at runtime on the phone)',
    ).toEqual([]);
  });

  it('keeps mobile-commands.toml free of names lib.rs no longer registers', () => {
    const stale = [...mobile].filter((n) => !reg.has(n)).sort();
    expect(stale, 'allowed in mobile-commands.toml but not registered').toEqual([]);
  });

  // Fails if a link door command is registered but left out of app-commands.toml
  // (silently DENIED at invoke time), or put in the phone-only file by mistake.
  it('registers the link door commands and allows each in app-commands.toml only', () => {
    const names = [
      'take_pending_link',
      'peek_pending_link_stamp',
      'link_superseded',
      'resolve_category_link',
      'category_share_link',
      'inbound_link',
      'notice_opened',
    ];
    for (const name of names) {
      expect(reg.has(name), `${name} is registered`).toBe(true);
      expect(app.has(name), `${name} is allowed in app-commands.toml`).toBe(true);
      expect(mobile.has(name), `${name} is not in mobile-commands.toml`).toBe(false);
    }
  });

  // Fails if one of the two retired take commands comes back in the registry or a
  // permission file. The names are built from halves so source scans for them do
  // not find this test.
  it('keeps the two old watch and badge commands out of the registry and both files', () => {
    for (const name of ['take_pending_' + 'watch_link', 'take_pending_' + 'badge_link']) {
      expect(reg.has(name), `${name} is registered`).toBe(false);
      expect(app.has(name), `${name} is in app-commands.toml`).toBe(false);
      expect(mobile.has(name), `${name} is in mobile-commands.toml`).toBe(false);
    }
  });
});
