import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Events Rust sends to ONE window with `emit_to`. A plain `listen` from
// '@tauri-apps/api/event' hears them in every window, so each must be heard
// through `getCurrentWebviewWindow().listen`.
const BARE_LISTEN =
  /(?<![.\w])listen\s*(?:<[^>]*>)?\(\s*(?:['"](?:chat-mention-ping|mirrored-notification|window-visibility)['"]|MIRRORED_NOTIFICATION_EVENT\b|PING_EVENT\b)/;

const SRC = fileURLToPath(new URL('..', import.meta.url));

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === 'node_modules' ? [] : sources(path);
    return /\.(ts|tsx)$/.test(name) && !/\.test\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}

describe('window-targeted events', () => {
  it('the guard catches a bare listen and passes a window-scoped one', () => {
    expect(BARE_LISTEN.test("listen<{ hidden: boolean }>('window-visibility', cb)")).toBe(true);
    expect(BARE_LISTEN.test('listen<MirroredNotice>(MIRRORED_NOTIFICATION_EVENT, cb)')).toBe(true);
    expect(BARE_LISTEN.test("getCurrentWebviewWindow().listen('window-visibility', cb)")).toBe(false);
    expect(BARE_LISTEN.test('win.listen<MentionPing>(PING_EVENT, cb)')).toBe(false);
  });

  it('no source listens to them through the global listen', () => {
    const offenders = sources(SRC)
      .filter((path) => BARE_LISTEN.test(readFileSync(path, 'utf8')))
      .map((path) => relative(SRC, path));
    expect(offenders).toEqual([]);
  });
});
