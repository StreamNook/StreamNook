import { useAppStore, SettingsTab } from '../stores/AppStore';
import { IS_MOBILE } from '../utils/platform';
import SectionNav from './settings/SectionNav';
import {
  X,
  ChevronLeft,
  Layout,
  PlayCircle,
  MessageSquare,
  Palette,
  Plug,
  Bell,
  Database,
  Command,
  Keyboard,
  HardDrive,
  HelpCircle,
  Sparkles,
  Shield,
  MonitorPlay,
  User,
  Search,
  LogOut,
  type LucideIcon,
} from 'lucide-react';
import { useState, useEffect, useRef } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { motion, AnimatePresence } from 'framer-motion';
// Panels are lazy: exactly one renders at a time, so the whole settings tree
// (the heaviest source directory in the app) stays out of the dialog's chunk
// and each tab loads on first visit. The tab bar itself stays instant.
import { lazy, Suspense } from 'react';
const InterfaceSettings = lazy(() => import('./settings/InterfaceSettings'));
const PlayerSettings = lazy(() => import('./settings/PlayerSettings'));
const ChatSettings = lazy(() => import('./settings/ChatSettings'));
const ModerationSettings = lazy(() => import('./settings/ModerationSettings'));
const OverlaySettings = lazy(() => import('./settings/OverlaySettings'));
const ThemeSettings = lazy(() => import('./settings/ThemeSettings'));
const IntegrationsSettings = lazy(() => import('./settings/IntegrationsSettings'));
const CacheSettings = lazy(() => import('./settings/CacheSettings'));
const NotificationsSettings = lazy(() => import('./settings/NotificationsSettings'));
const SupportSettings = lazy(() => import('./settings/SupportSettings'));
const WhatsNewSettings = lazy(() => import('./settings/WhatsNewSettings'));
const CommandPaletteSettings = lazy(() => import('./settings/CommandPaletteSettings'));
const KeybindingsSettings = lazy(() => import('./settings/KeybindingsSettings'));
const BackupSettings = lazy(() => import('./settings/BackupSettings'));
const ProfileSettings = lazy(() => import('./settings/ProfileSettings'));
const SettingsSearchResults = lazy(() => import('./settings/SettingsSearchResults'));
import type { SettingsIndexEntry } from './settings/searchIndex';
import { Tooltip } from './ui/Tooltip';

type TabMeta = {
  id: SettingsTab;
  label: string;
  icon: LucideIcon;
  tint: string;
  description: string;
};

const TABS: TabMeta[] = [
  { id: 'Player',          label: 'Player',          icon: PlayCircle,    tint: 'rgba(120, 155, 200, 0.22)', description: 'Streamlink, video player, and auto-switch' },
  { id: 'Chat',            label: 'Chat',            icon: MessageSquare, tint: 'rgba(150, 160, 210, 0.22)', description: 'Chat design, behavior, and pop-out' },
  { id: 'Moderation',      label: 'Moderation',      icon: Shield,        tint: 'rgba(210, 140, 140, 0.22)', description: 'Mod logs, visibility, and mass actions' },
  { id: 'Overlay',         label: 'Stream Overlay',  icon: MonitorPlay,   tint: 'rgba(130, 185, 210, 0.22)', description: 'Design a chat overlay for OBS and StreamElements' },
  { id: 'Interface',       label: 'Interface',       icon: Layout,        tint: 'rgba(140, 195, 170, 0.22)', description: 'Sidebar, compact mode, and chrome' },
  { id: 'Theme',           label: 'Theme',           icon: Palette,       tint: 'rgba(220, 145, 175, 0.20)', description: 'Color theme and theme editor' },
  { id: 'Integrations',    label: 'Integrations',    icon: Plug,          tint: 'rgba(180, 150, 210, 0.22)', description: 'Connected apps and services' },
  { id: 'Notifications',   label: 'Notifications',   icon: Bell,          tint: 'rgba(220, 180, 120, 0.20)', description: 'Toasts, sounds, and update prompts' },
  { id: 'Cache',           label: 'Cache',           icon: Database,      tint: 'rgba(150, 170, 185, 0.22)', description: 'Emote, badge, and metadata caches' },
  { id: 'Command Palette', label: 'Command Palette', icon: Command,       tint: 'rgba(140, 200, 180, 0.22)', description: 'The Ctrl+K palette and snippets' },
  { id: 'Keybindings',     label: 'Keybindings',     icon: Keyboard,      tint: 'rgba(190, 160, 205, 0.22)', description: 'Customizable keyboard shortcuts' },
  { id: 'Backup',          label: 'Backup',          icon: HardDrive,     tint: 'rgba(150, 175, 185, 0.22)', description: 'Back up, restore, and open your settings file' },
  { id: 'Support',         label: 'Support',         icon: HelpCircle,    tint: 'rgba(215, 165, 140, 0.22)', description: 'Logs, diagnostics, and feedback' },
  { id: "What's New",      label: "What's New",      icon: Sparkles,      tint: 'rgba(225, 195, 130, 0.20)', description: 'Recent releases and changelog' },
];

// Profile is special — surfaced as the avatar pill at the top of the sidebar,
// not as a regular tab row. Hero treatment still uses the standard tile recipe
// so the right pane reads consistently with every other tab.
const PROFILE_META: TabMeta = {
  id: 'Profile',
  label: 'Profile',
  icon: User,
  tint: 'rgba(140, 195, 205, 0.22)',
  description: 'Account, identity, and 7TV cosmetics',
};

// Canonical bevel recipes live in globals.css as --bevel-tile / --bevel-hero.
const TILE_BEVEL = 'var(--bevel-tile)';
const HERO_BEVEL = 'var(--bevel-hero)';

const SettingsDialog = () => {
  // Actions without a subscription (stable for the store's lifetime); state
  // through a shallow-compared selector so unrelated store writes (toasts,
  // viewer counts) stop re-rendering the dialog.
  const { closeSettings, signOutActiveAccount } = useAppStore.getState();
  const {
    isSettingsOpen,
    settingsInitialTab,
    settingsInitialSection,
    isAuthenticated,
    currentUser,
    settings,
  } = useAppStore(
    useShallow((s) => ({
      isSettingsOpen: s.isSettingsOpen,
      settingsInitialTab: s.settingsInitialTab,
      settingsInitialSection: s.settingsInitialSection,
      isAuthenticated: s.isAuthenticated,
      currentUser: s.currentUser,
      settings: s.settings,
    })),
  );
  const [activeTab, setActiveTab] = useState<SettingsTab>('Player');
  // Mobile drill-down: false shows the tab list full-screen, true shows the
  // selected tab's content with a back button. Desktop ignores this and keeps
  // both panes side by side.
  const [mobileDrilledIn, setMobileDrilledIn] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [signOutConfirm, setSignOutConfirm] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);

  const tabs = TABS;
  const activeMeta =
    activeTab === 'Profile' ? PROFILE_META : tabs.find((t) => t.id === activeTab) ?? tabs[0];
  const HeroIcon = activeMeta.icon;
  const profileActive = activeTab === 'Profile';
  const searching = searchQuery.trim().length > 0;
  // Default to the compact centered window. Users can opt into a full-page
  // layout that fills the entire app via Interface › Settings Window, giving
  // long tabs more room and less scrolling.
  const compactWindow = settings.compact_settings_window !== false;

  useEffect(() => {
    if (settingsInitialTab) {
      queueMicrotask(() => setActiveTab(settingsInitialTab));
    }
  }, [settingsInitialTab]);

  // When opened with a target section (e.g. via a right-click shortcut), scroll
  // to it once the tab's content is rendered. Double rAF defers past the
  // scroll-to-top effect below so this lands last. Mirrors handleResultSelect.
  // Deep-link scroll. Panels mount lazily, so the target section may not exist
  // yet on the first frames; poll by rAF (2s cap) until it does, then apply the
  // same pinned-header (data-settings-sticky) compensation as before. Sync
  // mounts resolve on the first attempt, matching the old double-rAF timing.
  const scrollToSection = (sectionId: string) => {
    const start = performance.now();
    const attempt = () => {
      const el = document.getElementById(sectionId);
      if (el && contentRef.current) {
        const containerTop = contentRef.current.getBoundingClientRect().top;
        const elTop = el.getBoundingClientRect().top;
        const sticky = contentRef.current.querySelector('[data-settings-sticky]');
        const stickyOffset = sticky ? sticky.getBoundingClientRect().height : 0;
        contentRef.current.scrollBy({
          top: elTop - containerTop - 8 - stickyOffset,
          behavior: 'smooth',
        });
        return;
      }
      if (performance.now() - start < 2000) requestAnimationFrame(attempt);
    };
    requestAnimationFrame(() => requestAnimationFrame(attempt));
  };

  useEffect(() => {
    if (!isSettingsOpen || !settingsInitialSection) return;
    scrollToSection(settingsInitialSection);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isSettingsOpen, settingsInitialSection, activeTab]);

  useEffect(() => {
    if (!isSettingsOpen) {
      queueMicrotask(() => {
        setActiveTab('Player');
        setSearchQuery('');
        setSignOutConfirm(false);
      });
    }
  }, [isSettingsOpen]);

  useEffect(() => {
    if (contentRef.current) {
      contentRef.current.scrollTo({ top: 0 });
    }
  }, [activeTab, searching]);

  const selectTab = (tab: SettingsTab) => {
    setSearchQuery('');
    setActiveTab(tab);
  };

  const handleResultSelect = (entry: SettingsIndexEntry) => {
    setSearchQuery('');
    setActiveTab(entry.tab as SettingsTab);
    if (!entry.sectionId) return;
    scrollToSection(entry.sectionId);
  };

  return (
    <AnimatePresence>
      {isSettingsOpen && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18 }}
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-[52px]"
          onClick={closeSettings}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.97, y: 10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: 10 }}
            transition={{ type: 'spring', stiffness: 380, damping: 30 }}
            onClick={(e) => e.stopPropagation()}
            className={`liquid-glass-panel flex overflow-hidden ${
              IS_MOBILE
                // A phone has no room for a floating dialog with a 240px rail
                // beside a content pane: the pane collapses to ~180px and every
                // word wraps onto its own line. Go full-screen and drill down
                // instead (list of tabs -> tap -> content with a back button).
                ? 'w-full h-full'
                : !compactWindow
                // Opt-in full-page (Interface › Settings Window) fills the window
                // edge to edge for every tab.
                ? 'w-full h-full'
                : activeTab === 'Overlay'
                  // The Overlay builder is a design studio: fill nearly the whole
                  // window (so the live preview + all controls fit at once, no
                  // scrolling) with only a thin uniform margin. NO max cap — it
                  // scales with the screen so the preview always fits. Still inset
                  // + rounded, so its corners never poke the app's window frame.
                  // max-w/h are set (not omitted) so the grow/shrink eases the
                  // CONSTRAINT too — a max-* going to/from `none` can't animate and
                  // would snap the size abruptly.
                  ? 'w-[calc(100vw-1.5rem)] h-[calc(100vh-1.5rem)] max-w-[100vw] max-h-[100vh]'
                  : 'w-[94vw] md:w-[90vw] lg:w-[86vw] xl:w-[82vw] max-w-[1480px] h-[90vh] max-h-[980px]'
            }`}
            // Rounded in every floating mode (including the larger Overlay studio),
            // so switching tabs only eases the SIZE, never sharpens the corners.
            // Only the explicit full-page mode drops to 0 to fill edge to edge
            // without revealing blurred backdrop wedges at the corners. Explicit
            // 12<->0 (not undefined) gives the radius a defined start/end so it
            // eases symmetrically both ways.
            //
            // Scope the transition to the SIZE properties with a snappy ease-out
            // (quint): the shared `.liquid-glass-panel` uses `transition: all`,
            // which drags the 96px backdrop-filter into every resize frame and
            // makes the grow/shrink stutter. Animating only size + radius keeps it
            // smooth and snappy.
            style={{
              borderRadius: compactWindow ? 12 : 0,
              transition: ['width', 'height', 'max-width', 'max-height', 'border-radius']
                .map((p) => `${p} 0.4s cubic-bezier(0.22, 1, 0.36, 1)`)
                .join(', '),
            }}
          >
            <aside
              className={`flex flex-col border-r border-white/[0.06] py-3 ${
                IS_MOBILE
                  // Drill-down step 1: the tab list owns the whole screen. Once a
                  // tab is picked it steps aside for the content pane.
                  ? `w-full flex-shrink ${mobileDrilledIn ? 'hidden' : ''}`
                  : 'w-[240px] flex-shrink-0'
              }`}
              style={IS_MOBILE ? { paddingTop: 'var(--sn-safe-top)' } : undefined}
            >
              <div className="px-2">
                <div className="flex items-center gap-1">
                  <button
                    type="button"
                    onClick={() => selectTab('Profile')}
                    className={`flex min-w-0 flex-1 items-center gap-3 rounded-md px-2 py-2 text-left transition-colors ${
                      profileActive && !searching
                        ? 'bg-white/[0.06] text-textPrimary'
                        : 'text-textSecondary hover:bg-white/[0.03] hover:text-textPrimary'
                    }`}
                  >
                    {isAuthenticated && currentUser?.profile_image_url ? (
                      <img
                        // Twitch returns the upload at variants like
                        // {id}-profile_image-{size}x{size}.png. Force the
                        // 300x300 variant so a 32-px render has plenty of
                        // source pixels to downscale cleanly. The regex is a
                        // no-op if the URL doesn't match that shape.
                        src={currentUser.profile_image_url.replace(
                          /-(28x28|50x50|70x70|150x150)\./,
                          '-300x300.',
                        )}
                        alt=""
                        width={32}
                        height={32}
                        className="h-8 w-8 flex-shrink-0 rounded-full object-cover ring-1 ring-white/10"
                        // translateZ lifts the avatar to its own compositor
                        // layer so the parent liquid-glass-panel's heavy
                        // backdrop-filter doesn't soften the render.
                        // image-rendering hint nudges Chromium toward a
                        // crisper downscale algorithm at this aggressive ratio.
                        style={{
                          transform: 'translateZ(0)',
                          imageRendering: '-webkit-optimize-contrast' as unknown as 'auto',
                        }}
                      />
                    ) : (
                      <span
                        className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full"
                        style={{
                          background: PROFILE_META.tint,
                          boxShadow: TILE_BEVEL,
                          border: '1px solid transparent',
                        }}
                      >
                        <User size={14} strokeWidth={2.25} className="text-textPrimary" />
                      </span>
                    )}
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[13px] font-medium text-textPrimary">
                        {isAuthenticated && currentUser
                          ? currentUser.display_name || currentUser.login
                          : 'Not signed in'}
                      </div>
                      <div className="truncate text-[11px] text-textMuted">
                        {isAuthenticated && currentUser
                          ? `@${currentUser.login}`
                          : 'Sign in with Twitch'}
                      </div>
                    </div>
                  </button>

                  {/* Quick sign-out for the active account, sitting next to the
                      identity pill. Two-step confirm (matching the Accounts
                      panel) so a misclick beside the Profile nav can't sign you
                      out. signOutActiveAccount promotes a linked account if one
                      exists, else fully signs out. */}
                  {isAuthenticated && currentUser &&
                    (signOutConfirm ? (
                      <div className="flex flex-shrink-0 items-center gap-0.5">
                        <button
                          type="button"
                          onClick={() => setSignOutConfirm(false)}
                          className="rounded-md px-2 py-1 text-[11px] text-textMuted transition-colors hover:bg-white/[0.05] hover:text-textPrimary"
                        >
                          Cancel
                        </button>
                        <button
                          type="button"
                          onClick={() => {
                            setSignOutConfirm(false);
                            void signOutActiveAccount();
                            closeSettings();
                          }}
                          className="rounded-md px-2 py-1 text-[11px] font-medium text-red-400 transition-colors hover:bg-red-500/10"
                        >
                          Sign out
                        </button>
                      </div>
                    ) : (
                      <Tooltip content="Sign out" side="bottom">
                        <button
                          type="button"
                          onClick={() => setSignOutConfirm(true)}
                          aria-label="Sign out"
                          className="flex-shrink-0 rounded-md p-1.5 text-textMuted transition-colors hover:bg-red-500/10 hover:text-red-400"
                        >
                          <LogOut size={15} />
                        </button>
                      </Tooltip>
                    ))}
                </div>
              </div>
              <div className="my-3 mx-4 border-b border-white/[0.06]" />
              <div className="px-3 pb-3">
                <div className="relative">
                  <Search
                    size={13}
                    strokeWidth={2.25}
                    className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-textMuted"
                  />
                  <input
                    ref={searchInputRef}
                    type="text"
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    placeholder="Search settings"
                    className="w-full rounded-md border border-white/[0.06] bg-white/[0.03] py-1.5 pl-7 pr-7 text-[12px] text-textPrimary placeholder:text-textMuted focus:border-white/[0.12] focus:bg-white/[0.05] focus:outline-none"
                  />
                  {searching && (
                    <button
                      onClick={() => {
                        setSearchQuery('');
                        searchInputRef.current?.focus();
                      }}
                      className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-textMuted hover:bg-white/[0.06] hover:text-textPrimary"
                    >
                      <X size={12} />
                    </button>
                  )}
                </div>
                {!IS_MOBILE && (
                <p className="mt-1.5 px-0.5 text-[10.5px] leading-4 text-textMuted">
                  <kbd className="sn-keycap sn-keycap--xs">Ctrl</kbd>{' '}
                  <kbd className="sn-keycap sn-keycap--xs">K</kbd> searches all of this from
                  anywhere
                </p>
                )}
              </div>
              <div className="px-4 pb-3">
                <span className="text-[11px] uppercase tracking-[0.12em] text-textMuted">
                  Settings
                </span>
              </div>
              <nav className="scrollbar-thin flex-1 space-y-0.5 overflow-y-auto px-2">
                {tabs.map((tab) => {
                  const Icon = tab.icon;
                  const isActive = activeTab === tab.id && !searching;
                  return (
                    <button
                      key={tab.id}
                      onClick={() => {
                        selectTab(tab.id);
                        if (IS_MOBILE) setMobileDrilledIn(true);
                      }}
                      className={`flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left transition-colors ${
                        isActive
                          ? 'bg-white/[0.06] text-textPrimary'
                          : 'text-textSecondary hover:bg-white/[0.03] hover:text-textPrimary'
                      }`}
                    >
                      <span
                        className="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md"
                        style={{
                          background: tab.tint,
                          boxShadow: TILE_BEVEL,
                          border: '1px solid transparent',
                        }}
                      >
                        <Icon size={14} strokeWidth={2.25} className="text-textPrimary" />
                      </span>
                      <span className="text-[13px] font-medium">{tab.label}</span>
                    </button>
                  );
                })}
              </nav>
            </aside>

            <section
              className={`flex min-w-0 flex-1 flex-col ${
                IS_MOBILE && !mobileDrilledIn ? 'hidden' : ''
              }`}
              style={IS_MOBILE ? { paddingTop: 'var(--sn-safe-top)' } : undefined}
            >
              <div
                className={`flex items-center px-3 pt-3 ${
                  IS_MOBILE ? 'justify-between' : 'justify-end'
                }`}
              >
                {/* Drill-down back. Without it the only way out of a tab on a
                    phone is to close Settings entirely and reopen. */}
                {IS_MOBILE && (
                  <button
                    onClick={() => setMobileDrilledIn(false)}
                    className="-ml-1 flex items-center gap-1 rounded px-2 py-1.5 text-[13px] text-textSecondary"
                    data-sn-touch
                  >
                    <ChevronLeft size={18} />
                    Settings
                  </button>
                )}
                <Tooltip content="Close" delay={200} side="bottom">
                  <button
                    onClick={closeSettings}
                    className="rounded p-1.5 text-textMuted transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
                  >
                    <X size={16} />
                  </button>
                </Tooltip>
              </div>

              <div className="flex items-center gap-3 px-8 pb-3 pt-0">
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
                  <h2 className="text-[15px] font-semibold leading-tight text-textPrimary">
                    {searching ? 'Search' : activeMeta.label}
                  </h2>
                  <p className="mt-0.5 truncate text-[11px] text-textMuted">
                    {searching ? `Results for "${searchQuery}"` : activeMeta.description}
                  </p>
                </div>
                {/* Portal target for tab-specific actions. Tabs that want
                    to surface controls in the dialog hero (e.g. Profile's
                    share buttons) render into this slot via createPortal.
                    ml-auto pushes it to the right edge of the hero flex
                    row so it sits opposite the icon+title block, NOT
                    inside the scrolling content pane below. */}
                <div
                  id="settings-hero-actions"
                  className="ml-auto flex items-center gap-1"
                />
              </div>

              <div
                ref={contentRef}
                className="scrollbar-thin flex-1 overflow-y-auto px-8 pb-8"
              >
               <div className="flex items-start gap-8">
                <div className="min-w-0 flex-1">
                <Suspense
                  fallback={
                    <div className="flex h-full items-center justify-center">
                      <span className="h-5 w-5 animate-spin rounded-full border-2 border-textSecondary/40 border-t-textSecondary" />
                    </div>
                  }
                >
                {searching ? (
                  <SettingsSearchResults
                    query={searchQuery}
                    onSelect={handleResultSelect}
                  />
                ) : (
                  <>
                    {activeTab === 'Profile' && <ProfileSettings />}
                    {activeTab === 'Interface' && <InterfaceSettings />}
                    {activeTab === 'Player' && <PlayerSettings />}
                    {activeTab === 'Chat' && <ChatSettings />}
                    {activeTab === 'Moderation' && <ModerationSettings />}
                    {activeTab === 'Overlay' && <OverlaySettings />}
                    {activeTab === 'Theme' && <ThemeSettings />}
                    {activeTab === 'Integrations' && <IntegrationsSettings />}
                    {activeTab === 'Notifications' && <NotificationsSettings />}
                    {activeTab === 'Cache' && <CacheSettings />}
                    {activeTab === 'Command Palette' && <CommandPaletteSettings />}
                    {activeTab === 'Keybindings' && <KeybindingsSettings />}
                    {activeTab === 'Backup' && <BackupSettings />}
                    {activeTab === 'Support' && <SupportSettings />}
                    {activeTab === "What's New" && <WhatsNewSettings />}
                  </>
                )}
                </Suspense>
                </div>
                {/* Section rail: sticky beside the panel, spring indicator
                    follows the section being read (settings/SectionNav). */}
                <SectionNav containerRef={contentRef} tabKey={activeTab} hidden={searching} />
               </div>
              </div>
            </section>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};

export default SettingsDialog;
