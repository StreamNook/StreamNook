// Focused settings surface for the MultiChat title-bar gear icon.
//
// Mirrors the core app's settings: a left category rail (with search) + a hero'd
// content pane, and it fills the whole MultiChat window with a snappy spring
// open. Scoped to what a chat-only window has — chat design, the window theme,
// platform connections, and the activity / mod-log pane behavior. No player,
// drops, or integrations tabs; a MultiChat window hosts none of those.

import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { X, Search, MessageSquare, Palette, Plug, Layout, type LucideIcon } from 'lucide-react';
import ChatSettings from '../settings/ChatSettings';
import ConnectionsSettings from './ConnectionsSettings';
import MultiChatThemePicker from './MultiChatThemePicker';
import SettingsSearchResults from '../settings/SettingsSearchResults';
import { findSettingTarget, flashSettingRow } from '../settings/settingsNavigation';
import { searchSettings, type SettingsIndexEntry } from '../settings/searchIndex';

type SettingsTab = 'chat' | 'theme' | 'connections' | 'panes';

// Canonical bevel recipes live in globals.css as --bevel-tile / --bevel-hero.
const TILE_BEVEL = 'var(--bevel-tile)';
const HERO_BEVEL = 'var(--bevel-hero)';

interface TabMeta {
  id: SettingsTab;
  label: string;
  icon: LucideIcon;
  tint: string;
  description: string;
}

const TABS: TabMeta[] = [
  {
    id: 'chat',
    label: 'Chat',
    icon: MessageSquare,
    tint: 'rgba(150, 160, 210, 0.22)',
    description: 'Highlights, commands, nicknames, and message design',
  },
  {
    id: 'theme',
    label: 'Theme',
    icon: Palette,
    tint: 'rgba(220, 145, 175, 0.20)',
    description: 'Color theme for your chat windows',
  },
  {
    id: 'connections',
    label: 'Connections',
    icon: Plug,
    tint: 'rgba(180, 150, 210, 0.22)',
    description: 'Connected platform accounts',
  },
  {
    id: 'panes',
    label: 'Panes',
    icon: Layout,
    tint: 'rgba(140, 195, 170, 0.22)',
    description: 'Activity feed and moderator log behavior',
  },
];

// Maps a search result's source tab to the MultiChat tab it lives under.
const RESULT_TAB: Record<string, SettingsTab> = {
  Chat: 'chat',
  Theme: 'theme',
  Connections: 'connections',
  Panes: 'panes',
};

// Searchable stand-ins for the MultiChat-only tabs, so typing e.g. "log in" or
// "color" jumps straight to the right tab. The Chat tab's many settings come from
// the shared core index (searchSettings scoped to ['Chat']).
const MC_TAB_ENTRIES: SettingsIndexEntry[] = [
  {
    tab: 'Theme',
    section: 'Appearance',
    title: 'Theme',
    description: 'color theme palette dark light appearance look for your chat windows',
  },
  {
    tab: 'Connections',
    section: 'Accounts',
    title: 'Account Connections',
    description: 'connect link sign in log in twitch kick youtube account platform',
  },
  {
    tab: 'Panes',
    section: 'Behavior',
    title: 'Activity & mod-log panes',
    description: 'keep collecting while closed activity feed moderator log events',
  },
];

const matchEntry = (e: SettingsIndexEntry, tokens: string[]) => {
  const hay = `${e.title} ${e.description ?? ''} ${e.section} ${e.tab}`.toLowerCase();
  return tokens.every((t) => hay.includes(t));
};

interface ChatOnlySettingsModalProps {
  open: boolean;
  onClose: () => void;
  /** Which tab to open on. Re-seeded every time the modal opens. */
  initialTab?: SettingsTab;
  /** Whether the activity + mod-log panes keep collecting while closed. */
  keepPanesCollecting: boolean;
  onKeepPanesCollectingChange: (value: boolean) => void;
}

export default function ChatOnlySettingsModal({
  open,
  onClose,
  initialTab,
  keepPanesCollecting,
  onKeepPanesCollectingChange,
}: ChatOnlySettingsModalProps) {
  const [tab, setTab] = useState<SettingsTab>(initialTab ?? 'chat');
  const [query, setQuery] = useState('');
  const contentRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  // Re-seed the tab + clear search whenever the modal opens (or is asked to open
  // on a different tab). Render-phase "adjust state on prop change" pattern, not
  // an effect — it converges immediately and avoids the synchronous-setState-in-
  // effect cascade. The modal stays mounted for its exit animation, so without
  // this a reopen would show the last-viewed tab and a stale search.
  const [lastSeed, setLastSeed] = useState<{ open: boolean; tab?: SettingsTab }>({ open: false });
  if (open && (!lastSeed.open || lastSeed.tab !== initialTab)) {
    setLastSeed({ open: true, tab: initialTab });
    setTab(initialTab ?? 'chat');
    setQuery('');
  } else if (!open && lastSeed.open) {
    setLastSeed({ open: false, tab: initialTab });
  }

  const searching = query.trim().length > 0;

  // Esc to close — only while open so it doesn't compete with other Esc consumers.
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [open, onClose]);

  const results = useMemo(() => {
    if (!searching) return [];
    const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
    const extra = MC_TAB_ENTRIES.filter((e) => matchEntry(e, tokens));
    const chat = searchSettings(query, 50, ['Chat']);
    return [...extra, ...chat];
  }, [query, searching]);

  const activeMeta = TABS.find((t) => t.id === tab) ?? TABS[0];
  const HeroIcon = activeMeta.icon;

  const selectTab = (id: SettingsTab) => {
    setQuery('');
    setTab(id);
  };

  const handleResultSelect = (entry: SettingsIndexEntry) => {
    setQuery('');
    const target = RESULT_TAB[entry.tab] ?? 'chat';
    setTab(target);
    // Only the Chat panel is long enough to need a scroll; the other tabs are
    // short, so switching to them is enough. Land on the row the result names,
    // falling back to its section.
    if (target === 'chat') {
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          const el = findSettingTarget(contentRef.current, entry);
          if (el && contentRef.current) {
            flashSettingRow(el);
            const cTop = contentRef.current.getBoundingClientRect().top;
            const eTop = el.getBoundingClientRect().top;
            contentRef.current.scrollBy({ top: eTop - cTop - 8, behavior: 'smooth' });
          }
        }),
      );
    }
  };

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          key="mc-settings"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.16 }}
          className="fixed inset-0 z-[200] bg-black/40 backdrop-blur-[40px]"
          onClick={onClose}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.98, y: 8 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.98, y: 8 }}
            transition={{ type: 'spring', stiffness: 420, damping: 32 }}
            onClick={(e) => e.stopPropagation()}
            className="flex h-full w-full overflow-hidden bg-background"
            role="dialog"
            aria-modal="true"
            aria-label="MultiChat Settings"
          >
            {/* Category rail */}
            <aside className="flex w-[210px] flex-shrink-0 flex-col border-r border-white/[0.06] py-3">
              <div className="px-4 pb-2">
                <h2 className="text-sm font-semibold text-textPrimary">MultiChat Settings</h2>
              </div>
              <div className="px-3 pb-3">
                <div className="relative">
                  <Search
                    size={13}
                    strokeWidth={2.25}
                    className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-textMuted"
                  />
                  <input
                    ref={searchRef}
                    type="text"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder="Search settings"
                    className="w-full rounded-md border border-white/[0.06] bg-white/[0.03] py-1.5 pl-7 pr-7 text-[12px] text-textPrimary placeholder:text-textMuted focus:border-white/[0.12] focus:bg-white/[0.05] focus:outline-none"
                  />
                  {searching && (
                    <button
                      type="button"
                      onClick={() => {
                        setQuery('');
                        searchRef.current?.focus();
                      }}
                      className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-textMuted hover:bg-white/[0.06] hover:text-textPrimary"
                    >
                      <X size={12} />
                    </button>
                  )}
                </div>
              </div>
              <nav className="scrollbar-thin flex-1 space-y-0.5 overflow-y-auto px-2">
                {TABS.map((t) => {
                  const Icon = t.icon;
                  const isActive = t.id === tab && !searching;
                  return (
                    <button
                      key={t.id}
                      type="button"
                      onClick={() => selectTab(t.id)}
                      className={`flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left transition-colors ${
                        isActive
                          ? 'bg-white/[0.06] text-textPrimary'
                          : 'text-textSecondary hover:bg-white/[0.03] hover:text-textPrimary'
                      }`}
                    >
                      <span
                        className="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md"
                        style={{ background: t.tint, boxShadow: TILE_BEVEL, border: '1px solid transparent' }}
                      >
                        <Icon size={14} strokeWidth={2.25} className="text-textPrimary" />
                      </span>
                      <span className="text-[13px] font-medium">{t.label}</span>
                    </button>
                  );
                })}
              </nav>
            </aside>

            {/* Content */}
            <section className="flex min-w-0 flex-1 flex-col">
              <div className="flex items-center justify-end px-3 pt-3">
                <button
                  type="button"
                  onClick={onClose}
                  className="rounded p-1.5 text-textMuted transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
                  aria-label="Close"
                >
                  <X size={16} />
                </button>
              </div>

              <div className="flex items-center gap-3 px-6 pb-3 pt-0">
                <span
                  className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-lg"
                  style={{
                    background: searching ? 'rgba(180, 180, 195, 0.18)' : activeMeta.tint,
                    boxShadow: HERO_BEVEL,
                    border: '1px solid transparent',
                  }}
                >
                  {searching ? (
                    <Search size={18} strokeWidth={2} className="text-textPrimary" />
                  ) : (
                    <HeroIcon size={18} strokeWidth={2} className="text-textPrimary" />
                  )}
                </span>
                <div className="min-w-0">
                  <h3 className="text-[15px] font-semibold leading-tight text-textPrimary">
                    {searching ? 'Search' : activeMeta.label}
                  </h3>
                  <p className="mt-0.5 truncate text-[11px] text-textMuted">
                    {searching ? `Results for "${query}"` : activeMeta.description}
                  </p>
                </div>
              </div>

              <div ref={contentRef} className="scrollbar-thin flex-1 overflow-y-auto px-6 pb-6">
                {searching ? (
                  <SettingsSearchResults query={query} results={results} onSelect={handleResultSelect} />
                ) : (
                  <>
                    {tab === 'chat' && <ChatSettings hidePlacement />}
                    {tab === 'theme' && <MultiChatThemePicker />}
                    {tab === 'connections' && <ConnectionsSettings />}
                    {tab === 'panes' && (
                      <div className="flex items-start justify-between gap-4">
                        <div className="min-w-0">
                          <div className="text-sm font-medium text-textPrimary">
                            Keep collecting while panes are closed
                          </div>
                          <div className="mt-1 text-xs leading-relaxed text-textSecondary">
                            When on, the activity feed and moderator logs keep gathering events even
                            while their panes are hidden, so reopening shows everything that happened
                            in the meantime. When off, closing a pane stops its collector and frees
                            its memory; it starts fresh on reopen, so events that occur while it's
                            closed aren't captured.
                          </div>
                        </div>
                        <button
                          type="button"
                          role="switch"
                          aria-checked={keepPanesCollecting}
                          onClick={() => onKeepPanesCollectingChange(!keepPanesCollecting)}
                          className={`relative mt-0.5 inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors ${
                            keepPanesCollecting ? 'bg-accent' : 'bg-white/15'
                          }`}
                        >
                          <span
                            className={`inline-block h-4 w-4 transform rounded-full bg-white shadow-sm transition-transform ${
                              keepPanesCollecting ? 'translate-x-[18px]' : 'translate-x-0.5'
                            }`}
                          />
                        </button>
                      </div>
                    )}
                  </>
                )}
              </div>
            </section>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
