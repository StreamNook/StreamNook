// Optional audio processing for the stream player. The graph is:
//
//   <video> -> MediaElementSource -> DynamicsCompressor -> Gain -> destination
//
// The compressor levels out loud and quiet moments; the gain stage then pushes
// the whole signal louder than the source without the harsh clipping you'd get
// from simply raising volume past 100% (the peaks are already tamed). When the
// feature is off, the element routes straight through (source -> destination),
// which is sonically transparent.
//
// Two hard rules of the Web Audio API shape this module:
//   1. An element can be tapped exactly once for its lifetime. A second
//      createMediaElementSource() on the same element throws, so the per-element
//      graph is memoized and reused (see `graphs`).
//   2. Once an element is tapped, it only makes sound if the source reaches the
//      destination. So "off" is an explicit source -> destination passthrough,
//      not a disconnect.
//
// Because of rule 1, the element is never tapped until the feature has been
// enabled at least once: while it has always been off, this module leaves
// playback completely untouched.
//
// A third rule is about lifetime: once a tapped element has played through its
// source node, that node stays registered with its AudioContext, and holds the
// element, for as long as the context is open. Disconnecting does not reliably
// release it; only closing the context does. So every element gets a context of
// its own, and the player hands the element back with `releaseAudioGraph` when
// it unmounts. A shared context kept every player the app ever built alive,
// with its whole DOM subtree, for the rest of the session.

import { Logger } from './logger';
import { IS_MAC } from './platform';
import type { AudioBoostSettings } from '../types';
import { DEFAULT_AUDIO_BOOST } from '../types';

/**
 * Whether this shell can route stream audio through Web Audio at all.
 *
 * WebKit cannot hand an MSE-backed media element to an AudioContext: the
 * MediaElementAudioSourceNode outputs zeros AND the element stops feeding the
 * speakers, so one call permanently silences the stream for that element's
 * lifetime. That is WebKit bug 180696, open since 2017 and still unfixed; the
 * bug thread has the explicit MSE confirmation ("Safari fails using MSE/MMS
 * with hls.js. MediaElementAudioSourceNode outputs zeros"), which is exactly
 * how StreamNook plays. macOS is the only desktop shell on WebKit.
 *
 * This has to be a hard refusal rather than a graceful fallback because of rule
 * 1 above: once an element is tapped there is no untapping it, so the passthrough
 * that "off" relies on is already silent by then.
 *
 * iOS, when it ships, needs the same guard for the same reason - it is WebKit
 * too, and Song Identification reaches this file from the mobile player.
 */
export const AUDIO_GRAPH_SUPPORTED = !IS_MAC;

/** Plain-language reason, for the controls that have to explain themselves. */
export const AUDIO_GRAPH_REFUSAL =
  "Not available on macOS: the system's video engine won't share stream audio with the app.";

interface MediaGraph {
  ctx: AudioContext;
  source: MediaElementAudioSourceNode;
  compressor: DynamicsCompressorNode;
  gain: GainNode;
}

// Per-element graphs, each with its own context (rule 3). The player releases
// its element on unmount, so only the element on screen holds an open context.
const graphs = new WeakMap<HTMLMediaElement, MediaGraph>();

const clamp = (v: number, min: number, max: number) =>
  Number.isFinite(v) ? Math.min(max, Math.max(min, v)) : min;

function createCtx(): AudioContext | null {
  const Ctor =
    window.AudioContext ||
    (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!Ctor) return null;
  try {
    return new Ctor();
  } catch (e) {
    Logger.warn('[AudioBoost] Could not create AudioContext:', e);
    return null;
  }
}

function getOrCreateGraph(video: HTMLMediaElement): MediaGraph | null {
  // Fail safe, not fail silent. Every public entry point below already checks,
  // but this is the one place that actually taps the element, so a future
  // caller that forgets the platform cannot mute the stream.
  if (!AUDIO_GRAPH_SUPPORTED) return null;

  const existing = graphs.get(video);
  if (existing) return existing;

  const ctx = createCtx();
  if (!ctx) return null;

  let source: MediaElementAudioSourceNode;
  try {
    source = ctx.createMediaElementSource(video);
  } catch (e) {
    // Already tapped, or the element can't be routed. Leave playback untouched.
    Logger.warn('[AudioBoost] createMediaElementSource failed:', e);
    void ctx.close().catch(() => {});
    return null;
  }

  const graph: MediaGraph = {
    ctx,
    source,
    compressor: ctx.createDynamicsCompressor(),
    gain: ctx.createGain(),
  };
  graphs.set(video, graph);
  return graph;
}

/**
 * Hand back an element the player is discarding. Closes its context, which is
 * the only thing that lets the element (and the player DOM around it) be
 * collected once it has played through the graph. A no-op for an element that
 * was never tapped. The element cannot make sound afterwards, so call this only
 * when it is leaving for good.
 */
export function releaseAudioGraph(video: HTMLMediaElement | null): void {
  if (!video) return;
  const graph = graphs.get(video);
  if (!graph) return;
  graphs.delete(video);
  for (const node of [graph.source, graph.compressor, graph.gain]) {
    try {
      node.disconnect();
    } catch {
      /* not connected */
    }
  }
  void graph.ctx.close().catch((e) => Logger.warn('[AudioBoost] could not close AudioContext:', e));
}

/**
 * `releaseAudioGraph`, but only once the element has actually left the page.
 * For effect cleanups: checked on the next task, after React has removed the
 * element on a real unmount, while a rehearsal unmount (StrictMode) leaves it in
 * place and it keeps its sound.
 */
export function releaseAudioGraphOnceGone(video: HTMLMediaElement): void {
  setTimeout(() => {
    if (!video.isConnected) releaseAudioGraph(video);
  }, 0);
}

// Fill in any missing fields from the defaults so callers can pass a possibly
// partial / undefined settings object straight from persisted state.
export function resolveAudioBoost(
  cfg: AudioBoostSettings | undefined | null,
): AudioBoostSettings {
  return { ...DEFAULT_AUDIO_BOOST, ...(cfg ?? {}) };
}

/**
 * Route the player's audio through the compressor + makeup-gain chain when
 * enabled, or straight through when not. Idempotent: safe to call on every
 * settings change. A no-op while the feature has never been on for this
 * element. The caller releases the element with `releaseAudioGraph` when it
 * discards it.
 */
export function applyAudioBoost(
  video: HTMLMediaElement | null,
  cfg: AudioBoostSettings,
): void {
  if (!video) return;
  if (!AUDIO_GRAPH_SUPPORTED) return;
  // Do no harm until the feature has actually been turned on at least once.
  if (!cfg.enabled && !graphs.has(video)) return;

  const graph = getOrCreateGraph(video);
  if (!graph) return;
  const { ctx } = graph;

  // A suspended context outputs silence (autoplay policy). This runs from a
  // settings toggle or a play event, both user gestures, so resume succeeds.
  if (ctx.state === 'suspended') void ctx.resume();

  const { source, compressor, gain } = graph;
  const t = ctx.currentTime;
  compressor.threshold.setValueAtTime(clamp(cfg.threshold, -100, 0), t);
  compressor.knee.setValueAtTime(clamp(cfg.knee, 0, 40), t);
  compressor.ratio.setValueAtTime(clamp(cfg.ratio, 1, 20), t);
  compressor.attack.setValueAtTime(clamp(cfg.attack, 0, 1), t);
  compressor.release.setValueAtTime(clamp(cfg.release, 0, 1), t);
  gain.gain.setValueAtTime(clamp(cfg.gain, 0, 4), t);

  // Rewire from scratch so toggling never stacks duplicate connections.
  try {
    source.disconnect();
  } catch {
    /* not connected yet */
  }
  try {
    compressor.disconnect();
  } catch {
    /* not connected yet */
  }
  try {
    gain.disconnect();
  } catch {
    /* not connected yet */
  }

  if (cfg.enabled) {
    source.connect(compressor);
    compressor.connect(gain);
    gain.connect(ctx.destination);
  } else {
    // Transparent passthrough (see rule 2 above).
    source.connect(ctx.destination);
  }
}

// ---------------------------------------------------------------------------
// On-demand audio capture for song identification. It branches a short
// recording tap off the SAME source node the boost graph uses (rule 1: an
// element can only be tapped once), so it works whether or not boost is on and
// never disturbs playback. The tap runs on the audio thread (AudioWorklet), not
// the main thread, so a capture never competes with the player's video work.
// Output is mono 16 kHz signed-16-bit PCM, the format the recognizer expects.
// ---------------------------------------------------------------------------

const CAPTURE_PROCESSOR_SOURCE = `
class SnCaptureProcessor extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0];
    if (input && input.length) {
      const channels = input.length;
      const frames = input[0].length;
      const mono = new Float32Array(frames);
      for (let c = 0; c < channels; c++) {
        const ch = input[c];
        for (let i = 0; i < frames; i++) mono[i] += ch[i];
      }
      if (channels > 1) for (let i = 0; i < frames; i++) mono[i] /= channels;
      this.port.postMessage(mono, [mono.buffer]);
    }
    return true;
  }
}
registerProcessor('sn-capture', SnCaptureProcessor);
`;

// A worklet module is registered per context, and each element has its own.
const captureWorkletReady = new WeakMap<AudioContext, Promise<void>>();
function ensureCaptureWorklet(ctx: AudioContext): Promise<void> {
  const pending = captureWorkletReady.get(ctx);
  if (pending) return pending;
  const blob = new Blob([CAPTURE_PROCESSOR_SOURCE], { type: 'application/javascript' });
  const url = URL.createObjectURL(blob);
  const ready = ctx.audioWorklet.addModule(url).finally(() => URL.revokeObjectURL(url));
  captureWorkletReady.set(ctx, ready);
  return ready;
}

// Linear-resample a mono Float32 buffer to `outRate` and convert to signed 16
// bit. Linear interpolation is plenty here: the fingerprint tolerates it.
function toMono16kPcm(input: Float32Array, inRate: number, outRate: number): Int16Array {
  const clampToI16 = (sample: number) => {
    const s = Math.max(-1, Math.min(1, sample));
    return s < 0 ? s * 0x8000 : s * 0x7fff;
  };
  if (inRate === outRate) {
    const out = new Int16Array(input.length);
    for (let i = 0; i < input.length; i++) out[i] = clampToI16(input[i]);
    return out;
  }
  const ratio = inRate / outRate;
  const outLen = Math.floor(input.length / ratio);
  const out = new Int16Array(outLen);
  for (let i = 0; i < outLen; i++) {
    const pos = i * ratio;
    const i0 = Math.floor(pos);
    const i1 = Math.min(i0 + 1, input.length - 1);
    const frac = pos - i0;
    out[i] = clampToI16(input[i0] * (1 - frac) + input[i1] * frac);
  }
  return out;
}

/**
 * Record `seconds` of the player's audio and return it as mono 16 kHz PCM, or
 * null if capture isn't possible. Safe to call regardless of the boost feature
 * state; it leaves playback untouched.
 */
export async function captureStreamSamples(
  video: HTMLMediaElement | null,
  seconds: number,
): Promise<Int16Array | null> {
  if (!video) return null;
  if (!AUDIO_GRAPH_SUPPORTED) return null;

  // This may be the first time the element is ever tapped (boost never enabled).
  // If so, nothing routes the source to the speakers yet, so add the passthrough
  // or the stream would go silent the moment we tap it.
  const firstTap = !graphs.has(video);
  const graph = getOrCreateGraph(video);
  if (!graph) return null;
  const { ctx } = graph;
  if (firstTap) {
    try {
      graph.source.connect(ctx.destination);
    } catch {
      /* already routed */
    }
  }

  if (ctx.state === 'suspended') {
    try {
      await ctx.resume();
    } catch {
      /* best effort; a muted/paused element is handled by the caller */
    }
  }

  try {
    await ensureCaptureWorklet(ctx);
  } catch (e) {
    Logger.warn('[AudioBoost] capture worklet load failed:', e);
    return null;
  }

  const node = new AudioWorkletNode(ctx, 'sn-capture');
  const chunks: Float32Array[] = [];
  node.port.onmessage = (e) => {
    chunks.push(e.data as Float32Array);
  };

  // The node must reach the destination to be pulled by the graph; it writes no
  // output, so this branch is silent and doesn't double the audio.
  graph.source.connect(node);
  node.connect(ctx.destination);

  await new Promise((resolve) => setTimeout(resolve, Math.round(seconds * 1000)));

  try {
    graph.source.disconnect(node);
  } catch {
    /* ignore */
  }
  try {
    node.disconnect();
  } catch {
    /* ignore */
  }
  node.port.onmessage = null;

  if (chunks.length === 0) return null;

  let total = 0;
  for (const c of chunks) total += c.length;
  const merged = new Float32Array(total);
  let offset = 0;
  for (const c of chunks) {
    merged.set(c, offset);
    offset += c.length;
  }

  return toMono16kPcm(merged, ctx.sampleRate, 16000);
}

// ---------------------------------------------------------------------------
// UI descriptors. Kept here (not in the .tsx that renders them) so the shared
// fader component file only exports components. One descriptor per adjustable
// parameter, in display order: Boost (makeup gain) first, then the five
// compressor controls. `value`/`display` are pre-converted for the UI
// (attack/release shown in ms) and `apply` converts back to storage.
// ---------------------------------------------------------------------------

export interface AudioBoostFaderDef {
  key: keyof AudioBoostSettings;
  label: string;
  display: string;
  value: number;
  min: number;
  max: number;
  step: number;
  hint: string;
  apply: (v: number) => Partial<AudioBoostSettings>;
}

export const audioBoostFaderDefs = (b: AudioBoostSettings): AudioBoostFaderDef[] => [
  {
    key: 'gain',
    label: 'Boost',
    value: b.gain,
    display: `${Math.round(b.gain * 100)}%`,
    min: 1,
    max: 3,
    step: 0.05,
    hint: 'How much louder to make the stream after compression. 100% is no extra boost; higher is louder.',
    apply: (v) => ({ gain: v }),
  },
  {
    key: 'threshold',
    label: 'Threshold',
    value: b.threshold,
    display: `${Math.round(b.threshold)} dB`,
    min: -100,
    max: 0,
    step: 1,
    hint: 'The level where compression kicks in. Lower catches more of the audio.',
    apply: (v) => ({ threshold: v }),
  },
  {
    key: 'ratio',
    label: 'Ratio',
    value: b.ratio,
    display: `${b.ratio.toFixed(1)}:1`,
    min: 1,
    max: 20,
    step: 0.5,
    hint: 'How hard to compress once over the threshold. Higher is more aggressive leveling.',
    apply: (v) => ({ ratio: v }),
  },
  {
    key: 'knee',
    label: 'Knee',
    value: b.knee,
    display: `${Math.round(b.knee)} dB`,
    min: 0,
    max: 40,
    step: 1,
    hint: 'How gradually compression eases in around the threshold. Higher is smoother.',
    apply: (v) => ({ knee: v }),
  },
  {
    key: 'attack',
    label: 'Attack',
    value: Math.round(b.attack * 1000),
    display: `${Math.round(b.attack * 1000)} ms`,
    min: 0,
    max: 200,
    step: 1,
    hint: 'How quickly it clamps down on a sudden loud sound.',
    apply: (v) => ({ attack: v / 1000 }),
  },
  {
    key: 'release',
    label: 'Release',
    value: Math.round(b.release * 1000),
    display: `${Math.round(b.release * 1000)} ms`,
    min: 0,
    max: 1000,
    step: 10,
    hint: 'How quickly it eases back off once things get quieter.',
    apply: (v) => ({ release: v / 1000 }),
  },
];

// All adjustable params (Boost + the five compressor controls) reset to
// defaults; the on/off state is left as-is.
export const audioBoostResetPatch = (): Partial<AudioBoostSettings> => ({
  gain: DEFAULT_AUDIO_BOOST.gain,
  threshold: DEFAULT_AUDIO_BOOST.threshold,
  knee: DEFAULT_AUDIO_BOOST.knee,
  ratio: DEFAULT_AUDIO_BOOST.ratio,
  attack: DEFAULT_AUDIO_BOOST.attack,
  release: DEFAULT_AUDIO_BOOST.release,
});
