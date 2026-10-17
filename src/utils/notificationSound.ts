// Shared Web-Audio notification sounds. Used by ToastManager for live/whisper/
// drops/etc. and by the chat highlight system for per-phrase sound cues. One
// global AudioContext is reused across callers so we don't leak contexts when
// many phrase matches fire in quick succession.

import { convertFileSrc } from '@tauri-apps/api/core';
import { Logger } from './logger';
import { isStreamerModeActive } from './streamerMode';

type RetroId = 'morse' | 'pager' | 'flip' | 'arcade' | 'gadget' | 'ringtone' | 'granvals';
export type SoundId = 'boop' | 'tick' | 'soft' | 'whisper' | 'gentle' | RetroId;

export const SOUND_LABELS: Record<SoundId, string> = {
  boop: 'Subtle Boop',
  tick: 'Cozy Knock',
  soft: 'Fireplace Crackle',
  whisper: 'Raindrop',
  gentle: 'Wind Chime',
  morse: 'Morse SMS',
  pager: 'Pager',
  flip: 'Flip Phone',
  arcade: 'Arcade',
  gadget: 'Spy Gadget',
  ringtone: 'Old Ringtone',
  granvals: 'Gran Vals',
};

/** Every built-in tone, in picker order: the soft set, then the retro set.
 *  Rust's `VALID_SOUND_IDS` (chat_rules.rs) lists the same ids. */
export const BUILT_IN_SOUNDS: SoundId[] = [
  'boop', 'tick', 'soft', 'whisper', 'gentle',
  'morse', 'pager', 'flip', 'arcade', 'gadget', 'ringtone', 'granvals',
];

/** One note of a retro tone: pitch in Hz, start and length in seconds. */
type Note = [hz: number, at: number, len: number];

/** The retro set: short phone and handheld style beeps, each a run of plain
 *  notes on one waveform. */
const RETRO: Record<RetroId, { wave: OscillatorType; notes: Note[] }> = {
  // "SMS" in Morse: ... -- ...
  morse: (() => {
    const u = 0.065;
    const notes: Note[] = [];
    let at = 0;
    for (const letter of ['...', '--', '...']) {
      for (const mark of letter) {
        const len = mark === '.' ? u : 3 * u;
        notes.push([1760, at, len]);
        at += len + u;
      }
      at += 2 * u;
    }
    return { wave: 'square' as OscillatorType, notes };
  })(),
  pager: {
    wave: 'square',
    notes: [
      [1568, 0, 0.06], [1568, 0.1, 0.06], [1568, 0.2, 0.06],
      [1568, 0.42, 0.06], [1568, 0.52, 0.06], [1568, 0.62, 0.06],
    ],
  },
  flip: { wave: 'triangle', notes: [[1319, 0, 0.07], [1568, 0.08, 0.07], [2093, 0.16, 0.18]] },
  arcade: {
    wave: 'square',
    notes: [[1047, 0, 0.05], [1319, 0.06, 0.05], [1568, 0.12, 0.05], [2093, 0.18, 0.2]],
  },
  gadget: {
    wave: 'square',
    notes: [[1480, 0, 0.07], [1480, 0.13, 0.07], [1480, 0.26, 0.05], [1865, 0.33, 0.16]],
  },
  ringtone: {
    wave: 'square',
    notes: [
      [784, 0, 0.09], [988, 0.1, 0.09], [1175, 0.2, 0.09],
      [1568, 0.3, 0.15], [1175, 0.48, 0.09], [1568, 0.58, 0.2],
    ],
  },
  // The opening phrase of Tarrega's Gran Vals (1902), the waltz the classic
  // phone ringtone came from: three short-short-long-long figures and a
  // closing note.
  granvals: (() => {
    const e = 0.12;
    const bars: [number, number][][] = [
      [[1319, 1], [1175, 1], [740, 2], [831, 2]],
      [[1109, 1], [988, 1], [587, 2], [659, 2]],
      [[988, 1], [880, 1], [554, 2], [659, 2]],
      [[880, 4]],
    ];
    const notes: Note[] = [];
    let at = 0;
    for (const [hz, beats] of bars.flat()) {
      notes.push([hz, at, beats * e - 0.02]);
      at += beats * e;
    }
    return { wave: 'square' as OscillatorType, notes };
  })(),
};

/** Peak level per waveform, so a square beep sits as loud as a soft tone. */
const RETRO_PEAK: Partial<Record<OscillatorType, number>> = { square: 0.035, triangle: 0.08 };

function playRetro(ctx: AudioContext, out: AudioNode, id: RetroId): void {
  const { wave, notes } = RETRO[id];
  const peak = RETRO_PEAK[wave] ?? 0.08;
  const t0 = ctx.currentTime;
  for (const [hz, at, len] of notes) {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = wave;
    osc.frequency.setValueAtTime(hz, t0 + at);
    // A few milliseconds of ramp at each end keeps the edges from clicking.
    gain.gain.setValueAtTime(0, t0 + at);
    gain.gain.linearRampToValueAtTime(peak, t0 + at + 0.005);
    gain.gain.setValueAtTime(peak, t0 + at + len - 0.012);
    gain.gain.linearRampToValueAtTime(0, t0 + at + len);
    osc.connect(gain);
    gain.connect(out);
    osc.start(t0 + at);
    osc.stop(t0 + at + len + 0.01);
  }
}

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

/** A volume setting (percent of the sound's own level, 0-200) as a gain
 *  multiplier. Absent means the sound's own level. */
function volumeScale(percent: number | undefined | null): number {
  if (percent == null || !Number.isFinite(percent)) return 1;
  return Math.max(0, Math.min(2, percent / 100));
}

/** A custom file's element volume at 100%. Element volume cannot exceed 1, so
 *  the headroom above 100% comes from starting below it. */
const FILE_BASE_VOLUME = 0.7;

const fileAudio = new Map<string, HTMLAudioElement>();

/** Custom sound file: resolved through settings, played via the asset
 *  protocol, one HTMLAudioElement per id reused across plays. A plain element,
 *  not a Web Audio graph: an element routed into the shared AudioContext stays
 *  pinned to it, and its cross-origin asset URL would play silent there. */
function playCustomSound(id: string, scale: number): void {
  void import('../stores/AppStore').then(({ useAppStore }) => {
    const entry = useAppStore.getState().settings.chat_highlights?.custom_sounds?.find((s) => s.id === id);
    if (!entry?.path) return;
    let el = fileAudio.get(id);
    if (!el || el.dataset.path !== entry.path) {
      el = new Audio(convertFileSrc(entry.path));
      el.dataset.path = entry.path;
      fileAudio.set(id, el);
    }
    el.volume = Math.min(1, FILE_BASE_VOLUME * scale);
    el.currentTime = 0;
    el.play().catch((err) => Logger.debug('[Sound] custom play failed:', err));
  });
}

/** Play a sound at `volume` percent of its own level (0-200, default 100). */
export function playSound(soundId: SoundRef | undefined | null, volume?: number | null): void {
  if (!soundId) return;
  const scale = volumeScale(volume);
  if (scale <= 0) return;
  if (soundId.startsWith('file:')) {
    playCustomSound(soundId, scale);
    return;
  }
  const ctx = getSharedAudioContext();
  if (!ctx) return;

  if (Object.hasOwn(RETRO, soundId)) {
    try {
      const volumeNode = ctx.createGain();
      volumeNode.gain.value = scale;
      volumeNode.connect(ctx.destination);
      playRetro(ctx, volumeNode, soundId as RetroId);
    } catch (error) {
      Logger.warn('Could not play notification sound:', error);
    }
    return;
  }

  try {
    const oscillator = ctx.createOscillator();
    const gainNode = ctx.createGain();
    // The tones below are shaped at their own level; the volume setting scales
    // the whole envelope after it.
    const volumeNode = ctx.createGain();
    volumeNode.gain.value = scale;
    oscillator.connect(gainNode);
    gainNode.connect(volumeNode);
    volumeNode.connect(ctx.destination);

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

// Notification sounds (go-live, whisper, drops, badges, ...) share one gate.
// Overlapping tones add together at the output, so a burst of go-lives from a
// single poll would otherwise stack into one ping several times louder than
// normal. One ping per window says "something happened"; the toasts and the
// notification center carry the count.
const NOTIFICATION_SOUND_GAP_MS = 2000;
let lastNotificationSoundAt = 0;

export function playNotificationSound(soundId: SoundRef | undefined | null, volume?: number | null): void {
  const now = Date.now();
  if (now - lastNotificationSoundAt < NOTIFICATION_SOUND_GAP_MS) return;
  lastNotificationSoundAt = now;
  playSound(soundId || 'boop', volume);
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
