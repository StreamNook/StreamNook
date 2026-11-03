import {
  createContext,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion, stagger } from 'framer-motion';
import { IS_MOBILE } from '../../utils/platform';
import { markBadgeArtFailed } from '../../utils/badgeArtFailures';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { activeStripToken, chipLeftView, openStrip, releaseStrip, stripPlace, type StripPlace } from './badgeFoldStrip';

const EASE = [0.2, 0.8, 0.2, 1] as const;
const OVERSHOOT = [0.2, 0.9, 0.25, 1.15] as const;
// The chat list's scroll root on desktop and phone (it carries data-scroll-root).
const LIST_ROOT = '[data-scroll-root]';
const OPEN_INTENT_MS = 60;
const CLOSE_GRACE_MS = 150;
// A badge is 1.4em times the badge scale; strip badges sit 0.25em apart.
const SLOT_EM = (scale: number) => 1.4 * scale + 0.25;

/** A peek thumbnail's art, and Twitch's own copy to try once if a disk copy fails. */
export interface PeekArt {
  url: string;
  remote?: string;
}

/** The box the chip scrolls in: its chat list, else its nearest scrolling
 *  ancestor (the activity feed, a settings panel), else none (the window). */
function scrollRootOf(chip: Element): Element | null {
  const list = chip.closest(LIST_ROOT);
  if (list) return list;
  for (let el = chip.parentElement; el && el !== document.body; el = el.parentElement) {
    const y = getComputedStyle(el).overflowY;
    if (y === 'auto' || y === 'scroll' || y === 'hidden') return el;
  }
  return null;
}

/** The pane the chip sits in, else the window: the strip's room. */
function paneBounds(chip: Element): { left: number; right: number } {
  const r = scrollRootOf(chip)?.getBoundingClientRect();
  return r ? { left: r.left, right: r.right } : { left: 0, right: window.innerWidth };
}

function applyPlace(strip: HTMLElement, p: StripPlace): void {
  strip.style.left = p.left === null ? '' : `${p.left}px`;
  strip.style.right = p.right === null ? '' : `${p.right}px`;
  strip.style.top = `${p.top}px`;
  strip.style.maxWidth = `${p.maxWidth}px`;
}

const STRIP = {
  hidden: (dir: 1 | -1) => ({ opacity: 0, x: -4 * dir, y: '-50%', scale: 0.96, pointerEvents: 'none' as const }),
  shown: (dir: 1 | -1) => ({
    opacity: 1,
    x: 0,
    y: '-50%',
    scale: 1,
    pointerEvents: 'auto' as const,
    // The glass goes opaque fast, so the row's text never ghosts through it
    // while the badges fan out; the glass leaves last on the way back.
    transition: {
      duration: 0.18,
      ease: EASE,
      opacity: { duration: 0.06, ease: 'linear' },
      delayChildren: stagger(0.025, { from: dir === 1 ? 'first' : 'last' }),
    },
  }),
  gone: (dir: 1 | -1) => ({
    opacity: 0,
    x: -4 * dir,
    y: '-50%',
    scale: 0.96,
    pointerEvents: 'none' as const,
    transition: { duration: 0.12, ease: EASE, opacity: { duration: 0.06, delay: 0.06 } },
  }),
};

// Each badge starts back at the pill (`i` slots away, counted from the pill)
// and springs out to its place with a slight overshoot: they fan out of the
// pill, toward whichever side the strip opened. Motion "reduced" keeps the fade.
interface ItemCustom {
  i: number;
  dir: 1 | -1;
  slot: number;
}
const ITEM = {
  hidden: ({ i, dir, slot }: ItemCustom) => ({ opacity: 0, x: `${-dir * (i + 1) * slot}em`, scale: 0.6 }),
  shown: { opacity: 1, x: 0, scale: 1, transition: { duration: 0.26, ease: OVERSHOOT } },
  gone: ({ i, dir, slot }: ItemCustom) => ({ opacity: 0, x: `${-dir * (i + 1) * slot * 0.5}em`, scale: 0.8, transition: { duration: 0.12, ease: EASE } }),
};

const FoldContext = createContext<{ dir: 1 | -1; slot: number; count: number }>({ dir: 1, slot: SLOT_EM(1), count: 0 });

/** The pill's face: thumbnails of the next two hidden badges, then the count. */
function PeekFace({ peeks, count }: { peeks: readonly PeekArt[]; count: number }) {
  return (
    <>
      {peeks.slice(0, 2).map((p, i) => (
        <img
          key={`${i}-${p.url}`}
          src={p.url}
          alt=""
          aria-hidden="true"
          className="sn-badge-fold-peek"
          loading="lazy"
          decoding="async"
          onError={(e) => {
            const el = e.currentTarget;
            if (p.remote && p.remote !== p.url && !el.dataset.remote) {
              el.dataset.remote = '1';
              el.src = p.remote;
              return;
            }
            markBadgeArtFailed(p.url, p.remote);
          }}
        />
      ))}
      {/* Its own box, so it sits above a rare pill's foil layers. */}
      <span className="sn-badge-fold-count">{count > 99 ? '99+' : `+${count}`}</span>
    </>
  );
}

/** Tilts a rare pill's foil and glare toward the pointer (the Hiyori holo). */
function tiltFoil(el: HTMLElement, clientX: number, clientY: number): void {
  const r = el.getBoundingClientRect();
  const x = ((clientX - r.left) / r.width) * 100;
  const y = ((clientY - r.top) / r.height) * 100;
  el.style.setProperty('--sn-foil-mx', `${x}%`);
  el.style.setProperty('--sn-foil-my', `${y}%`);
  el.style.setProperty('--sn-foil-px', `${50 + (x - 50) / 2.5}%`);
  el.style.setProperty('--sn-foil-py', `${50 + (y - 50) / 2.5}%`);
}

/** Back to the foil's resting angle. */
function restFoil(el: HTMLElement): void {
  for (const k of ['--sn-foil-mx', '--sn-foil-my', '--sn-foil-px', '--sn-foil-py']) el.style.removeProperty(k);
}

/**
 * The "peek" pill after a row's kept badges. Hovering it (after real pointer
 * movement, so chat scrolling under a parked cursor never opens it) or
 * pressing it fans the hidden badges out of the pill onto a glass strip beside
 * it: nothing in chat moves. The strip is portaled to <body> (rows
 * paint-contain) only while open or closing, and rides its row each frame as
 * chat scrolls; it never pauses chat. A press holds it open until a second
 * press, an outside press, Escape, a scroll of the chat list, or its row
 * scrolling away.
 */
export function BadgeFoldChip({
  count,
  peeks,
  tier = 0,
  children,
  still = false,
}: {
  count: number;
  /** Art of the first two hidden badges, for the pill's face. */
  peeks: readonly PeekArt[];
  /** The chatter's rarity, 0 plain to 4 legendary (from every add-on they wear,
   *  never from the viewer's limit): the pill's foil. */
  tier?: number;
  children: () => ReactNode;
  still?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [mounted, setMounted] = useState(false);
  const [token] = useState(() => ({}));
  const [fontSize, setFontSize] = useState('');
  const [badgeScale, setBadgeScale] = useState(1);
  const [dir, setDir] = useState<1 | -1>(1);
  const chipRef = useRef<HTMLButtonElement>(null);
  const stripRef = useRef<HTMLDivElement>(null);
  const timer = useRef<number | null>(null);
  // Opened by a click or tap: pointer-leave no longer folds it.
  const held = useRef(false);
  // Opened from the keyboard: focus moves into the strip, so Tab reaches the
  // folded badges the way it reaches every other badge.
  const focusStrip = useRef(false);
  const stripId = useId();

  const clearTimer = () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
  };
  const close = () => {
    clearTimer();
    held.current = false;
    releaseStrip(token);
    setOpen(false);
  };
  const needed = (chip: HTMLElement, scale: number) =>
    count * SLOT_EM(scale) * parseFloat(getComputedStyle(chip.parentElement ?? chip).fontSize || '13') + 16;
  const show = (deliberate: boolean) => {
    const chip = chipRef.current;
    if (!chip) return;
    clearTimer();
    if (deliberate) held.current = true;
    openStrip({ token, close });
    const row = chip.parentElement ?? chip;
    const scale = parseFloat(getComputedStyle(chip).getPropertyValue('--sn-badge-scale')) || 1;
    setFontSize(getComputedStyle(row).fontSize);
    setBadgeScale(scale);
    setDir(stripPlace(chip.getBoundingClientRect(), paneBounds(chip), window.innerWidth, needed(chip, scale)).dir);
    setMounted(true);
    setOpen(true);
  };
  const scheduleClose = () => {
    clearTimer();
    if (held.current) return;
    timer.current = window.setTimeout(close, CLOSE_GRACE_MS);
  };

  const outsideHandler = useOutsidePress(open, [chipRef, stripRef], close);

  // Place the strip before its first paint (the rAF loop then keeps it there).
  useLayoutEffect(() => {
    const chip = chipRef.current;
    if (!open || !chip || !stripRef.current) return;
    applyPlace(stripRef.current, stripPlace(chip.getBoundingClientRect(), paneBounds(chip), window.innerWidth, 0, dir));
    if (focusStrip.current) {
      focusStrip.current = false;
      stripRef.current.querySelector<HTMLElement>('button, [tabindex]')?.focus();
    }
  }, [open, mounted, dir]);

  // Tab past either end of the strip goes back to the pill, not to the page's end.
  const onStripKey = (e: ReactKeyboardEvent) => {
    if (e.key !== 'Tab' || !stripRef.current) return;
    const stops = [...stripRef.current.querySelectorAll<HTMLElement>('button, [tabindex]')];
    const at = stops.indexOf(document.activeElement as HTMLElement);
    if ((e.shiftKey && at <= 0) || (!e.shiftKey && at === stops.length - 1)) {
      e.preventDefault();
      chipRef.current?.focus();
    }
  };

  // While open: ride the row each frame; fold when the chip leaves its list's
  // view (below the floating header), when the viewer scrolls the chat list,
  // or on Escape or window blur.
  useEffect(() => {
    if (!open) return;
    const chip0 = chipRef.current;
    const list = chip0 ? scrollRootOf(chip0) : null;
    const headerInset = Number(list?.querySelector('[data-header-inset]')?.getAttribute('data-header-inset')) || 0;
    let frame = 0;
    const tick = () => {
      const chip = chipRef.current;
      const strip = stripRef.current;
      if (!chip || !chip.isConnected) return close();
      const c = chip.getBoundingClientRect();
      const r = list?.getBoundingClientRect();
      const v = r
        ? { left: r.left, top: r.top + headerInset, right: r.right, bottom: r.bottom }
        : { left: 0, top: 0, right: window.innerWidth, bottom: window.innerHeight };
      if (chipLeftView(c, v)) return close();
      if (strip) applyPlace(strip, stripPlace(c, r ?? { left: 0, right: window.innerWidth }, window.innerWidth, 0, dir));
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      const focused = document.activeElement;
      const inside = !!focused && (chipRef.current?.contains(focused) || stripRef.current?.contains(focused));
      close();
      if (inside) chipRef.current?.focus();
    };
    // The viewer scrolling the chat list (not chat scrolling itself, and not a
    // finger jittering on the chip or strip) folds it.
    const onUserScroll = (e: Event) => {
      const t = e.target as Node | null;
      if (t && (chipRef.current?.contains(t) || stripRef.current?.contains(t))) return;
      close();
    };
    const scrollTarget: EventTarget = list ?? window;
    window.addEventListener('keydown', onKey);
    scrollTarget.addEventListener('wheel', onUserScroll, { capture: true, passive: true });
    scrollTarget.addEventListener('touchmove', onUserScroll, { capture: true, passive: true });
    window.addEventListener('blur', close);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener('keydown', onKey);
      scrollTarget.removeEventListener('wheel', onUserScroll, { capture: true });
      scrollTarget.removeEventListener('touchmove', onUserScroll, { capture: true });
      window.removeEventListener('blur', close);
    };
    // `close` reads refs and stable setters only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, dir]);

  // Unmounting (row trimmed, chatter no longer folds) folds it.
  useEffect(
    () => () => {
      clearTimer();
      releaseStrip(token);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  const stop = (e: { stopPropagation(): void }) => e.stopPropagation();
  const label = count === 1 ? '1 more badge' : `${count} more badges`;
  // A chip inside another control: the face only (after every hook).
  if (still) {
    return (
      <span className="sn-badge-fold-chip" data-tier={tier} aria-label={label}>
        <PeekFace peeks={peeks} count={count} />
      </span>
    );
  }
  return (
    <>
      <button
        ref={chipRef}
        type="button"
        data-no-row-tap=""
        data-tier={tier}
        className="sn-badge-fold-chip"
        aria-label={label}
        aria-expanded={open}
        aria-controls={open ? stripId : undefined}
        onPointerMove={
          IS_MOBILE
            ? undefined
            : (e) => {
                // A touch drag on a touchscreen laptop is not a hover (as Tooltip skips it).
                if (e.pointerType === 'touch') return;
                if (tier > 0) tiltFoil(e.currentTarget, e.clientX, e.clientY);
                if (open) return clearTimer();
                if (timer.current === null) {
                  timer.current = window.setTimeout(() => {
                    timer.current = null;
                    show(false);
                  }, OPEN_INTENT_MS);
                }
              }
        }
        onPointerLeave={
          IS_MOBILE
            ? undefined
            : (e) => {
                if (tier > 0) restFoil(e.currentTarget);
                if (open) scheduleClose();
                else clearTimer();
              }
        }
        onClick={(e) => {
          e.stopPropagation();
          // A hover-opened strip becomes held; a held one folds. Enter or Space
          // (a click with no pointer, detail 0) also moves focus into the strip.
          if (open && activeStripToken() === token && held.current) close();
          else {
            focusStrip.current = e.detail === 0;
            if (open && focusStrip.current) {
              focusStrip.current = false;
              stripRef.current?.querySelector<HTMLElement>('button, [tabindex]')?.focus();
            }
            show(true);
          }
        }}
      >
        <PeekFace peeks={peeks} count={count} />
      </button>
      {mounted &&
        typeof document !== 'undefined' &&
        createPortal(
          <AnimatePresence onExitComplete={() => setMounted(false)}>
            {open && (
              <FoldContext.Provider value={{ dir, slot: SLOT_EM(badgeScale), count }}>
                <motion.div
                  ref={stripRef}
                  id={stripId}
                  role="group"
                  aria-label="More badges"
                  className="sn-badge-fold-strip fixed"
                  style={{ fontSize, ['--sn-badge-scale' as string]: String(badgeScale), transformOrigin: dir === 1 ? 'left center' : 'right center' }}
                  variants={STRIP}
                  custom={dir}
                  initial="hidden"
                  animate="shown"
                  exit="gone"
                  onPointerEnter={IS_MOBILE ? undefined : clearTimer}
                  onPointerLeave={IS_MOBILE ? undefined : scheduleClose}
                  onPointerDownCapture={outsideHandler}
                  onPointerDown={stop}
                  onPointerMove={stop}
                  onPointerUp={stop}
                  onPointerCancel={stop}
                  onWheel={stop}
                  onTouchStart={stop}
                  onTouchMove={stop}
                  onContextMenu={stop}
                  onKeyDown={onStripKey}
                  onClick={(e) => {
                    e.stopPropagation();
                    window.setTimeout(close, 0);
                  }}
                >
                  <div className={IS_MOBILE ? 'sn-badge-fold-glass chrome-glaze chrome-glaze--control chrome-glaze--frosted' : 'sn-badge-fold-glass sn-popover'}>
                    {children()}
                  </div>
                </motion.div>
              </FoldContext.Provider>
            )}
          </AnimatePresence>,
          document.body,
        )}
    </>
  );
}

/** One unfolded badge, `index` in strip order; it springs out of the pill. */
export function BadgeFoldItem({ index, children }: { index: number; children: ReactNode }) {
  const { dir, slot, count } = useContext(FoldContext);
  // Counted from the pill: rightward the first is nearest, leftward the last.
  const i = dir === 1 ? index : count - 1 - index;
  return (
    <motion.span className="inline-flex" variants={ITEM} custom={{ i, dir, slot }}>
      {children}
    </motion.span>
  );
}
