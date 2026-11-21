// A pinned mention's remote snapshot leaves out the tags in pin-dropped-tags.json
// (written by the Rust test `the_dropped_tag_list_is_the_fixture_every_renderer_test_reads`).
// No tag the row renderer reads may be one of them, in any access form, in the
// renderer or in any file it imports.
import { readFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { describe, expect, it } from 'vitest';

const ROOT = join(__dirname, '..', '..');
const dropped: string[] = JSON.parse(
  readFileSync(join(ROOT, 'src-tauri', 'tests', 'fixtures', 'pin-dropped-tags.json'), 'utf8'),
).dropped;

const resolveImport = (from: string, spec: string): string | null => {
  if (!spec.startsWith('.')) return null;
  const base = join(dirname(from), spec);
  for (const candidate of [`${base}.tsx`, `${base}.ts`, join(base, 'index.tsx'), join(base, 'index.ts')]) {
    if (existsSync(candidate)) return candidate;
  }
  return null;
};

/** The renderer and every relative file it imports (transitively). */
const rendererFiles = (entry: string): string[] => {
  const seen = new Set<string>();
  const queue = [entry];
  while (queue.length) {
    const file = queue.pop()!;
    if (seen.has(file)) continue;
    seen.add(file);
    const text = readFileSync(file, 'utf8');
    for (const m of text.matchAll(/(?:from|import)\s*\(?\s*['"]([^'"]+)['"]/g)) {
      const next = resolveImport(file, m[1]);
      if (next && !next.includes('.test.')) queue.push(next);
    }
  }
  return [...seen];
};

/** Every tag name read through `tags`: .get('x'), ['x'], ?.['x'], and tags.x. */
const tagReads = (text: string): string[] => {
  const names = new Set<string>();
  for (const m of text.matchAll(/tags\??\.get\(\s*['"`]([^'"`]+)['"`]\s*\)/g)) names.add(m[1]);
  for (const m of text.matchAll(/tags\??\.?\[\s*['"`]([^'"`]+)['"`]\s*\]/g)) names.add(m[1]);
  for (const m of text.matchAll(/\btags\??\.(?!get\b|has\b|entries\b|keys\b|values\b|size\b|forEach\b)([A-Za-z_]\w*)/g)) names.add(m[1]);
  return [...names];
};

describe('the tags a pinned mention never carries', () => {
  it('lists some, and the renderer reads none of them', () => {
    expect(dropped.length).toBeGreaterThan(0);
    const files = rendererFiles(join(ROOT, 'src', 'components', 'ChatMessage.tsx'));
    expect(files.length).toBeGreaterThan(5);
    const reads = new Map<string, string[]>();
    for (const file of files) for (const name of tagReads(readFileSync(file, 'utf8'))) reads.set(name, [...(reads.get(name) ?? []), file]);
    expect(reads.size).toBeGreaterThan(5);
    for (const tag of dropped) expect(reads.get(tag), `${tag} is read by ${(reads.get(tag) ?? []).join(', ')}`).toBeUndefined();
  });
});
