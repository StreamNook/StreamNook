import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Events Rust sends to ONE window with `emit_to`. The global `listen` and
// `once` from '@tauri-apps/api/event' hear them in every window, so a file that
// names one of them must not import those at all: it hears them through
// `getCurrentWebviewWindow().listen` / `getCurrentWindow().listen`.
const TARGETED = [
  'chat-mention-ping',
  'mirrored-notification',
  'MIRRORED_NOTIFICATION_EVENT',
  'window-visibility',
  'emote-palette://target',
  'emote-palette://insert',
  'chat-overlay-click-through',
];

// `mirrored-notification-activate` is main-only and fine to hear globally.
const NAMES = new RegExp(`(${TARGETED.map((t) => t.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&')).join('|')})(?!-)`);

const GLOBAL_LISTEN = [
  /import\s*\{[^}]*\b(listen|once)\b[^}]*\}\s*from\s*['"]@tauri-apps\/api\/event['"]/,
  /import\s*\*\s*as\s+\w+\s+from\s*['"]@tauri-apps\/api\/event['"]/,
  /import\(\s*['"]@tauri-apps\/api\/event['"]\s*\)/,
];

export function hearsTargetedGlobally(source: string): boolean {
  return NAMES.test(source) && GLOBAL_LISTEN.some((r) => r.test(source));
}

const SRC = fileURLToPath(new URL('..', import.meta.url));

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === 'node_modules' ? [] : sources(path);
    return /\.(ts|tsx)$/.test(name) && !/\.test\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}

describe('window-targeted events', () => {
  it('the guard catches every way of hearing them globally', () => {
    const bad = [
      "import { listen } from '@tauri-apps/api/event';\nlisten<Record<string, boolean>>('window-visibility', cb);",
      "import { once } from '@tauri-apps/api/event';\nonce('chat-mention-ping', cb);",
      "import * as ev from '@tauri-apps/api/event';\nev.listen('mirrored-notification', cb);",
      "const { listen } = await import('@tauri-apps/api/event');\nconst E = 'emote-palette://target';\nlisten(E, cb);",
      "import { emit, listen } from '@tauri-apps/api/event';\nlisten(MIRRORED_NOTIFICATION_EVENT, cb);",
    ];
    for (const s of bad) expect(hearsTargetedGlobally(s), s).toBe(true);
    const good = [
      "import { emit } from '@tauri-apps/api/event';\ngetCurrentWebviewWindow().listen('chat-mention-ping', cb);",
      "import { listen } from '@tauri-apps/api/event';\nlisten('mirrored-notification-activate', cb);",
      "getCurrentWindow().listen('chat-overlay-click-through', cb);",
    ];
    for (const s of good) expect(hearsTargetedGlobally(s), s).toBe(false);
  });

  it('no source hears them globally', () => {
    const offenders = sources(SRC)
      .filter((path) => hearsTargetedGlobally(readFileSync(path, 'utf8')))
      .map((path) => relative(SRC, path));
    expect(offenders).toEqual([]);
  });
});
