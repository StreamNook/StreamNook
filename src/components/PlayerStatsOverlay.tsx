import { useEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import type Hls from 'hls.js';
import { Activity, Radio, X } from 'lucide-react';
import { LL_DISPLAY_CALIBRATION } from '../utils/latency';

// Live playback telemetry overlay (the "behind live" + FPS readout). Reads hls.js
// and the <video> element directly each second while open, so it costs nothing
// when collapsed. "Behind live" is the playhead's distance from the live edge. On the
// LL-HLS-origin path it's `hls.latency` (the gap to the newest real part the origin
// serves — honest because only parts with bytes are listed, no phantom-future edge).
// PROGRAM-DATE-TIME is deliberately NOT the primary source: Twitch's PDT base is not
// reliable wall-clock and varies per stream, so `now - PDT` could read ~0 (clamped) or
// wildly high. PDT/edge remain fallbacks for non-LL streams without a usable latency.

interface AdSourceLike {
  mode?: string;
  entitled?: boolean;
  region?: string | null;
}

interface Metrics {
  latency: number | null;
  /** Configured live-sync cushion (seconds behind the frontier the path is DESIGNED to ride). */
  syncTarget: number | null;
  resolution: string | null;
  fps: number | null;
  bitrateMbps: number | null;
  bandwidthMbps: number | null;
  bufferSec: number | null;
  dropped: number;
  droppedPct: number | null;
  /** Whether the STREAMER has low latency enabled on their end (Twitch sends
   *  PREFETCH hints). null = unknown/non-live. Lets the user tell "I'm 6s behind
   *  because the streamer disabled LL" from "because my app did". */
  sourceLowLatency: boolean | null;
  /** Current playback speed (1.0 = real time; >1 = catching up to live). */
  playbackRate: number | null;
}

const EMPTY: Metrics = {
  latency: null,
  syncTarget: null,
  resolution: null,
  fps: null,
  bitrateMbps: null,
  bandwidthMbps: null,
  bufferSec: null,
  dropped: 0,
  droppedPct: null,
  sourceLowLatency: null,
  playbackRate: null,
};

function readMetrics(hls: Hls | null, video: HTMLVideoElement | null): Metrics {
  const m: Metrics = { ...EMPTY };

  if (video) {
    try {
      const b = video.buffered;
      if (b.length > 0) m.bufferSec = Math.max(0, b.end(b.length - 1) - video.currentTime);
    } catch {
      /* buffered can throw if the element is mid-teardown */
    }
    if (typeof video.playbackRate === 'number') m.playbackRate = video.playbackRate;
    try {
      const q = video.getVideoPlaybackQuality?.();
      if (q) {
        m.dropped = q.droppedVideoFrames;
        if (q.totalVideoFrames > 0) {
          m.droppedPct = (q.droppedVideoFrames / q.totalVideoFrames) * 100;
        }
      }
    } catch {
      /* not all engines expose playback quality */
    }
  }

  if (hls) {
    // Streamer-side low latency: the player stamps the delivery path at
    // construction. 'll'/'promotion' mean Twitch sent PREFETCH hints (streamer
    // LL on); 'plain' means they didn't (streamer LL off).
    const srcHint = (hls as unknown as { __snPathHint?: string }).__snPathHint;
    if (srcHint === 'll' || srcHint === 'promotion') m.sourceLowLatency = true;
    else if (srcHint === 'plain') m.sourceLowLatency = false;

    const lvlIndex = hls.currentLevel >= 0 ? hls.currentLevel : hls.loadLevel;
    const lvl = lvlIndex >= 0 ? hls.levels?.[lvlIndex] : undefined;
    if (lvl) {
      if (lvl.height) m.resolution = `${lvl.height}p`;
      if (lvl.bitrate) m.bitrateMbps = lvl.bitrate / 1_000_000;
      const fr = (lvl.attrs as Record<string, string> | undefined)?.['FRAME-RATE'];
      const frn = fr ? parseFloat(fr) : NaN;
      if (Number.isFinite(frn)) m.fps = Math.round(frn);
    }
    if (typeof hls.bandwidthEstimate === 'number' && hls.bandwidthEstimate > 0) {
      m.bandwidthMbps = hls.bandwidthEstimate / 1_000_000;
    }
    // "Behind live" = distance from the playhead to the live edge.
    //
    // On the parts-based LL path ('ll'), `hls.latency` is honest (the origin lists only
    // real parts, no phantom-future edge), but it measures playhead-to-edge, which sits
    // ~1s above the glass-to-glass "latency to broadcaster" figure Twitch reports. We
    // subtract a fixed calibration so the displayed number is comparable to Twitch's
    // (the real ride is ~1s more; the governor drives the true value to ~3.5s). The
    // stable path shows hls.latency directly, with PDT then the playlist edge as
    // fallbacks when hls.latency is unavailable.
    let lat: number | null = null;
    const hlsLat = typeof hls.latency === 'number' && hls.latency > 0 ? hls.latency : null;
    const playingDate = hls.playingDate;
    const pdtLat = playingDate ? (Date.now() - playingDate.getTime()) / 1000 : null;
    const pathHint = (hls as unknown as { __snPathHint?: string }).__snPathHint;
    if (pathHint === 'll' && hlsLat != null) {
      lat = Math.max(0, hlsLat - LL_DISPLAY_CALIBRATION);
    } else if (hlsLat != null) {
      lat = hlsLat;
    } else if (pdtLat != null && pdtLat > 0.2 && pdtLat < 60) {
      lat = pdtLat;
    } else if (lvl?.details && video) {
      const edge = lvl.details.edge;
      if (Number.isFinite(edge)) lat = Math.max(0, edge - video.currentTime);
    }
    m.latency = lat;
    const sync = hls.config.liveSyncDuration;
    if (typeof sync === 'number' && Number.isFinite(sync)) m.syncTarget = sync;
  }

  return m;
}

// Thresholds are RELATIVE to the path's configured cushion: every mode rides
// ~syncTarget behind the frontier plus ~2s of encode+CDN pipeline by design,
// so a healthy 6s-cushion channel sitting at 7.5s must read green, while an
// LL channel at the same 7.5s is genuinely drifting. Absolute thresholds made
// healthy non-LL channels permanently amber.
function latencyClass(latency: number | null, syncTarget: number | null): string {
  if (latency == null) return 'text-textPrimary';
  const target = syncTarget ?? 4;
  if (latency <= target + 3) return 'text-emerald-400';
  if (latency <= target + 6) return 'text-amber-400';
  return 'text-red-400';
}

function Row({ label, value, valueClass }: { label: string; value: string; valueClass?: string }) {
  return (
    <div className="flex items-center justify-between gap-4">
      <span className="text-textSecondary">{label}</span>
      <span className={`font-medium tabular-nums ${valueClass ?? 'text-textPrimary'}`}>{value}</span>
    </div>
  );
}

interface Props {
  hlsRef: RefObject<Hls | null>;
  videoRef: RefObject<HTMLVideoElement | null>;
  /** Whether the panel is open. Toggled from the Plyr settings menu's "Stats" item. */
  open: boolean;
  onToggle: () => void;
  onGoLive: () => void;
  adSource?: AdSourceLike | null;
}

// Persisted drag position, stored as the panel's top-left offset (px) inside the
// player container. null means "never moved" so the default bottom-left anchor wins.
const POS_KEY = 'sn-stats-overlay-pos';

interface Pos {
  x: number;
  y: number;
}

function loadPos(): Pos | null {
  try {
    const raw = localStorage.getItem(POS_KEY);
    if (!raw) return null;
    const p = JSON.parse(raw) as Partial<Pos>;
    if (typeof p?.x === 'number' && typeof p?.y === 'number') return { x: p.x, y: p.y };
  } catch {
    /* corrupt value falls back to the default anchor */
  }
  return null;
}

const PlayerStatsOverlay = ({ hlsRef, videoRef, open, onToggle, onGoLive, adSource }: Props) => {
  const [metrics, setMetrics] = useState<Metrics>(EMPTY);
  const [pos, setPos] = useState<Pos | null>(loadPos);

  const panelRef = useRef<HTMLDivElement>(null);
  const posRef = useRef<Pos | null>(pos);
  // Drag origin: the panel's top-left at grab time plus the pointer's start point,
  // so movement is applied as a delta rather than snapping the corner to the cursor.
  const dragRef = useRef<{ startX: number; startY: number; originX: number; originY: number } | null>(null);

  const clampToParent = (x: number, y: number): Pos => {
    const panel = panelRef.current;
    const parent = panel?.offsetParent as HTMLElement | null;
    if (!panel || !parent) return { x, y };
    const maxX = Math.max(0, parent.clientWidth - panel.offsetWidth);
    const maxY = Math.max(0, parent.clientHeight - panel.offsetHeight);
    return { x: Math.max(0, Math.min(x, maxX)), y: Math.max(0, Math.min(y, maxY)) };
  };

  const persistPos = (p: Pos) => {
    try {
      localStorage.setItem(POS_KEY, JSON.stringify(p));
    } catch {
      /* private mode / quota: position just won't persist */
    }
  };

  const commitPos = (p: Pos) => {
    posRef.current = p;
    setPos(p);
  };

  const onDragStart = (e: React.PointerEvent) => {
    const panel = panelRef.current;
    if (!panel) return;
    const parent = panel.offsetParent as HTMLElement | null;
    const parentRect = parent?.getBoundingClientRect();
    const rect = panel.getBoundingClientRect();
    dragRef.current = {
      startX: e.clientX,
      startY: e.clientY,
      originX: parentRect ? rect.left - parentRect.left : rect.left,
      originY: parentRect ? rect.top - parentRect.top : rect.top,
    };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  };

  const onDragMove = (e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    commitPos(clampToParent(d.originX + (e.clientX - d.startX), d.originY + (e.clientY - d.startY)));
  };

  const onDragEnd = (e: React.PointerEvent) => {
    if (!dragRef.current) return;
    dragRef.current = null;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* capture may already be gone */
    }
    if (posRef.current) persistPos(posRef.current);
  };

  useEffect(() => {
    if (!open) return;
    const tick = () => setMetrics(readMetrics(hlsRef.current, videoRef.current));
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [open, hlsRef, videoRef]);

  // Pull the panel back inside the player whenever it opens or the container
  // resizes (fullscreen toggle, window resize), so a saved spot can't strand it offscreen.
  useEffect(() => {
    if (!open || !posRef.current) return;
    const reclamp = () => {
      if (!posRef.current) return;
      const clamped = clampToParent(posRef.current.x, posRef.current.y);
      commitPos(clamped);
      persistPos(clamped);
    };
    reclamp();
    window.addEventListener('resize', reclamp);
    return () => window.removeEventListener('resize', reclamp);
  }, [open]);

  // Opened from the Plyr settings menu ("Stats"); closed via the panel's X.
  if (!open) return null;

  const sourceLabel = adSource
    ? adSource.entitled
      ? adSource.mode === 'turbo'
        ? 'Turbo (direct)'
        : 'Sub (direct)'
      : adSource.mode === 'auth-only'
        ? 'Direct (ads)'
        : `Plugin${adSource.region ? ` ${adSource.region}` : ''}`
    : null;

  // Go Live is for genuine drift (a scrub-back or latency creep), not the path's
  // natural floor: every mode rides ~syncTarget behind the download frontier plus
  // ~2s of encode+CDN pipeline it can never recover, so "behind live" cannot go
  // below roughly syncTarget + 2 by design. Only offer the button well past that.
  const goLiveFloor = (metrics.syncTarget ?? 4) + 4;
  const showGoLive = metrics.latency != null && metrics.latency > goLiveFloor;

  return (
    <div
      ref={panelRef}
      // Opt this panel out of the player's wheel/middle-click volume handling —
      // it's a draggable HUD, not part of the video surface.
      data-no-wheel-volume
      className={`absolute z-50 w-56 pointer-events-auto ${pos ? '' : 'bottom-16 left-4'}`}
      style={pos ? { left: pos.x, top: pos.y } : undefined}
    >
      <div className="stats-hud px-3.5 py-3 text-xs text-textPrimary">
        <div className="flex items-center justify-between mb-3">
          <div
            onPointerDown={onDragStart}
            onPointerMove={onDragMove}
            onPointerUp={onDragEnd}
            onPointerCancel={onDragEnd}
            className="flex items-center gap-1.5 flex-1 cursor-grab active:cursor-grabbing select-none touch-none"
          >
            <Activity size={13} className="text-accent" />
            <span className="text-textPrimary font-semibold tracking-wide">Stream Stats</span>
          </div>
          <button onClick={onToggle} aria-label="Close stats" className="text-textSecondary hover:text-textPrimary transition-colors">
            <X size={13} />
          </button>
        </div>

        {/* Behind-live is the headline metric, so it leads as a large readout
            instead of one more row in the list. */}
        <div className="flex items-end justify-between mb-3">
          <span className="text-[10px] uppercase tracking-wider text-textSecondary">Behind live</span>
          <span className={`text-2xl font-semibold tabular-nums leading-none ${latencyClass(metrics.latency, metrics.syncTarget)}`}>
            {metrics.latency != null ? metrics.latency.toFixed(1) : '-'}
            {metrics.latency != null && <span className="text-sm font-medium text-textSecondary ml-0.5">s</span>}
          </span>
        </div>

        <div className="space-y-1.5">
          <Row
            label="Speed"
            value={metrics.playbackRate != null ? `${metrics.playbackRate.toFixed(2)}x` : '-'}
            valueClass={
              metrics.playbackRate != null && Math.abs(metrics.playbackRate - 1) > 0.005
                ? 'text-amber-400'
                : undefined
            }
          />
          {metrics.sourceLowLatency != null && (
            <Row
              label="Source low latency"
              value={metrics.sourceLowLatency ? 'On (streamer)' : 'Off (streamer)'}
              valueClass={metrics.sourceLowLatency ? 'text-emerald-400' : 'text-textSecondary'}
            />
          )}
          <Row
            label="Resolution"
            value={metrics.resolution ? `${metrics.resolution}${metrics.fps ? metrics.fps : ''}` : '-'}
          />
          <Row label="Dropped" value={`${metrics.dropped}${metrics.droppedPct != null ? ` (${metrics.droppedPct.toFixed(2)}%)` : ''}`} />
          <Row label="Video bitrate" value={metrics.bitrateMbps != null ? `${metrics.bitrateMbps.toFixed(1)} Mbps` : '-'} />
          <Row label="Bandwidth" value={metrics.bandwidthMbps != null ? `${metrics.bandwidthMbps.toFixed(1)} Mbps` : '-'} />
          <Row label="Buffer" value={metrics.bufferSec != null ? `${metrics.bufferSec.toFixed(1)}s` : '-'} />
          {sourceLabel && <Row label="Source" value={sourceLabel} />}
        </div>

        {showGoLive && (
          <button
            onClick={onGoLive}
            className="mt-3 w-full flex items-center justify-center gap-1.5 py-1.5 rounded-md glass-button text-white font-medium hover:bg-white/10 transition-colors"
          >
            <Radio size={13} className="text-red-400" />
            Go Live
          </button>
        )}
      </div>
    </div>
  );
};

export default PlayerStatsOverlay;
