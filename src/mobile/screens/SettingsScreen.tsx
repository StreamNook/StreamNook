// Full-screen settings panel host. The section list lives inline on the You
// tab; this screen renders one tab's panel with a back header. Reuses the
// existing desktop panel components in a phone-shaped container.
import React, { Suspense, lazy } from 'react';
import { ArrowLeft, Bell, Database, HelpCircle, MessageSquare, Palette, PlayCircle, ShieldCheck, Sparkles } from 'lucide-react';
import { useMobileNavStore } from '../navStore';
import { DrillInScreen } from '../ui/DrillInScreen';

const PANELS: Record<string, React.LazyExoticComponent<React.ComponentType>> = {
  Player: lazy(() => import('../../components/settings/PlayerSettings')),
  // Phone-only: the desktop app resolves playback through its plugin seam, so
  // these settings have no desktop panel to live in.
  'Ad-free': lazy(() => import('./AdFreeSettings')),
  Chat: lazy(() => import('../../components/settings/ChatSettings')),
  Theme: lazy(() => import('../../components/settings/ThemeSettings')),
  // Mobile gets its own notifications panel: the desktop one configures
  // toasts and the Dynamic Island, which do not exist here.
  Notifications: lazy(() => import('./MobileNotificationsSettings')),
  Cache: lazy(() => import('../../components/settings/CacheSettings')),
  Support: lazy(() => import('../../components/settings/SupportSettings')),
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
  { id: 'Theme', label: 'Theme', icon: Palette, description: 'Color theme, glass, and fonts' },
  {
    id: 'Notifications',
    label: 'Notifications',
    icon: Bell,
    description: 'System alerts for live channels and drops',
  },
  { id: 'Cache', label: 'Cache', icon: Database, description: 'Emote, badge, and metadata caches' },
  { id: 'Support', label: 'Support', icon: HelpCircle, description: 'Logs, diagnostics, and feedback' },
  { id: "What's New", label: "What's New", icon: Sparkles, description: 'Recent releases' },
];

export const SettingsScreen: React.FC = () => {
  const settingsView = useMobileNavStore((s) => s.settingsView);
  const closeSettings = useMobileNavStore((s) => s.closeSettings);

  // Retained so the panel still renders while the layer slides OUT.
  // `settingsView` is already null by then, so reading it directly would blank
  // the header and body for the whole exit. Adjust-state-during-render, the same
  // idiom MobileApp uses for the tab slide direction.
  const [lastView, setLastView] = React.useState(settingsView);
  if (settingsView && settingsView !== lastView) setLastView(settingsView);

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
        <h1 className="text-lg font-bold text-textPrimary">{lastView}</h1>
      </div>
      <div
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
