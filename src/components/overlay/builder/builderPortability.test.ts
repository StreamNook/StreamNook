// Run with: npm test
//
// The overlay builder runs in the app AND on streamnook.app, synced one way from
// this repo. It only works there if nothing it reaches imports Tauri or an
// app-only store. This walks every runtime import reachable from the builder and
// fails on the first one that does, naming the chain that got there.

import { test } from 'vitest';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

const ROOT = resolve(__dirname, '../../..');
const ENTRY = join(__dirname, 'OverlayBuilder.tsx');

// Allowed app-side shared modules. Anything else under stores/ or services/ is
// app-only state or I/O that the site does not have.
const FORBIDDEN = [/^@tauri-apps\//, /[\\/]stores[\\/](?!TooltipStore)/, /[\\/]services[\\/](?!paintStyle)/];

function runtimeImports(file: string): string[] {
  const src = readFileSync(file, 'utf8');
  const out: string[] = [];
  const re = /^\s*import\s+(type\s+)?(?:[^'"]*?\sfrom\s+)?['"]([^'"]+)['"]/gm;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src))) {
    if (m[1]) continue; // `import type` is erased at build time
    out.push(m[2]);
  }
  return out;
}

function resolveLocal(from: string, spec: string): string | null {
  if (!spec.startsWith('.')) return null;
  const base = resolve(dirname(from), spec);
  for (const ext of ['', '.ts', '.tsx', '/index.ts', '/index.tsx']) {
    const p = base + ext;
    if (existsSync(p) && !p.endsWith('/') && /\.(ts|tsx)$/.test(p)) return p;
  }
  return null; // assets (png, css) and the like
}

test('nothing the overlay builder reaches imports Tauri or app-only state', () => {
  const seen = new Set<string>();
  const stack: { file: string; chain: string[] }[] = [{ file: ENTRY, chain: ['OverlayBuilder.tsx'] }];
  while (stack.length) {
    const { file, chain } = stack.pop()!;
    if (seen.has(file)) continue;
    seen.add(file);
    for (const spec of runtimeImports(file)) {
      const local = resolveLocal(file, spec);
      const label = local ? local.slice(ROOT.length + 1) : spec;
      const bad = FORBIDDEN.some((re) => re.test(local ?? spec));
      assert.ok(!bad, `${[...chain, label].join(' -> ')} is app-only; the site cannot run it`);
      if (local) stack.push({ file: local, chain: [...chain, label] });
    }
  }
  assert.ok(seen.size > 5, 'walked the builder graph');
});
