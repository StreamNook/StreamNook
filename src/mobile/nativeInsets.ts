// Pull half of the native inset bridge. MainActivity pushes CSS vars on every
// inset change, but a page (re)load starts with a fresh document that missed
// the last push; the synchronous `SNInsets` JavascriptInterface fills that gap.
interface NativeInsets {
  top: number;
  right: number;
  bottom: number;
  left: number;
  kb: number;
  /** Left/right BACK gesture strips (systemGestures). */
  gestureLeft: number;
  gestureRight: number;
  /** Top/bottom home + quick-switch strips (mandatorySystemGestures). */
  gestureTop: number;
  gestureBottom: number;
}

export function applyNativeInsetsOnce(): void {
  try {
    const bridge = (window as Window & { SNInsets?: { get(): string } }).SNInsets;
    const raw = bridge?.get();
    if (!raw) return;
    const v = JSON.parse(raw) as Partial<NativeInsets>;
    if (typeof v.top !== 'number') return;
    const d = document.documentElement;
    d.style.setProperty('--sn-inset-t', `${v.top}px`);
    d.style.setProperty('--sn-inset-r', `${v.right ?? 0}px`);
    d.style.setProperty('--sn-inset-b', `${v.bottom ?? 0}px`);
    d.style.setProperty('--sn-inset-l', `${v.left ?? 0}px`);
    d.style.setProperty('--sn-kb', `${v.kb ?? 0}px`);
    d.style.setProperty('--sn-gesture-l', `${v.gestureLeft ?? 0}px`);
    d.style.setProperty('--sn-gesture-r', `${v.gestureRight ?? 0}px`);
    d.style.setProperty('--sn-gesture-t', `${v.gestureTop ?? 0}px`);
    d.style.setProperty('--sn-gesture-b', `${v.gestureBottom ?? 0}px`);
    d.dataset.snNativeInsets = 'true';
  } catch {
    /* bridge absent (desktop, dev browser): env() fallback applies */
  }
}

/** One CSS length var off <html>, in px. 0 when absent or unparseable. */
function cssPx(name: string): number {
  if (typeof window === 'undefined') return 0;
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const n = parseFloat(raw);
  return Number.isFinite(n) ? n : 0;
}

export interface ResolvedInsets {
  top: number;
  right: number;
  bottom: number;
  left: number;
  gestureLeft: number;
  gestureRight: number;
  gestureTop: number;
  gestureBottom: number;
}

/**
 * Numeric insets, for layout math that cannot be expressed in CSS.
 *
 * The floating mini player is the caller that needs this: its clamp has to keep
 * the box out of the status bar AND out of the system gesture strips, and that
 * clamp is arithmetic on a drag position rather than a style.
 *
 * Reads the CSS vars rather than the bridge directly, so it picks up whichever
 * source last wrote them (MainActivity's push, or applyNativeInsetsOnce after a
 * reload) and degrades to zeroes on desktop and in a dev browser.
 */
export function readInsets(): ResolvedInsets {
  return {
    top: cssPx('--sn-inset-t'),
    right: cssPx('--sn-inset-r'),
    bottom: cssPx('--sn-inset-b'),
    left: cssPx('--sn-inset-l'),
    gestureLeft: cssPx('--sn-gesture-l'),
    gestureRight: cssPx('--sn-gesture-r'),
    gestureTop: cssPx('--sn-gesture-t'),
    gestureBottom: cssPx('--sn-gesture-b'),
  };
}
