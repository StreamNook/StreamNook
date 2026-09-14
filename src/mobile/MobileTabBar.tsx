// Floating glass pill tab bar: detached from the bottom edge, transparent
// glass with the canonical inset bevel (--bevel-tile), riding above the
// gesture inset. The You tab becomes your avatar once signed in.
import React from 'react';
import { Compass, Gift, Heart, UserCircle } from 'phosphor-react';
import { useAppStore } from '../stores/AppStore';
import { useMobileNavStore, type MobileTab } from './navStore';

const TABS: { id: MobileTab; label: string; Icon: typeof Heart }[] = [
  { id: 'following', label: 'Following', Icon: Heart },
  { id: 'browse', label: 'Browse', Icon: Compass },
  { id: 'rewards', label: 'Rewards', Icon: Gift },
  { id: 'you', label: 'You', Icon: UserCircle },
];

export const MobileTabBar: React.FC = () => {
  const activeTab = useMobileNavStore((s) => s.activeTab);
  const setTab = useMobileNavStore((s) => s.setTab);
  const avatarUrl = useAppStore((s) => s.currentUser?.profile_image_url);

  return (
    <nav
      className="fixed z-30 rounded-full mx-auto max-w-[520px]"
      style={{
        // Capped width. Stretched across a tablet or an unfolded Fold the tabs
        // end up a hand-span apart, and nothing about a nav bar needs 1200px.
        left: 'calc(var(--sn-safe-l, 0px) + 20px)',
        right: 'calc(var(--sn-safe-r, 0px) + 20px)',
        bottom: 'calc(var(--sn-safe-b, 0px) + 14px)',
        background: 'color-mix(in srgb, var(--color-background-secondary) 78%, transparent)',
        backdropFilter: 'blur(22px)',
        WebkitBackdropFilter: 'blur(22px)',
        boxShadow: 'var(--bevel-tile), 0 8px 24px -12px rgba(0,0,0,0.45)',
      }}
    >
      <div className="flex" style={{ height: 'var(--sn-tabbar-h, 56px)' }}>
        {TABS.map(({ id, label, Icon }) => {
          const active = id === activeTab;
          const isYouWithAvatar = id === 'you' && !!avatarUrl;
          return (
            <button
              key={id}
              onClick={() => setTab(id)}
              className={`sn-touch flex-1 flex items-center justify-center transition-colors ${
                active ? 'text-accent' : 'text-textMuted'
              }`}
              aria-current={active ? 'page' : undefined}
              aria-label={label}
            >
              {isYouWithAvatar ? (
                <img
                  src={avatarUrl}
                  alt=""
                  draggable={false}
                  className={`w-[26px] h-[26px] rounded-full object-cover ${
                    active ? 'ring-2 ring-accent' : ''
                  }`}
                />
              ) : (
                <Icon size={24} weight={active ? 'fill' : 'regular'} />
              )}
            </button>
          );
        })}
      </div>
    </nav>
  );
};
