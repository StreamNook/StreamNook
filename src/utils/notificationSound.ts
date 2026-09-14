// Shared Web-Audio notification sounds. Used by ToastManager for live/whisper/
// drops/etc. and by the chat highlight system for per-phrase sound cues. One
// global AudioContext is reused across callers so we don't leak contexts when
// many phrase matches fire in quick succession.

import { convertFileSrc } from '@tauri-apps/api/core';
import { Logger } from './logger';
import { isStreamerModeActive } from './streamerMode';

export type SoundId = 'boop' | 'tick' | 'soft' | 'whisper' | 'gentle';

export const SOUND_LABELS: Record<SoundId, string> = {
  boop: 'Subtle Boop',
  tick: 'Cozy Knock',
  soft: 'Fireplace Crackle',
  whisper: 'Raindrop',
  gentle: 'Wind Chime',
};

let globalAudioContext: AudioContext | null = null;

function getSharedAudioContext(): AudioContext | null {
  try {
    if (!globalAudioContext) {
      const AC = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
      if (!AC) return null;
      globalAudioContext = new AC();
    }
    if (globalAudioContext.state === 'suspended') {
      globalAudioContext.resume().catch(() => {});
    }
    return globalAudioContext;
  } catch {
    return null;
  }
}

/** Any sound reference: a built-in tone id or a custom `file:<id>`. */
export type SoundRef = SoundId | string;

const fileAudio = new Map<string, HTMLAudioElement>();

/** Custom sound file: resolved through settings, played via the asset
 *  protocol, one HTMLAudioElement per id reused across plays. */
function playCustomSound(id: string): void {
  void import('../stores/AppStore').then(({ useAppStore }) => {
    const entry = useAppStore.getState().settings.chat_highlights?.custom_sounds?.find((s) => s.id === id);
    if (!entry?.path) return;
    let el = fileAudio.get(id);
    if (!el || el.dataset.path !== entry.path) {
      el = new Audio(convertFileSrc(entry.path));
      el.dataset.path = entry.path;
      el.volume = 0.7;
      fileAudio.set(id, el);
    }
    el.currentTime = 0;
    el.play().catch((err) => Logger.debug('[Sound] custom play failed:', err));
  });
}

export function playSound(soundId: SoundRef | undefined | null): void {
  if (!soundId) return;
  if (soundId.startsWith('file:')) {
    playCustomSound(soundId);
    return;
  }
  const ctx = getSharedAudioContext();
  if (!ctx) return;

  try {
    const oscillator = ctx.createOscillator();
    const gainNode = ctx.createGain();
    oscillator.connect(gainNode);
    gainNode.connect(ctx.destination);

    const t = ctx.currentTime;

    switch (soundId) {
      case 'tick':
        oscillator.type = 'triangle';
        oscillator.frequency.setValueAtTime(280, t);
        oscillator.frequency.exponentialRampToValueAtTime(180, t + 0.08);
        gainNode.gain.setValueAtTime(0, t);
        gainNode.gain.linearRampToValueAtTime(0.08, t + 0.01);
        gainNode.gain.exponentialRampToValueAtTime(0.001, t + 0.15);
        oscillator.start(t);
        oscillator.stop(t + 0.15);
        break;

      case 'soft':
        oscillator.type = 'triangle';
        oscillator.frequency.setValueAtTime(420, t);
        oscillator.frequency.exponentialRampToValueAtTime(320, t + 0.3);
        gainNode.gain.setValueAtTime(0, t);
        gainNode.gain.linearRampToValueAtTime(0.06, t + 0.04);
        gainNode.gain.setValueAtTime(0.05, t + 0.15);
        gainNode.gain.exponentialRampToValueAtTime(0.001, t + 0.5);
        oscillator.start(t);
        oscillator.stop(t + 0.5);
        break;

      case 'whisper':
        oscillator.type = 'sine';
        oscillator.frequency.setValueAtTime(520, t);
        oscillator.frequency.exponentialRampToValueAtTime(380, t + 0.12);
        gainNode.gain.setValueAtTime(0, t);
        gainNode.gain.linearRampToValueAtTime(0.07, t + 0.02);
        gainNode.gain.exponentialRampToValueAtTime(0.001, t + 0.25);
        oscillator.start(t);
        oscillator.stop(t + 0.25);
        break;

      case 'gentle':
        oscillator.type = 'sine';
        oscillator.frequency.setValueAtTime(550, t);
        oscillator.frequency.setValueAtTime(580, t + 0.1);
        oscillator.frequency.exponentialRampToValueAtTime(480, t + 0.6);
        gainNode.gain.setValueAtTime(0, t);
        gainNode.gain.linearRampToValueAtTime(0.05, t + 0.08);
        gainNode.gain.setValueAtTime(0.04, t + 0.25);
        gainNode.gain.exponentialRampToValueAtTime(0.001, t + 0.7);
        oscillator.start(t);
        oscillator.stop(t + 0.7);
        break;

      case 'boop':
      default:
        oscillator.type = 'sine';
        oscillator.frequency.setValueAtTime(800, t);
        oscillator.frequency.exponentialRampToValueAtTime(400, t + 0.1);
        gainNode.gain.setValueAtTime(0, t);
        gainNode.gain.linearRampToValueAtTime(0.15, t + 0.05);
        gainNode.gain.exponentialRampToValueAtTime(0.01, t + 0.3);
        oscillator.start(t);
        oscillator.stop(t + 0.3);
        break;
    }
  } catch (error) {
    Logger.warn('Could not play notification sound:', error);
  }
}

// Per-key cooldown tracking. Skipping plays inside the cooldown window avoids
// audio spam in fast chats where the same phrase matches many messages in a
// row. The key is caller-defined (typically a phrase id) so different keys
// don't share a cooldown.
const lastPlayedAt = new Map<string, number>();
const DEFAULT_COOLDOWN_MS = 3000;
// Older messages (history backfill, reconnect replay) shouldn't fire sounds —
// only treat a message as "live" if its server-stamped send time is within
// this window of now. tmi-sent-ts is millis since epoch.
const BACKFILL_SKIP_MS = 5000;

export interface SoundPlayOptions {
  key: string;
  soundId: SoundRef | null | undefined;
  cooldownMs?: number;
  // Server-stamped send time (e.g. tmi-sent-ts). When set, plays only fire if
  // the message is at most BACKFILL_SKIP_MS old — prevents the audio storm
  // that would otherwise hit when a chat first loads recent history.
  sentAtMs?: number | null;
}

export function playSoundThrottled({
  key,
  soundId,
  cooldownMs = DEFAULT_COOLDOWN_MS,
  sentAtMs,
}: SoundPlayOptions): void {
  if (!soundId) return;
  // Streamer mode: no pings on stream.
  if (isStreamerModeActive()) return;
  if (sentAtMs != null && Number.isFinite(sentAtMs) && Date.now() - sentAtMs > BACKFILL_SKIP_MS) {
    return;
  }
  const now = Date.now();
  const last = lastPlayedAt.get(key) ?? 0;
  if (now - last < cooldownMs) return;
  lastPlayedAt.set(key, now);
  playSound(soundId);
}
