import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { isWindowHidden, onWindowVisibility } from './windowVisibility';
import { Logger } from './logger';
import {
  ATTACK_MS,
  CALM_ATTACK_MS,
  CALM_RELEASE_MS,
  cssColour,
  follow,
  frameColours,
  RELEASE_MS,
  SEGMENTS,
  VH,
  VW,
  type Rgb,
} from './mediaGlowColor';

/** What Rust returns for one sampled thumbnail: the single colour that stream
 *  reads as, or null where the image had no colour worth borrowing. Edge
 *  colours are not asked for here — the light that needs them computes its own
 *  from the live frame (see `./mediaGlowColor`). */
export interface Glow {
  overall: string | null;
}

/** A thumbnail is scaled to this before it is read. 16x9 is 144 pixels —
 *  enough to tell a stream's colour, small enough that the readback is
 *  measured in microseconds. Going bigger buys nothing: the answer is one
 *  colour either way. */
const W = 16;
const H = 9;

/** Hosts worth ATTEMPTING a CORS read from, for the colour only.
 *
 *  A canvas cannot be read from a cross-origin image unless that image was
 *  requested with CORS, so there is no way to sample a thumbnail without asking
 *  for it. What there IS a choice about is which image carries the risk.
 *
 *  This list used to be applied to the visible `<img>`, and that was the bug: a
 *  host that does not answer with the header makes the request fail, `onError`
 *  fires, and the card shows its placeholder instead of the picture. The
 *  thumbnail is the content; a colour is decoration, and decoration does not
 *  get to break content.
 *
 *  It now only decides whether a SEPARATE, undisplayed image is worth fetching.
 *  A host that is not listed, or one that is listed and fails anyway, costs
 *  nothing but the tint.
 */
const CORS_SAFE_THUMB_HOSTS = ['static-cdn.jtvnw.net', 'i.ytimg.com'];

function canTryCors(url: string): boolean {
  try {
    return CORS_SAFE_THUMB_HOSTS.includes(new URL(url).hostname);
  } catch {
    return false;
  }
}

/** One canvas for every thumbnail ever sampled.
 *
 *  It used to be one per call, which on an infinite-scrolling grid is a canvas
 *  plus its backing store per card. Safe to share because everything that
 *  touches it happens synchronously between the decode and the invoke, so two
 *  samples can never interleave on it. */
let thumbCanvas: HTMLCanvasElement | null = null;

/**
 * Tint a card from its thumbnail, using an image of our own.
 *
 * Deliberately NOT the `<img>` the card displays. Reading pixels needs a CORS
 * request, and a CORS request can fail; when that happens on the displayed
 * element the card loses its picture. Fetching a second, undisplayed copy means
 * the worst case is a card with no tint.
 *
 * The extra request is the price of that, and it is a small one: the same URL
 * the browser just fetched, from a CDN, once per card per session (the caller
 * marks the card), scheduled at idle so it never competes with the thumbnails
 * someone is actually waiting for.
 *
 * Silently does nothing on any failure, and the card keeps the theme colour.
 */
export async function sampleImageGlow(url: string, target: HTMLElement): Promise<void> {
  if (!canTryCors(url)) return;
  try {
    const img = new Image();
    img.crossOrigin = 'anonymous';
    img.decoding = 'async';
    img.src = url;
    await img.decode();
    if (!img.naturalWidth) return;
    if (!thumbCanvas) {
      thumbCanvas = document.createElement('canvas');
      thumbCanvas.width = W;
      thumbCanvas.height = H;
    }
    const canvas = thumbCanvas;
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    if (!ctx) return;
    ctx.drawImage(img, 0, 0, W, H);
    const bytes = ctx.getImageData(0, 0, W, H).data;
    let bin = '';
    for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    const glow = await invoke<Glow | null>('submit_media_frame', {
      key: url,
      rgbaB64: btoa(bin),
      width: W,
      height: H,
    });
    if (glow?.overall) target.style.setProperty('--stream-glow', glow.overall);
  } catch {
    // Tainted canvas, or the element went away mid-sample. Either way the card
    // keeps the theme colour and nothing is broken.
  }
}

/**
 * Props to spread onto any card thumbnail so its card takes the image's colour.
 *
 * The image finds its own card with `closest('.media-card')` rather than being
 * handed a ref, because the live-stream cards in Home are hand-rolled markup
 * several levels deep — but they already carry `.media-card`, so there is
 * nothing to plumb. That is also why this exists at all: the shared MediaCard
 * covers Continue Watching, clips and VODs, and nothing else. The Following and
 * Browse grids never touch it.
 *
 * Sampled at most once per card, and the marker is what makes that true.
 *
 * A bare `ref` callback re-runs on every render, so the guard has to catch
 * every case — and "does this card already have a colour" does not. A genuinely
 * achromatic thumbnail correctly comes back with NO colour, sets no property,
 * and so passed that guard on every subsequent render: one IPC round trip per
 * grey card per render, for as long as the grid was on screen. The Home grids
 * re-render whenever viewer counts update, so that was a steady background
 * drip, invisible because each individual call worked exactly as designed.
 *
 * Marking the element rather than remembering URLs in a module-level Set keeps
 * it bounded for free: the mark dies with the card, and a card that comes back
 * pays one sample, which is what Rust's cache is for. Set BEFORE the await, so
 * two renders in the same tick cannot both fire.
 */
export function glowThumbProps(url: string | undefined) {
  const run = (el: HTMLImageElement | null) => {
    if (!el || !url || !el.complete || !el.naturalWidth) return;
    const card = el.closest<HTMLElement>('.media-card');
    if (!card || card.dataset.snGlow === url) return;
    card.dataset.snGlow = url;
    // Idle, so the second fetch never competes with thumbnails someone is
    // waiting on. The timeout stops it being starved forever on a busy page: a
    // colour that arrives late is fine, one that never arrives is not.
    const go = () => void sampleImageGlow(url, card);
    if (typeof requestIdleCallback === 'function') requestIdleCallback(go, { timeout: 4000 });
    else setTimeout(go, 250);
  };
  // No `crossOrigin`, and its absence is the point: these props go onto the
  // image a user actually sees, and nothing about a tint may put that at risk.
  // The sampling copy asks for CORS on its own and is allowed to fail.
  return {
    // `complete` is already true for a cached image by the time React attaches
    // onLoad, so that event alone silently misses them.
    onLoad: (e: { currentTarget: HTMLImageElement }) => run(e.currentTarget),
    ref: run,
  };
}

/* ===================== the live half: an ambilight ========================
 *
 * Everything above answers "what colour IS this stream" for a card, once, from
 * a still. Everything below answers a different question — "what is on screen
 * right now" — many times a second, and the two want opposite algorithms.
 *
 * This half does NOT go through Rust, and that is the deliberate reversal of
 * how the still half works. Three reasons, in order of weight:
 *
 *   1. Rate. A light that reads as synced to the picture has to be resampled
 *      roughly every 60ms. Through Rust that is an IPC round trip per frame
 *      per player carrying the frame itself — the bulk media copy the
 *      efficiency standard names outright, at about a hundred times the
 *      traffic the once-every-two-seconds version had.
 *   2. Latency. A round trip lands the colour a frame or more after the frame
 *      it came from, and the in-flight guard drops samples under load. Local
 *      arithmetic lands in the same frame, which is the whole ask.
 *   3. It is arithmetic over a few hundred pixels that already live in this
 *      process, in a buffer this process just allocated.
 *
 * Rust keeps the half where Rust earns it: the still colour is cached by key,
 * shared across surfaces, and survives the UI being torn down. The live colour
 * is none of those things — it is worth 240ms and then it is wrong.
 */

/** Floor on the gap between readbacks, in ms. The sampler is driven by the
 *  decoder, so this is what stops a 60fps stream being read sixty times a
 *  second: the light is not more convincing at 60Hz than at 18, and the
 *  readback is the only part of this with a real cost. */
const FAST_MS = 55;

/** Everything that is not the immersive player. A card rim or a MultiNook tile
 *  wants a colour, not a light show, and four tiles at 18Hz is thirty-odd times
 *  the readback for something nobody is looking at directly. */
const CALM_MS = 1000;

/** How often the watchdog checks that the decoder callback is still attached to
 *  the element that is actually playing. Slow on purpose: it is insurance, not
 *  a sampler, and while the callback is healthy its sample is thrown away by
 *  the elapsed-time throttle anyway. */
const WATCHDOG_MS = 2000;

/** How often to re-check whether a full-screen overlay is covering the player.
 *
 *  A selector match over the document is not something to do eighteen times a
 *  second, and it does not need to be: overlays open and close on human
 *  timescales. Short enough that the light is gone before an overlay has
 *  finished animating in, long enough to be free. */
const COVER_CHECK_MS = 150;

/** Is something full-screen sitting over the player?
 *
 *  Same criterion as the CSS rule in globals.css, and for the same reason: a
 *  `fixed inset-0` element carrying a backdrop filter re-samples everything
 *  behind it whenever that content changes, so sampling a frame to drive light
 *  underneath one is worse than useless. Kept structural rather than a list of
 *  overlay flags because those overlays live in a dozen components. */
function coveredByOverlay(): boolean {
  return !!document.querySelector('.fixed.inset-0[class*="backdrop-blur"]');
}

/** Publish it for the stylesheet.
 *
 *  An attribute rather than a `body:has(...)` rule, which is what this was
 *  first. `:has()` is live, and the selector keyed on `.backdrop-blur-md` — a
 *  class the badges overlay puts on every collected tile and every checkmark —
 *  so the engine had to consider it every time that menu opened, with immersive
 *  off and nothing playing. A feature that is switched off should cost nothing,
 *  and that did not. Set only while immersive is actually sampling, and removed
 *  on teardown. */
function publishCovered(covered: boolean) {
  if (covered) document.body.dataset.playerCovered = 'true';
  else delete document.body.dataset.playerCovered;
}

/**
 * Tint a container from whatever is playing inside it.
 *
 * Publishes `--stream-glow` (one colour for the whole frame) plus
 * `--glow-t0..t7` and `--glow-b0..b7` (the top and bottom edges, left to right)
 * onto `target`, so the CSS decides what to do with them and falls back to the
 * theme accent when there is nothing.
 *
 * `fast` is what separates the immersive strip from everything else. Off, this
 * samples once a second and drifts, which is all a card rim or a MultiNook tile
 * needs. On, it is driven by the decoder itself through
 * `requestVideoFrameCallback` — the colour is recomputed when a new frame is
 * actually presented, not on a timer that happens to run nearby. That is the
 * literal meaning of synced to the picture, and it is also why the cost scales
 * with the stream rather than with wall-clock: a 30fps stream does half the
 * work of a 60fps one, and a paused one does none.
 *
 * Costs nothing when off: no canvas is allocated and no callback is registered
 * until `enabled` is true and a stream is playing.
 */
export function useMediaGlow(
  video: React.RefObject<HTMLVideoElement | null>,
  target: React.RefObject<HTMLElement | null>,
  key: string | null,
  enabled: boolean,
  fast = false,
) {
  // Held across renders so a re-render does not throw the canvas away.
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  // The eased colours, which have to survive re-renders or the light restarts
  // from black every time React touches this component.
  const liveRef = useRef<{ overall: Rgb | null; top: (Rgb | null)[]; bottom: (Rgb | null)[] }>({
    overall: null,
    top: [],
    bottom: [],
  });
  // What was last written to the DOM, so a static scene stops writing.
  const wroteRef = useRef<string[]>([]);
  // One line in the log the first time a colour lands, so "is this working" is
  // answerable from streamnook.log instead of by eye.
  const announced = useRef(false);

  useEffect(() => {
    if (!enabled) return;
    if (!key) {
      // Enabled with nothing to key on samples nothing and says nothing, which
      // is how the main player sat dead while MultiNook worked.
      Logger.warn('[MediaGlow] enabled but no key — nothing will be sampled');
      return;
    }

    let stopped = false;
    let timer: ReturnType<typeof setInterval> | null = null;
    // Cached, because the check is a document-wide selector match.
    let covered = false;
    let coverCheckedAt = 0;
    let lastAr = '';
    let rvfc: number | null = null;
    let last = 0;
    liveRef.current = { overall: null, top: [], bottom: [] };
    wroteRef.current = [];

    const sample = (now: number) => {
      const el = video.current;
      const host = target.current;
      if (!el || !host || stopped) return;
      // Nothing to read from a stream that has not painted a frame yet, and a
      // hidden window is not being looked at. The page cannot tell it is
      // minimized on its own (see services::window_visibility).
      if (el.readyState < 2 || el.videoWidth === 0 || isWindowHidden()) return;

      // Nothing behind a full-screen overlay is worth sampling, and sampling it
      // is actively harmful: the colours would keep changing under a backdrop
      // filter that has to re-blur the window every time they do.
      if (now - coverCheckedAt >= COVER_CHECK_MS) {
        coverCheckedAt = now;
        const nowCovered = coveredByOverlay();
        if (nowCovered !== covered) {
          covered = nowCovered;
          // Only the immersive player owns the attribute. Every sampler runs
          // this check, and a MultiNook tile tearing down would otherwise clear
          // it while the main player still needs its layers held opaque.
          if (fast) publishCovered(covered);
          // Warn level, because this app logs nothing below it, and because
          // this is the line that answers "did the guard actually fire" from a
          // user's log instead of from a theory about their machine.
          Logger.warn(`[MediaGlow] ${covered ? 'covered by an overlay, pausing' : 'uncovered, resuming'}`);
        }
      }
      if (covered) return;

      wroteTo = host;

      // Motion off keeps the colour and drops the chase. Read per sample rather
      // than captured, so flipping the switch takes effect without a remount.
      const calm = !fast || document.documentElement.dataset.motion === 'off';
      const gap = calm ? CALM_MS : FAST_MS;
      if (now - last < gap) return;
      const dt = last === 0 ? gap : Math.min(now - last, 1000);
      last = now;

      // The picture's aspect, republished from the decoder itself. It used to
      // come from onLoadedMetadata, which never fires when the stream is
      // already decoding by the time the handler attaches.
      //
      // Twice, in two forms. The ratio shapes the invisible frame; the bare
      // NUMBER is what lets CSS work out the letterbox height on its own
      // (`(100cqh - min(100cqh, 100cqw / arn)) / 2`), which is what sizes the
      // light to the gap it lives in at every pane size without this ever
      // measuring a layout box.
      //
      // Guarded, because this used to run on every frame: a stream's aspect
      // changes approximately never, and writing a custom property dirties
      // style for the element and everything under it. Eighteen pointless
      // invalidations a second, for two values that had not moved.
      if (el.videoHeight > 0) {
        const ar = `${el.videoWidth} / ${el.videoHeight}`;
        if (ar !== lastAr) {
          lastAr = ar;
          host.style.setProperty('--sn-video-ar', ar);
          host.style.setProperty('--sn-video-arn', String(el.videoWidth / el.videoHeight));
        }
      }

      let canvas = canvasRef.current;
      if (!canvas) {
        canvas = document.createElement('canvas');
        canvas.width = VW;
        canvas.height = VH;
        canvasRef.current = canvas;
      }
      // `willReadFrequently` keeps Chromium's copy of this canvas on the CPU
      // side. Without it every getImageData is a GPU readback that can stall the
      // compositor, which is the one way this could make the app worse to run
      // next to a game — and at this rate it would do so every frame.
      const ctx = canvas.getContext('2d', { willReadFrequently: true });
      if (!ctx) return;

      let d: Uint8ClampedArray;
      try {
        ctx.drawImage(el, 0, 0, VW, VH);
        d = ctx.getImageData(0, 0, VW, VH).data;
      } catch {
        // A DRM-protected stream taints the canvas and drawImage throws. There
        // is no way to read that frame and never will be, so stop entirely
        // rather than throwing on every presented frame.
        stopped = true;
        if (timer) clearInterval(timer);
        Logger.debug('[MediaGlow] frame not readable, disabling for this stream');
        return;
      }

      const attack = calm ? CALM_ATTACK_MS : ATTACK_MS;
      const rise = calm ? CALM_RELEASE_MS : RELEASE_MS;
      const live = liveRef.current;
      const seen = frameColours(d);

      // A first reading lands whole. Easing up from nothing would show the
      // light fade in from black over a quarter second every time a stream
      // starts, which reads as a bug rather than as an arrival.
      const step = (prev: Rgb | null, next: Rgb) =>
        prev ? follow(prev, next, dt, attack, rise) : next;
      live.overall = step(live.overall, seen.overall);
      for (let i = 0; i < SEGMENTS; i++) {
        live.top[i] = step(live.top[i] ?? null, seen.top[i]);
        live.bottom[i] = step(live.bottom[i] ?? null, seen.bottom[i]);
      }

      // Write only what changed. Sixteen property writes in one task coalesce
      // into a single style recalculation, but a talking head against a fixed
      // backdrop settles to writing nothing at all.
      const wrote = wroteRef.current;
      const put = (slot: number, name: string, c: Rgb) => {
        const text = cssColour(c);
        if (wrote[slot] === text) return;
        wrote[slot] = text;
        host.style.setProperty(name, text);
      };
      if (live.overall) put(0, '--stream-glow', live.overall);
      for (let i = 0; i < SEGMENTS; i++) {
        const t = live.top[i];
        const b = live.bottom[i];
        if (t) put(1 + i, `--glow-t${i}`, t);
        if (b) put(1 + SEGMENTS + i, `--glow-b${i}`, b);
      }

      if (!announced.current && live.overall) {
        announced.current = true;
        // Warn, not info: this app logs warnings and errors only (see the
        // DiagnosticLogger line at startup), so an info line here would never
        // reach streamnook.log — the mistake that made an earlier diagnostic
        // useless.
        Logger.warn(
          `[MediaGlow] sampling OK for ${key} (${calm ? 'calm' : 'live'}): ` +
            `overall=${cssColour(live.overall)} top=[${live.top.map((c) => (c ? cssColour(c) : '-')).join(',')}]`,
        );
      }
    };

    // Driven by the decoder when it can be: one callback per presented frame,
    // which is what ties the light to the content rather than to a clock. Not
    // universal (and absent from older WebView2 runtimes), so the timer stays
    // as the fallback rather than as the primary path.
    // Captured once rather than read again in the cleanup. React detaches refs
    // around unmount, so a cleanup that reads `.current` can find null and
    // silently skip both the cancel and the property removal — leaving a
    // callback chained to a dead element and stale colours on the container.
    // The container the sampler last wrote to, so teardown removes the
    // properties from the element that actually has them.
    let wroteTo: HTMLElement | null = null;

    // Arm the decoder callback on whatever element is current, once per
    // element. A callback is bound to the element it was registered on, and if
    // that element is replaced — a media-type switch can do it while the key
    // stays the same — the chain is left on a detached node and never fires
    // again. Nothing would surface: the light would simply stop following.
    let armed: HTMLVideoElement | null = null;
    const arm = () => {
      const el = video.current;
      if (!fast || !el || el === armed) return;
      if (typeof el.requestVideoFrameCallback !== 'function') return;
      armed = el;
      const onFrame = (now: number) => {
        // A chain left over from a previous element stops here rather than
        // competing with the new one.
        if (stopped || video.current !== el) return;
        sample(now);
        rvfc = el.requestVideoFrameCallback(onFrame);
      };
      rvfc = el.requestVideoFrameCallback(onFrame);
    };
    arm();

    // The timer is the fallback when there is no `requestVideoFrameCallback`,
    // and the watchdog when there is: it re-arms after an element swap and
    // keeps the light alive if the chain ever breaks. `sample` is throttled by
    // elapsed time, so while the decoder callback is healthy this costs a
    // comparison and nothing else.
    //
    // `performance.now()` rather than Date.now(): the gap arithmetic above is a
    // duration, and the wall clock can step.
    // Keyed off whether the ENGINE has the callback, not whether it happened to
    // arm just now: the video element may not be mounted yet on the first run,
    // and picking the rate from that would leave the watchdog running at the
    // sampling rate forever once the decoder callback took over.
    const canRideTheDecoder =
      typeof HTMLVideoElement !== 'undefined' &&
      typeof HTMLVideoElement.prototype.requestVideoFrameCallback === 'function';
    timer = setInterval(
      () => {
        arm();
        sample(performance.now());
      },
      fast ? (canRideTheDecoder ? WATCHDOG_MS : FAST_MS) : CALM_MS,
    );

    // One immediate sample so the glow appears with the stream rather than a
    // beat after it.
    sample(performance.now());

    // Sample again the moment the window comes back rather than waiting out the
    // interval with a stale colour.
    const offVisibility = onWindowVisibility(() => {
      if (!isWindowHidden()) sample(performance.now());
    });

    return () => {
      stopped = true;
      if (timer) clearInterval(timer);
      // The attribute belongs to the immersive sampler; nothing may be left
      // holding the player's layers opaque once it is gone.
      if (fast) publishCovered(false);
      if (rvfc !== null) armed?.cancelVideoFrameCallback?.(rvfc);
      offVisibility();
      // Drop the readback canvas with everything else, so a disabled or torn
      // down player really is holding nothing. It is small, but "disabled is
      // nearly free" is only true if it is actually true.
      canvasRef.current = null;
      // Clean up exactly what was dirtied. Tracked as the sampler writes rather
      // than read from the ref here, because a ref read in cleanup can find
      // null (React detaches around unmount) and would then leave the colours
      // behind — and because the element the sampler used is the only one that
      // definitely has them.
      const host = wroteTo;
      host?.style.removeProperty('--stream-glow');
      host?.style.removeProperty('--sn-video-ar');
      host?.style.removeProperty('--sn-video-arn');
      for (let i = 0; i < SEGMENTS; i++) {
        host?.style.removeProperty(`--glow-t${i}`);
        host?.style.removeProperty(`--glow-b${i}`);
      }
    };
  }, [enabled, key, fast, video, target]);
}
