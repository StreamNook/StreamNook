import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Events Rust sends to one window at a time with `emit_to`. The global `listen`
// and `once` from '@tauri-apps/api/event' (or the `event` namespace of
// '@tauri-apps/api') hear them in every window, so a file that names one must
// not import those at all: it hears them through
// `getCurrentWebviewWindow().listen` / `getCurrentWindow().listen`.
const LITERALS = [
  'chat-mention-ping',
  'mirrored-notification',
  'window-visibility',
  'emote-palette://target',
  'emote-palette://insert',
  'chat-overlay-click-through',
  // A window with no player asking the main window about the song (songListening.ts).
  'song-identify-request',
  'song-identify-state',
  'song-identify-level',
  'song-identify-result',
  'song-identify-cancel',
];

const GLOBAL_LISTEN = [
  /import\s*\{[^}]*\b(listen|once)\b[^}]*\}\s*from\s*['"]@tauri-apps\/api\/event['"]/,
  /import\s*\*\s*as\s+\w+\s+from\s*['"]@tauri-apps\/api\/event['"]/,
  /import\(\s*['"]@tauri-apps\/api\/event['"]\s*\)/,
  /import\s*\{[^}]*\bevent\b[^}]*\}\s*from\s*['"]@tauri-apps\/api['"]/,
  /import\s*\*\s*as\s+\w+\s+from\s*['"]@tauri-apps\/api['"]/,
  /import\(\s*['"]@tauri-apps\/api['"]\s*\)/,
];

const escape = (t: string) => t.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&');
const LITERAL = `['"\`](?:${LITERALS.map(escape).join('|')})['"\`]`;

/** Exported constants that hold one of the names, so importing one from
 *  another file still counts. */
function exportedConstants(sources: string[]): string[] {
  const re = new RegExp(`export\\s+const\\s+(\\w+)\\s*(?::\\s*\\w+\\s*)?=\\s*${LITERAL}`, 'g');
  return [...new Set(sources.flatMap((s) => [...s.matchAll(re)].map((x) => x[1])))];
}

/** A literal name (never a longer name it starts, like
 *  `mirrored-notification-activate`) or one of `constants`. */
function namesPattern(constants: string[]): RegExp {
  const literal = `(?:${LITERALS.map(escape).join('|')})(?![-\\w])`;
  return constants.length ? new RegExp(`${literal}|\\b(?:${constants.join('|')})\\b`) : new RegExp(literal);
}

function hearsTargetedGlobally(source: string, names: RegExp): boolean {
  return names.test(source) && GLOBAL_LISTEN.some((r) => r.test(source));
}

const SRC = fileURLToPath(new URL('..', import.meta.url));

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === 'node_modules' ? [] : sourceFiles(path);
    return /\.(ts|tsx)$/.test(name) && !/\.test\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}

describe('window-targeted events', () => {
  it('the guard catches every way of hearing them globally', () => {
    const names = namesPattern(exportedConstants(["export const VIS_EVENT = 'window-visibility';"]));
    const bad = [
      "import { listen } from '@tauri-apps/api/event';\nlisten<Record<string, boolean>>('window-visibility', cb);",
      "import { once } from '@tauri-apps/api/event';\nonce('chat-mention-ping', cb);",
      "import * as ev from '@tauri-apps/api/event';\nev.listen('mirrored-notification', cb);",
      "const { listen } = await import('@tauri-apps/api/event');\nconst E = 'emote-palette://target';\nlisten(E, cb);",
      "import { event } from '@tauri-apps/api';\nevent.listen('window-visibility', cb);",
      "import { listen } from '@tauri-apps/api/event';\nimport { VIS_EVENT } from './vis';\nlisten(VIS_EVENT, cb);",
    ];
    for (const s of bad) expect(hearsTargetedGlobally(s, names), s).toBe(true);
    const good = [
      "import { invoke } from '@tauri-apps/api/core';\ngetCurrentWebviewWindow().listen('chat-mention-ping', cb);",
      "import { listen } from '@tauri-apps/api/event';\nlisten('mirrored-notification-activate', cb);",
      "getCurrentWindow().listen('chat-overlay-click-through', cb);",
    ];
    for (const s of good) expect(hearsTargetedGlobally(s, names), s).toBe(false);
  });

  it('no source hears them globally', () => {
    const files = sourceFiles(SRC);
    const texts = files.map((path) => readFileSync(path, 'utf8'));
    const names = namesPattern(exportedConstants(texts));
    const offenders = files.filter((_, i) => hearsTargetedGlobally(texts[i], names)).map((path) => relative(SRC, path));
    expect(offenders).toEqual([]);
  });
});
