import { useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion, stagger } from 'framer-motion';
import { ProviderMark } from './ProviderLogo';
import { Tooltip } from './ui/Tooltip';
import { useOutsidePress } from '../hooks/useOutsidePress';
import { PROVIDERS, WATCHABLE_PROVIDERS, type ProviderId } from '../types/providers';
import { setSuggestOff, suggestOn, toggledOff, useSuggestOff } from '../utils/suggestPlatforms';

// The badge fold's motion (chat/BadgeFold.tsx), so both pop-outs feel alike.
const EASE = [0.2, 0.8, 0.2, 1] as const;
const OVERSHOOT = [0.2, 0.9, 0.25, 1.15] as const;
const OPEN_INTENT_MS = 60;
const CLOSE_GRACE_MS = 150;
// A strip slot: a 28px button and a 2px gap.
const SLOT_PX = 30;

// Centered on the bar through `y`: motion writes the transform inline, so a
// translate class would be overwritten.
const STRIP = {
  hidden: { opacity: 0, y: '-50%', scale: 0.96 },
  shown: {
    opacity: 1,
    y: '-50%',
    scale: 1,
    transition: { duration: 0.18, ease: EASE, opacity: { duration: 0.06, ease: 'linear' as const }, delayChildren: stagger(0.025) },
  },
  gone: { opacity: 0, y: '-50%', scale: 0.96, transition: { duration: 0.12, ease: EASE, opacity: { duration: 0.06, delay: 0.06 } } },
};

// Each logo starts back at the stack and springs out to its place: `i` slots
// from the stack, toward `dir` (1 rightward from a stack on the left, -1
// leftward from one on the right).
interface ItemCustom {
  i: number;
  dir: 1 | -1;
}
const ITEM = {
  hidden: ({ i, dir }: ItemCustom) => ({ opacity: 0, x: -dir * (i + 1) * SLOT_PX * 0.5, scale: 0.6 }),
  shown: { opacity: 1, x: 0, scale: 1, transition: { duration: 0.26, ease: OVERSHOOT } },
  gone: ({ i, dir }: ItemCustom) => ({ opacity: 0, x: -dir * (i + 1) * SLOT_PX * 0.25, scale: 0.8, transition: { duration: 0.12, ease: EASE } }),
};

// Coin and mark sizes: the search bar's, and the smaller sidebar header's.
const SIZES = {
  md: { coin: 22, mark: 13, overlap: 5, item: 'h-7 w-7', itemMark: 15 },
  sm: { coin: 18, mark: 11, overlap: 4, item: 'h-6 w-6', itemMark: 13 },
} as const;

/**
 * Which platforms the All view searches and suggests from, inside the search
 * bar. Closed, the platforms that are on sit as a stack of coins; a short
 * hover (or a click, which keeps it open) fans every platform out of the
 * stack, and a press turns one on or off. At least one stays on. The choice
 * is Rust's (`set_suggest_platforms`), so the sidebar's Recommended list and
 * Home's Discover grid follow it too.
 *
 * `onChange` hears each press: the platforms now off, and the one turned back
 * on (null when one was turned off), so results on screen can follow.
 * `align="end"` is for a stack at a row's right end (the sidebar's Recommended
 * header): the logos fan out leftward, staying inside the row.
 */
export function PlatformStackPicker({
  onChange,
  size = 'md',
  align = 'start',
}: {
  onChange?: (off: ProviderId[], turnedOn: ProviderId | null) => void;
  size?: keyof typeof SIZES;
  align?: 'start' | 'end';
}) {
  const s = SIZES[size];
  const dir: 1 | -1 = align === 'end' ? -1 : 1;
  const off = useSuggestOff();
  const on = suggestOn(off);
  const [open, setOpen] = useState(false);
  const held = useRef(false);
  const timer = useRef<number | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const stackRef = useRef<HTMLButtonElement>(null);

  const clearTimer = () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
  };
  const close = () => {
    clearTimer();
    held.current = false;
    setOpen(false);
  };
  const scheduleClose = () => {
    clearTimer();
    if (held.current) return;
    timer.current = window.setTimeout(close, CLOSE_GRACE_MS);
  };
  const outsideHandler = useOutsidePress(open, [rootRef], close);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      e.preventDefault();
      close();
      stackRef.current?.focus();
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('blur', close);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('blur', close);
    };
    // `close` reads refs and stable setters only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => clearTimer, []);

  const press = (provider: ProviderId) => {
    const next = toggledOff(off, provider);
    if (next.length === off.length && next.every((p) => off.includes(p))) return;
    const turnedOn = off.includes(provider);
    void setSuggestOff(next);
    onChange?.(next, turnedOn ? provider : null);
  };

  const names = on.map((p) => PROVIDERS[p].label).join(', ');
  return (
    <div
      ref={rootRef}
      className="relative flex h-full items-center"
      onPointerDownCapture={outsideHandler}
      onPointerLeave={(e) => {
        if (e.pointerType === 'touch') return;
        if (open) scheduleClose();
        else clearTimer();
      }}
      onPointerEnter={() => {
        if (open) clearTimer();
      }}
    >
      <button
        ref={stackRef}
        type="button"
        aria-label={`Platforms on: ${names}`}
        aria-expanded={open}
        onMouseDown={(e) => e.preventDefault()}
        onPointerMove={(e) => {
          if (e.pointerType === 'touch' || open || timer.current !== null) return;
          timer.current = window.setTimeout(() => {
            timer.current = null;
            setOpen(true);
          }, OPEN_INTENT_MS);
        }}
        onClick={() => {
          clearTimer();
          if (open && held.current) return close();
          held.current = true;
          setOpen(true);
        }}
        className={`flex items-center rounded-lg transition-opacity duration-150 hover:bg-white/5 ${size === 'sm' ? 'h-6 px-1' : 'h-7 px-1.5'} ${open ? 'opacity-0' : size === 'sm' ? 'opacity-75 hover:opacity-100' : 'opacity-100'}`}
      >
        {on.map((p, i) => (
          <span
            key={p}
            className="relative flex items-center justify-center rounded-full bg-black/70 shadow-[0_0_0_1.5px_rgba(0,0,0,0.55)]"
            style={{ width: s.coin, height: s.coin, marginLeft: i === 0 ? 0 : -s.overlap, zIndex: on.length - i }}
          >
            <ProviderMark provider={p} size={s.mark} />
          </span>
        ))}
      </button>

      <AnimatePresence>
        {open && (
          <motion.div
            role="group"
            aria-label="Platforms to search and suggest from"
            className={`sn-popover absolute top-1/2 z-10 flex items-center gap-0.5 rounded-[10px] p-0.5 ${align === 'end' ? 'right-0' : 'left-0'}`}
            style={{ transformOrigin: align === 'end' ? 'right center' : 'left center' }}
            variants={STRIP}
            initial="hidden"
            animate="shown"
            exit="gone"
          >
            {WATCHABLE_PROVIDERS.map((p, i) => {
              const isOn = !off.includes(p);
              const last = isOn && on.length === 1;
              const label = PROVIDERS[p].label;
              const tip = last ? `${label} stays on: one platform always does` : isOn ? `${label} is on` : `${label} is off: not searched or suggested`;
              return (
                <Tooltip key={p} content={tip} side="bottom" delay={250}>
                <motion.button
                  type="button"
                  custom={{ i: dir === 1 ? i : WATCHABLE_PROVIDERS.length - 1 - i, dir }}
                  variants={ITEM}
                  aria-pressed={isOn}
                  aria-label={label}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => press(p)}
                  className={`relative flex ${s.item} items-center justify-center rounded-lg transition-colors duration-150 ${isOn ? 'bg-white/10 hover:bg-white/15' : 'hover:bg-white/10'} ${last ? 'cursor-default' : ''}`}
                >
                  <span className={`-translate-y-px transition-[opacity,filter] duration-150 ${isOn ? '' : 'opacity-60 grayscale brightness-200'}`}>
                    <ProviderMark provider={p} size={s.itemMark} />
                  </span>
                  <span
                    aria-hidden="true"
                    className={`absolute bottom-[2px] left-1/2 h-1 w-1 -translate-x-1/2 rounded-full bg-textPrimary transition-opacity duration-150 ${isOn ? 'opacity-80' : 'opacity-0'}`}
                  />
                </motion.button>
                </Tooltip>
              );
            })}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
