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
});
