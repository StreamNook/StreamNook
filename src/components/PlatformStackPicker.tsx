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

// Each logo starts back at the stack and springs out to its place.
const ITEM = {
  hidden: (i: number) => ({ opacity: 0, x: -(i + 1) * SLOT_PX * 0.5, scale: 0.6 }),
  shown: { opacity: 1, x: 0, scale: 1, transition: { duration: 0.26, ease: OVERSHOOT } },
  gone: (i: number) => ({ opacity: 0, x: -(i + 1) * SLOT_PX * 0.25, scale: 0.8, transition: { duration: 0.12, ease: EASE } }),
};

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
 */
export function PlatformStackPicker({ onChange }: { onChange?: (off: ProviderId[], turnedOn: ProviderId | null) => void }) {
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
        aria-label={`Searching ${names}`}
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
        className={`flex h-7 items-center rounded-lg px-1.5 transition-opacity duration-150 hover:bg-white/5 ${open ? 'opacity-0' : 'opacity-100'}`}
      >
        {on.map((p, i) => (
          <span
            key={p}
            className="relative flex h-[22px] w-[22px] items-center justify-center rounded-full bg-black/70 shadow-[0_0_0_1.5px_rgba(0,0,0,0.55)]"
            style={{ marginLeft: i === 0 ? 0 : -5, zIndex: on.length - i }}
          >
            <ProviderMark provider={p} size={13} />
          </span>
        ))}
      </button>

      <AnimatePresence>
        {open && (
          <motion.div
            role="group"
            aria-label="Platforms to search and suggest from"
            className="sn-popover absolute left-0 top-1/2 z-10 flex items-center gap-0.5 rounded-[10px] p-0.5"
            style={{ transformOrigin: 'left center' }}
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
                  custom={i}
                  variants={ITEM}
                  aria-pressed={isOn}
                  aria-label={label}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => press(p)}
                  className={`relative flex h-7 w-7 items-center justify-center rounded-lg transition-colors duration-150 ${isOn ? 'bg-white/10 hover:bg-white/15' : 'hover:bg-white/10'} ${last ? 'cursor-default' : ''}`}
                >
                  <span className={`-translate-y-px transition-[opacity,filter] duration-150 ${isOn ? '' : 'opacity-60 grayscale brightness-200'}`}>
                    <ProviderMark provider={p} size={15} />
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
