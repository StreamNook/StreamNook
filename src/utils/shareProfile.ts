import { Logger } from './logger';
// Profile card capture/share pipeline.
//
// True screen capture via Tauri commands — NOT a DOM-clone re-render. The
// frontend computes the card's absolute screen coordinates
// (window.outerPosition + element.getBoundingClientRect * DPR) and asks Rust
// to grab the monitor pixels in that region. Result: whatever the compositor
// put on screen (backdrop-filter, 7TV paint gradients, custom fonts, animated
// paints, drop shadows) is preserved 1:1.
//
// Outputs:
//   - Static paint   →  single PNG capture (capture_screen_region)
//   - Animated paint →  animated WebP via libwebp (capture_animated_webp),
//                        with a GIF fallback (gifenc) if the native encoder
//                        errors out. WebP isn't bound by GIF's 256-color
//                        palette, so paint gradients and neon glows render
//                        without quantization banding.
//
// Output paths (try in order):
//   1. navigator.clipboard.write([new ClipboardItem({ [mime]: blob })])
//      — Chromium accepts image/png and image/webp natively. image/gif
//      support varies. The user pastes directly into Discord.
//   2. downloadBlob — anchor-click triggers the WebView2 download flow.
//      User picks a location and drags the file into Discord.

export type CaptureMime = 'image/png' | 'image/gif' | 'image/webp';

/** Capture format override. 'auto' (default) runs detectAnimatedPaint and
 *  picks PNG or WebP based on whether any animated element is present in
 *  the card. 'static' forces PNG even if animated elements exist (useful
 *  when the user wants a clean still). 'animated' forces WebP even if
 *  no animation is detected (the result is a multi-frame WebP of
 *  identical frames — wasteful but valid). */
export type CaptureMode = 'auto' | 'static' | 'animated';

export interface CaptureProfileOptions {
  mode?: CaptureMode;
}

export interface CaptureResult {
  blob: Blob;
  mime: CaptureMime;
  filename: string;
  frameCount: number;
}

const GIF_FRAME_COUNT = 24;
const GIF_DEFAULT_DURATION_MS = 2000;
const GIF_MAX_DURATION_MS = 3000;
// Target playback rate. Frontend computes target_frame_count =
// WEBP_TARGET_FPS * durationMs / 1000 and passes that as the ceiling. Rust
// captures as fast as xcap actually allows up to that ceiling; if xcap
// can sustain the rate the WebP plays at 60fps, otherwise it plays at
// whatever rate xcap can hit (still at correct source speed, just with
// fewer unique frames). xcap 0.0.14 has no region-capture API so each
// frame reads the full monitor framebuffer — that's the hard ceiling on
// high-res displays, and the path past it is WebView2 CapturePreview.
const WEBP_TARGET_FPS = 60;
// 8s ceiling covers nearly all 7TV paints and tier animations. Beyond 8s
// gets into pathological-CSS territory; the Rust side stream-encodes so
// memory cost scales with frame size, not duration.
const WEBP_MAX_DURATION_MS = 8000;

// Walk the subtree looking for any signal that the card is animated.
// Three signals, in order of strength:
//
//  1. CSS animation-name. 7TV paints with `repeat: true` rotate the gradient
//     via a keyframe animation; the badge glow does the same.
//
//  2. background-image URL pointing to an animated format (gif / webp /
//     apng) OR a 7TV CDN URL. 7TV emotes are commonly animated WebP served
//     from cdn.7tv.app without a file extension in the URL, so the CDN host
//     itself is the most reliable hint.
//
//  3. <img> elements whose currentSrc is an animated format or 7TV-hosted.
//     Twitch and 7TV emote markup ranges between <img> and background-image
//     across versions of their embed code.
//
// Any single signal short-circuits to animated=true. Worst case for a false
// positive is encoding a static card as a 24-frame WebP with 24 identical
// frames — bigger file, but visually correct. Worst case for a false
// negative (the pre-fix behavior) is freezing an animated card on one
// arbitrary frame of its animation, which is what the user just hit.
export function detectAnimatedPaint(element: HTMLElement): boolean {
  const ANIMATED_URL_RE = /\.(gif|webp|apng)\b|cdn\.7tv\.app|static-cdn\.jtvnw\.net.*\/(?:animated|gif)/i;

  const all: Element[] = [element, ...Array.from(element.querySelectorAll<HTMLElement>('*'))];
  for (const el of all) {
    const cs = getComputedStyle(el);

    const name = cs.animationName;
    if (name && name !== 'none' && name !== 'normal') return true;

    const bg = cs.backgroundImage;
    if (bg && bg !== 'none' && ANIMATED_URL_RE.test(bg)) return true;
  }

  const imgs = element.querySelectorAll('img');
  for (const img of Array.from(imgs)) {
    const src = (img as HTMLImageElement).currentSrc || (img as HTMLImageElement).src || '';
    if (src && ANIMATED_URL_RE.test(src)) return true;
  }

  return false;
}

// Frontend-callable version of the same duration capping Rust uses
// internally — exported so the progress UI can show a meaningful
// estimate of how long the recording phase will take.
export function estimateCaptureDurationMs(element: HTMLElement): number {
  return Math.round(Math.min(getAnimationDurationMs(element), WEBP_MAX_DURATION_MS));
}

/** Freezes every CSS animation in the subtree at its current frame and
 *  returns a restore function. Used before static (PNG) captures so the
 *  snapshot lands on a stable frame instead of an in-between moment of
 *  the 7TV paint rotation / tier glow / etc. — without this, the user
 *  sees the captured PNG as a glitched/torn frame because the GPU
 *  compositor may commit mid-capture. animation-play-state only affects
 *  CSS animations; animated WebP backgrounds (the other 7TV paint
 *  variety) play autonomously and aren't paused by this — but any single
 *  frame of an animated WebP is also a valid still, so that's fine. */
export function pauseCardAnimations(element: HTMLElement): () => void {
  const all: HTMLElement[] = [
    element,
    ...Array.from(element.querySelectorAll<HTMLElement>('*')),
  ];
  const restore: Array<() => void> = [];

  for (const el of all) {
    const cs = getComputedStyle(el);
    if (cs.animationName && cs.animationName !== 'none' && cs.animationName !== 'normal') {
      const original = el.style.animationPlayState;
      el.style.animationPlayState = 'paused';
      restore.push(() => {
        el.style.animationPlayState = original;
      });
    }
  }

  return () => {
    for (const fn of restore) fn();
  };
}

// Pulls the longest animation-duration across the subtree so we sample the
// full cycle of the slowest animation. Falls back to 2s if nothing animates
// (shouldn't happen since detectAnimatedPaint gates the call).
function getAnimationDurationMs(element: HTMLElement): number {
  const all: Element[] = [element, ...Array.from(element.querySelectorAll<HTMLElement>('*'))];
  let maxMs = 0;
  for (const el of all) {
    const dur = getComputedStyle(el).animationDuration;
    if (!dur) continue;
    for (const raw of dur.split(',')) {
      const t = raw.trim();
      let v = 0;
      if (t.endsWith('ms')) v = parseFloat(t);
      else if (t.endsWith('s')) v = parseFloat(t) * 1000;
      if (!Number.isNaN(v) && v > maxMs) maxMs = v;
    }
  }
  return maxMs || GIF_DEFAULT_DURATION_MS;
}

interface ScreenRect {
  /** Screen-physical (desktop-global) coords: innerPosition + rect * DPR.
   *  Used by both paths — static (xcap) and animated (ffmpeg gdigrab)
   *  consume the same coordinate system. */
  x: number;
  y: number;
  width: number;
  height: number;
  /** Element's border-radius in PHYSICAL pixels. 0 = square corners.
   *  Rust applies an anti-aliased alpha mask to the four corner regions
   *  so the output matches the rounded card shape instead of leaking
   *  whatever's behind the card through the square corners. */
  radiusPx: number;
}

async function computeScreenRect(element: HTMLElement): Promise<ScreenRect> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const win = getCurrentWindow();
  // innerPosition (NOT outerPosition). On Windows 10/11, top-level windows
  // — even with decorations: false — carry an invisible "extended frame"
  // margin that DWM reserves for drop shadows. innerPosition gives the
  // client-area top-left, which is what the WebView's CSS viewport (0,0)
  // maps to in screen-physical coords. outerPosition shifts up-and-left
  // by ~7-8 physical px and slices the right/bottom edges off the crop.
  const [winPos, scale] = await Promise.all([win.innerPosition(), win.scaleFactor()]);
  const rect = element.getBoundingClientRect();

  // Element rect in screen-physical coords. ffmpeg's gdigrab -offset_x/y
  // and xcap's capture_image both work in desktop-global pixels with the
  // primary monitor's top-left at (0,0), so the same numbers serve both
  // paths.
  const xPhys = Math.floor(winPos.x + rect.left * scale);
  const yPhys = Math.floor(winPos.y + rect.top * scale);
  const rPhys = Math.ceil(winPos.x + rect.right * scale);
  const bPhys = Math.ceil(winPos.y + rect.bottom * scale);

  // CSS border-radius -> physical pixels for the alpha mask. parseFloat
  // takes the first value of the shorthand ("12px" or "12px 12px ...").
  // The Profile card uses uniform rounded-xl so the first value suffices.
  const radiusCss = parseFloat(getComputedStyle(element).borderRadius) || 0;
  const radiusPx = Math.round(radiusCss * scale);

  return {
    x: xPhys,
    y: yPhys,
    width: Math.max(1, rPhys - xPhys),
    height: Math.max(1, bPhys - yPhys),
    radiusPx,
  };
}

async function captureBytes(rect: ScreenRect): Promise<ArrayBuffer> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<ArrayBuffer>('capture_screen_region', {
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height,
  });
}

// Decode a PNG byte buffer (from capture_screen_region) into ImageData for
// pixel-level work. Used by the GIF fallback path; the static PNG path
// blobs the bytes directly and never needs ImageData. imageSmoothing
// disabled because the draw is 1:1 — any smoothing would soften the crisp
// pixels we just captured.
async function pngBytesToImageData(pngBytes: ArrayBuffer): Promise<ImageData> {
  const img = await createImageBitmap(new Blob([pngBytes], { type: 'image/png' }));
  const canvas = document.createElement('canvas');
  canvas.width = img.width;
  canvas.height = img.height;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('Failed to acquire 2D context');
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(img, 0, 0);
  img.close();
  return ctx.getImageData(0, 0, canvas.width, canvas.height);
}

export async function captureProfileCard(
  element: HTMLElement,
  options: CaptureProfileOptions = {},
): Promise<CaptureResult> {
  const mode = options.mode ?? 'auto';
  // Mode override: 'static' always PNG, 'animated' always WebP, 'auto'
  // falls back to the existing detector. The user-facing share UI passes
  // an explicit mode based on which button was clicked; detection is
  // only used to decide whether to enable the Animated button at all.
  const animated =
    mode === 'animated' ? true : mode === 'static' ? false : detectAnimatedPaint(element);
  const rect = await computeScreenRect(element);
  const base = 'streamnook-profile';

  if (!animated) {
    // Static path: capture_screen_region returns rectangular PNG bytes;
    // mask the rounded corners JS-side via canvas.roundRect clip + redraw
    // so the output's four corners are transparent and the file reads as
    // the rounded card shape instead of a flat rectangle.
    const bytes = await captureBytes(rect);
    const blob =
      rect.radiusPx > 0
        ? await maskRoundedCornersOnPng(bytes, rect.radiusPx)
        : new Blob([bytes], { type: 'image/png' });
    return { blob, mime: 'image/png', filename: `${base}.png`, frameCount: 1 };
  }

  // Animated: try WebP first. If the native encoder errors (libwebp link
  // problem, monitor disconnected mid-capture, etc.), fall back to GIF so
  // the share button never silently fails.
  try {
    const blob = await captureWebP(element, rect);
    return {
      blob,
      mime: 'image/webp',
      filename: `${base}.webp`,
      // Reported as the *target* frame count derived from WEBP_TARGET_FPS.
      // Rust captures as fast as xcap allows up to this ceiling and the
      // actual count may be lower if monitor capture latency is the
      // bottleneck — we don't get the achieved count back here.
      frameCount: Math.max(
        2,
        Math.ceil(
          (Math.min(getAnimationDurationMs(element), WEBP_MAX_DURATION_MS) *
            WEBP_TARGET_FPS) /
            1000,
        ),
      ),
    };
  } catch (e) {
    Logger.warn('[shareProfile] WebP capture failed, falling back to GIF:', e);
    const blob = await captureGIF(element, rect);
    return {
      blob,
      mime: 'image/gif',
      filename: `${base}.gif`,
      frameCount: GIF_FRAME_COUNT,
    };
  }
}

// Native animated-WebP path. Rust does the whole pipeline: capture N
// frames evenly across durationMs of real wall-clock time, alpha-mask the
// rounded corners on each frame, feed raw RGBA to libwebp's
// WebPAnimEncoder. One IPC call returns the final WebP bytes. No 256-color
// palette, no dithering, no inter-frame jitter.
//
// durationMs is the source animation's CSS cycle length (capped at
// WEBP_MAX_DURATION_MS). The WebP loop length is exactly durationMs, so
// frame N-1 at ts = (N-1)/N * durationMs holds for durationMs/N before the
// WebP wraps back to frame 0 — the source has just completed its own
// cycle at the same moment, so the wrap is seamless.
async function captureWebP(element: HTMLElement, rect: ScreenRect): Promise<Blob> {
  const { invoke } = await import('@tauri-apps/api/core');

  const durationMs = Math.round(
    Math.min(getAnimationDurationMs(element), WEBP_MAX_DURATION_MS),
  );
  // Ceiling on frame count. Rust captures up to this many frames at
  // xcap's natural cadence; if xcap is faster than the implied per-frame
  // interval (durationMs/N), Rust throttles to it. If slower, Rust just
  // runs flat-out and we get whatever fits in durationMs.
  const targetFrameCount = Math.max(
    2,
    Math.ceil((durationMs * WEBP_TARGET_FPS) / 1000),
  );

  // The Rust capture_animated_webp drives ffmpeg's gdigrab which uses
  // desktop-global coords, so we pass the same screen-physical rect as
  // the static path.
  const bytes = await invoke<ArrayBuffer>('capture_animated_webp', {
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height,
    radiusPx: rect.radiusPx,
    targetFrameCount,
    durationMs,
  });
  return new Blob([bytes], { type: 'image/webp' });
}

// JS-side rounded-corner alpha mask for the static PNG path. Uses Canvas
// roundRect (WebView2 99+) to clip to a rounded rectangle, then redraws
// the captured image inside the clip. Pixels outside the clip remain at
// the canvas default (transparent), so the resulting PNG has alpha 0 in
// the four corner regions.
async function maskRoundedCornersOnPng(
  pngBytes: ArrayBuffer,
  radiusPx: number,
): Promise<Blob> {
  const img = await createImageBitmap(new Blob([pngBytes], { type: 'image/png' }));
  const canvas = document.createElement('canvas');
  canvas.width = img.width;
  canvas.height = img.height;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('Failed to acquire 2D context');

  ctx.beginPath();
  ctx.roundRect(0, 0, canvas.width, canvas.height, radiusPx);
  ctx.clip();
  ctx.drawImage(img, 0, 0);
  img.close();

  return new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(
      (b) => (b ? resolve(b) : reject(new Error('canvas.toBlob returned null'))),
      'image/png',
    );
  });
}

// 4x4 Bayer matrix in [0..15] order. We center it around 0 (–7.5 to +7.5) and
// scale by a strength factor so each pixel gets a structured offset before
// quantization. This breaks gradient bands by spreading values across
// multiple palette entries — the eye reads it as a smooth transition with
// faint stippling rather than hard color steps.
const BAYER_4X4: ReadonlyArray<ReadonlyArray<number>> = [
  [0, 8, 2, 10],
  [12, 4, 14, 6],
  [3, 11, 1, 9],
  [15, 7, 13, 5],
];
const BAYER_DITHER_STRENGTH = 8;

function ditherInPlace(rgba: Uint8ClampedArray, width: number, height: number): void {
  for (let y = 0; y < height; y++) {
    const row = BAYER_4X4[y & 3];
    const yOff = y * width;
    for (let x = 0; x < width; x++) {
      const noise = ((row[x & 3] - 7.5) * BAYER_DITHER_STRENGTH) / 8;
      const i = (yOff + x) * 4;
      const r = rgba[i] + noise;
      const g = rgba[i + 1] + noise;
      const b = rgba[i + 2] + noise;
      rgba[i] = r < 0 ? 0 : r > 255 ? 255 : r;
      rgba[i + 1] = g < 0 ? 0 : g > 255 ? 255 : g;
      rgba[i + 2] = b < 0 ? 0 : b > 255 ? 255 : b;
    }
  }
}

async function captureGIF(element: HTMLElement, rect: ScreenRect): Promise<Blob> {
  const { GIFEncoder, quantize, applyPalette } = await import('gifenc');

  const duration = Math.min(getAnimationDurationMs(element), GIF_MAX_DURATION_MS);
  // Frame delay applies both to source-time spacing between captures AND to
  // GIF playback delay between frames, so the recorded and played loop
  // lengths agree.
  const frameDelayMs = Math.max(40, Math.round(duration / GIF_FRAME_COUNT));

  // Phase 1 — capture every frame's pixel buffer up front. Holding ~30 MB of
  // RGBA in memory is fine and lets us build a single GLOBAL palette across
  // the whole animation in phase 2. Per-frame palettes flicker between
  // frames (each one picks slightly different 256 colors out of the gradient)
  // which reads as inter-frame jitter on top of the banding.
  const frames: ImageData[] = [];
  for (let i = 0; i < GIF_FRAME_COUNT; i++) {
    const bytes = await captureBytes(rect);
    frames.push(await pngBytesToImageData(bytes));
    if (i < GIF_FRAME_COUNT - 1) await wait(frameDelayMs);
  }
  if (frames.length === 0) throw new Error('No frames captured');

  // Phase 2 — Bayer-dither every frame's RGBA in place. Done BEFORE quantize
  // sees the data so the resulting palette captures the dithered color set,
  // not the original posterized one. Trades the visible gradient stairs for
  // a fine stipple pattern that's much less perceptible at typical viewing
  // sizes.
  for (const f of frames) {
    ditherInPlace(f.data, f.width, f.height);
  }

  // Phase 3 — derive the global palette from every frame's pixels.
  //  - Sampling every frame (not just 4 fixed indices) guarantees the palette
  //    sees every hue the animation traverses. Capture round-trip time drifts
  //    real-time spacing, so a 4-index sample can entirely miss the saturated
  //    peaks of a rotating gradient.
  //  - rgb565 keeps 5/6/5 bits per channel of input precision (65 K source
  //    colors) instead of the default rgb444 (4 K), which matters for the
  //    saturated paint gradients and bits/neon badge glows.
  const perFrameBytes = frames[0].data.length;
  const sampleData = new Uint8ClampedArray(perFrameBytes * frames.length);
  {
    let offset = 0;
    for (const f of frames) {
      sampleData.set(f.data, offset);
      offset += f.data.length;
    }
  }
  const globalPalette = quantize(sampleData, 256, { format: 'rgb565' });

  // Phase 4 — write frames using the global palette. First frame carries the
  // palette declaration; subsequent frames omit `palette` so they reuse the
  // global one (smaller file + no inter-frame color jitter).
  const gif = GIFEncoder();
  for (let i = 0; i < frames.length; i++) {
    const f = frames[i];
    const indexed = applyPalette(f.data, globalPalette, 'rgb565');
    gif.writeFrame(indexed, f.width, f.height, {
      palette: i === 0 ? globalPalette : undefined,
      delay: frameDelayMs,
    });
  }
  gif.finish();
  return new Blob([gif.bytes()], { type: 'image/gif' });
}

function wait(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

export async function copyImageToClipboard(blob: Blob, mime: string): Promise<boolean> {
  try {
    if (typeof ClipboardItem === 'undefined' || !navigator.clipboard?.write) return false;
    await navigator.clipboard.write([new ClipboardItem({ [mime]: blob })]);
    return true;
  } catch {
    return false;
  }
}

// Anchor-click trick. In Tauri's WebView this triggers the platform download
// flow (system save dialog on Windows). Cleaner than wiring up plugin-fs +
// plugin-dialog explicitly, and works in regular browsers too if this ever
// runs outside Tauri.
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
