// Floating glass pill tab bar: detached from the bottom edge, riding above the
// gesture inset. The same object as desktop Home's floating strip: a dark
// glass capsule over the grid, with the selected tab a lit capsule that glides
// between tabs. The You tab becomes your avatar once signed in.
import React from 'react';
import { motion } from 'framer-motion';
import { Compass, Heart, Package, UserCircle } from 'phosphor-react';
import { useAppStore } from '../stores/AppStore';
import { useMobileNavStore, type MobileTab } from './navStore';

const TABS: { id: MobileTab; label: string; Icon: typeof Heart }[] = [
  { id: 'following', label: 'Following', Icon: Heart },
  { id: 'browse', label: 'Browse', Icon: Compass },
  { id: 'rewards', label: 'Rewards', Icon: Package },
  { id: 'you', label: 'You', Icon: UserCircle },
];

export const MobileTabBar: React.FC<{
  /** The full-screen player covers the shell: keep the bar out of the
   *  compositor entirely rather than blurring a backdrop nobody can see. */
  hidden?: boolean;
}> = ({ hidden = false }) => {
  const activeTab = useMobileNavStore((s) => s.activeTab);
  const setTab = useMobileNavStore((s) => s.setTab);
  const avatarUrl = useAppStore((s) => s.currentUser?.profile_image_url);

  return (
    <nav
      // glass-panel--dark exists for exactly this contrast problem (chrome over
      // a bright thumbnail grid), and the shared classes ride the Glassiness
      // slider, which the old hand-rolled blur ignored.
      className="fixed z-30 mx-auto max-w-[520px] glass-panel glass-panel--dark !rounded-full px-1.5"
      style={{
        visibility: hidden ? 'hidden' : undefined,
        // Capped width. Stretched across a tablet or an unfolded Fold the tabs
        // end up a hand-span apart, and nothing about a nav bar needs 1200px.
        left: 'calc(var(--sn-safe-l, 0px) + 20px)',
        right: 'calc(var(--sn-safe-r, 0px) + 20px)',
        bottom: 'calc(var(--sn-safe-b, 0px) + 14px)',
        boxShadow: '0 8px 24px -12px rgba(0,0,0,0.45)',
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
                active ? 'text-textPrimary' : 'text-textMuted'
              }`}
              aria-current={active ? 'page' : undefined}
              aria-label={label}
            >
              {/* The lit capsule hugs the glyph, not the whole tab column, and
                  glides between tabs on one shared layoutId, the way the
                  desktop strip's highlight does. */}
              <span className="relative flex items-center justify-center w-14 h-10">
                {active && (
                  <motion.span
                    layoutId="mobileTabHighlight"
                    className="absolute inset-0 chrome-glaze chrome-glaze--flat chrome-glaze--control"
                    transition={{ type: 'spring', stiffness: 350, damping: 30 }}
                  />
                )}
                <span className="relative z-10 flex items-center">
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
                </span>
              </span>
            </button>
          );
        })}
      </div>
    </nav>
  );
};
