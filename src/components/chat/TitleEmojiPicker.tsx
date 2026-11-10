import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, ReactNode, RefObject } from 'react';
import { createPortal } from 'react-dom';
import { motion } from 'framer-motion';
import { Clock, Search } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { usePerformanceQuiet } from '../../utils/lowPower';
import { getAppleEmojiUrl, vendorEmojiUrl } from '../../services/emojiService';
import { Tooltip } from '../ui/Tooltip';

/**
 * The emoji menu for a stream title. Twitch titles take Unicode emoji, never
 * Twitch emotes, so this is the emote menu's emoji set on its own: the row
 * Rust picked (the title's own emoji and the ones used in titles before),
 * then the categories, with the same search words. A pick goes in at the
 * title's caret and the menu stays open for the next one.
 *
 * It draws itself on the page (portaled, fixed under its button) so the
 * editor's scrolling body never clips it; React carries its presses back to
 * the editor, which therefore counts them as inside.
 */

type EmojiData = {
  EMOJI_CATEGORIES: Record<string, string[]>;
  EMOJI_KEYWORDS: Record<string, string[]>;
};

// The emoji set is ~110 KB of source: loaded on first open, kept after.
let emojiDataCache: EmojiData | null = null;

/** Each category's tab glyph. */
const TAB_GLYPH: Record<string, string> = {
  Smileys: '😀',
  People: '👋',
  Animals: '🐶',
  Food: '🍕',
  Travel: '🚗',
  Activities: '⚽',
  Objects: '💡',
  Symbols: '💜',
  Flags: '🏁',
};

const YOURS = 'yours';
const PICKER_W = 336;
/** About the menu's height, to decide whether it opens below or above. */
const PICKER_H = 380;
const SEARCH_CAP = 160;

interface Props {
  /** The title's emoji button: the menu sits under it and it toggles the menu. */
  anchorRef: RefObject<HTMLElement | null>;
  recent: string[];
  recentHeading: string;
  onPick: (emoji: string) => void;
  onClose: () => void;
}

export function TitleEmojiPicker({ anchorRef, recent, recentHeading, onPick, onClose }: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [data, setData] = useState<EmojiData | null>(emojiDataCache);
  const [tab, setTab] = useState(YOURS);
  const [query, setQuery] = useState('');
  const [place, setPlace] = useState<CSSProperties | null>(null);
  const style = useAppStore((s) => s.settings.chat_design?.emoji_style ?? 'apple');
  const motionOff = useAppStore((s) => s.settings.motion_mode) === 'off';
  const perfQuiet = usePerformanceQuiet();
  const instant = motionOff || perfQuiet;

  const markInside = useOutsidePress(true, [rootRef, anchorRef], onClose);

  useEffect(() => {
    if (data) return;
    let alive = true;
    void import('../../services/emojiCategories').then((mod) => {
      emojiDataCache = mod;
      if (alive) setData(mod);
    });
    return () => {
      alive = false;
    };
  }, [data]);

  // Under the button, its right edge on the button's; above it when the
  // window has no room below.
  useLayoutEffect(() => {
    const placeIt = () => {
      const r = anchorRef.current?.getBoundingClientRect();
      if (!r) return;
      // Edges on the title box's edges (the button sits 7 px inside it).
      const right = Math.max(8, window.innerWidth - r.right - 7);
      const below = window.innerHeight - r.bottom >= PICKER_H + 12 || r.top < PICKER_H + 12;
      setPlace(below ? { top: r.bottom + 12, right } : { bottom: window.innerHeight - r.top + 8, right });
    };
    placeIt();
    window.addEventListener('resize', placeIt);
    window.addEventListener('scroll', placeIt, true);
    return () => {
      window.removeEventListener('resize', placeIt);
      window.removeEventListener('scroll', placeIt, true);
    };
  }, [anchorRef]);

  const found = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q || !data) return null;
    const out: string[] = [];
    for (const [category, list] of Object.entries(data.EMOJI_CATEGORIES)) {
      const all = category.toLowerCase().includes(q);
      for (const e of list) {
        if (out.length === SEARCH_CAP) return out;
        if (all || e.includes(q) || data.EMOJI_KEYWORDS[e]?.some((k) => k.includes(q))) out.push(e);
      }
    }
    return out;
  }, [query, data]);

  const srcFor = (emoji: string) =>
    style === 'apple' || style === 'system' ? getAppleEmojiUrl(emoji) : (vendorEmojiUrl(emoji, style) ?? getAppleEmojiUrl(emoji));

  const cell = (emoji: string, key: string) => (
    <button
      key={key}
      type="button"
      aria-label={emoji}
      // Keeps the caret where it is (the title, or the search box).
      onMouseDown={(e) => e.preventDefault()}
      onClick={() => onPick(emoji)}
      className="grid h-9 place-items-center rounded-lg transition-colors hover:bg-white/[0.09] focus-visible:bg-white/[0.09] outline-none"
    >
      {style === 'system' ? (
        <span className="text-[22px] leading-none">{emoji}</span>
      ) : (
        <img
          src={srcFor(emoji)}
          alt={emoji}
          draggable={false}
          loading="lazy"
          decoding="async"
          className="h-[26px] w-[26px] object-contain"
          onError={(e) => {
            const t = e.currentTarget;
            if (!t.dataset.fe0f && t.src.endsWith('.png') && !t.src.includes('-fe0f')) {
              t.dataset.fe0f = '1';
              t.src = t.src.replace(/\.png$/, '-fe0f.png');
              return;
            }
            t.style.display = 'none';
            if (t.nextSibling?.textContent !== emoji) t.insertAdjacentText('afterend', emoji);
          }}
        />
      )}
    </button>
  );

  const heading = (text: string) => (
    <p className="mx-0.5 mb-1 mt-2.5 text-[11px] font-semibold uppercase tracking-wider text-textSecondary">{text}</p>
  );
  const grid = (list: string[], prefix: string) => (
    <div className="grid grid-cols-8 gap-0.5">{list.map((e, i) => cell(e, `${prefix}-${i}`))}</div>
  );

  const categories = data ? Object.keys(data.EMOJI_CATEGORIES) : [];

  let body;
  if (found) {
    body = found.length ? (
      <>
        {heading('Results')}
        {grid(found, 'q')}
      </>
    ) : (
      <p className="px-1 py-6 text-center text-[13px] text-textSecondary">No emoji match that</p>
    );
  } else if (tab === YOURS) {
    body = (
      <>
        {heading(recentHeading)}
        {grid(recent, 'r')}
        {data && (
          <>
            {heading('Smileys')}
            {grid(data.EMOJI_CATEGORIES.Smileys ?? [], 's')}
          </>
        )}
      </>
    );
  } else {
    body = (
      <>
        {heading(tab)}
        {grid(data?.EMOJI_CATEGORIES[tab] ?? [], tab)}
      </>
    );
  }

  if (!place) return null;
  return createPortal(
    <motion.div
      ref={rootRef}
      onPointerDownCapture={markInside}
      onKeyDown={(e) => {
        if (e.key !== 'Escape') return;
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }}
      role="dialog"
      aria-label="Emoji for your title"
      initial={{ opacity: 0, y: -4, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={instant ? { duration: 0 } : { duration: 0.16, ease: [0.2, 0.8, 0.2, 1] }}
      style={{ ...place, width: PICKER_W }}
      className="glass-flyout fixed z-[calc(var(--sn-z-over-player-lift,0)_+_210)] flex flex-col rounded-xl !p-2.5"
    >
      <label className="glass-input flex h-[34px] items-center gap-2 !rounded-lg px-2.5 text-textSecondary">
        <Search size={15} className="flex-shrink-0" aria-hidden />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search emoji"
          className="min-w-0 flex-1 bg-transparent text-[13.5px] text-textPrimary placeholder-textSecondary outline-none"
        />
      </label>
      <div
        className="custom-scrollbar -mr-1.5 mt-0.5 max-h-[216px] overflow-y-auto pb-2 pr-1.5"
        style={{ maskImage: 'linear-gradient(to bottom, black calc(100% - 18px), transparent)' }}
      >
        {body}
      </div>
      {!found && (
        <div className="mt-2 flex justify-between rounded-[10px] bg-white/[0.035] p-1">
          <TabButton label={recentHeading} on={tab === YOURS} onClick={() => setTab(YOURS)}>
            <Clock size={15} className="text-textSecondary" aria-hidden />
          </TabButton>
          {categories.map((c) => (
            <TabButton key={c} label={c} on={tab === c} onClick={() => setTab(c)}>
              {style === 'system' ? (
                <span className="text-[15px] leading-none">{TAB_GLYPH[c] ?? c[0]}</span>
              ) : (
                <img src={srcFor(TAB_GLYPH[c] ?? '')} alt="" draggable={false} className="h-[18px] w-[18px]" />
              )}
            </TabButton>
          ))}
        </div>
      )}
      <p className="mt-2 border-t border-white/[0.05] px-1 pt-2 text-[12px] leading-snug text-textSecondary/80">
        Twitch titles take emoji, not Twitch emotes.
      </p>
    </motion.div>,
    document.body,
  );
}

function TabButton({ label, on, onClick, children }: { label: string; on: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <Tooltip content={label} side="top">
      <button
        type="button"
        aria-label={label}
        aria-pressed={on}
        onMouseDown={(e) => e.preventDefault()}
        onClick={onClick}
        className={`grid h-7 w-8 place-items-center rounded-[7px] transition-colors ${
          on ? 'bg-white/[0.12] shadow-[inset_0_1px_0_rgba(255,255,255,0.1)]' : 'hover:bg-white/[0.06]'
        }`}
      >
        {children}
      </button>
    </Tooltip>
  );
}

export default TitleEmojiPicker;
