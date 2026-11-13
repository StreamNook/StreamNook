import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(() => {
  vi.resetModules();
  vi.unstubAllGlobals();
  vi.doUnmock('./platform');
  vi.doUnmock('@tauri-apps/api/core');
});

describe('spokenLine', () => {
  it('is one trimmed line of at most 300 characters, or null', async () => {
    const { spokenLine } = await import('./speech');
    expect(spokenLine('  hello \n  world ')).toBe('hello world');
    expect(spokenLine(' \n ')).toBeNull();
    expect(spokenLine('a'.repeat(500))).toHaveLength(300);
  });
});

describe('speakText on a page with speechSynthesis', () => {
  function synth(voices: unknown[]) {
    const speak = vi.fn();
    vi.stubGlobal('window', {
      speechSynthesis: { getVoices: () => voices, speak, addEventListener: vi.fn(), removeEventListener: vi.fn() },
      setTimeout,
      clearTimeout,
    });
    vi.stubGlobal('SpeechSynthesisUtterance', class { constructor(public text: string) {} });
    return speak;
  }

  it('speaks when a voice exists', async () => {
    vi.doMock('./platform', () => ({ IS_ANDROID: false }));
    const speak = synth([{}]);
    const { speakText } = await import('./speech');
    expect(await speakText('hi there')).toBe(true);
    expect(speak).toHaveBeenCalledTimes(1);
  });

  it('answers false (the caller shows the text) when there is no voice', async () => {
    vi.doMock('./platform', () => ({ IS_ANDROID: false }));
    const speak = synth([]);
    const { speakText } = await import('./speech');
    expect(await speakText('hi there')).toBe(false);
    expect(speak).not.toHaveBeenCalled();
  });

  it('answers false for nothing to say', async () => {
    vi.doMock('./platform', () => ({ IS_ANDROID: false }));
    synth([{}]);
    const { speakText } = await import('./speech');
    expect(await speakText('   ')).toBe(false);
  });
});

describe('speakText on Android', () => {
  it('goes through speak_android and reads { spoken }', async () => {
    vi.doMock('./platform', () => ({ IS_ANDROID: true }));
    const invoke = vi.fn().mockResolvedValueOnce({ spoken: true }).mockResolvedValueOnce({ spoken: false }).mockRejectedValueOnce(new Error('x'));
    vi.doMock('@tauri-apps/api/core', () => ({ invoke }));
    const { speakText } = await import('./speech');
    expect(await speakText('one')).toBe(true);
    expect(invoke).toHaveBeenCalledWith('speak_android', { text: 'one' });
    expect(await speakText('two')).toBe(false);
    expect(await speakText('three')).toBe(false);
  });
});
