// Low-latency playback diagnostic recorder.
//
// Streams structured JSON-line records to a backend file (via start_ll_diag /
// append_ll_diag) so a live drift or A/V-desync session can be analyzed from
// RECORDED FACTS instead of inference. The decisive signals it captures:
//   - per-track buffered ends (audio vs video) over time, plus whether the buffer is
//     muxed ("audiovideo") or split — this alone confirms or refutes "video trails
//     audio";
//   - each fragment/part's DEMUXED PTS/DTS vs its declared duration (real seams?);
//   - currentTime, playbackRate, hls.latency, dropped/total frames, PDT, and errors.
//
// Cheap: a handful of property reads per second + a batched IPC every 2s. Active only
// while explicitly started (the LL path), stopped on teardown.

import { invoke } from '@tauri-apps/api/core';
import Hls from 'hls.js';

type AnyHls = Hls & Record<string, unknown>;

// MASTER SWITCH for the low-latency diagnostic harness (the .jsonl captures,
// per-frame recording, hls.js note mirror, seek/longtask observers). OFF by
// default now that the freeze/latency investigation is done — when off,
// startLLDiagnostics no-ops, so nothing records, no files are created, and the
// per-frame work never runs. Flip it back on for a future investigation from
// the devtools console with `__snDiag(true)` (persisted), no rebuild needed.
let llDiagEnabled = false;
try {
  llDiagEnabled = localStorage.getItem('streamnook_lldiag') === 'true';
} catch {
  /* localStorage unavailable */
}
export const isLLDiagEnabled = (): boolean => llDiagEnabled;
export const setLLDiagEnabled = (on: boolean): void => {
  llDiagEnabled = on;
  try {
    localStorage.setItem('streamnook_lldiag', String(on));
  } catch {
    /* ignore */
  }
  // eslint-disable-next-line no-console
  console.warn(`[LLDIAG] diagnostics ${on ? 'ENABLED' : 'disabled'} (takes effect on next stream load)`);
};
try {
  (window as unknown as { __snDiag?: (on: boolean) => void }).__snDiag = setLLDiagEnabled;
} catch {
  /* no window */
}

let pending: string[] = [];
let tickTimer: ReturnType<typeof setInterval> | null = null;
let flushTimer: ReturnType<typeof setInterval> | null = null;
let teardown: Array<() => void> = [];
let active = false;
// The backend session file this recorder owns. Every webview window (solo,
// popouts, MultiNook) has its own copy of this module but the backend keeps ONE
// session; appends carry this path so a recorder superseded by a newer session
// in another window writes nothing instead of polluting that capture with its
// idle player's ticks.
let sessionPath = '';
// Latest per-source-buffer end seen on BUFFER_APPENDED, carried into each tick so we
// always have audio-end and video-end sampled at the same moment.
let lastTrackEnds: Record<string, number> = {};
let sbKinds = ''; // "audiovideo" (muxed) or "audio+video" (split) — recorded once

function rec(obj: Record<string, unknown>): void {
  // Round floats to ms/3-dp so the file stays readable; never throw.
  try {
    pending.push(JSON.stringify(obj));
  } catch {
    /* ignore */
  }
}

function rangesToArr(tr: TimeRanges | null | undefined): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  if (!tr) return out;
  for (let i = 0; i < tr.length; i++) out.push([+tr.start(i).toFixed(3), +tr.end(i).toFixed(3)]);
  return out;
}

function endOf(tr: TimeRanges | null | undefined): number | null {
  return tr && tr.length ? +tr.end(tr.length - 1).toFixed(3) : null;
}

async function flush(): Promise<void> {
  if (!pending.length || !sessionPath) return;
  const batch = pending;
  pending = [];
  try {
    await invoke('append_ll_diag', { lines: batch, path: sessionPath });
  } catch {
    /* drop on failure; never affect playback */
  }
}

export async function startLLDiagnostics(
  hls: Hls,
  video: HTMLVideoElement,
  label: string,
): Promise<void> {
  stopLLDiagnostics();
  if (!llDiagEnabled) return; // master switch off: no session, no overhead
  let path = '';
  try {
    path = await invoke<string>('start_ll_diag', { label });
  } catch (e) {
    // eslint-disable-next-line no-console
    console.warn('[LLDIAG] could not start session', e);
    return;
  }
  active = true;
  sessionPath = path;
  lastTrackEnds = {};
  sbKinds = '';
  // eslint-disable-next-line no-console
  console.log(`%c[LLDIAG] recording playback diagnostics to:\n${path}`, 'color:#1a9c4a;font-weight:bold');
  rec({ ev: 'session', t: Date.now(), path, label, ua: navigator.userAgent });

  const h = hls as AnyHls;

  const onAppended = (_e: unknown, data: { type?: string; timeRanges?: Record<string, TimeRanges> }) => {
    const ranges = data?.timeRanges ?? {};
    const keys = Object.keys(ranges);
    if (!sbKinds && keys.length) {
      sbKinds = keys.slice().sort().join('+');
      rec({ ev: 'sbkind', t: Date.now(), kinds: sbKinds });
    }
    const ends: Record<string, number | null> = {};
    for (const k of keys) {
      const e = endOf(ranges[k]);
      ends[k] = e;
      if (e != null) lastTrackEnds[k] = e;
    }
    rec({ ev: 'appended', t: Date.now(), type: data?.type ?? null, ends });
  };

  const onFragBuffered = (_e: unknown, data: { frag?: Record<string, number>; part?: { index?: number } }) => {
    const f = data?.frag;
    if (!f) return;
    rec({
      ev: 'frag',
      t: Date.now(),
      sn: f.sn,
      type: f.type,
      level: f.level,
      start: round(f.start),
      dur: round(f.duration),
      sPTS: round(f.startPTS),
      ePTS: round(f.endPTS),
      sDTS: round(f.startDTS),
      eDTS: round(f.endDTS),
      // measured PTS span vs the duration the playlist declared (seam check)
      ptsSpan: f.startPTS != null && f.endPTS != null ? round(f.endPTS - f.startPTS) : null,
      part: data?.part?.index ?? -1,
    });
  };

  const onError = (_e: unknown, data: { type?: string; details?: string; fatal?: boolean; error?: { message?: string } }) => {
    rec({ ev: 'err', t: Date.now(), type: data?.type, details: data?.details, fatal: data?.fatal, msg: data?.error?.message });
  };

  // Client-measured playlist load timing, the counterpart of the relay's o_pl
  // serve timing. The recurring levelLoadTimeOut pairs claim >1.6s loads while
  // the server-side hold is bounded at 1.0s; comparing pl_load (browser view)
  // with o_pl (server view) per request attributes the missing time to the
  // network/loader layer or the server, ending the guesswork.
  const onLevelLoaded = (
    _e: unknown,
    data: {
      stats?: { loading?: { start?: number; first?: number; end?: number } };
      deliveryDirectives?: { msn?: number; part?: number } | null;
    },
  ) => {
    const l = data?.stats?.loading;
    if (!l?.start) return;
    rec({
      ev: 'pl_load',
      t: Date.now(),
      ttfb: l.first != null ? round(l.first - l.start) : null,
      total: l.end != null ? round(l.end - l.start) : null,
      msn: data?.deliveryDirectives?.msn ?? -1,
      part: data?.deliveryDirectives?.part ?? -1,
    });
  };

  // Attribute every seek: a stall incident showed currentTime jumping BACKWARD
  // 3.2s (the felt "refresh") with no visible actor — hls.js's live-edge
  // resync among the suspects, but its warning is muted by debug:false. The
  // from->to pair plus timing names the seeker next time.
  let lastCt = 0;
  const onTimeUpdate = () => {
    lastCt = video.currentTime;
  };
  const onSeeking = () => {
    rec({ ev: 'seek', t: Date.now(), from: round(lastCt), to: round(video.currentTime) });
  };
  video.addEventListener('timeupdate', onTimeUpdate);
  video.addEventListener('seeking', onSeeking);
  teardown.push(() => {
    video.removeEventListener('timeupdate', onTimeUpdate);
    video.removeEventListener('seeking', onSeeking);
  });

  // The element's own state transitions, so a start that never plays shows
  // which step it stopped at (no `seeked`, no `canplay`, a `waiting` that never
  // turns into `playing`).
  const mediaEvents = ['seeked', 'waiting', 'playing', 'canplay', 'canplaythrough', 'stalled', 'pause', 'play', 'error', 'emptied'];
  const onMediaEvent = (e: Event) => {
    rec({ ev: 'media', t: Date.now(), type: e.type, ct: round(video.currentTime), rs: video.readyState, seeking: video.seeking });
  };
  for (const name of mediaEvents) video.addEventListener(name, onMediaEvent);
  teardown.push(() => {
    for (const name of mediaEvents) video.removeEventListener(name, onMediaEvent);
  });

  // JS main-thread stall detector. A "long task" is any single block of the
  // main thread over 50ms; we only record meaningful ones (>150ms) as `jsblock`
  // events. This is the frontend counterpart of the backend `rt_stall` heartbeat
  // — together they attribute any freeze to the JS thread, the Rust runtime, or
  // both, on one timeline. (A 1.6s tick gap was seen in a past capture, so the
  // JS side demonstrably does freeze.)
  try {
    const obs = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (entry.duration > 150) {
          rec({ ev: 'jsblock', t: Date.now(), ms: Math.round(entry.duration) });
        }
      }
    });
    obs.observe({ entryTypes: ['longtask'] });
    teardown.push(() => obs.disconnect());
  } catch {
    // longtask entry type unsupported in this engine
  }

  hls.on(Hls.Events.BUFFER_APPENDED, onAppended as never);
  hls.on(Hls.Events.FRAG_BUFFERED, onFragBuffered as never);
  hls.on(Hls.Events.ERROR, onError as never);
  hls.on(Hls.Events.LEVEL_LOADED, onLevelLoaded as never);
  teardown.push(() => hls.off(Hls.Events.BUFFER_APPENDED, onAppended as never));
  teardown.push(() => hls.off(Hls.Events.FRAG_BUFFERED, onFragBuffered as never));
  teardown.push(() => hls.off(Hls.Events.ERROR, onError as never));
  teardown.push(() => hls.off(Hls.Events.LEVEL_LOADED, onLevelLoaded as never));

  tickTimer = setInterval(() => {
    if (!active) return;
    // Always read the element hls.js is CURRENTLY attached to (`hls.media`), not a
    // captured `video` ref that can go stale across channel switches and then read
    // currentTime=0 while a different element is actually playing. Fall back to the
    // passed video only if hls.media is gone.
    const v = ((h.media as HTMLVideoElement | null) ?? video);
    let q: VideoPlaybackQuality | null = null;
    try {
      q = v.getVideoPlaybackQuality ? v.getVideoPlaybackQuality() : null;
    } catch {
      q = null;
    }
    const mediaEnd = endOf(v.buffered);
    const ct = +v.currentTime.toFixed(3);
    let pdt: number | null = null;
    try {
      const d = h.playingDate as Date | null | undefined;
      pdt = d ? d.getTime() : null;
    } catch {
      pdt = null;
    }
    rec({
      ev: 'tick',
      t: Date.now(),
      ct,
      rate: v.playbackRate,
      // Why a playhead inside the buffer does not move: a seek that never
      // completed, a readyState stuck below HAVE_FUTURE_DATA, or a media error.
      rs: v.readyState,
      seeking: v.seeking,
      mErr: v.error?.code ?? null,
      paused: v.paused,
      mediaEnd,
      fwd: mediaEnd != null ? +(mediaEnd - ct).toFixed(3) : null, // forward buffer
      ranges: rangesToArr(v.buffered),
      // per-track ends carried from the last appends — the audio-vs-video answer
      vEnd: lastTrackEnds.video ?? null,
      aEnd: lastTrackEnds.audio ?? null,
      avEnd: lastTrackEnds.audiovideo ?? null,
      avGap:
        lastTrackEnds.audio != null && lastTrackEnds.video != null
          ? +(lastTrackEnds.audio - lastTrackEnds.video).toFixed(3) // >0 ⇒ video trails audio
          : null,
      lat: numOrNull(h.latency),
      sync: numOrNull(h.liveSyncPosition),
      drift: numOrNull(h.drift),
      pdt,
      wallLat: pdt != null ? +((Date.now() - pdt) / 1000).toFixed(3) : null, // true behind-broadcast
      dropped: q?.droppedVideoFrames ?? null,
      totalFrames: q?.totalVideoFrames ?? null,
    });
  }, 1000);

  flushTimer = setInterval(() => {
    void flush();
  }, 2000);
}

/**
 * Record a free-form note into the active capture (no-op when not recording).
 * Used to mirror hls.js's internal warn-level log lines: every remaining
 * unattributed behavior (the backward seeker in particular) announces itself
 * through hls.js's logger, which `debug: false` mutes.
 */
export function llDiagNote(msg: string): void {
  if (!active) return;
  rec({ ev: 'note', t: Date.now(), msg: msg.slice(0, 500) });
}

export function stopLLDiagnostics(): void {
  active = false;
  if (tickTimer) {
    clearInterval(tickTimer);
    tickTimer = null;
  }
  if (flushTimer) {
    clearInterval(flushTimer);
    flushTimer = null;
  }
  for (const fn of teardown) {
    try {
      fn();
    } catch {
      /* ignore */
    }
  }
  teardown = [];
  if (pending.length) {
    void flush();
  }
  // Path-scoped: tearing down a recorder that was already superseded by a newer
  // session (another window) must not end THAT session.
  if (sessionPath) {
    const path = sessionPath;
    sessionPath = '';
    void invoke('stop_ll_diag', { path }).catch(() => {
      /* ignore */
    });
  }
}

function round(n: number | null | undefined): number | null {
  return typeof n === 'number' && Number.isFinite(n) ? +n.toFixed(3) : null;
}

function numOrNull(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? +v.toFixed(3) : null;
}
