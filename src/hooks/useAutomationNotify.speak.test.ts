import { beforeEach, describe, expect, it, vi } from 'vitest';

const listeners = new Map<string, (e: { payload: unknown }) => void>();
const invoke = vi.fn();
const addToast = vi.fn();
const speakText = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
      listeners.set(name, cb);
      return () => listeners.delete(name);
    },
  }),
}));
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast }) } }));
vi.mock('../utils/notificationSound', () => ({ playSound: vi.fn() }));
vi.mock('../utils/speech', () => ({
  speakText: (...args: unknown[]) => speakText(...args),
  speechAvailable: () => Promise.resolve(true),
}));

import { startAutomationSpeak } from './useAutomationNotify';

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('startAutomationSpeak', () => {
  beforeEach(() => {
    listeners.clear();
    invoke.mockReset();
    addToast.mockReset();
    speakText.mockReset();
  });

  it('speaks a line only when this window wins the claim', async () => {
    const stop = startAutomationSpeak();
    await flush();
    invoke.mockResolvedValue(false);
    listeners.get('automations://speak')!({ payload: { id: 5, text: 'hi', channel: '' } });
    await flush();
    expect(invoke).toHaveBeenCalledWith('automations_notify_claim', { id: 5 });
    expect(speakText).not.toHaveBeenCalled();
    invoke.mockResolvedValue(true);
    speakText.mockResolvedValue(true);
    listeners.get('automations://speak')!({ payload: { id: 6, text: 'hello', channel: '' } });
    await flush();
    expect(speakText).toHaveBeenCalledWith('hello');
    expect(addToast).not.toHaveBeenCalled();
    stop();
  });

  it('shows the text as a toast when this device cannot speak', async () => {
    const stop = startAutomationSpeak();
    await flush();
    invoke.mockResolvedValue(true);
    speakText.mockResolvedValue(false);
    listeners.get('automations://speak')!({ payload: { id: 7, text: 'Thanks Fan', channel: '' } });
    await flush();
    expect(addToast).toHaveBeenCalledWith('Thanks Fan', 'info');
    stop();
  });

  it('stops listening when its window goes away', async () => {
    const stop = startAutomationSpeak();
    await flush();
    expect(listeners.has('automations://speak')).toBe(true);
    stop();
    expect(listeners.has('automations://speak')).toBe(false);
  });
});
