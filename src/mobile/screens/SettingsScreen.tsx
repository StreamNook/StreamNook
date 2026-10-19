// Full-screen settings panel host. The section list lives inline on the You
// tab; this screen renders one tab's panel with a back header. Reuses the
// existing desktop panel components in a phone-shaped container.
import React, { Suspense, lazy, useEffect, useRef } from 'react';
import {
  ArrowLeft,
  Bell,
  Database,
  HelpCircle,
  History,
  MessageSquare,
  Palette,
  PlayCircle,
  ShieldCheck,
  ShieldHalf,
  SlidersHorizontal,
  Sparkles,
} from 'lucide-react';
import { useMobileNavStore } from '../navStore';
import { DrillInScreen } from '../ui/DrillInScreen';
import { findSettingRow, flashSettingRow } from '../../components/settings/settingsNavigation';
import { setPhoneSettingGlyphs } from '../../components/settings/phoneGlyphs';
import { settingIcon } from './settingsIcons';

// The shared settings rows lead with a glyph on the phone; the table they read
// it from is handed over here, with the phone shell, before any page draws.
setPhoneSettingGlyphs(settingIcon);

const PANELS: Record<string, React.LazyExoticComponent<React.ComponentType>> = {
  Player: lazy(() => import('../../components/settings/PlayerSettings')),
  // Phone-only: the desktop app resolves playback through its plugin seam, so
  // these settings have no desktop panel to live in.
  'Ad-free': lazy(() => import('./AdFreeSettings')),
  Chat: lazy(() => import('../../components/settings/ChatSettings')),
  // The phone's own: swipe to moderate and the user card's buttons. Desktop's
  // Moderation tab holds its hover dock, line buttons and mod logs instead.
  Moderation: lazy(() => import('../../components/settings/ModerationCardSettings')),
  Theme: lazy(() => import('../../components/settings/ThemeSettings')),
  // Mobile gets its own notifications panel: the desktop one configures
  // toasts and the Dynamic Island, which do not exist here.
  Notifications: lazy(() => import('./MobileNotificationsSettings')),
  Cache: lazy(() => import('../../components/settings/CacheSettings')),
  Support: lazy(() => import('../../components/settings/SupportSettings')),
  'Changed settings': lazy(() => import('./ChangedSettings')),
  Backup: lazy(() => import('./SettingsBackup')),
  // Mobile gets its own What's New: the desktop panel lists GitHub releases
  // from the 8.x DESKTOP line, so it described changes that never shipped to
  // the phone. This reads the Android update manifest instead.
  "What's New": lazy(() => import('./MobileWhatsNew')),
};

// The section rows the You tab renders inline. Kept beside PANELS so a new
// panel and its row cannot drift apart.
export const SETTINGS_ROWS: {
  id: keyof typeof PANELS;
  label: string;
  icon: typeof Bell;
  description: string;
}[] = [
  { id: 'Player', label: 'Player', icon: PlayCircle, description: 'Video quality, latency, and behavior' },
  {
    id: 'Ad-free',
    label: 'Ad-free',
    icon: ShieldCheck,
    description: 'How streams are served and which relays are used',
  },
  { id: 'Chat', label: 'Chat', icon: MessageSquare, description: 'Chat design and behavior' },
  {
    id: 'Moderation',
    label: 'Moderation',
    icon: ShieldHalf,
    description: 'Swipe to moderate and user card buttons',
  },
  { id: 'Theme', label: 'Theme', icon: Palette, description: 'Color theme, glass, and fonts' },
  {
    id: 'Notifications',
    label: 'Notifications',
    icon: Bell,
    description: 'System alerts for live channels and drops',
  },
  { id: 'Cache', label: 'Cache', icon: Database, description: 'Emote, badge, and metadata caches' },
  { id: 'Support', label: 'Support', icon: HelpCircle, description: 'Logs, diagnostics, and feedback' },
  {
    id: 'Changed settings',
    label: 'Changed settings',
    icon: SlidersHorizontal,
    description: 'Everything you set differently from a fresh install',
  },
  { id: 'Backup', label: 'Backup', icon: History, description: 'Daily copies, restore, and a file you can keep' },
  { id: "What's New", label: "What's New", icon: Sparkles, description: 'Recent releases' },
];

/** How long a page gets to draw its sections before a landing gives up. The
 *  panels load lazily, so the first open of one waits on its chunk. */
const LAND_WAIT_MS = 2000;
/** Lets the push finish first, so the page arrives and then moves. */
const LAND_DELAY_MS = 240;

/** A section by its heading, exact first, then ignoring case: the search
 *  catalog and a panel can capitalize a heading differently. */
function findSection(root: HTMLElement, name: string): HTMLElement | null {
  const exact = root.querySelector<HTMLElement>(`[data-settings-section="${CSS.escape(name)}"]`);
  if (exact) return exact;
  const lower = name.toLowerCase();
  return (
    Array.from(root.querySelectorAll<HTMLElement>('[data-settings-section]')).find(
      (el) => el.dataset.settingsSection?.toLowerCase() === lower,
    ) ?? null
  );
}

export const SettingsScreen: React.FC = () => {
  const settingsView = useMobileNavStore((s) => s.settingsView);
  const closeSettings = useMobileNavStore((s) => s.closeSettings);
  const settingsTarget = useMobileNavStore((s) => s.settingsTarget);
  const clearSettingsTarget = useMobileNavStore((s) => s.clearSettingsTarget);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Retained so the panel still renders while the layer slides OUT.
  // `settingsView` is already null by then, so reading it directly would blank
  // the header and body for the whole exit. Adjust-state-during-render, the same
  // idiom MobileApp uses for the tab slide direction.
  const [lastView, setLastView] = React.useState(settingsView);
  if (settingsView && settingsView !== lastView) setLastView(settingsView);

  // A page opened from another page (Changed settings, then back) reuses this
  // scroller, so each page starts at its own top rather than the last one's
  // offset. Declared before the landing effect so a landing scrolls after it.
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [lastView]);

  // Land a search hit or a changed row on its section, and on its row when the
  // page draws one by the catalog's title. The two phone-only panels carry no
  // section anchors, so a hit there stays at the page's top.
  useEffect(() => {
    if (!settingsTarget || !settingsView) return;
    let cancelled = false;
    let timer: number | undefined;
    const started = Date.now();
    const land = () => {
      if (cancelled) return;
      const root = scrollRef.current;
      const section = root ? findSection(root, settingsTarget.section) : null;
      if (!root || !section) {
        if (Date.now() - started < LAND_WAIT_MS) {
          timer = window.setTimeout(land, 60);
        } else {
          clearSettingsTarget();
        }
        return;
      }
      const row = settingsTarget.title ? findSettingRow(section, settingsTarget.title) : null;
      const el = row ?? section;
      const top = el.getBoundingClientRect().top - root.getBoundingClientRect().top + root.scrollTop - 12;
      root.scrollTo({ top: Math.max(0, top), behavior: 'smooth' });
      if (row) flashSettingRow(row);
      clearSettingsTarget();
    };
    timer = window.setTimeout(land, LAND_DELAY_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [settingsTarget, settingsView, clearSettingsTarget]);

  const Panel = lastView ? PANELS[lastView] : null;
  return (
    <DrillInScreen
      open={!!settingsView}
      layerKey="settings"
      className="absolute inset-0 z-50 bg-background flex flex-col"
      style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
    >
      <div className="flex items-center gap-1 px-2 py-2 shrink-0 border-b border-borderSubtle">
        <button
          onClick={closeSettings}
          className="sn-touch flex items-center justify-center text-textSecondary"
          aria-label="Back"
        >
          <ArrowLeft size={22} />
        </button>
        {/* A drill-in title, 17/700 as the iPhone sets it, a step under the
            tabs' large title. */}
        <h1 className="text-[17px] font-bold leading-[22px] text-textPrimary">{lastView}</h1>
      </div>
      <div
        ref={scrollRef}
        className="flex-1 min-h-0 overflow-y-auto px-4 py-3"
        style={{ paddingBottom: 'calc(var(--sn-safe-b, 0px) + 16px)' }}
      >
        {Panel ? (
          <Suspense fallback={<div className="py-10 text-center text-sm text-textMuted">Loading…</div>}>
            <Panel />
          </Suspense>
        ) : (
          <div className="py-10 text-center text-sm text-textMuted">Unknown section.</div>
        )}
      </div>
    </DrillInScreen>
  );
};
