// A VOD's controls on the phone, the iPhone's `VodSeekBar` and transport: the
// seek bar over the bottom of the picture (what has played in the accent, the
// parts Twitch muted striped, a tick where the category changed, the knob, and
// a storyboard frame while scrubbing), then back and forward ten seconds and
// the speed. Play and pause stay the center button's, as on a live stream.
import React, { useEffect, useRef, useState } from 'react';
import { ArrowClockwise, ArrowCounterClockwise, Check } from 'phosphor-react';
import type { VodStartInfo } from '../../types';
import { MobileSheet } from '../ui/MobileSheet';
import { VOD_RATES, clock, rateLabel, seekAt, storyboardFrame } from '../watch/channelMedia';

/** The scrub preview's width. */
const PREVIEW = 128;

/** The playhead and the length, following the element. The length is the
 *  element's once it knows one, else the start's (a recording VOD runs long). */
export function useVodClock(videoRef: React.RefObject<HTMLVideoElement | null>, vod: VodStartInfo | null) {
  const [position, setPosition] = useState(0);
  const [length, setLength] = useState(vod?.length_seconds ?? 0);
  const playing = vod?.video_id ?? null;
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !playing) return;
    const read = () => {
      if (Number.isFinite(video.currentTime)) setPosition(video.currentTime);
      if (Number.isFinite(video.duration) && video.duration > 0) setLength(video.duration);
    };
    read();
    const events = ['timeupdate', 'durationchange', 'loadedmetadata', 'seeked'] as const;
    for (const e of events) video.addEventListener(e, read);
    return () => {
      for (const e of events) video.removeEventListener(e, read);
    };
  }, [videoRef, playing]);
  return { position, length: length > 0 ? length : (vod?.length_seconds ?? 0) };
}

export const VodSeekBar: React.FC<{
  vod: VodStartInfo;
  position: number;
  length: number;
  onSeek: (seconds: number) => void;
  onTouch: () => void;
}> = ({ vod, position, length, onSeek, onTouch }) => {
  const [scrub, setScrub] = useState<number | null>(null);
  const [barWidth, setBarWidth] = useState(0);
  const barRef = useRef<HTMLDivElement | null>(null);
  const duration = Math.max(length, 1);
  const shown = scrub != null ? scrub * duration : Math.min(position, duration);
  const fraction = shown / duration;
  const pct = (f: number) => `${Math.min(100, Math.max(0, f * 100))}%`;

  const fractionAt = (x: number) => {
    const box = barRef.current?.getBoundingClientRect();
    return box ? seekAt(x, box.left, box.width, 1) : 0;
  };
  const frame = scrub != null ? storyboardFrame(vod.storyboard, scrub * duration) : null;
  const frameScale = frame ? PREVIEW / Math.max(frame.width, 1) : 1;

  return (
    <div
      className="w-full"
      onClick={(e) => e.stopPropagation()}
      role="slider"
      aria-label="Position"
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Math.round(shown)}
      aria-valuetext={`${clock(shown)} of ${clock(duration)}`}
    >
      <div
        ref={barRef}
        className="relative h-7 touch-none"
        onPointerDown={(e) => {
          e.stopPropagation();
          (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
          setBarWidth(e.currentTarget.clientWidth);
          setScrub(fractionAt(e.clientX));
          onTouch();
        }}
        onPointerMove={(e) => {
          if (scrub == null) return;
          setScrub(fractionAt(e.clientX));
          onTouch();
        }}
        onPointerUp={() => {
          if (scrub != null) onSeek(scrub * duration);
          setScrub(null);
          onTouch();
        }}
        onPointerCancel={() => setScrub(null)}
      >
        <div className="absolute inset-x-0 top-1/2 h-1 -translate-y-1/2 rounded-full bg-white/25" />
        <div className="absolute left-0 top-1/2 h-1 -translate-y-1/2 rounded-full bg-accent" style={{ width: pct(fraction) }} />
        {/* Muted stretches, striped so they read as "no sound here" rather than as more progress. */}
        {(vod.muted_segments ?? []).map((r) => (
          <div
            key={`${r.start_secs}-${r.end_secs}`}
            className="pointer-events-none absolute top-1/2 h-1 -translate-y-1/2"
            style={{
              left: pct(r.start_secs / duration),
              width: pct((Math.min(r.end_secs, duration) - r.start_secs) / duration),
              background: 'repeating-linear-gradient(90deg, rgba(255,255,255,0.6) 0 2px, transparent 2px 4px)',
            }}
          />
        ))}
        {/* Where each chapter after the first begins. */}
        {(vod.chapters ?? [])
          .filter((c) => c.start_secs > 1 && c.start_secs < duration)
          .map((c) => (
            <div
              key={c.start_secs}
              className="pointer-events-none absolute top-1/2 h-2.5 w-0.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white/85"
              style={{ left: pct(c.start_secs / duration) }}
            />
          ))}
        <div
          className={`pointer-events-none absolute top-1/2 h-3.5 w-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white transition-transform duration-[120ms] ease-out ${
            scrub != null ? 'scale-[1.3]' : ''
          }`}
          style={{ left: pct(fraction) }}
        />
        {frame && scrub != null && (
          <div
            className="pointer-events-none absolute bottom-[34px] overflow-hidden rounded-lg border border-white/15 bg-black shadow-[0_4px_16px_rgba(0,0,0,0.5)]"
            style={{
              width: PREVIEW,
              height: frame.height * frameScale,
              left: Math.min(Math.max(0, barWidth * scrub - PREVIEW / 2), Math.max(0, barWidth - PREVIEW)),
              backgroundImage: `url("${frame.url}")`,
              backgroundSize: `${frame.sheetWidth * frameScale}px ${frame.sheetHeight * frameScale}px`,
              backgroundPosition: `${-frame.x * frameScale}px ${-frame.y * frameScale}px`,
            }}
          />
        )}
      </div>
      {/* The two times keep their width as they count. */}
      <div className="flex justify-between gap-2 text-[11.5px] font-medium leading-4 tabular-nums text-white/80">
        <span>{clock(shown, duration)}</span>
        <span>{clock(duration)}</span>
      </div>
    </div>
  );
};

/** Back and forward ten seconds, and the speed. */
export const VodTransport: React.FC<{
  rate: number;
  onSkip: (by: number) => void;
  onRate: (rate: number) => void;
  onTouch: () => void;
  iconSize: number;
}> = ({ rate, onSkip, onRate, onTouch, iconSize }) => {
  const [rateOpen, setRateOpen] = useState(false);
  return (
    <div className="flex shrink-0 items-center">
      <button
        onClick={(e) => {
          e.stopPropagation();
          onSkip(-10);
          onTouch();
        }}
        className="sn-bar-btn relative flex items-center justify-center text-white"
        aria-label="Back 10 seconds"
      >
        <ArrowCounterClockwise size={iconSize} />
        <span className="absolute text-[8px] font-bold leading-none" style={{ marginTop: 2 }}>
          10
        </span>
      </button>
      <button
        onClick={(e) => {
          e.stopPropagation();
          onSkip(10);
          onTouch();
        }}
        className="sn-bar-btn relative flex items-center justify-center text-white"
        aria-label="Forward 10 seconds"
      >
        <ArrowClockwise size={iconSize} />
        <span className="absolute text-[8px] font-bold leading-none" style={{ marginTop: 2 }}>
          10
        </span>
      </button>
      <button
        onClick={(e) => {
          e.stopPropagation();
          setRateOpen(true);
          onTouch();
        }}
        className="sn-bar-btn flex items-center justify-center text-white"
        aria-label={`Speed ${rateLabel(rate)}`}
      >
        <span className="rounded-[5px] border-[1.5px] border-white px-[5px] py-px text-[12px] font-bold leading-4 tabular-nums">
          {rateLabel(rate)}
        </span>
      </button>
      <MobileSheet open={rateOpen} onClose={() => setRateOpen(false)} title="Speed">
        <div className="flex flex-col" onClick={(e) => e.stopPropagation()}>
          {VOD_RATES.map((r) => (
            <button
              key={r}
              type="button"
              onClick={() => {
                onRate(r);
                setRateOpen(false);
              }}
              className="flex min-h-12 items-center justify-between text-[15px] text-textPrimary"
            >
              {rateLabel(r)}
              {r === rate && <Check size={16} weight="bold" className="text-accent" />}
            </button>
          ))}
        </div>
      </MobileSheet>
    </div>
  );
};
