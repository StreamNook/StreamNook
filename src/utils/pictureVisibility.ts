// Whether a moving profile picture is actually seen, for every picture in the
// window at once.
//
// A picture is in view when it is on screen (one shared IntersectionObserver),
// drawn (no ancestor at `display: none`, `visibility: hidden` or opacity 0, and
// not inside a container the app marks covered) and not under a covering layer
// (an overlay, or the player in full screen; see ./coverage). Window hidden and
// minimized are the caller's, through windowVisibility.
//
// Cheap by construction: one observer, one set of document listeners and one
// store subscription for every picture, installed while at least one is
// watched. Nothing samples per picture or per frame. Re-checks are driven by
// the events that change what is covering (store flags for the overlays and
// full screen, a click or key, the end of a show or hide transition) and are
// coalesced into one pass at most every `MIN_GAP_MS`. The only timer runs
// while something is covering, to notice it leave even when no event says so.
import { useAppStore } from '../stores/AppStore';
import { COVERED_CONTAINER_SELECTOR, coveringLayers, hiddenUnderLayer, layerStillCovers } from './coverage';
import { isWindowHidden, onWindowVisibility } from './windowVisibility';

type Notify = (inView: boolean) => void;

interface Watched {
  notify: Notify;
  onScreen: boolean;
  /** Null until the first answer, which is always delivered. */
  inView: boolean | null;
}

/** Floor between passes. Covering changes on human timescales. */
const MIN_GAP_MS = 100;
/** Re-checks after a click, key or store change, for overlays that animate
 *  out (and unmount) after the change that closed them. */
const SETTLE_MS = [350, 1000];
/** How often to notice a covering layer leaving while one is up. Reads two
 *  properties per layer, no selector match. */
const UNCOVER_CHECK_MS = 500;

/** Transitions that can show or hide something. Colour and shadow hovers,
 *  which fire constantly, change nothing here. */
const SHOW_HIDE_PROPERTIES = new Set([
  'opacity',
  'visibility',
  'display',
  'transform',
  'translate',
  'width',
  'min-width',
  'max-width',
  'height',
  'max-height',
]);

/** Store fields that open or close something covering, or change the layout
 *  that pictures sit in. */
const COVERING_STATE_KEYS = [
  'isSettingsOpen',
  'isCommandPaletteOpen',
  'showLiveStreamsOverlay',
  'showMarketplaceOverlay',
  'showDropsOverlay',
  'showBadgesOverlay',
  'showEmoteSetsOverlay',
  'showWhispersOverlay',
  'showDashboardOverlay',
  'profileModalUser',
  'clipModal',
  'vodModal',
  'isPlayerFullscreen',
  'playerOverlayVisible',
  'isWindowFullscreen',
  'isTheaterMode',
  'isHomeActive',
  'streamUrl',
] as const;

/**
 * The visibility decision for one picture, given what the observer and the
 * pass found. Exported for tests.
 */
export function pictureInView({
  onScreen,
  drawn,
  covered,
}: {
  onScreen: boolean;
  drawn: boolean;
  covered: boolean;
}): boolean {
  return onScreen && drawn && !covered;
}

const watched = new Map<Element, Watched>();
let observer: IntersectionObserver | null = null;
let layers: Element[] = [];
let dirtyAll = false;
const dirtyRoots = new Set<Element>();
let passTimer: ReturnType<typeof setTimeout> | null = null;
let lastPass = 0;
const settleTimers: ReturnType<typeof setTimeout>[] = [];
let uncoverTimer: ReturnType<typeof setInterval> | null = null;
let teardown: (() => void) | null = null;

/** Drawn at all: nothing above it hides it by style, and the app has not
 *  marked its container covered. */
function isDrawn(el: Element): boolean {
  if (el.closest(COVERED_CONTAINER_SELECTOR)) return false;
  const check = (el as Element & { checkVisibility?: (o?: object) => boolean }).checkVisibility;
  if (typeof check !== 'function') return true;
  return check.call(el, { opacityProperty: true, visibilityProperty: true });
}

/** Under a covering layer at its centre (clamped to the part in the window). */
function isCovered(el: Element): boolean {
  if (layers.length === 0) return false;
  const rect = el.getBoundingClientRect();
  const left = Math.max(rect.left, 0);
  const right = Math.min(rect.right, window.innerWidth);
  const top = Math.max(rect.top, 0);
  const bottom = Math.min(rect.bottom, window.innerHeight);
  if (right <= left || bottom <= top) return false;
  const stack = document.elementsFromPoint((left + right) / 2, (top + bottom) / 2);
  return hiddenUnderLayer(stack, el, layers);
}

function settle(el: Element, entry: Watched): void {
  const inView = pictureInView({
    onScreen: entry.onScreen,
    drawn: entry.onScreen && isDrawn(el),
    covered: entry.onScreen && isCovered(el),
  });
  if (inView === entry.inView) return;
  entry.inView = inView;
  entry.notify(inView);
}

function sameLayers(a: readonly Element[], b: readonly Element[]): boolean {
  return a.length === b.length && a.every((layer, i) => layer === b[i]);
}

function runPass(): void {
  passTimer = null;
  lastPass = Date.now();
  // A hidden window is paused by the caller already; catch up once it is back.
  if (isWindowHidden()) return;
  const next = coveringLayers();
  const layersChanged = !sameLayers(next, layers);
  layers = next;
  const all = dirtyAll || layersChanged;
  const roots = [...dirtyRoots];
  dirtyAll = false;
  dirtyRoots.clear();
  for (const [el, entry] of watched) {
    if (!entry.onScreen) continue;
    if (!all && !roots.some((root) => root.contains(el))) continue;
    settle(el, entry);
  }
  syncUncoverTimer();
}

function schedulePass(): void {
  if (passTimer !== null || watched.size === 0) return;
  // A timer, not a frame callback: the pass must still run when the page is
  // not producing frames, and it reads layout once either way.
  const wait = Math.max(0, Math.min(MIN_GAP_MS, MIN_GAP_MS - (Date.now() - lastPass)));
  passTimer = setTimeout(runPass, wait);
}

function markAll(): void {
  dirtyAll = true;
  schedulePass();
}

/** A change that may finish later (an overlay animating out): check now and
 *  again once it has had time to leave. */
function markAllAndSettle(): void {
  markAll();
  while (settleTimers.length) clearTimeout(settleTimers.pop());
  for (const ms of SETTLE_MS) settleTimers.push(setTimeout(markAll, ms));
}

function syncUncoverTimer(): void {
  if (layers.length > 0 && !uncoverTimer) {
    uncoverTimer = setInterval(() => {
      if (!layers.every((layer) => layerStillCovers(layer))) markAll();
    }, UNCOVER_CHECK_MS);
  } else if (layers.length === 0 && uncoverTimer) {
    clearInterval(uncoverTimer);
    uncoverTimer = null;
  }
}

function onTransitionEnd(event: Event): void {
  const name = (event as TransitionEvent).propertyName;
  if (name && !SHOW_HIDE_PROPERTIES.has(name)) return;
  if (event.target instanceof Element) {
    dirtyRoots.add(event.target);
    schedulePass();
  }
}

function install(): void {
  const opts: AddEventListenerOptions = { capture: true, passive: true };
  document.addEventListener('pointerup', markAllAndSettle, opts);
  document.addEventListener('keyup', markAllAndSettle, opts);
  document.addEventListener('fullscreenchange', markAllAndSettle, opts);
  document.addEventListener('transitionend', onTransitionEnd, opts);
  document.addEventListener('transitioncancel', onTransitionEnd, opts);
  document.addEventListener('animationend', onTransitionEnd, opts);
  window.addEventListener('resize', markAll, opts);
  const stopVisibility = onWindowVisibility(markAll);
  const stopStore = useAppStore.subscribe((state, prev) => {
    const now = state as unknown as Record<string, unknown>;
    const before = prev as unknown as Record<string, unknown>;
    if (COVERING_STATE_KEYS.some((key) => now[key] !== before[key])) markAllAndSettle();
  });
  teardown = () => {
    document.removeEventListener('pointerup', markAllAndSettle, opts);
    document.removeEventListener('keyup', markAllAndSettle, opts);
    document.removeEventListener('fullscreenchange', markAllAndSettle, opts);
    document.removeEventListener('transitionend', onTransitionEnd, opts);
    document.removeEventListener('transitioncancel', onTransitionEnd, opts);
    document.removeEventListener('animationend', onTransitionEnd, opts);
    window.removeEventListener('resize', markAll, opts);
    stopVisibility();
    stopStore();
    if (passTimer) clearTimeout(passTimer);
    while (settleTimers.length) clearTimeout(settleTimers.pop());
    if (uncoverTimer) clearInterval(uncoverTimer);
    passTimer = null;
    uncoverTimer = null;
    layers = [];
    dirtyAll = false;
    dirtyRoots.clear();
  };
}

/**
 * Watch one picture; `notify` hears whether it is in view whenever that
 * changes (first answer on the observer's first report). Returns the
 * unsubscribe. Without IntersectionObserver (tests, very old engines) it is
 * always in view.
 */
export function watchPictureVisibility(el: Element, notify: Notify): () => void {
  if (typeof IntersectionObserver === 'undefined') {
    notify(true);
    return () => {};
  }
  // A small margin starts the animation just before a picture scrolls in.
  // Two thresholds so a picture clipped to nothing (a collapsed panel) that
  // grows back is reported, not just one crossing the edge.
  observer ??= new IntersectionObserver(
    (entries) => {
      for (const io of entries) {
        const entry = watched.get(io.target);
        if (!entry) continue;
        const rect = io.intersectionRect;
        entry.onScreen = io.isIntersecting && rect.width > 0 && rect.height > 0;
        settle(io.target, entry);
      }
    },
    { rootMargin: '64px', threshold: [0, 0.01] },
  );
  if (watched.size === 0) install();
  watched.set(el, { notify, onScreen: false, inView: null });
  observer.observe(el);
  return () => {
    observer?.unobserve(el);
    watched.delete(el);
    if (watched.size === 0) {
      teardown?.();
      teardown = null;
    }
  };
}
