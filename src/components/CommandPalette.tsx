// CommandPalette — global Ctrl/Cmd+K command surface.
//
// Centered glass modal: search input on top, sectioned results below, hint
// row at the bottom. Behaviour:
//
//   - Empty query: suggestions (recent commands + a slice of quick actions).
//   - Typed query: matches the full static catalog (settings, quick actions,
//     snippets) + live store snapshots (followed channels, recent chatters)
//     + debounced Twitch live search (`search_channels`) + debounced category
//     search (`search_categories`).
//
//   - Active row enrichment: when the active row is a streamer (followed or
//     search result), its Twitch description is lazy-fetched once and shown
//     below the subtitle. Cached across sessions of the modal.
//
//   - Keyboard: arrow keys navigate across sections, Enter executes, Esc closes.
//
// Mounted in `App.tsx` AND `MultiChatWindow.tsx` — each Tauri WebView has its
// own AppStore instance, so the palette is window-local.

import { useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Search, ArrowDown, ArrowUp, CornerDownLeft, Star, X as XIcon, Lightbulb } from 'lucide-react';
import { useAppStore } from '../stores/AppStore';
import { useChatUserStore } from '../stores/chatUserStore';
import { useSnippetStore } from '../stores/snippetStore';
import {
  getStaticItems,
  getFollowedItems,
  getRecentChatterItems,
  searchTwitchChannels,
  searchTwitchCategories,
  scoreItem,
  loadRecentCommandIds,
  pushRecentCommand,
  enrichStreamerDescription,
  getCachedDescription,
  type PaletteItem,
  type PaletteSection,
} from '../utils/commandPaletteSources';

// Render order — empty-state-only sections (Now Playing / Jump To / Tips) come
// right after Recent so the resting-state composition is: what you did last,
// what you're doing now, where you might want to go, what else is possible.
// Snippets sit dead last in the typed-query view so an explicit "Open Settings
// · X" search doesn't get buried by copypasta noise.
const SECTION_ORDER: PaletteSection[] = [
  'Recent',
  'Now Playing',
  'Jump To',
  'Quick Actions',
  'Current Stream',
  'Share',
  'Settings',
  'Categories',
  'Followed Channels',
  'Recent Chatters',
  'Streamers',
  'Tips',
  'Snippets',
];

// Static IDs of the high-level surfaces featured in the empty-state "Jump To"
// section. Pulled out as a constant so the order stays stable and so the
// curation is one obvious place to edit.
const JUMP_TO_IDS = [
  'qa.openDrops',
  'qa.openBadges',
  'qa.openWhispers',
  'qa.openLists',
  'qa.openMultiChat',
  'qa.goHome',
  'qa.openSettings',
  'qa.openWhatsNew',
  'qa.openPaletteWiki',
];

const TWITCH_DEBOUNCE_MS = 250;
const CATEGORY_DEBOUNCE_MS = 350;
const MAX_RESULTS_PER_SECTION = 6;
const DESCRIPTION_HOVER_DELAY_MS = 180;

// Per-section tint for the RowAvatar fallback tile. Same desaturated palette
// family as SettingsDialog's sidebar tiles so the two surfaces read as one
// system. Rows that have a real avatar (streamers, chatters) keep the round
// avatar — round = person, squircle = command.
const SECTION_TINTS: Partial<Record<PaletteSection, string>> = {
  Settings:        'rgba(165, 170, 170, 0.20)',
  'Quick Actions': 'rgba(150, 165, 180, 0.20)',
  'Jump To':       'rgba(160, 180, 170, 0.20)',
  'Now Playing':   'rgba(150, 165, 180, 0.20)',
  'Current Stream':'rgba(150, 165, 180, 0.20)',
  Recent:          'rgba(170, 165, 180, 0.20)',
  Share:           'rgba(185, 175, 165, 0.20)',
  Tips:            'rgba(195, 180, 150, 0.18)',
  Snippets:        'rgba(160, 180, 170, 0.20)',
  Categories:      'rgba(170, 165, 185, 0.20)',
};

// Canonical bevel recipe lives in globals.css as --bevel-tile.
const TILE_BEVEL = 'var(--bevel-tile)';

interface FlatRow {
  item: PaletteItem;
  /** Global index across all sections — used for keyboard nav. */
  globalIndex: number;
}

interface RenderSection {
  section: PaletteSection;
  rows: FlatRow[];
  /** Optional override for the section's visible label. Used by Now Playing
   *  / Current Stream to surface "shroud · Valorant · 8.2k viewers" inline
   *  with the section header — saves a row, makes the palette context-aware
   *  at a glance. */
  headerOverride?: string;
}

interface SearchResults {
  /** The trimmed query these items were fetched for. */
  forQuery: string;
  items: PaletteItem[];
}

const EMPTY_RESULTS: SearchResults = { forQuery: '', items: [] };

function resultsFor(queryKey: string, results: SearchResults): PaletteItem[] {
  if (queryKey.length < 2) return [];
  return queryKey.startsWith(results.forQuery) ? results.items : [];
}

export default function CommandPalette() {
  const isOpen = useAppStore((s) => s.isCommandPaletteOpen);
  const closeCommandPalette = useAppStore((s) => s.closeCommandPalette);

  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  // Search results remember the query they answered. What the palette shows
  // is derived from that: nothing below two characters, and a result set
  // only while the current query still extends the one it was issued for
  // (so typing onward keeps the last hits on screen until fresh ones land,
  // while a different prefix drops them at once). No effect clears results.
  const [twitchResultsRaw, setTwitchResults] = useState<SearchResults>(EMPTY_RESULTS);
  const [categoryResultsRaw, setCategoryResults] = useState<SearchResults>(EMPTY_RESULTS);
  const queryKey = query.trim();
  const twitchResults = resultsFor(queryKey, twitchResultsRaw);
  const categoryResults = resultsFor(queryKey, categoryResultsRaw);
  /** Tick state used to nudge a re-render after a lazy description fetch
   *  resolves. The actual description lives in the module-level cache. */
  const [, setEnrichmentTick] = useState(0);

  // Subscribe so re-renders pick up new chatters / followed-channel snapshots
  // while the palette is open. The snippet store subscriptions cover edits
  // made in the Command Palette settings page (favorite toggle, alias edit,
  // custom snippet add/remove) so they reflect on the very next render. The
  // currentStream subscription drives social-link row regeneration when the
  // watched channel changes.
  const followedStreams = useAppStore((s) => s.followedStreams);
  const currentStream = useAppStore((s) => s.currentStream);
  const chatUsersVersion = useChatUserStore((s) => s.users);
  const snippetFavoritesVersion = useSnippetStore((s) => s.favoriteIds);
  const snippetAliasesVersion = useSnippetStore((s) => s.aliases);
  const snippetCustomVersion = useSnippetStore((s) => s.customSnippets);

  // Latest query for the debounced searches to compare against when they
  // resolve. Synced in an effect (not during render) so the ref is never read
  // or written on the render path.
  const queryRef = useRef('');
  useEffect(() => {
    queryRef.current = query;
  }, [query]);
  // Bumped by items that want the input focused after they run. Routed through
  // state so the item list itself never closes over a ref.
  const [focusRequest, setFocusRequest] = useState(0);
  useEffect(() => {
    if (focusRequest === 0) return;
    inputRef.current?.focus();
  }, [focusRequest]);
  const twitchDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const categoryDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const enrichDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  // Gates the four async .then() resolvers below (twitch search, category
  // search, eager + lazy bio enrichment). Promises can resolve after the
  // component unmounts; without this guard the resolver would call setState
  // on a dead component and hold the closure alive until the await returns.
  // Set true at the *start* of the effect so React.StrictMode's double-invoke
  // (mount → cleanup → mount) restores the gate on the second mount; refs
  // persist across StrictMode's fake unmount, so setting only on teardown
  // leaves the ref permanently false.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (twitchDebounceRef.current) clearTimeout(twitchDebounceRef.current);
      if (categoryDebounceRef.current) clearTimeout(categoryDebounceRef.current);
      if (enrichDebounceRef.current) clearTimeout(enrichDebounceRef.current);
    };
  }, []);

  // Reset state every time the palette opens. Done during render on the
  // closed-to-open transition so the stale query never paints; results are
  // derived from the query, so clearing it clears them too.
  const [wasOpen, setWasOpen] = useState(isOpen);
  if (isOpen !== wasOpen) {
    setWasOpen(isOpen);
    if (isOpen) {
      setQuery('');
      setActiveIndex(0);
    }
  }
  useEffect(() => {
    if (!isOpen) return;
    queueMicrotask(() => inputRef.current?.focus());
  }, [isOpen]);

  // Debounced Twitch live search — fires once the query has 2+ chars.
  useEffect(() => {
    if (!isOpen) return;
    if (twitchDebounceRef.current) clearTimeout(twitchDebounceRef.current);
    if (query.trim().length < 2) return;
    twitchDebounceRef.current = setTimeout(() => {
      const issued = query.trim();
      void searchTwitchChannels(issued).then((items) => {
        if (!mountedRef.current) return;
        if (queryRef.current.trim() === issued) setTwitchResults({ forQuery: issued, items });
      });
    }, TWITCH_DEBOUNCE_MS);
    return () => {
      if (twitchDebounceRef.current) clearTimeout(twitchDebounceRef.current);
    };
  }, [query, isOpen]);

  // Debounced Twitch category search — slightly longer debounce since each
  // hit fans out into two rows and stacks below the live-channel results.
  useEffect(() => {
    if (!isOpen) return;
    if (categoryDebounceRef.current) clearTimeout(categoryDebounceRef.current);
    if (query.trim().length < 2) return;
    categoryDebounceRef.current = setTimeout(() => {
      const issued = query.trim();
      void searchTwitchCategories(issued).then((items) => {
        if (!mountedRef.current) return;
        if (queryRef.current.trim() === issued) setCategoryResults({ forQuery: issued, items });
      });
    }, CATEGORY_DEBOUNCE_MS);
    return () => {
      if (categoryDebounceRef.current) clearTimeout(categoryDebounceRef.current);
    };
  }, [query, isOpen]);

  // Build the flat, ranked result set.
  const sections = useMemo<RenderSection[]>(() => {
    const queryLower = query.trim().toLowerCase();
    const followed = getFollowedItems();
    const chatters = getRecentChatterItems();
    const staticItems = getStaticItems();

    if (!queryLower) {
      // Empty-state: a structured "what should I do?" composition rather than
      // an arbitrary slice. Order top to bottom:
      //   Recent       — last picks if any
      //   Now Playing  — Current Stream actions, with the streamer + game in
      //                  the header (only when watching)
      //   Jump To      — top-level surfaces (Drops, Badges, MultiChat, …)
      //   Tips         — discoverability for mode prefixes & search; each tip
      //                  pre-fills the input when selected
      const recentIds = loadRecentCommandIds();
      const allKnown = new Map<string, PaletteItem>();
      for (const it of [...staticItems, ...followed, ...chatters]) allKnown.set(it.id, it);
      const recentItems: PaletteItem[] = [];
      for (const id of recentIds) {
        const it = allKnown.get(id);
        if (it) recentItems.push({ ...it, section: 'Recent' });
      }

      // Now Playing surfaces the Current Stream actions when watching; the
      // header gets the streamer name + game + viewer count so the palette
      // reads as context-aware on open. Re-tagged to 'Now Playing' so the
      // section header is the right one — the items themselves don't change.
      const nowPlayingItems: PaletteItem[] = currentStream
        ? staticItems
            .filter((i) => i.section === 'Current Stream' && !i.id.startsWith('cs.social.'))
            .slice(0, 5)
            .map((i) => ({ ...i, section: 'Now Playing' as const }))
        : [];
      // Social-link rows (if cached) come right under the actions as a
      // continuation of Now Playing — same section so they group together.
      const nowPlayingSocials: PaletteItem[] = currentStream
        ? staticItems
            .filter((i) => i.id.startsWith('cs.social.'))
            .slice(0, 4)
            .map((i) => ({ ...i, section: 'Now Playing' as const }))
        : [];

      // Jump To: high-level navigation, pulled by ID from the static catalog
      // in a fixed order. Re-tagged so the section label reads "Jump To"
      // instead of "Quick Actions".
      const jumpToById = new Map(staticItems.map((i) => [i.id, i]));
      const jumpToItems: PaletteItem[] = JUMP_TO_IDS.flatMap((id) => {
        const it = jumpToById.get(id);
        return it ? [{ ...it, section: 'Jump To' as const }] : [];
      });

      // Tips: each one pre-fills the input with a starter so the user can
      // immediately try the feature. Built inline here (instead of in
      // commandPaletteSources) so they can close over setQuery.
      const tipsItems: PaletteItem[] = [
        {
          id: 'tip.search',
          section: 'Tips',
          title: 'Search Twitch for a streamer or game',
          subtitle: 'Just start typing a name. Live results in ~250ms.',
          keywords: 'search streamer game twitch',
          initial: '🔎',
          run: () => {
            setQuery('');
            setFocusRequest((n) => n + 1);
          },
        },
        {
          id: 'tip.snippets',
          section: 'Tips',
          title: 'Browse the snippet library',
          subtitle: 'Star favorites, set aliases, add your own',
          keywords: 'snippets copypasta library favorites alias settings',
          initial: '★',
          run: () => useAppStore.getState().openSettings('Command Palette'),
        },
      ];

      const composed = [
        ...recentItems,
        ...nowPlayingItems,
        ...nowPlayingSocials,
        ...jumpToItems,
        ...tipsItems,
      ];
      const grouped = groupAndOrder(composed);
      // Apply a contextual header to Now Playing — "shroud · Valorant · 8.2k
      // viewers" — so the streamer's identity is visible inline without
      // burning a row. Falls back to the plain "Now Playing" label when
      // metadata is incomplete.
      if (currentStream) {
        for (const section of grouped) {
          if (section.section !== 'Now Playing') continue;
          const parts = [currentStream.user_name || currentStream.user_login];
          if (currentStream.game_name) parts.push(currentStream.game_name);
          if (typeof currentStream.viewer_count === 'number' && currentStream.viewer_count > 0) {
            parts.push(`${currentStream.viewer_count.toLocaleString()} viewers`);
          }
          section.headerOverride = `Now Playing · ${parts.join(' · ')}`;
        }
      }
      return grouped;
    }

    const pool: PaletteItem[] = [
      ...staticItems,
      ...followed,
      ...chatters,
      ...twitchResults,
      ...categoryResults,
    ];

    // De-duplicate by id — followed-channel rows and Twitch search rows can
    // both surface the same broadcaster; prefer the followed entry.
    const seen = new Set<string>();
    const unique: PaletteItem[] = [];
    for (const item of pool) {
      const dedupeKey = item.section === 'Streamers' && followed.some((f) => f.title.toLowerCase() === item.title.toLowerCase())
        ? `__dup_${item.title.toLowerCase()}`
        : item.id;
      if (seen.has(dedupeKey)) continue;
      seen.add(dedupeKey);
      unique.push(item);
    }

    const scored: PaletteItem[] = [];
    for (const item of unique) {
      const score = scoreItem(item, queryLower);
      if (score < 0) continue;
      scored.push({ ...item, score });
    }
    return groupAndOrder(scored);
  }, [
    query,
    twitchResults,
    categoryResults,
    followedStreams,
    currentStream,
    chatUsersVersion,
    snippetFavoritesVersion,
    snippetAliasesVersion,
    snippetCustomVersion,
  ]);

  // Eagerly warm the current stream's about cache so its social-link rows
  // are ready when the user opens the palette — beats waiting for them to
  // arrow onto an active row and trigger the per-row enrichment. Bump the
  // enrichment tick on resolution so getStaticItems' social_links pull
  // re-runs and the rows appear.
  useEffect(() => {
    if (!currentStream?.user_id || !currentStream.user_login) return;
    void enrichStreamerDescription(currentStream.user_id, currentStream.user_login).then(() => {
      if (!mountedRef.current) return;
      setEnrichmentTick((t) => t + 1);
    });
  }, [currentStream?.user_id, currentStream?.user_login]);

  const flatRows = useMemo<FlatRow[]>(() => {
    const flat: FlatRow[] = [];
    let i = 0;
    for (const sec of sections) {
      for (const row of sec.rows) {
        flat.push({ item: row.item, globalIndex: i++ });
      }
    }
    return flat;
  }, [sections]);

  // Keep the highlight inside the list when it shrinks under it. Adjusted
  // during render so the out-of-range index never paints.
  const maxIndex = Math.max(0, flatRows.length - 1);
  if (activeIndex > maxIndex) setActiveIndex(maxIndex);

  useEffect(() => {
    if (!listRef.current) return;
    const el = listRef.current.querySelector<HTMLElement>(`[data-row-index="${activeIndex}"]`);
    if (el) el.scrollIntoView({ block: 'nearest' });
  }, [activeIndex]);

  // Lazy streamer bio enrichment — kicks in when the active row points at a
  // Twitch user we haven't fetched yet. Debounced so quick arrow-key passes
  // through the list don't all trigger API calls; only the row the user
  // actually settles on does.
  useEffect(() => {
    if (enrichDebounceRef.current) clearTimeout(enrichDebounceRef.current);
    const row = flatRows[activeIndex];
    if (!row?.item.twitchUserId) return;
    const userId = row.item.twitchUserId;
    if (getCachedDescription(userId)) return;
    // The chatter login is the username (chatUserStore stores it that way);
    // for streamer rows the title is the display name, so prefer the keyword
    // bag which holds the lowercase login as its first token.
    const loginGuess = (row.item.keywords ?? '').split(/\s+/)[0] ?? row.item.title.toLowerCase();
    enrichDebounceRef.current = setTimeout(() => {
      void enrichStreamerDescription(userId, loginGuess).then((desc) => {
        if (!mountedRef.current) return;
        if (desc) setEnrichmentTick((t) => t + 1);
      });
    }, DESCRIPTION_HOVER_DELAY_MS);
    return () => {
      if (enrichDebounceRef.current) clearTimeout(enrichDebounceRef.current);
    };
  }, [activeIndex, flatRows]);

  function executeRow(row: FlatRow | undefined) {
    if (!row) return;
    pushRecentCommand(row.item.id);
    closeCommandPalette();
    queueMicrotask(() => {
      try {
        const result = row.item.run();
        if (result && typeof (result as Promise<void>).catch === 'function') {
          (result as Promise<void>).catch(() => {
            // Action surfaces its own error toast.
          });
        }
      } catch {
        // Action surfaces its own error toast.
      }
    });
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (flatRows.length === 0) return;
      setActiveIndex((i) => (i + 1) % flatRows.length);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (flatRows.length === 0) return;
      setActiveIndex((i) => (i - 1 + flatRows.length) % flatRows.length);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      executeRow(flatRows[activeIndex]);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closeCommandPalette();
    } else if (e.key === 'Home') {
      e.preventDefault();
      setActiveIndex(0);
    } else if (e.key === 'End') {
      e.preventDefault();
      setActiveIndex(Math.max(0, flatRows.length - 1));
    }
  }

  const placeholder = 'Search streamers, settings, snippets, actions...';

  return (
    <AnimatePresence>
      {isOpen && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.12 }}
          className="fixed inset-0 z-[100] flex items-start justify-center bg-black/40 backdrop-blur-2xl pt-[10vh]"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) closeCommandPalette();
          }}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.97, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: 12 }}
            transition={{ type: 'spring', stiffness: 350, damping: 28 }}
            // Shares chassis with SettingsDialog — `liquid-glass-panel` is the
            // canonical 96px-blur diffused recipe. !rounded-2xl overrides the
            // 12px radius the recipe ships with; the palette wants a softer
            // corner than panels do.
            className="liquid-glass-panel !rounded-2xl w-[94vw] max-w-[760px] overflow-hidden flex flex-col"
            onKeyDown={onKeyDown}
          >
            {/* Search input row — taller + larger typography to read as a
                primary surface, not a tooltip. */}
            <div className="flex items-center gap-3 px-5 py-4 border-b border-white/[0.06]">
              <Search size={18} className="text-textMuted flex-shrink-0" aria-hidden />
              <input
                ref={inputRef}
                type="text"
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value);
                  setActiveIndex(0);
                }}
                placeholder={placeholder}
                className="flex-1 bg-transparent border-none outline-none text-base text-textPrimary placeholder:text-textMuted"
                spellCheck={false}
                autoComplete="off"
              />
              {query && (
                <button
                  type="button"
                  onClick={() => {
                    setQuery('');
                    inputRef.current?.focus();
                  }}
                  className="text-textMuted hover:text-textPrimary p-1 rounded transition-colors"
                  aria-label="Clear search"
                >
                  <XIcon size={14} />
                </button>
              )}
            </div>

            {/* Results body */}
            <div ref={listRef} className="max-h-[68vh] overflow-y-auto scrollbar-thin py-2">
              {flatRows.length === 0 ? (
                <EmptyMessage query={query} />
              ) : (
                sections.map((sec) => (
                  <SectionBlock
                    key={sec.section}
                    section={sec}
                    activeIndex={activeIndex}
                    onActivate={(idx) => setActiveIndex(idx)}
                    onPick={(idx) => executeRow(flatRows[idx])}
                  />
                ))
              )}
            </div>

            {/* Footer hint row */}
            <div className="flex items-center justify-between gap-3 px-5 py-2.5 border-t border-white/[0.06] text-[10px] uppercase tracking-wider text-textMuted bg-white/[0.02]">
              <div className="flex items-center gap-3">
                <HintChip icon={<ArrowUp size={10} />} label="" />
                <HintChip icon={<ArrowDown size={10} />} label="Navigate" />
                <HintChip icon={<CornerDownLeft size={10} />} label="Open" />
                <HintChip icon={<span className="font-semibold">Esc</span>} label="Close" />
              </div>
              <div className="flex items-center gap-3">
                {flatRows.length > 0 && (
                  <span>{flatRows.length} result{flatRows.length === 1 ? '' : 's'}</span>
                )}
              </div>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

// ---------- Empty message ---------------------------------------------------

function EmptyMessage({ query }: { query: string }) {
  return (
    <div className="px-6 py-10 text-center">
      <div className="text-sm text-textSecondary">No matches for "{query.trim()}"</div>
      <div className="mt-1 text-xs text-textMuted">Try a streamer name, setting, snippet, or quick action.</div>
    </div>
  );
}

// ---------- Section block ---------------------------------------------------

interface SectionBlockProps {
  section: RenderSection;
  activeIndex: number;
  onActivate: (idx: number) => void;
  onPick: (idx: number) => void;
}

function SectionBlock({ section, activeIndex, onActivate, onPick }: SectionBlockProps) {
  const headerLabel = section.headerOverride ?? section.section;
  // Tips rows get a softer, italicized treatment — they're discoverability
  // hints, not normal commands; the visual difference signals "try this" vs
  // "execute this".
  const isTips = section.section === 'Tips';
  return (
    <div className="mb-2 last:mb-0">
      <div className="flex items-center gap-1.5 px-5 pt-3 pb-1.5 text-[11px] uppercase tracking-[0.12em] text-textMuted font-medium">
        {isTips && <Lightbulb size={11} className="text-amber-300/80" aria-hidden />}
        <span className="truncate">{headerLabel}</span>
      </div>
      {section.rows.map((row) => {
        const isActive = activeIndex === row.globalIndex;
        const cachedDesc = row.item.twitchUserId ? getCachedDescription(row.item.twitchUserId) : undefined;
        // For snippets, `details` carries the full content for the active-row
        // preview. For streamers, `cachedDesc` lazy-fills from the bio fetch.
        // Show whichever is available; snippets take precedence (their content
        // IS the row's purpose).
        const details = row.item.details ?? cachedDesc;
        // Snippet preview gets line-clamp-4 for multi-paragraph copypastas;
        // streamer bios stay tighter at 3 lines.
        const detailsClamp = row.item.section === 'Snippets' ? 'line-clamp-4' : 'line-clamp-3';
        return (
          <button
            key={row.item.id}
            type="button"
            data-row-index={row.globalIndex}
            onMouseMove={() => onActivate(row.globalIndex)}
            onClick={() => onPick(row.globalIndex)}
            // Left-accent bar marks the active row — a stronger affordance
            // than a background-only highlight at the larger row size.
            className={`relative flex w-full items-start gap-3.5 px-5 py-2.5 text-left transition-colors ${
              isActive
                ? 'bg-white/[0.06] text-textPrimary'
                : 'text-textSecondary hover:bg-white/[0.03] hover:text-textPrimary'
            }`}
          >
            {isActive && (
              <span
                aria-hidden
                className="pointer-events-none absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-r-full bg-accent/85"
              />
            )}
            <RowAvatar item={row.item} />
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5">
                <span className="truncate text-[14px] font-medium">{row.item.title}</span>
                {row.item.favorite && (
                  <Star
                    size={11}
                    className="flex-shrink-0 fill-amber-400 text-amber-400"
                    aria-label="Favorite"
                  />
                )}
              </div>
              {row.item.subtitle && (
                <div className="truncate text-[11.5px] text-textMuted">{row.item.subtitle}</div>
              )}
              {isActive && details && (
                <div className={`mt-1.5 text-[11.5px] text-textSecondary ${detailsClamp} whitespace-pre-line leading-relaxed`}>
                  {details}
                </div>
              )}
            </div>
            {row.item.shortcut && (
              <kbd className="sn-keycap sn-keycap--xs ml-auto flex-shrink-0 self-center">
                {row.item.shortcut}
              </kbd>
            )}
          </button>
        );
      })}
    </div>
  );
}

// ---------- Row avatar ------------------------------------------------------

function RowAvatar({ item }: { item: PaletteItem }) {
  if (item.avatarUrl) {
    return (
      <img
        src={item.avatarUrl}
        alt=""
        className="h-8 w-8 flex-shrink-0 rounded-full bg-surface object-cover mt-0.5"
        onError={(e) => {
          (e.currentTarget as HTMLImageElement).style.visibility = 'hidden';
        }}
      />
    );
  }
  const tint = SECTION_TINTS[item.section] ?? 'rgba(151, 177, 185, 0.18)';
  return (
    <div
      className="grid h-8 w-8 flex-shrink-0 place-items-center rounded-md text-[12px] font-semibold text-textPrimary mt-0.5"
      style={{
        background: tint,
        boxShadow: TILE_BEVEL,
        border: '1px solid transparent',
      }}
    >
      {item.initial ?? item.title.slice(0, 1).toUpperCase()}
    </div>
  );
}

// ---------- Hint chip -------------------------------------------------------

function HintChip({ icon, label }: { icon: React.ReactNode; label: string }) {
  return (
    <span className="inline-flex items-center gap-1">
      <span className="inline-grid h-4 min-w-[1rem] place-items-center rounded border border-borderSubtle bg-glass/40 px-1 text-textSecondary">
        {icon}
      </span>
      {label && <span>{label}</span>}
    </span>
  );
}

// ---------- Grouping helper -------------------------------------------------

function groupAndOrder(items: PaletteItem[]): RenderSection[] {
  const bySection = new Map<PaletteSection, PaletteItem[]>();
  for (const it of items) {
    const list = bySection.get(it.section) ?? [];
    list.push(it);
    bySection.set(it.section, list);
  }
  const out: RenderSection[] = [];
  let globalIndex = 0;
  for (const section of SECTION_ORDER) {
    const list = bySection.get(section);
    if (!list || list.length === 0) continue;
    // Snippets: favorites win the tie-breaker so starred entries always sit
    // at the top of the section regardless of how the query scored them.
    if (section === 'Snippets') {
      list.sort((a, b) => {
        if (!!a.favorite !== !!b.favorite) return a.favorite ? -1 : 1;
        return (b.score ?? 0) - (a.score ?? 0);
      });
    } else {
      list.sort((a, b) => (b.score ?? 0) - (a.score ?? 0));
    }
    // Per-section caps. Snippets goes higher so the user can browse the
    // library. Empty-state-only sections (Now Playing, Jump To, Tips) are
    // pre-curated lists; capping them at 6 would silently drop items the
    // composer expected to be visible.
    const cap =
      section === 'Snippets'
        ? 10
        : section === 'Jump To' || section === 'Now Playing' || section === 'Tips'
          ? 12
          : MAX_RESULTS_PER_SECTION;
    const trimmed = list.slice(0, cap);
    out.push({
      section,
      rows: trimmed.map((item) => ({ item, globalIndex: globalIndex++ })),
    });
  }
  return out;
}
