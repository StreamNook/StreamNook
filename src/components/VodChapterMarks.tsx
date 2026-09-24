import { useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import type { VodChapter } from '../types';
import { formatVodTime } from '../utils/vodProgress';

/**
 * Chapter boundaries on a VOD's Plyr progress bar, plus the name of the
 * chapter under the pointer.
 *
 * Portaled into Plyr's `.plyr__progress` slot like VodMutedMarks, WITHOUT
 * the `.sn-timeline-host` class, so Plyr's own range stays seekable. The
 * gaps take no pointer events at all.
 *
 * Where the name shows depends on whether the VOD has storyboard previews:
 *
 * - with a preview, Plyr draws a thumb it already positions under the
 *   pointer and updates its time by writing one `<span>`, so the name is a
 *   sibling span inside that thumb and the only state is the title (a move
 *   inside one chapter commits nothing);
 * - without one, Plyr's seek tooltip is rewritten wholesale on every move,
 *   so it is hidden (host class, see globals.css) and this draws time plus
 *   chapter itself, following the pointer.
 *
 * Pure presentation: Rust sorts, merges and clamps the chapters.
 */

interface Hover {
  frac: number;
  secs: number;
  title: string;
}

export default function VodChapterMarks({
  host,
  chapters,
  videoRef,
  fallbackLengthSecs,
  hasPreview,
}: {
  /** Plyr's `.plyr__progress` element. */
  host: HTMLElement | null;
  chapters: VodChapter[];
  videoRef: RefObject<HTMLVideoElement | null>;
  /** `length_seconds` from Rust, used only until the element reports one. */
  fallbackLengthSecs: number;
  /** Plyr was built with storyboard previews for this VOD. */
  hasPreview: boolean;
}) {
  // The denominator is the element's own duration (the bar's space), with
  // GQL's integer length only until the element reports one.
  const [duration, setDuration] = useState(0);
  const [hover, setHover] = useState<Hover | null>(null);
  // Plyr's preview thumb time container, keyed by the host it was found
  // under so a rebuilt bar never portals into the previous bar's detached
  // node.
  const [thumbTime, setThumbTime] = useState<{ host: HTMLElement; el: HTMLElement } | null>(null);
  // The host's box for the current hover session: read once on entry, never
  // per move.
  const rectRef = useRef<DOMRect | null>(null);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const read = () =>
      setDuration(Number.isFinite(video.duration) && video.duration > 0 ? video.duration : 0);
    read();
    video.addEventListener('loadedmetadata', read);
    video.addEventListener('durationchange', read);
    return () => {
      video.removeEventListener('loadedmetadata', read);
      video.removeEventListener('durationchange', read);
    };
  }, [videoRef]);

  // Plyr renders its preview thumb a microtask after construction; look for
  // its time container briefly, the same retry the control-bar injections use.
  useEffect(() => {
    if (!host || !hasPreview) return;
    let attempts = 0;
    let cancelled = false;
    const find = () => {
      if (cancelled) return;
      const el = host.querySelector<HTMLElement>('.plyr__preview-thumb__time-container');
      if (el) setThumbTime({ host, el });
      else if (attempts++ < 25) setTimeout(find, 200);
    };
    find();
    return () => {
      cancelled = true;
    };
  }, [host, hasPreview]);

  // Without a preview, this component owns the hover tooltip.
  useEffect(() => {
    if (!host || hasPreview || chapters.length < 2) return;
    host.classList.add('sn-chapter-host');
    return () => host.classList.remove('sn-chapter-host');
  }, [host, hasPreview, chapters.length]);

  // Hover tracking on the host: its content box is the true 0-100% playhead
  // space (see the .sn-muted-marks note in globals.css).
  useEffect(() => {
    if (!host || chapters.length < 2) return;
    const video = videoRef.current;
    const onEnter = () => {
      rectRef.current = host.getBoundingClientRect();
    };
    const onMove = (e: PointerEvent) => {
      const r = rectRef.current ?? (rectRef.current = host.getBoundingClientRect());
      if (r.width <= 0) return;
      const frac = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
      const d = video && Number.isFinite(video.duration) && video.duration > 0 ? video.duration : fallbackLengthSecs;
      const secs = frac * d;
      const ch = chapters.find((c) => secs >= c.start_secs && secs < c.end_secs) ?? chapters[chapters.length - 1];
      setHover((prev) => {
        if (prev && prev.title === ch.title && (hasPreview || Math.abs(prev.frac - frac) < 0.0005)) return prev;
        return { frac, secs, title: ch.title };
      });
    };
    const onLeave = () => {
      rectRef.current = null;
      setHover(null);
    };
    host.addEventListener('pointerenter', onEnter);
    host.addEventListener('pointerdown', onMove);
    host.addEventListener('pointermove', onMove);
    host.addEventListener('pointerleave', onLeave);
    return () => {
      host.removeEventListener('pointerenter', onEnter);
      host.removeEventListener('pointerdown', onMove);
      host.removeEventListener('pointermove', onMove);
      host.removeEventListener('pointerleave', onLeave);
    };
  }, [host, chapters, videoRef, fallbackLengthSecs, hasPreview]);

  const lengthSecs = duration || fallbackLengthSecs;

  // The gap list only changes with the chapters or the bar's length, so a
  // per-move hover render diffs nothing here.
  const gaps = useMemo(() => {
    if (lengthSecs <= 0) return null;
    return chapters.slice(1).map((c) => {
      const left = (c.start_secs / lengthSecs) * 100;
      if (!(left > 0 && left < 100)) return null;
      return <div key={c.start_secs} className="sn-chapter-gap" style={{ left: `${left}%` }} />;
    });
  }, [chapters, lengthSecs]);

  if (!host || lengthSecs <= 0 || chapters.length < 2) return null;

  const thumbTimeEl = thumbTime && thumbTime.host === host ? thumbTime.el : null;

  return (
    <>
      {createPortal(
        <div className="sn-chapter-marks" aria-hidden="true">
          {gaps}
        </div>,
        host,
      )}
      {hover && hasPreview && thumbTimeEl && createPortal(<span className="sn-chapter-tip--thumb">{hover.title}</span>, thumbTimeEl)}
      {hover && !hasPreview &&
        createPortal(
          <div className="sn-chapter-tip" style={{ left: `${hover.frac * 100}%` }}>
            {formatVodTime(hover.secs)}
            <span className="sn-chapter-tip__title">{hover.title}</span>
          </div>,
          host,
        )}
    </>
  );
}
