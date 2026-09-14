// Emote suggestions while typing, as a strip above the composer.
//
// The desktop has the same idea driven by the Tab key, which a phone does not
// have, so this is its own component rather than that one with props bolted on:
// the desktop version has nothing tappable in it and lives only on that
// platform, and giving it click handling would hand desktop an affordance it
// does not have today.
//
// Movement is a native horizontal scroll with snap points rather than a drag
// handler. That is deliberate. Chromium decides whether a gesture scrolls at
// touchstart and never revisits it, so hand-rolled dragging inside a scroller
// means fighting for the gesture and losing intermittently. Letting the
// platform scroll it means momentum, snapping and edge behaviour all come for
// free and always feel right.
import React, { useEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { inlineEmoteTier, sevenTvTierUrl } from '../../services/emoteService';
import type { EmoteTabCandidate } from '../../utils/chatInputWord';

// Tile plus its gap. The snap index is derived from this, so it has to match
// the rendered width.
const TILE = 52;
const GAP = 6;
const STRIDE = TILE + GAP;

const Thumb: React.FC<{ tok: EmoteTabCandidate }> = ({ tok }) => {
  const emote = tok.emote;
  if (!emote) {
    // A chatter suggestion has no art, so it shows its name instead.
    return (
      <span className="px-1 text-[10px] font-semibold text-textSecondary text-center leading-tight line-clamp-2">
        {tok.name}
      </span>
    );
  }
  const tier = inlineEmoteTier();
  const src =
    emote.provider === '7tv'
      ? emote.localUrl || sevenTvTierUrl(emote.id, tier)
      : emote.localUrl || emote.url;
  return (
    <img
      src={src}
      alt={emote.name}
      loading="lazy"
      draggable={false}
      className="max-w-full max-h-full object-contain"
      onError={(e) => {
        const t = e.currentTarget;
        if (emote.localUrl && t.src !== emote.url) t.src = emote.url;
      }}
    />
  );
};

interface Props {
  candidates: EmoteTabCandidate[];
  onSelect: (candidate: EmoteTabCandidate) => void;
}

export const MobileEmoteCarousel: React.FC<Props> = ({ candidates, onSelect }) => {
  const scroller = useRef<HTMLDivElement>(null);
  // Stamped with the list it belongs to and compared during render, rather than
  // reset from an effect when the list changes. Writing state from an effect is
  // an error under this repo's hook rules, and it would also paint one frame of
  // the previous selection first.
  const [snap, setSnap] = useState<{ list: EmoteTabCandidate[]; index: number }>({
    list: candidates,
    index: 0,
  });
  const active = snap.list === candidates ? snap.index : 0;

  // A new query is a new list, so put it back at the front. Scroll position is
  // DOM, not state, so this stays out of the rule above.
  useEffect(() => {
    if (scroller.current) scroller.current.scrollLeft = 0;
  }, [candidates]);

  const current = candidates[active] ?? candidates[0];
  if (!current) return null;

  return (
    <motion.div
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 4 }}
      transition={{ duration: 0.12, ease: 'easeOut' }}
      className="sn-popover absolute z-[60] left-0 right-0 mb-2 !bg-background"
      style={{ bottom: '100%' }}
    >
      <div
        ref={scroller}
        onScroll={(e) => {
          const idx = Math.round(e.currentTarget.scrollLeft / STRIDE);
          setSnap({
            list: candidates,
            index: Math.max(0, Math.min(candidates.length - 1, idx)),
          });
        }}
        // Scrollbars are already hidden shell-wide under `.sn-mobile`.
        className="flex items-center overflow-x-auto px-3 py-2.5"
        style={{
          gap: GAP,
          scrollSnapType: 'x mandatory',
          // Locks the gesture to horizontal panning from the start, which is
          // the only moment Chromium reads it.
          touchAction: 'pan-x',
          // Chat text sits directly behind this; a long press that starts a
          // text selection would steal the gesture.
          WebkitTouchCallout: 'none',
        }}
      >
        {candidates.map((tok, i) => (
          <button
            key={`${tok.name}-${i}`}
            onClick={() => onSelect(tok)}
            className={`shrink-0 rounded-md flex items-center justify-center transition-colors select-none ${
              i === active ? 'bg-white/10 ring-1 ring-white/15' : 'active:bg-white/5'
            }`}
            style={{ width: TILE, height: TILE, scrollSnapAlign: 'start' }}
            aria-label={tok.name}
          >
            <Thumb tok={tok} />
          </button>
        ))}
      </div>

      <div className="px-3.5 py-1.5 border-t border-white/5 flex items-center justify-between gap-2">
        <span className="text-[12px] font-semibold text-textPrimary truncate">
          {current.name}
          {current.emote && (
            <span className="ml-1.5 text-[9px] uppercase tracking-wider text-textMuted">
              {current.emote.provider}
            </span>
          )}
          {current.chatter && (
            <span className="ml-1.5 text-[9px] uppercase tracking-wider text-textMuted">user</span>
          )}
        </span>
        <span className="text-[10px] text-textMuted shrink-0">Swipe, tap to use</span>
      </div>
    </motion.div>
  );
};
