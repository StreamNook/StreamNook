import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { ChevronLeft, ChevronRight, Gamepad2, Loader2, Plus, Search } from 'lucide-react';
import { useOutsidePress } from '../../hooks/useOutsidePress';

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

/** The menu's size. The page makes this much room under the Add tile. */
const MENU_WIDTH = 440;
const MENU_HEIGHT = 420;
/** Columns in the menu's art grid, for the arrow keys. */
const MENU_COLUMNS = 4;

/** The nearest ancestor that scrolls vertically, if any. */
function scrollParent(el: HTMLElement | null): HTMLElement | null {
  for (let node = el?.parentElement; node; node = node.parentElement) {
    const { overflowY } = getComputedStyle(node);
    if ((overflowY === 'auto' || overflowY === 'scroll') && node.scrollHeight > node.clientHeight) return node;
  }
  return null;
}

/** A game's box art at 3:4, filling its tile; a pad glyph when there is none. */
function BoxArt({ url, muted = false }: { url?: string; muted?: boolean }) {
  const [failed, setFailed] = useState(false);
  return (
    <span className="glaze-inset glaze-chip block aspect-[3/4] w-full overflow-hidden bg-black/30 [html[data-theme-tone=light]_&]:bg-black/[0.08]">
      {url && !failed ? (
        <img
          src={url}
          alt=""
          loading="lazy"
          draggable={false}
          onError={() => setFailed(true)}
          className={`h-full w-full object-cover transition-[filter,opacity] duration-200 ${muted ? 'opacity-70 grayscale-[65%]' : ''}`}
        />
      ) : (
        <span className="flex h-full w-full items-center justify-center">
          <Gamepad2 size={20} className="text-textMuted" />
        </span>
      )}
    </span>
  );
}

function statusLine(game: Game | undefined): { text: string; tone: string } {
  if (!game) return { text: '', tone: '' };
  if (!game.id) return { text: 'Not on Twitch', tone: 'text-warning' };
  if (game.drop_campaigns > 0) {
    return {
      text: game.drop_campaigns === 1 ? '1 drop now' : `${game.drop_campaigns} drops now`,
      tone: 'text-accent-on-surface',
    };
  }
  return { text: 'No drops now', tone: 'text-textMuted' };
}

/** The tiles' grid: as many columns as fit, each at least a box art's width. */
const TILE_GRID = 'grid grid-cols-[repeat(auto-fill,minmax(92px,1fr))] gap-x-3 gap-y-4';

/** The small capsule actions in a tile's chin. */
const TILE_ACTION =
  'inline-flex h-6 items-center justify-center rounded-full px-1.5 text-[11.5px] font-semibold text-textSecondary transition-colors hover:bg-[color-mix(in_srgb,var(--color-text-primary)_10%,transparent)] hover:text-textPrimary disabled:pointer-events-none disabled:opacity-30';

/**
 * A list of games added from Twitch's own categories, never typed free-hand:
 * every pick is Twitch's name for the game, so it matches drop campaigns
 * exactly. Saved games show as box-art tiles, the name and whether it has
 * drops now in the chin under the art (nothing is drawn over the art); a
 * tile's chin swaps to its actions under the pointer or focus, in the same
 * line, so nothing moves. The last tile opens the picker: a glass menu with a
 * search field over a grid of box art, games running drops now first.
 * `numbered` marks a list whose order matters and lets each tile move earlier
 * or later; `muted` draws the art faded, for a list of games to leave out.
 */
export function GamePicker({
  items,
  numbered = false,
  muted = false,
  placeholder = 'Search for a game',
  addLabel = 'Add a game',
  onChange,
}: {
  items: string[];
  numbered?: boolean;
  muted?: boolean;
  placeholder?: string;
  /** The Add tile's words. */
  addLabel?: string;
  onChange: (next: string[]) => void;
}) {
  // What each saved name is, resolved by the app (art, drops, whether Twitch
  // knows it). Picks seed it, so a new tile draws at once.
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
  const addRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const [menuStyle, setMenuStyle] = useState<CSSProperties>({ visibility: 'hidden' });

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

  const close = () => {
    setOpen(false);
    setQuery('');
  };

  // The app's one way a menu closes on a press anywhere else. The Add tile is
  // inside, so pressing it toggles instead of closing and reopening.
  const onMenuPress = useOutsidePress(open, [menuRef, addRef], close);

  // The menu is portalled to <body> with fixed coordinates: settings pages
  // scroll and clip, so an in-place menu was cut off. When it opens without
  // room below the Add tile, the page scrolls itself to make that room; it
  // opens upward only when the page cannot scroll far enough.
  useLayoutEffect(() => {
    if (!open) return;
    const room = () => {
      const r = addRef.current?.getBoundingClientRect();
      return r ? window.innerHeight - r.bottom - 16 : MENU_HEIGHT;
    };
    const short = MENU_HEIGHT - room();
    const page = scrollParent(addRef.current);
    if (short > 0 && page) {
      const canScroll = page.scrollHeight - page.clientHeight - page.scrollTop;
      if (canScroll > 0) page.scrollBy({ top: Math.min(short, canScroll), behavior: 'smooth' });
    }
    const reposition = () => {
      const el = addRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const width = Math.min(MENU_WIDTH, window.innerWidth - 32);
      const below = window.innerHeight - r.bottom - 16;
      const above = r.top - 16;
      const up = below < 260 && above > below;
      const space = up ? above : below;
      setMenuStyle({
        position: 'fixed',
        left: Math.round(Math.max(16, Math.min(r.left, window.innerWidth - width - 16))),
        width: Math.round(width),
        zIndex: 9999,
        maxHeight: Math.max(0, Math.min(MENU_HEIGHT, space)),
        ...(up ? { bottom: Math.round(window.innerHeight - r.top + 8) } : { top: Math.round(r.bottom + 8) }),
      });
    };
    reposition();
    inputRef.current?.focus();
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

  // Keep the keyboard's pick in view as the arrows move it through the grid.
  useEffect(() => {
    if (!open) return;
    gridRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [active, open]);

  const pick = (g: Game | undefined) => {
    if (!g) return;
    setKnown((prev) => new Map(prev).set(key(g.name), g));
    onChange([...items, g.name]);
    // The menu stays open, so several games go in one visit; the field clears.
    setQuery('');
    inputRef.current?.focus();
  };
  const move = (from: number, to: number) => {
    if (to < 0 || to >= items.length) return;
    const next = [...items];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    onChange(next);
  };
  const step = (by: number) => setActive((a) => Math.max(0, Math.min(shown.length - 1, a + by)));

  const heading = query.trim() ? (searching ? 'Searching' : 'Matches') : 'Running drops now';

  return (
    <div className={`mt-3 ${TILE_GRID}`}>
      {items.map((item, i) => {
        const game = known.get(key(item));
        const status = statusLine(game);
        const name = game?.name ?? item;
        return (
          <div key={item} className="group min-w-0" data-game-tile={name}>
            <BoxArt url={game?.box_art_url} muted={muted} />
            {/* Two lines kept for every name, so a row's status lines align. */}
            <div className="mt-2 flex min-h-[2lh] min-w-0 items-start gap-1.5 text-[12.5px] leading-tight">
              {numbered && (
                <span className="shrink-0 text-[11.5px] font-semibold tabular-nums text-textMuted">{i + 1}</span>
              )}
              <span className="line-clamp-2 min-w-0 break-words text-[12.5px] font-semibold leading-tight text-textPrimary">
                {name}
              </span>
            </div>
            {/* The status and the actions share one line, cross-fading, so the
                tile keeps its height whichever shows. */}
            <div className="mt-0.5 grid h-6 items-center">
              <span
                className={`truncate text-[11.5px] transition-opacity duration-150 [grid-area:1/1] group-focus-within:opacity-0 group-hover:opacity-0 ${status.tone}`}
              >
                {status.text}
              </span>
              <span className="-ml-1.5 flex items-center opacity-0 transition-opacity duration-150 [grid-area:1/1] group-focus-within:opacity-100 group-hover:opacity-100">
                {numbered && (
                  <>
                    <button
                      type="button"
                      aria-label={`Move ${name} earlier`}
                      disabled={i === 0}
                      onClick={() => move(i, i - 1)}
                      className={TILE_ACTION}
                    >
                      <ChevronLeft size={14} />
                    </button>
                    <button
                      type="button"
                      aria-label={`Move ${name} later`}
                      disabled={i === items.length - 1}
                      onClick={() => move(i, i + 1)}
                      className={TILE_ACTION}
                    >
                      <ChevronRight size={14} />
                    </button>
                  </>
                )}
                <button
                  type="button"
                  aria-label={`Remove ${name}`}
                  onClick={() => onChange(items.filter((_, j) => j !== i))}
                  className={`${TILE_ACTION} hover:!text-error`}
                >
                  Remove
                </button>
              </span>
            </div>
          </div>
        );
      })}

      <div className="min-w-0">
        <button
          ref={addRef}
          type="button"
          aria-haspopup="dialog"
          aria-expanded={open}
          onClick={() => (open ? close() : setOpen(true))}
          className={`glaze-chip flex aspect-[3/4] w-full flex-col items-center justify-center gap-1.5 border border-dashed text-textSecondary transition-colors hover:border-solid hover:text-textPrimary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--color-text-primary)] ${
            open
              ? 'border-solid border-[color-mix(in_srgb,var(--color-accent)_55%,transparent)] bg-[color-mix(in_srgb,var(--color-accent)_10%,transparent)] text-textPrimary'
              : 'border-[color-mix(in_srgb,var(--color-text-primary)_22%,transparent)] hover:border-[color-mix(in_srgb,var(--color-text-primary)_32%,transparent)] hover:bg-[color-mix(in_srgb,var(--color-text-primary)_5%,transparent)]'
          }`}
        >
          <Plus size={20} />
        </button>
        <div className="mt-2 text-[12.5px] font-semibold leading-tight text-textSecondary">{addLabel}</div>
      </div>

      {open &&
        createPortal(
          // The menu glass: near-opaque at every Glassiness, solid at 0%.
          <div
            ref={menuRef}
            role="dialog"
            aria-label={addLabel}
            onPointerDownCapture={onMenuPress}
            style={menuStyle}
            className="glass-flyout flex flex-col overflow-hidden rounded-2xl animate-in fade-in"
          >
            <div className="relative shrink-0 p-1.5">
              <Search size={14} className="pointer-events-none absolute left-[18px] top-1/2 -translate-y-1/2 text-textMuted" />
              <input
                ref={inputRef}
                type="text"
                value={query}
                placeholder={placeholder}
                aria-label={placeholder}
                role="combobox"
                aria-expanded
                aria-controls="game-picker-grid"
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'ArrowRight') {
                    if (!query) {
                      e.preventDefault();
                      step(1);
                    }
                  } else if (e.key === 'ArrowLeft') {
                    if (!query) {
                      e.preventDefault();
                      step(-1);
                    }
                  } else if (e.key === 'ArrowDown') {
                    e.preventDefault();
                    step(MENU_COLUMNS);
                  } else if (e.key === 'ArrowUp') {
                    e.preventDefault();
                    step(-MENU_COLUMNS);
                  } else if (e.key === 'Enter') {
                    e.preventDefault();
                    pick(shown[active]);
                  } else if (e.key === 'Escape') {
                    e.preventDefault();
                    close();
                    addRef.current?.focus();
                  }
                }}
                className="glass-input w-full rounded-full py-2 pl-9 pr-9 text-[13px] text-textPrimary"
              />
              {searching && (
                <Loader2 size={14} className="absolute right-[18px] top-1/2 -translate-y-1/2 animate-spin text-accent" />
              )}
            </div>
            {shown.length > 0 ? (
              <>
                <div className="flex shrink-0 items-center justify-between px-3 pb-1.5 pt-1 text-[11px] font-semibold uppercase tracking-[0.12em] text-textMuted">
                  <span>{heading}</span>
                  <span className="tabular-nums">{shown.length}</span>
                </div>
                <div
                  ref={gridRef}
                  id="game-picker-grid"
                  role="listbox"
                  className="grid min-h-0 flex-1 grid-cols-4 content-start gap-x-1 gap-y-1 overflow-y-auto overscroll-contain px-1.5 pb-1.5 custom-scrollbar"
                >
                  {shown.map((g, i) => {
                    const status = statusLine(g);
                    return (
                      <button
                        key={g.id || g.name}
                        type="button"
                        role="option"
                        data-index={i}
                        aria-selected={i === active}
                        onMouseEnter={() => setActive(i)}
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={() => pick(g)}
                        className={`min-w-0 rounded-xl p-1.5 text-left transition-colors ${
                          i === active ? 'bg-white/[0.08]' : ''
                        }`}
                      >
                        <BoxArt url={g.box_art_url} />
                        <span className="mt-1.5 line-clamp-2 block break-words text-[12px] font-semibold leading-tight text-textPrimary">
                          {g.name}
                        </span>
                        <span className={`mt-0.5 block truncate text-[11px] ${status.tone}`}>{status.text}</span>
                      </button>
                    );
                  })}
                </div>
              </>
            ) : (
              <div className="px-3 pb-5 pt-4 text-center text-[12.5px] text-textSecondary">
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
  );
}
