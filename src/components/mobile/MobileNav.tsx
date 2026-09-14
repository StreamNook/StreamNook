import { Heart, Compass, Gift, Settings as SettingsIcon, Tv } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore } from '../../stores/AppStore';

/**
 * Bottom tab bar. Replaces the desktop Sidebar, which is built around edge-hover
 * reveal and drag-to-resize and so has no touch equivalent.
 *
 * Deliberately drives the EXISTING store fields rather than introducing routing:
 * `navigateToHomeTab` already sets isHomeActive + homeActiveTab together, and
 * `isHomeActive` is what swaps Home for the watch view. That means no store
 * changes, and the desktop keeps working untouched.
 */

type Tab = {
  key: string;
  label: string;
  Icon: typeof Heart;
  isActive: (s: { isHomeActive: boolean; homeActiveTab: string; showDropsOverlay: boolean }) => boolean;
  onPress: () => void;
};

export default function MobileNav() {
  const { isHomeActive, homeActiveTab, showDropsOverlay, streamUrl } = useAppStore(
    useShallow((s) => ({
      isHomeActive: s.isHomeActive,
      homeActiveTab: s.homeActiveTab,
      showDropsOverlay: s.showDropsOverlay,
      streamUrl: s.streamUrl,
    })),
  );
  // Actions are stable for the store's lifetime, so read them without subscribing.
  const { navigateToHomeTab, setShowDropsOverlay, openSettings, toggleHome } =
    useAppStore.getState();

  const tabs: Tab[] = [
    {
      key: 'following',
      label: 'Following',
      Icon: Heart,
      isActive: (s) => s.isHomeActive && s.homeActiveTab === 'following' && !s.showDropsOverlay,
      onPress: () => {
        setShowDropsOverlay(false);
        navigateToHomeTab('following');
      },
    },
    {
      key: 'browse',
      label: 'Browse',
      Icon: Compass,
      isActive: (s) =>
        s.isHomeActive &&
        (s.homeActiveTab === 'browse' || s.homeActiveTab === 'category' || s.homeActiveTab === 'search') &&
        !s.showDropsOverlay,
      onPress: () => {
        setShowDropsOverlay(false);
        navigateToHomeTab('browse');
      },
    },
    // Only offered once something is actually playing, so the tab can never be a
    // dead end that drops you on a blank player.
    ...(streamUrl
      ? [
          {
            key: 'watch',
            label: 'Watch',
            Icon: Tv,
            isActive: (s: { isHomeActive: boolean; showDropsOverlay: boolean }) =>
              !s.isHomeActive && !s.showDropsOverlay,
            onPress: () => {
              setShowDropsOverlay(false);
              // toggleHome flips rather than sets, so guard it: pressing Watch
              // while already watching must be a no-op, not a bounce back to Home.
              if (useAppStore.getState().isHomeActive) toggleHome();
            },
          } as Tab,
        ]
      : []),
    {
      key: 'drops',
      label: 'Drops',
      Icon: Gift,
      isActive: (s) => s.showDropsOverlay,
      onPress: () => setShowDropsOverlay(true),
    },
    {
      key: 'settings',
      label: 'Settings',
      Icon: SettingsIcon,
      isActive: () => false,
      onPress: () => openSettings(),
    },
  ];

  const state = { isHomeActive, homeActiveTab, showDropsOverlay };

  return (
    <nav
      // Sits above the player and chat but below modals. The bottom padding is the
      // gesture-pill inset: targetSdk 36 forces edge-to-edge, so without it the
      // labels sit underneath the system navigation.
      className="relative z-30 flex flex-shrink-0 items-stretch border-t border-borderLight/60 bg-background"
      style={{ paddingBottom: 'var(--sn-safe-bottom)' }}
    >
      {tabs.map(({ key, label, Icon, isActive, onPress }) => {
        const active = isActive(state);
        return (
          <button
            key={key}
            type="button"
            onClick={onPress}
            aria-current={active ? 'page' : undefined}
            className={`flex flex-1 flex-col items-center justify-center gap-0.5 py-2 text-[11px] transition-colors ${
              active ? 'text-accent' : 'text-textSecondary'
            }`}
          >
            <Icon className="h-5 w-5" strokeWidth={active ? 2.4 : 1.8} />
            <span className="leading-none">{label}</span>
          </button>
        );
      })}
    </nav>
  );
}
