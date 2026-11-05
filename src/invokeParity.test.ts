// Every command the webview invokes by a literal name must be a command the
// Rust side registers. The ACL tests (src-tauri/tests/acl_parity.rs and
// src/mobile/acl_parity.test.ts) start from the registry, so they cannot see a
// name the TypeScript calls that the registry lacks: Tauri rejects it at run
// time with "command not found" and the feature looks silently broken.
//
// Covers `invoke('name')` and `invoke<Type>('name')` with a quoted literal;
// `plugin:` names belong to Tauri plugins and are skipped. A command named by
// a variable (`invoke(command, ...)`) is not seen.
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = resolve(__dirname, '..');
const toPosix = (p: string) => p.split('\\').join('/');

/** Command names in lib.rs's generate_handler! block (the one registry desktop and Android share). */
function registered(source: string): Set<string> {
  const start = source.indexOf('generate_handler![');
  if (start < 0) throw new Error('generate_handler! block not found in lib.rs');
  const end = source.indexOf('])', start);
  const names = new Set<string>();
  for (const raw of source.slice(start + 'generate_handler!['.length, end).split('\n')) {
    const line = raw.replace(/\/\/.*$/, '').trim();
    if (!line || line.startsWith('#[')) continue;
    const m = line.match(/^([A-Za-z_0-9:]+),?$/);
    if (m) names.add(m[1].split('::').pop() as string);
  }
  return names;
}

/** Every `invoke('literal')` / `invoke<T>('literal')` name in a source text. */
export function invokedNames(source: string): string[] {
  return [...source.matchAll(/(?<![.\w])invoke(?:<[^()]*>)?\(\s*(['"`])([^'"`$]+)\1/g)].map((m) => m[2]);
}

function sources(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...sources(path));
    // This file's own samples of the call forms are not calls.
    else if (/\.tsx?$/.test(entry.name) && entry.name !== 'invokeParity.test.ts') out.push(path);
  }
  return out;
}

describe('invokeParity', () => {
  const handlers = registered(readFileSync(join(root, 'src-tauri/src/lib.rs'), 'utf8'));
  const calls = new Map<string, string[]>();
  for (const file of sources(join(root, 'src'))) {
    for (const name of invokedNames(readFileSync(file, 'utf8'))) {
      if (name.startsWith('plugin:')) continue;
      calls.set(name, [...(calls.get(name) ?? []), toPosix(relative(root, file))]);
    }
  }

  it('reads a plausible registry and a plausible set of calls', () => {
    expect(handlers.size).toBeGreaterThan(300);
    expect(calls.size).toBeGreaterThan(300);
  });

  it('registers every command the TypeScript invokes by name', () => {
    const missing = [...calls].filter(([name]) => !handlers.has(name)).map(([name, files]) => `${name} (${files[0]})`);
    expect(missing, 'invoked but not in generate_handler!').toEqual([]);
  });

  it('finds the call forms it claims to', () => {
    const found = invokedNames(
      [
        "await invoke('plain_name');",
        "invoke<Array<{ a: string }>>('generic_name', { x: 1 });",
        'invoke<Foo>(\n  "multi_line",\n);',
        "invoke('plugin:opener|open_url');",
        "tauri.invoke('member_call');",
        'invoke(command, args);',
        'invoke(`templated_${kind}`);',
      ].join('\n'),
    );
    expect(found).toEqual(['plain_name', 'generic_name', 'multi_line', 'plugin:opener|open_url']);
  });
});
