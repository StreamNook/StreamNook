import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { ChevronDown, ChevronUp, Gamepad2, Loader2, Search, X } from 'lucide-react';

/** A game as `suggest_drop_games` and `describe_games` return it. */
interface Game {
  /** Empty when Twitch knows no game by this name. */
  id: string;
  name: string;
  box_art_url: string;
  /** Drop campaigns running for this game now; 0 when it has none. */
  drop_campaigns: number;
}

const key = (name: string) => name.toLowerCase();

/** The menu's full height; the page makes this much room under the field. */
const MENU_HEIGHT = 340;

/** The nearest ancestor that scrolls vertically, if any. */
function scrollParent(el: HTMLElement | null): HTMLElement | null {
  for (let node = el?.parentElement; node; node = node.parentElement) {
    const { overflowY } = getComputedStyle(node);
    if ((overflowY === 'auto' || overflowY === 'scroll') && node.scrollHeight > node.clientHeight) return node;
  }
  return null;
}

function BoxArt({ url, className }: { url?: string; className: string }) {
  const [failed, setFailed] = useState(false);
  if (!url || failed) {
    return (
      <span className={`${className} flex shrink-0 items-center justify-center bg-white/[0.06]`}>
        <Gamepad2 size={13} className="text-textMuted" />
      </span>
    );
  }
  return (
    <img
      src={url}
      alt=""
      loading="lazy"
      onError={() => setFailed(true)}
      className={`${className} shrink-0 bg-white/[0.06] object-cover`}
    />
  );
}

function statusLine(game: Game | undefined): { text: string; tone: string } {
  if (!game) return { text: '', tone: '' };
  if (!game.id) return { text: 'Not found on Twitch', tone: 'text-amber-300/80' };
  if (game.drop_campaigns > 0) {
    return {
      text: game.drop_campaigns === 1 ? '1 drop running now' : `${game.drop_campaigns} drops running now`,
      tone: 'text-accent',
    };
  }
  return { text: 'No drops right now', tone: 'text-textMuted' };
}

/**
 * A list of games added by search, never typed free-hand: every pick is
 * Twitch's own name for the game, so it matches drop campaigns exactly. Games
 * running drops now are suggested first, and each saved game says whether it
 * has drops right now. `numbered` marks a list whose order matters and lets
 * each row move up or down.
 */
export function GamePicker({
  items,
  numbered = false,
  placeholder = 'Search for a game',
  onChange,
}: {
  items: string[];
  numbered?: boolean;
  placeholder?: string;
  onChange: (next: string[]) => void;
}) {
  // What each saved name is, resolved by the app (art, drops, whether Twitch
  // knows it). Picks seed it, so a new row draws at once.
  const [known, setKnown] = useState<Map<string, Game>>(new Map());
  const itemsKey = items.join('\n');
  useEffect(() => {
    if (items.length === 0) return;
    let live = true;
    invoke<Game[]>('describe_games', { names: items })
      .then((games) => {
        if (!live) return;
        setKnown((prev) => {
          const next = new Map(prev);
          for (const g of games) next.set(key(g.name), g);
          return next;
        });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
    // itemsKey stands for items: refetch only when the names change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [itemsKey]);

  const [query, setQuery] = useState('');
  const [open, setOpen] = useState(false);
  const [results, setResults] = useState<Game[]>([]);
  const [searching, setSearching] = useState(false);
  const [active, setActive] = useState(0);
  const boxRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [menuStyle, setMenuStyle] = useState<CSSProperties>({});

  useEffect(() => {
    if (!open) return;
    let live = true;
    // Twitch's search runs per keystroke, so wait for typing to pause.
    const timer = setTimeout(
      () => {
        setSearching(true);
        // No cap that would hide a game: every drop game, plus Twitch's matches.
        invoke<Game[]>('suggest_drop_games', { query, limit: 300 })
          .then((list) => {
            if (!live) return;
            setResults(list);
            setActive(0);
          })
          .catch(() => live && setResults([]))
          .finally(() => live && setSearching(false));
      },
      query.trim() ? 200 : 0,
    );
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [query, open]);

  useEffect(() => {
    if (!open) return;
    const onClickOutside = (e: MouseEvent) => {
      const t = e.target as Node;
      if (boxRef.current?.contains(t) || menuRef.current?.contains(t)) return;
      setOpen(false);
    };
    document.addEventListener('mousedown', onClickOutside);
    return () => document.removeEventListener('mousedown', onClickOutside);
  }, [open]);

  // The menu is portalled to <body> with fixed coordinates: settings pages
  // scroll and clip, so an in-place menu was cut off. When it opens without
  // room below the field, the page scrolls itself to make that room, so the
  // whole list shows without scrolling the page by hand; it opens upward
  // only when the page cannot scroll far enough.
  useLayoutEffect(() => {
    if (!open) return;
    const room = () => {
      const r = boxRef.current?.getBoundingClientRect();
      return r ? window.innerHeight - r.bottom - 16 : MENU_HEIGHT;
    };
    const short = MENU_HEIGHT - room();
    const page = scrollParent(boxRef.current);
    if (short > 0 && page) {
      const canScroll = page.scrollHeight - page.clientHeight - page.scrollTop;
      if (canScroll > 0) page.scrollBy({ top: Math.min(short, canScroll), behavior: 'smooth' });
    }
    const reposition = () => {
      const el = boxRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const below = window.innerHeight - r.bottom - 16;
      const above = r.top - 16;
      const up = below < 200 && above > below;
      const space = up ? above : below;
      setMenuStyle({
        position: 'fixed',
        left: Math.round(r.left),
        width: Math.round(r.width),
        zIndex: 9999,
        maxHeight: Math.max(0, Math.min(MENU_HEIGHT, space)),
        ...(up ? { bottom: Math.round(window.innerHeight - r.top + 6) } : { top: Math.round(r.bottom + 6) }),
      });
    };
    reposition();
    const onScroll = (e: Event) => {
      if (menuRef.current?.contains(e.target as Node)) return;
      reposition();
    };
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', onScroll, true);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [open]);

  const taken = new Set(items.map(key));
  const shown = results.filter((g) => !taken.has(key(g.name)));

  const pick = (g: Game | undefined) => {
    if (!g) return;
    setKnown((prev) => new Map(prev).set(key(g.name), g));
    onChange([...items, g.name]);
    setQuery('');
  };
  const move = (from: number, to: number) => {
    if (to < 0 || to >= items.length) return;
    const next = [...items];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    onChange(next);
  };

  return (
    <div className="mt-2 space-y-2">
      {items.length > 0 && (
        <div className="space-y-1.5">
          {items.map((item, i) => {
            const game = known.get(key(item));
            const status = statusLine(game);
            return (
              <div
                key={item}
                className="group flex items-center gap-3 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2"
              >
                {numbered && (
                  <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded bg-white/5 font-mono text-[11px] text-textSecondary">
                    {i + 1}
                  </span>
                )}
                <BoxArt url={game?.box_art_url} className="h-10 w-[30px] rounded" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px] font-medium text-textPrimary">{game?.name ?? item}</div>
                  {status.text && <div className={`truncate text-[11px] ${status.tone}`}>{status.text}</div>}
                </div>
                <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
                  {numbered && (
                    <>
                      <button
                        type="button"
                        aria-label={`Move ${item} up`}
                        disabled={i === 0}
                        onClick={() => move(i, i - 1)}
                        className="rounded p-1 text-textMuted transition-colors hover:bg-white/10 hover:text-textPrimary disabled:pointer-events-none disabled:opacity-30"
                      >
                        <ChevronUp size={14} />
                      </button>
                      <button
                        type="button"
                        aria-label={`Move ${item} down`}
                        disabled={i === items.length - 1}
                        onClick={() => move(i, i + 1)}
                        className="rounded p-1 text-textMuted transition-colors hover:bg-white/10 hover:text-textPrimary disabled:pointer-events-none disabled:opacity-30"
                      >
                        <ChevronDown size={14} />
                      </button>
                    </>
                  )}
                  <button
                    type="button"
                    aria-label={`Remove ${item}`}
                    onClick={() => onChange(items.filter((_, j) => j !== i))}
                    className="rounded p-1 text-textMuted transition-colors hover:bg-red-500/10 hover:text-red-300"
                  >
                    <X size={14} />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      <div className="relative" ref={boxRef}>
        <Search size={14} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-textMuted" />
        <input
          type="text"
          value={query}
          placeholder={placeholder}
          aria-label={placeholder}
          role="combobox"
          aria-expanded={open}
          onFocus={() => setOpen(true)}
          onChange={(e) => {
            setQuery(e.target.value);
            setOpen(true);
          }}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') {
              e.preventDefault();
              setActive((a) => Math.min(a + 1, shown.length - 1));
            } else if (e.key === 'ArrowUp') {
              e.preventDefault();
              setActive((a) => Math.max(a - 1, 0));
            } else if (e.key === 'Enter') {
              e.preventDefault();
              pick(shown[active]);
            } else if (e.key === 'Escape') {
              setQuery('');
              setOpen(false);
            }
          }}
          className="glass-input w-full rounded-md py-1.5 pl-8 pr-8 text-[13px] text-textPrimary"
        />
        {searching && (
          <Loader2 size={14} className="absolute right-3 top-1/2 -translate-y-1/2 animate-spin text-accent" />
        )}
        {open &&
          createPortal(
            // The app's menu surface: its tint and frost ride the Glassiness
            // slider, solid with no blur at 0%.
            <div ref={menuRef} style={menuStyle} className="sn-popover flex flex-col overflow-hidden p-1.5">
              {shown.length > 0 ? (
                <>
                  {!query.trim() && (
                    <div className="flex items-center justify-between px-2 pb-1 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-textMuted">
                      <span>Running drops now</span>
                      <span className="tabular-nums">{shown.length}</span>
                    </div>
                  )}
                  <div role="listbox" className="min-h-0 flex-1 overflow-y-auto overscroll-contain custom-scrollbar">
                    {shown.map((g, i) => {
                      const status = statusLine(g);
                      return (
                        <button
                          key={g.id || g.name}
                          type="button"
                          role="option"
                          aria-selected={i === active}
                          onMouseEnter={() => setActive(i)}
                          onMouseDown={(e) => e.preventDefault()}
                          onClick={() => pick(g)}
                          className={`flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left transition-colors ${
                            i === active ? 'bg-white/[0.08]' : ''
                          }`}
                        >
                          <BoxArt url={g.box_art_url} className="h-10 w-[30px] rounded" />
                          <div className="min-w-0 flex-1">
                            <div className="truncate text-[13px] font-medium text-textPrimary">{g.name}</div>
                            <div className={`truncate text-[11px] ${status.tone}`}>{status.text}</div>
                          </div>
                        </button>
                      );
                    })}
                  </div>
                </>
              ) : (
                <div className="px-2 py-2 text-center text-[12px] text-textSecondary">
                  {searching
                    ? 'Searching...'
                    : query.trim().length < 2
                      ? 'Type a game name'
                      : 'No game by that name on Twitch'}
                </div>
              )}
            </div>,
            document.body,
          )}
      </div>
    </div>
  );
}
