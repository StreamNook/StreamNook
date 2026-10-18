// Twin of src-tauri/tests/gql_manifest_parity.rs, for hosts where cargo test
// cannot run (the WSL Android build host has no GTK for a host-target cargo).
//
// Every Twitch GQL operation the app sends must be listed in the watchdog
// manifest, and every entry there must still exist in source. StreamNook's
// Discord bot checks each listed operation anonymously once a day, so an
// operation missing from the manifest is one nobody watches. The manifest is
// private (the bot's repo, read from STREAMNOOK_GQL_MANIFEST or
// ../StreamNookBot/data/); a checkout without it skips the coverage checks.
// The Rust file explains what counts as an operation; keep the two in step.
import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const root = resolve(__dirname, '..');
const manifestPath = process.env.STREAMNOOK_GQL_MANIFEST ?? resolve(root, '..', 'StreamNookBot', 'data', 'twitch-gql-manifest.json');
const hasManifest = existsSync(manifestPath);

interface Check {
  expect?: string;
  reason?: string;
  query?: string;
}
interface Entry {
  id?: string;
  type?: string;
  kind?: string;
  hash?: string;
  match?: string;
  file?: string;
  client?: string;
  auth?: string;
  feature?: string;
  tree?: string;
  sentFrom?: string;
  checks?: Check[];
}
interface Manifest {
  clientIds: Record<string, string>;
  operations: Entry[];
  ignore?: Entry[];
  transport?: Entry[];
}

const manifest: Manifest = hasManifest
  ? (JSON.parse(readFileSync(manifestPath, 'utf8')) as Manifest)
  : { clientIds: {}, operations: [] };
const onDesktop = (e: Entry) => !e.tree || e.tree === 'desktop';

interface Lit {
  line: number;
  start: number;
  raw: string;
}

const isIdent = (c: string | undefined) => !!c && /[A-Za-z0-9_]/.test(c);

/** String literals (raw text, escapes as written) plus the file with literals
 * and comments blanked, so braces can be matched on code alone. */
function lex(src: string): { lits: Lit[]; code: string } {
  const lits: Lit[] = [];
  const code = src.split('');
  const blank = (from: number, to: number) => {
    for (let k = from; k < Math.min(to, src.length); k++) if (code[k] !== '\n') code[k] = ' ';
  };
  let i = 0;
  let line = 1;
  while (i < src.length) {
    const c = src[i];
    if (c === '\n') {
      line++;
      i++;
    } else if (c === '/' && src[i + 1] === '/') {
      const s = i;
      while (i < src.length && src[i] !== '\n') i++;
      blank(s, i);
    } else if (c === '/' && src[i + 1] === '*') {
      const s = i;
      i += 2;
      while (i + 1 < src.length && !(src[i] === '*' && src[i + 1] === '/')) {
        if (src[i] === '\n') line++;
        i++;
      }
      i += 2;
      blank(s, i);
    } else if (c === 'r' && (src[i + 1] === '"' || src[i + 1] === '#') && !isIdent(src[i - 1])) {
      let j = i + 1;
      let hashes = 0;
      while (src[j] === '#') {
        hashes++;
        j++;
      }
      if (src[j] !== '"') {
        i++;
        continue;
      }
      const close = '"' + '#'.repeat(hashes);
      const start = j + 1;
      let end = src.indexOf(close, start);
      if (end < 0) end = src.length;
      lits.push({ line, start, raw: src.slice(start, end) });
      line += (src.slice(i, end).match(/\n/g) || []).length;
      blank(i, end + close.length);
      i = end + close.length;
    } else if (c === "'") {
      if (src[i + 1] === '\\') {
        const k = src.indexOf("'", i + 2);
        i = k < 0 ? src.length : k + 1;
      } else {
        const ch = String.fromCodePoint(src.codePointAt(i + 1) ?? 32);
        i += src[i + 1 + ch.length] === "'" ? 2 + ch.length : 1;
      }
    } else if (c === '"') {
      const start = i + 1;
      const first = line;
      let j = start;
      while (j < src.length && src[j] !== '"') {
        if (src[j] === '\\') j++;
        if (src[j] === '\n') line++;
        j++;
      }
      const end = Math.min(j, src.length);
      lits.push({ line: first, start, raw: src.slice(start, end) });
      blank(i, end + 1);
      i = end + 1;
    } else {
      i++;
    }
  }
  return { lits, code: code.join('') };
}

function testModules(code: string): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  const re = /#\[cfg\(test\)\]\s*(?:pub\s+)?mod\s+\w+\s*\{/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(code))) {
    let k = m.index + m[0].length - 1;
    let depth = 0;
    for (; k < code.length; k++) {
      if (code[k] === '{') depth++;
      else if (code[k] === '}' && --depth === 0) break;
    }
    out.push([m.index, k + 1]);
    re.lastIndex = k + 1;
  }
  return out;
}

const isHash = (raw: string) => /^[0-9a-f]{64}$/.test(raw);

function leadTrimmed(raw: string): string {
  let t = raw;
  for (;;) {
    const before = t.length;
    t = t.trimStart();
    for (const esc of ['\\n', '\\t', '\\r']) if (t.startsWith(esc)) t = t.slice(2);
    if (t.startsWith('\\') && /\s/.test(t[1] ?? '')) t = t.slice(1);
    if (t.length === before) return t;
  }
}

function isDocument(raw: string): boolean {
  if (!raw.includes('{')) return false;
  const t = leadTrimmed(raw);
  const kw = t.match(/^(query|mutation)([\s\S]?)/);
  if (kw) return /[\s({]/.test(kw[2]);
  return /^\{+\s*[A-Za-z][A-Za-z0-9_]*\s*[({]/.test(t);
}

function walk(dir: string, keep: (name: string) => boolean, out: string[] = []): string[] {
  let names: string[];
  try {
    names = readdirSync(dir);
  } catch {
    return out;
  }
  for (const name of names) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      if (!['node_modules', 'build', 'generated', '.gradle'].includes(name)) walk(p, keep, out);
    } else if (keep(name)) out.push(p);
  }
  return out;
}

function scan() {
  const text = new Map<string, string>();
  const hashes: Array<{ file: string; hash: string; line: number }> = [];
  const docs: Array<{ file: string; doc: string; line: number }> = [];
  for (const path of walk(resolve(root, 'src-tauri/src'), (n) => n.endsWith('.rs'))) {
    const file = relative(root, path).replace(/\\/g, '/');
    const src = readFileSync(path, 'utf8').replace(/\r\n/g, '\n');
    const { lits, code } = lex(src);
    const tests = testModules(code);
    for (const lit of lits) {
      if (tests.some(([a, z]) => lit.start >= a && lit.start < z)) continue;
      if (isHash(lit.raw)) hashes.push({ file, hash: lit.raw, line: lit.line });
      else if (isDocument(lit.raw)) docs.push({ file, doc: lit.raw, line: lit.line });
    }
    text.set(file, src);
  }
  return { text, hashes, docs };
}

describe.skipIf(!hasManifest)('Twitch GQL watchdog manifest', () => {
  const source = scan();
  const ops = manifest.operations.filter(onDesktop);
  const ignore = manifest.ignore ?? [];

  it('has well-formed entries', () => {
    const problems: string[] = [];
    const ids = new Set<string>();
    for (const op of manifest.operations) {
      const id = op.id ?? '';
      if (!id || ids.has(id)) problems.push(`missing or duplicate id: ${id}`);
      ids.add(id);
      if (op.type !== 'query' && op.type !== 'mutation') problems.push(`${id}: type must be query or mutation`);
      if (op.kind === 'persisted' ? !isHash(op.hash ?? '') : op.kind === 'inline' ? (op.match ?? '').length < 8 : true)
        problems.push(`${id}: persisted needs a hash, inline needs a match of 8+ chars`);
      if (!manifest.clientIds[op.client ?? '']) problems.push(`${id}: client ${op.client} is not in clientIds`);
      for (const key of ['file', 'feature', 'auth'] as const) if (!op[key]) problems.push(`${id}: missing ${key}`);
      const checks = op.checks ?? [];
      if (!checks.length) problems.push(`${id}: needs at least one check`);
      for (const c of checks) {
        if (!['public', 'signin', 'mutation', 'skip'].includes(c.expect ?? '')) problems.push(`${id}: bad expect ${c.expect}`);
        if (c.expect === 'skip' && !c.reason) problems.push(`${id}: a skipped check needs a reason`);
        if (c.expect !== 'skip' && (c.expect === 'mutation') !== (op.type === 'mutation'))
          problems.push(`${id}: a mutation's checks expect "mutation", and only a mutation's do`);
        if (c.expect !== 'skip' && op.kind === 'inline' && !c.query) problems.push(`${id}: an inline check needs a query`);
      }
    }
    expect(problems).toEqual([]);
  });

  it('lists every Twitch GQL operation in source', () => {
    const missing: string[] = [];
    for (const h of source.hashes)
      if (!ops.some((o) => o.hash === h.hash && o.file === h.file)) missing.push(`${h.file}:${h.line} persisted hash ${h.hash}`);
    for (const d of source.docs)
      if (!ops.concat(ignore).some((o) => o.file === d.file && (o.match ? d.doc.includes(o.match) : ignore.includes(o))))
        missing.push(`${d.file}:${d.line} inline document \`${leadTrimmed(d.doc).slice(0, 90)}\``);
    expect(missing).toEqual([]);
  });

  it('has no stale entries', () => {
    const stale: string[] = [];
    for (const op of ops.concat(ignore)) {
      const text = source.text.get(op.file ?? '');
      if (text === undefined) stale.push(`${op.id}: file ${op.file} does not exist`);
      else if (op.hash) {
        if (!source.hashes.some((h) => h.file === op.file && h.hash === op.hash)) stale.push(`${op.id}: hash gone from ${op.file}`);
      } else if (op.match && !text.includes(op.match)) stale.push(`${op.id ?? 'ignore'}: match gone from ${op.file}`);
    }
    expect(stale).toEqual([]);
  });

  it('has an entry for every file that sends Twitch GQL', () => {
    // A document assembled from fragments shows in no single literal, so any
    // file that calls a sender must have an entry (or be a pure transport file).
    const senders = ['gql.twitch.tv', 'twitch_web_gql::post', 'gql_query(', 'gql_public_read(', 'gql_web_query('];
    const known = new Set<string>();
    for (const op of ops) {
      known.add(op.file ?? '');
      if (op.sentFrom) known.add(op.sentFrom.split(':')[0]);
    }
    for (const e of [...ignore, ...(manifest.transport ?? [])]) known.add(e.file ?? '');
    const unknown: string[] = [];
    for (const [file, text] of source.text) {
      const cut = testModules(lex(text).code)[0]?.[0] ?? text.length;
      const live = text.slice(0, cut);
      if (!known.has(file) && senders.some((s) => live.includes(s))) unknown.push(file);
    }
    expect(unknown).toEqual([]);
  });
});

describe('Twitch GQL', () => {
  it('is the only place Twitch GQL is called: never the frontend or the Kotlin shell', () => {
    const files = [
      ...walk(resolve(root, 'src'), (n) => /\.tsx?$/.test(n) && !n.includes('.test.')),
      ...walk(resolve(root, 'src-tauri/gen/android/app/src'), (n) => /\.(kt|java)$/.test(n)),
    ];
    const offenders = files.filter((p) => readFileSync(p, 'utf8').includes('gql.twitch.tv')).map((p) => relative(root, p));
    expect(offenders).toEqual([]);
  });
});
