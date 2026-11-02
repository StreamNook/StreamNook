// The dialog wrapper is the only door to the dialog plugin, and it tells Rust a
// dialog is open for exactly as long as one is, even when the dialog fails.
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const h = vi.hoisted(() => ({
  calls: [] as string[],
  // A factory, so a rejection is created only when the plugin is called and
  // is always handled.
  openResult: (): Promise<unknown> => Promise.resolve('C:/pick.json'),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: { open: boolean }) => {
    h.calls.push(`${cmd}:${args.open}`);
    return Promise.resolve(null);
  },
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: () => {
    h.calls.push('plugin:open');
    return h.openResult();
  },
  save: () => {
    h.calls.push('plugin:save');
    return Promise.resolve(null);
  },
  ask: () => {
    h.calls.push('plugin:ask');
    return Promise.resolve(true);
  },
}));
vi.mock('./platform', () => ({ IS_MOBILE: false }));

import { ask, open, save } from './dialog';

const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => {
  h.calls = [];
  h.openResult = () => Promise.resolve('C:/pick.json');
});

describe('dialog wrapper', () => {
  it('marks a dialog open around the plugin call', async () => {
    await expect(open({ multiple: false })).resolves.toBe('C:/pick.json');
    await flush();
    expect(h.calls).toEqual(['set_dialog_open:true', 'plugin:open', 'set_dialog_open:false']);
  });

  it('clears the mark when the dialog fails', async () => {
    h.openResult = () => Promise.reject(new Error('portal gone'));
    await expect(open()).rejects.toThrow('portal gone');
    await flush();
    expect(h.calls).toEqual(['set_dialog_open:true', 'plugin:open', 'set_dialog_open:false']);
  });

  it('save and ask go through the same door', async () => {
    await save();
    await ask('Upload?');
    await flush();
    expect(h.calls).toEqual([
      'set_dialog_open:true',
      'plugin:save',
      'set_dialog_open:false',
      'set_dialog_open:true',
      'plugin:ask',
      'set_dialog_open:false',
    ]);
  });
});

describe('the only importer', () => {
  it('nothing else in src imports the dialog plugin', () => {
    const src = resolve(__dirname, '..');
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) {
          walk(path);
          continue;
        }
        if (!/\.(ts|tsx)$/.test(name) || /\.test\.(ts|tsx)$/.test(name)) continue;
        const rel = relative(src, path).replace(/\\/g, '/');
        if (rel === 'utils/dialog.ts') continue;
        if (/['"]@tauri-apps\/plugin-dialog['"]/.test(readFileSync(path, 'utf8'))) offenders.push(rel);
      }
    };
    walk(src);
    expect(offenders).toEqual([]);
  });
});
