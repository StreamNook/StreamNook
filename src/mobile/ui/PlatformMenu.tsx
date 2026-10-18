// The platform choice in a screen's title row, so it spends no row of its own:
// the chosen platform's logo with a small chevron saying it opens, and a small
// glass menu hung from it listing every option with its logo and a check on
// the chosen one. Following filters with it, Browse switches platform with it.
//
// A menu rather than a sheet: it holds three short rows, and a sheet rising
// from the bottom for that pulls the eye away from the title it belongs to.
import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { CaretDown, Check } from 'phosphor-react';
import { ProviderMark } from '../../components/ProviderLogo';
import type { ProviderId } from '../../types/providers';
import { useMobileNavStore } from '../navStore';

export interface PlatformOption<T extends string> {
  value: T;
  label: string;
  /** The logos that stand for it; "All platforms" wears every one. */
  marks: ProviderId[];
}

export const ALL_PLATFORMS: PlatformOption<'all'> = { value: 'all', label: 'All platforms', marks: ['twitch', 'kick'] };
export const TWITCH_PLATFORM: PlatformOption<'twitch'> = { value: 'twitch', label: 'Twitch', marks: ['twitch'] };
export const KICK_PLATFORM: PlatformOption<'kick'> = { value: 'kick', label: 'Kick', marks: ['kick'] };

/** One or more marks side by side, a pair 3px apart. These marks' ink fills
 *  their whole box (the Kick K runs edge to edge), so the overlap the iPhone's
 *  vector marks take made the two touch. */
export const PlatformMarks: React.FC<{ marks: ProviderId[]; size?: number }> = ({ marks, size = 18 }) => {
  const pair = marks.length > 1;
  const each = pair ? Math.round(size * 0.82) : size;
  return (
    <span className="flex items-center" aria-hidden>
      {marks.map((m, i) => (
        <span key={m} style={i > 0 ? { marginLeft: 3 } : undefined}>
          <ProviderMark provider={m} size={each} />
        </span>
      ))}
    </span>
  );
};

// The menu portals into <body>, outside .sn-mobile where --sn-safe-* live.
const SAFE_R = 'var(--sn-inset-r, env(safe-area-inset-right, 0px))';

export function PlatformMenu<T extends string>({
  options,
  value,
  onChange,
  size = 18,
}: {
  options: PlatformOption<T>[];
  value: T;
  onChange: (next: T) => void;
  size?: number;
}) {
  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState<{ top: number; right: number } | null>(null);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const menuId = useId();
  const pushSheet = useMobileNavStore((s) => s.pushSheet);
  const popSheet = useMobileNavStore((s) => s.popSheet);
  const chosen = options.find((o) => o.value === value) ?? options[0];

  const close = useCallback(() => setOpen(false), []);

  // Android back closes the menu before it leaves the screen, as it does a sheet.
  useEffect(() => {
    if (!open) return;
    pushSheet(menuId);
    const onBack = (e: Event) => {
      if ((e as CustomEvent<string>).detail === menuId) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('sn:close-sheet', onBack);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('sn:close-sheet', onBack);
      window.removeEventListener('keydown', onKey);
      popSheet(menuId);
    };
  }, [open, menuId, pushSheet, popSheet, close]);

  const pick = (next: T) => {
    setOpen(false);
    if (next !== value) onChange(next);
  };

  return (
    <>
      <button
        ref={trigger}
        onClick={() => {
          if (open) {
            close();
            return;
          }
          // Hung from the trigger's measured edge, so it lines up with the logo
          // on any title row without each screen passing a position.
          const r = trigger.current?.getBoundingClientRect();
          if (!r) return;
          setAnchor({ top: r.bottom + 4, right: Math.max(0, window.innerWidth - r.right) });
          setOpen(true);
        }}
        className="sn-touch flex items-center justify-center gap-1 px-1 active:opacity-70 transition-opacity"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Platform: ${chosen.label}`}
      >
        <PlatformMarks marks={chosen.marks} size={size} />
        <CaretDown size={10} weight="bold" className="text-textSecondary" />
      </button>
      {createPortal(
        <AnimatePresence>
          {open && anchor && (
            <motion.div
              key="platform-menu"
              className="fixed inset-0 z-[9000]"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.12, ease: 'linear' }}
            >
              {/* Clear: a tap anywhere outside closes the menu and does
                  nothing else, so it never lands on a card behind it. */}
              <div className="absolute inset-0" onClick={close} />
              <motion.div
                role="menu"
                aria-label="Platform"
                className="sn-menu absolute min-w-[210px] origin-top-right"
                style={{
                  top: anchor.top,
                  right: `max(${anchor.right}px, calc(${SAFE_R} + 12px))`,
                }}
                initial={{ scale: 0.94, y: -4 }}
                animate={{ scale: 1, y: 0 }}
                exit={{ scale: 0.94, y: -4 }}
                transition={{ type: 'spring', stiffness: 560, damping: 40, mass: 0.6 }}
              >
                {options.map((o) => {
                  const on = o.value === value;
                  return (
                    <button
                      key={o.value}
                      role="menuitemradio"
                      aria-checked={on}
                      onClick={() => pick(o.value)}
                      className="sn-menu-row"
                    >
                      {/* A fixed slot, so the pair and the single marks put
                          their labels on one line. */}
                      <span className="flex w-[30px] shrink-0 items-center justify-center">
                        <PlatformMarks marks={o.marks} size={size} />
                      </span>
                      <span className="flex-1 truncate">{o.label}</span>
                      {/* The slot stays when empty, so the menu is one width
                          whichever row is chosen. */}
                      <span className="flex w-4 shrink-0 items-center justify-center text-accent">
                        {on && <Check size={16} weight="bold" />}
                      </span>
                    </button>
                  );
                })}
              </motion.div>
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}
    </>
  );
}
