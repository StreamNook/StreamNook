import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// MultiNook lives in its own store, so the boot watch on the app store never
// sees it open: entering the grid notes the boot action itself. While the app
// boots, that drops the link that is still waiting in Rust.

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

const app = vi.hoisted(() => ({
  state: {
    isBooting: true,
    isHomeActive: false,
    streamUrl: null as string | null,
    settings: {} as Record<string, unknown>,
    addToast: () => {},
    currentStream: null,
    toggleHome: () => {},
  },
}));
vi.mock('./AppStore', () => ({
  useAppStore: Object.assign(vi.fn(), { getState: () => app.state, setState: vi.fn(), subscribe: vi.fn() }),
}));
vi.mock('../utils/settingsBroadcast', () => ({ patchSettings: vi.fn().mockResolvedValue(undefined) }));

const win = vi.hoisted(() => ({ main: true }));
vi.mock('../utils/platform', async (original) => ({
  ...(await original<typeof import('../utils/platform')>()),
  IS_MAIN_WINDOW: true,
  isMainWindow: () => win.main,
}));

import { usemultiNookStore } from './multiNookStore';
import { noteLinkStamp, noticeWorkSettled, resetNoticeReportForTest, runAsAutomatic } from '../utils/noticeReport';

const superseded = () => invokeMock.mock.calls.filter(([c]) => c === 'link_superseded');

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  resetNoticeReportForTest();
  win.main = true;
  app.state.isBooting = true;
  noteLinkStamp(9);
  usemultiNookStore.setState({ slots: [], isMultiNookActive: false });
});

describe('toggleMultiNook while the app boots', () => {
  it('entering the grid tells Rust the page acted, once, with the stamp it has heard', async () => {
    // Fails against the old store: nothing in MultiNook notes the action, and
    // the watch on the app store cannot see this store's flag.
    await usemultiNookStore.getState().toggleMultiNook();
    await noticeWorkSettled();
    expect(superseded()).toHaveLength(1);
    expect(superseded()[0][1]).toEqual({ seen: 9 });
    expect(usemultiNookStore.getState().isMultiNookActive).toBe(true);
  });

  it('tells Rust nothing once boot is over', async () => {
    app.state.isBooting = false;
    await usemultiNookStore.getState().toggleMultiNook();
    await noticeWorkSettled();
    expect(superseded()).toHaveLength(0);
  });

  it('tells Rust nothing from a popout', async () => {
    win.main = false;
    await usemultiNookStore.getState().toggleMultiNook();
    await noticeWorkSettled();
    expect(superseded()).toHaveLength(0);
  });

  it('tells Rust nothing for a reopen the app makes by itself', async () => {
    let pending: unknown;
    runAsAutomatic(() => {
      pending = usemultiNookStore.getState().toggleMultiNook();
    });
    await pending;
    await noticeWorkSettled();
    expect(superseded()).toHaveLength(0);
  });

  it('leaving the grid tells Rust nothing: only entering opens a page', async () => {
    usemultiNookStore.setState({ isMultiNookActive: true });
    await usemultiNookStore.getState().toggleMultiNook();
    await noticeWorkSettled();
    expect(superseded()).toHaveLength(0);
  });

  it('notes it as the first statement of the entering branch (source pin)', () => {
    const text = readFileSync(resolve(__dirname, 'multiNookStore.ts'), 'utf8').replace(/\r\n/g, '\n');
    const from = text.indexOf('toggleMultiNook: async () => {');
    const branch = text.indexOf('if (newState) {', from);
    const first = text.indexOf('\n', text.indexOf('// Entering multi-nook mode', branch)) + 1;
    expect(text.slice(first).trimStart()).toMatch(
      /^\/\/ [^\n]+\n\s*if \(useAppStore\.getState\(\)\.isBooting && isMainWindow\(\)\) noteBootAction\(invoke\);/,
    );
  });
});
