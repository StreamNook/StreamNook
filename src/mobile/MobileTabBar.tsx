// Floating glass pill tab bar: detached from the bottom edge, riding above the
// gesture inset. The same object as desktop Home's floating strip: the whole
// bar wears the glaze, and the selected tab is a darker pill set into it that
// glides between tabs. The You tab becomes your avatar once signed in.
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
      // The dock glaze: clear glass with a light blur and a thin black film,
      // tuned on a phone over a bright thumbnail grid. It still rides the
      // Glassiness slider.
      className="fixed z-30 mx-auto max-w-[520px] chrome-glaze chrome-glaze--dock px-1.5"
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
              // Unselected icons are the theme's text colour, not muted: over
              // bright thumbnails a muted icon disappears. The selected one takes
              // the theme accent inside the dark pill.
              className={`sn-touch flex-1 flex items-center justify-center transition-colors ${
                active ? 'text-accent' : 'text-textPrimary'
              }`}
              aria-current={active ? 'page' : undefined}
              aria-label={label}
            >
              {/* The selected pill hugs the glyph, not the whole tab column,
                  leaves the same strip of glass above and below it, and
                  glides between tabs on one shared layoutId, the way the
                  desktop strip's highlight does. */}
              <span className="relative flex items-center justify-center w-[62px] h-11">
                {active && (
                  <motion.span
                    layoutId="mobileTabHighlight"
                    className="absolute inset-0 glaze-selected"
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
                    <Icon size={25} weight={active ? 'fill' : 'regular'} />
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
