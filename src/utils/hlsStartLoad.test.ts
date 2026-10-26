import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type Hls from 'hls.js';
import { HLS_START_CONFIG, resumeLoadAfterRecover, startLoadOnManifest } from './hlsStartLoad';

function fakeHls(opts: { startPosition?: number; loadingEnabled?: boolean } = {}) {
  const raw = {
    config: { startPosition: opts.startPosition ?? -1 },
    loadingEnabled: opts.loadingEnabled ?? false,
    startLoad: vi.fn(),
  };
  return { hls: raw as unknown as Pick<Hls, 'startLoad' | 'config' | 'loadingEnabled'>, raw };
}

describe('startLoadOnManifest', () => {
  it('starts loading once, at the configured start position', () => {
    const { hls, raw } = fakeHls({ startPosition: 42 });
    expect(startLoadOnManifest(hls, { loadStopped: false })).toBe(true);
    expect(raw.startLoad).toHaveBeenCalledTimes(1);
    expect(raw.startLoad).toHaveBeenCalledWith(42);
  });

  it('starts a live player at the edge, with no park or a park that is not holding', () => {
    const a = fakeHls();
    expect(startLoadOnManifest(a.hls, null)).toBe(true);
    expect(a.raw.startLoad).toHaveBeenCalledWith(-1);
    const b = fakeHls();
    expect(startLoadOnManifest(b.hls, undefined)).toBe(true);
    expect(b.raw.startLoad).toHaveBeenCalledTimes(1);
  });

  it('leaves loading stopped when the park already stopped it', () => {
    const { hls, raw } = fakeHls();
    expect(startLoadOnManifest(hls, { loadStopped: true })).toBe(false);
    expect(raw.startLoad).not.toHaveBeenCalled();
  });
});

describe('resumeLoadAfterRecover', () => {
  it('starts loading again when the recovery left it stopped', () => {
    const { hls, raw } = fakeHls({ startPosition: -1, loadingEnabled: false });
    expect(resumeLoadAfterRecover(hls, { loadStopped: false })).toBe(true);
    expect(raw.startLoad).toHaveBeenCalledTimes(1);
    expect(raw.startLoad).toHaveBeenCalledWith(-1);
  });

  it('does not start a second load when the recovery already restarted it', () => {
    const { hls, raw } = fakeHls({ loadingEnabled: true });
    expect(resumeLoadAfterRecover(hls, null)).toBe(false);
    expect(raw.startLoad).not.toHaveBeenCalled();
  });

  it('never undoes a stop the park made on purpose', () => {
    const { hls, raw } = fakeHls({ loadingEnabled: false });
    expect(resumeLoadAfterRecover(hls, { loadStopped: true })).toBe(false);
    expect(raw.startLoad).not.toHaveBeenCalled();
  });
});

/**
 * A loader that follows hls.js 1.7.2's attach rule: when media attaches while
 * the loader is stopped, it starts loading again if `config.autoStartLoad` is on.
 */
function attachingHls(config: { autoStartLoad?: boolean }) {
  const state = { loading: false };
  return {
    config,
    state,
    startLoad() {
      state.loading = true;
    },
    stopLoad() {
      state.loading = false;
    },
    mediaAttached() {
      if (config.autoStartLoad && !state.loading) this.startLoad();
    },
  };
}

describe('the attach rule with the start config', () => {
  it('stays stopped on attach when HLS_START_CONFIG is merged in', () => {
    const defaults: { autoStartLoad: boolean } = { autoStartLoad: true };
    const hls = attachingHls({ ...defaults, ...HLS_START_CONFIG });
    hls.startLoad();
    hls.stopLoad();
    hls.mediaAttached();
    expect(hls.state.loading).toBe(false);
  });

  it('restarts on attach without it', () => {
    const hls = attachingHls({ autoStartLoad: true });
    hls.startLoad();
    hls.stopLoad();
    hls.mediaAttached();
    expect(hls.state.loading).toBe(true);
  });
});

describe('the desktop players hold loading until the page starts it', () => {
  const root = resolve(__dirname, '..');
  const source = (file: string) => readFileSync(resolve(root, file), 'utf8').replace(/\r\n/g, '\n');

  it('turns hls.js auto start off', () => {
    expect(HLS_START_CONFIG.autoStartLoad).toBe(false);
  });

  it.each(['components/VideoPlayer.tsx', 'components/multi-nook/useMultiNookPlayer.ts'])(
    '%s builds hls.js with that config, starts on the manifest and resumes after a recover',
    (file) => {
      const src = source(file);
      expect(src).toContain('...HLS_START_CONFIG,');
      expect(src).not.toMatch(/autoStartLoad:\s*true/);
      expect(src).toMatch(/MANIFEST_PARSED, \([^)]*\) => \{\n\s*startLoadOnManifest\(hls, park\);/);
      const recovers = src.match(/hls\.recoverMediaError\(\);/g) ?? [];
      const resumes = src.match(/resumeLoadAfterRecover\(hls, park\);/g) ?? [];
      expect(recovers.length).toBeGreaterThan(0);
      expect(resumes.length).toBe(recovers.length);
    },
  );
});
