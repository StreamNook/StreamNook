// One-shot maintenance script: strip decorative emoji from logs, comments, and
// toast/notification text, while leaving feature content (emoji picker data,
// copypastas, hype phrases, joke arrays) and typographic arrows/(c)/(r)/tm intact.
//
//   node _emoji_strip.mjs          # dry run, prints per-file counts
//   node _emoji_strip.mjs --write  # apply changes
//
// Strategy: a small string/comment-aware tokenizer marks two kinds of regions as
// "strippable": (1) comment bodies, (2) string literals that are arguments to a
// known log/toast call. Emoji are removed ONLY inside those regions, so data
// literals (arrays/objects/match arms) are never touched.

import fs from 'node:fs';

const WRITE = process.argv.includes('--write');
const ROOT = 'C:/StreamNook';
const LIMIT = 800; // max chars a call span may cover before we stop treating it as active (bleed backstop)

// --- emoji matching (keep arrows, (c), (r), tm) ---
const EXCL = '\\u00A9\\u00AE\\u2122';
const BASE = `(?:(?![${EXCL}])(?:\\p{Extended_Pictographic}|\\p{Regional_Indicator}))`;
const MOD = `[\\u{1F3FB}-\\u{1F3FF}\\uFE0E\\uFE0F\\u20E3\\u200D]`;
const SEQ = `${BASE}${MOD}*`;
const RUN = `(?:${SEQ})+`;
const countRe = () => new RegExp(RUN, 'gu');
const stripRe = () => new RegExp(`( ?)(${RUN})( ?)`, 'gu');

function countEmoji(s) {
  const m = s.match(countRe());
  return m ? m.reduce((n, r) => n + [...r.matchAll(new RegExp(SEQ, 'gu'))].length, 0) : 0;
}
function stripEmojiText(s) {
  // remove emoji runs; collapse one surrounding space to a single space when emoji sat between two spaces
  return s.replace(stripRe(), (_m, b, _r, a) => (b && a) ? ' ' : '');
}

// --- call detection ---
const BARE_TS = new Set(['addToast', 'sendNativeNotification', 'showError', 'showToast', 'showSuccess', 'showJoke', 'showInfo', 'showWarning']);
const RE_CONSOLE = /(?:^|\.)console\.(log|info|warn|error|debug|trace|group|groupCollapsed|groupEnd|table|dir|assert)$/;
const RE_LOGGER = /(?:^|\.)Logger\.(debug|info|warn|error|trace|log)$/;
const RE_RUST = /(?:^|::)(println|eprintln|print|eprint|trace|debug|info|warn|error)!$/;

function calleeBefore(src, parenIdx) {
  let k = parenIdx - 1;
  while (k >= 0 && /\s/.test(src[k])) k--;
  let end = k + 1;
  while (k >= 0 && /[\w$.!:]/.test(src[k])) k--;
  return src.slice(k + 1, end);
}
function isCallTS(name) {
  if (!name) return false;
  const last = name.split('.').pop();
  return BARE_TS.has(last) || RE_CONSOLE.test(name) || RE_LOGGER.test(name);
}
function isCallRust(name) {
  return !!name && RE_RUST.test(name);
}

// `'` / `"` / backtick only start a string in value position (avoids JSX-text apostrophes)
function valueExpected(prev) {
  return prev === null || !/[\w$)\]}>]/.test(prev);
}

function computeMask(src, lang) {
  const n = src.length;
  const strip = new Uint8Array(n);
  const isTS = lang === 'ts';
  const openCalls = []; // start indices of currently-open target-call parens
  let i = 0;
  let prev = null; // last significant code char (for value/regex disambiguation)

  const active = () => openCalls.length > 0 && (i - openCalls[openCalls.length - 1] <= LIMIT);
  const markStr = () => { if (active()) strip[i] = 1; };

  function lineComment() { while (i < n && src[i] !== '\n') { strip[i] = 1; i++; } }
  function blockComment(nesting) {
    let depth = 1; strip[i] = 1; strip[i + 1] = 1; i += 2;
    while (i < n && depth > 0) {
      if (nesting && src[i] === '/' && src[i + 1] === '*') { strip[i] = strip[i + 1] = 1; depth++; i += 2; continue; }
      if (src[i] === '*' && src[i + 1] === '/') { strip[i] = strip[i + 1] = 1; depth--; i += 2; continue; }
      strip[i] = 1; i++;
    }
  }
  function dq() { i++; while (i < n) { if (src[i] === '\\') { markStr(); i++; markStr(); i++; continue; } if (src[i] === '"') { i++; break; } markStr(); i++; } prev = '"'; }
  function sq() { i++; while (i < n) { if (src[i] === '\\') { markStr(); i++; markStr(); i++; continue; } if (src[i] === "'") { i++; break; } markStr(); i++; } prev = "'"; }
  function tpl() {
    i++;
    while (i < n) {
      if (src[i] === '\\') { markStr(); i++; markStr(); i++; continue; }
      if (src[i] === '`') { i++; break; }
      if (src[i] === '$' && src[i + 1] === '{') { i += 2; code(true); continue; }
      markStr(); i++;
    }
    prev = '`';
  }
  function regex() { i++; let cls = false; while (i < n) { if (src[i] === '\\') { i += 2; continue; } if (src[i] === '[') { cls = true; i++; continue; } if (src[i] === ']') { cls = false; i++; continue; } if (src[i] === '\n') break; if (src[i] === '/' && !cls) { i++; break; } i++; } while (i < n && /[a-z]/i.test(src[i])) i++; prev = ')'; }
  function rustChar() {
    // consume a complete char literal '\x' or 'x'; otherwise it's a lifetime/label -> skip one char
    const m = /^'(\\.|[^'\\])'/.exec(src.slice(i, i + 5));
    if (m) { i += m[0].length; prev = "'"; } else { i++; }
  }
  function rustRaw() {
    // r"...", r#"..."#, br#"..."#  (i is at 'r' or 'b')
    let j = i; if (src[j] === 'b') j++; if (src[j] !== 'r') return false; j++;
    let hashes = 0; while (src[j] === '#') { hashes++; j++; }
    if (src[j] !== '"') return false;
    j++; const term = '"' + '#'.repeat(hashes);
    i = j;
    while (i < n) { if (src.startsWith(term, i)) { i += term.length; break; } markStr(); i++; }
    prev = '"'; return true;
  }

  function code(stopAtBrace) {
    const parens = []; // booleans: was this paren a target call?
    while (i < n) {
      const c = src[i];
      if (c === '/' && src[i + 1] === '/') { lineComment(); continue; }
      if (c === '/' && src[i + 1] === '*') { blockComment(!isTS); continue; }
      if (isTS) {
        if (c === '/' && valueExpected(prev)) { regex(); continue; }
        if (c === '`') { tpl(); continue; }
        if ((c === '"' || c === "'") && valueExpected(prev)) { c === '"' ? dq() : sq(); continue; }
        if (c === '"' || c === "'") { i++; if (!/\s/.test(c)) prev = c; continue; } // apostrophe/quote in JSX text
      } else {
        if (c === '"') { dq(); continue; }
        if ((c === 'r' || c === 'b') && (src[i + 1] === '"' || src[i + 1] === '#')) { if (rustRaw()) continue; }
        if (c === "'") { rustChar(); continue; }
      }
      if (c === '(') {
        const isCall = isTS ? isCallTS(calleeBefore(src, i)) : isCallRust(calleeBefore(src, i));
        parens.push(isCall); if (isCall) openCalls.push(i);
        i++; prev = '('; continue;
      }
      if (c === ')') { const was = parens.pop(); if (was) openCalls.pop(); i++; prev = ')'; continue; }
      if (c === '}') { if (stopAtBrace && parens.length === 0) { i++; return; } i++; prev = '}'; continue; }
      if (!/\s/.test(c)) prev = c;
      i++;
    }
  }

  code(false);
  return strip;
}

function processText(src, lang) {
  const strip = computeMask(src, lang);
  const n = src.length;
  let out = '';
  let j = 0;
  while (j < n) {
    if (strip[j]) { let k = j; while (k < n && strip[k]) k++; out += stripEmojiText(src.slice(j, k)); j = k; }
    else { out += src[j]; j++; }
  }
  return out;
}

// --- walk target dirs ---
function walk(dir, exts, acc) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = dir + '/' + e.name;
    if (e.isDirectory()) { if (e.name === 'node_modules' || e.name === 'target' || e.name === 'dist') continue; walk(p, exts, acc); }
    else if (exts.some((x) => e.name.endsWith(x))) acc.push(p);
  }
  return acc;
}

const tsFiles = walk(ROOT + '/src', ['.ts', '.tsx'], []);
const rsFiles = walk(ROOT + '/src-tauri/src', ['.rs'], []);
const all = [...tsFiles.map((f) => [f, 'ts']), ...rsFiles.map((f) => [f, 'rust'])];

const report = [];
let totalRemoved = 0;
for (const [file, lang] of all) {
  const src = fs.readFileSync(file, 'utf8');
  const before = countEmoji(src);
  if (before === 0) continue;
  const out = processText(src, lang);
  const after = countEmoji(out);
  const removed = before - after;
  if (removed > 0) {
    totalRemoved += removed;
    report.push({ file: file.replace(ROOT + '/', ''), before, after, removed });
    if (WRITE) fs.writeFileSync(file, out, 'utf8');
  }
}

report.sort((a, b) => b.removed - a.removed);
console.log(`${WRITE ? 'WROTE' : 'DRY RUN'} — files changed: ${report.length}, emoji removed: ${totalRemoved}\n`);
for (const r of report) console.log(`${String(r.removed).padStart(4)}  (${r.before}->${r.after})  ${r.file}`);
