import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { makeCloseAppLayers } from './closeAppLayers';

const source = readFileSync(resolve(__dirname, 'closeAppLayers.ts'), 'utf8');

function setup(seen: boolean) {
  const ref = { current: seen };
  const setShowChangelog = vi.fn();
  const setSelectedBadge = vi.fn();
  const close = makeCloseAppLayers({ changelogRecordsSeenRef: ref, setShowChangelog, setSelectedBadge });
  return { ref, setShowChangelog, setSelectedBadge, close };
}

describe('makeCloseAppLayers', () => {
  // Catches a close that keeps the changelog a person asked for open, or one
  // that forgets the badge detail.
  it('closes a changelog opened on request and clears the badge detail', () => {
    const { close, setShowChangelog, setSelectedBadge } = setup(false);
    close();
    expect(setShowChangelog).toHaveBeenCalledOnce();
    expect(setShowChangelog).toHaveBeenCalledWith(false);
    expect(setSelectedBadge).toHaveBeenCalledOnce();
    expect(setSelectedBadge).toHaveBeenCalledWith(null);
  });

  // Catches a close that shuts the changelog an update opened, which would skip
  // the step that records the version as seen.
  it('keeps a changelog an update opened, and still clears the badge detail', () => {
    const { close, setShowChangelog, setSelectedBadge } = setup(true);
    close();
    expect(setShowChangelog).not.toHaveBeenCalled();
    expect(setSelectedBadge).toHaveBeenCalledWith(null);
  });

  // Catches a snapshot of the ref taken when the closer was built: the drain
  // builds it once and calls it for the rest of the session.
  it('reads the ref at call time, not at build time', () => {
    const { ref, close, setShowChangelog } = setup(true);
    close();
    expect(setShowChangelog).not.toHaveBeenCalled();
    ref.current = false;
    close();
    expect(setShowChangelog).toHaveBeenCalledOnce();
  });
});

describe('closeAppLayers source', () => {
  // Catches a rewrite that goes through the changelog's own close handler,
  // which saves settings, or one that drops a closer.
  it('reads the ref, calls both setters and saves nothing', () => {
    expect(source).toContain('changelogRecordsSeenRef.current');
    expect(source).toContain('setShowChangelog(false)');
    expect(source).toContain('setSelectedBadge(null)');
    expect(source).not.toContain('updateSettings');
  });

  // Catches the per-render handler ref coming back: the closer is built once
  // from stable parts, so no ref to a changing close handler may exist. The
  // positive control keeps a wrong path or an empty read from passing.
  it('leaves no handler ref in App', () => {
    const app = readFileSync(resolve(__dirname, '..', 'App.tsx'), 'utf8');
    expect(app).toContain('changelogRecordsSeenRef');
    expect(app).not.toContain('handleChangelog' + 'CloseRef');
  });
});
