import { describe, expect, it, vi } from 'vitest';

vi.mock('../../stores/AppStore', () => ({ useAppStore: Object.assign(vi.fn(), { getState: vi.fn() }) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { atDefault, withDefaults } from './settingReset';
import type { Settings } from '../../types';

const asSettings = (v: unknown) => v as Settings;

// Rust's defaults hold typed groups only; `chat_input` is frontend-owned.
const defaults = asSettings({
  quality: 'best',
  font: null,
  chat_design: { font_size: 16, message_spacing: 11 },
  video_player: { audio_boost: { enabled: false, gain: 1.5 } },
});

describe('atDefault', () => {
  it('is true for a typed value equal to the Rust default', () => {
    const s = asSettings({ ...defaults, chat_design: { font_size: 16, message_spacing: 11 } });
    expect(atDefault(s, defaults, ['chat_design.font_size'])).toBe(true);
  });

  it('is false once a typed value moves off the Rust default', () => {
    const s = asSettings({ ...defaults, chat_design: { font_size: 14, message_spacing: 11 } });
    expect(atDefault(s, defaults, ['chat_design.font_size'])).toBe(false);
    expect(atDefault(s, defaults, ['chat_design.message_spacing'])).toBe(true);
  });

  it('compares a frontend-owned key against the row fallback, absent or present', () => {
    expect(atDefault(defaults, defaults, [['chat_input.spellcheck', true]])).toBe(true);
    const on = asSettings({ ...defaults, chat_input: { spellcheck: true } });
    expect(atDefault(on, defaults, [['chat_input.spellcheck', true]])).toBe(true);
    const off = asSettings({ ...defaults, chat_input: { spellcheck: false } });
    expect(atDefault(off, defaults, [['chat_input.spellcheck', true]])).toBe(false);
  });

  it('treats absent and null as the same', () => {
    const s = asSettings({ ...defaults, font: undefined });
    expect(atDefault(s, defaults, ['font'])).toBe(true);
  });

  it('needs every named key at default for a multi-control row', () => {
    const s = asSettings({ ...defaults, chat_design: { font_size: 16, message_spacing: 8 } });
    expect(atDefault(s, defaults, ['chat_design.font_size', 'chat_design.message_spacing'])).toBe(false);
  });

  it('compares lists by contents, not identity', () => {
    const d = asSettings({ ...defaults, discovery_languages: ['en'] });
    const s = asSettings({ ...d, discovery_languages: ['en'] });
    expect(atDefault(s, d, ['discovery_languages'])).toBe(true);
  });
});

describe('withDefaults', () => {
  it('restores a typed value from Rust without touching its siblings or the input', () => {
    const s = asSettings({ ...defaults, chat_design: { font_size: 12, message_spacing: 4 } });
    const next = withDefaults(s, defaults, ['chat_design.font_size']);
    expect(next.chat_design).toEqual({ font_size: 16, message_spacing: 4 });
    expect((s.chat_design as unknown as { font_size: number }).font_size).toBe(12);
  });

  it('writes the row fallback for a key Rust does not hold', () => {
    const s = asSettings({ ...defaults, chat_input: { spellcheck: false, other: 1 } });
    const next = withDefaults(s, defaults, [['chat_input.spellcheck', true]]);
    expect((next as unknown as { chat_input: unknown }).chat_input).toEqual({ spellcheck: true, other: 1 });
  });

  it('removes a key whose default is null, so the save clears it', () => {
    const s = asSettings({ ...defaults, font: 'Inter' });
    const next = withDefaults(s, defaults, ['font']);
    expect('font' in next).toBe(false);
  });

  it('copies object defaults rather than sharing them', () => {
    const s = asSettings({ ...defaults, video_player: { audio_boost: { enabled: true, gain: 3 } } });
    const next = withDefaults(s, defaults, ['video_player.audio_boost']);
    expect(next.video_player.audio_boost).toEqual({ enabled: false, gain: 1.5 });
    expect(next.video_player.audio_boost).not.toBe(defaults.video_player.audio_boost);
  });
});
