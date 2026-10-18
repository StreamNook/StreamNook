// Floating glass pill tab bar: detached from the bottom edge, riding above the
// gesture inset. The same object as desktop Home's floating strip: the whole
// bar wears the glaze, and the selected tab is a darker pill set into it that
// glides between tabs. The You tab becomes your picture once signed in. Under
// the bar, a floor of the page's own background fades content out instead of
// letting it show through the glass.
import React, { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { ChatsCircle, Compass, Heart, Package, UserCircle } from 'phosphor-react';
import { useAppStore } from '../stores/AppStore';
import { useMobileNavStore, type MobileTab } from './navStore';
import { usePicksStore } from './profile/picksStore';
import { KeyboardDismiss } from './ui/KeyboardDismiss';
import { useChatTabsStore } from './chat/chatTabsStore';

const TABS: { id: MobileTab; label: string; Icon: typeof Heart }[] = [
  { id: 'following', label: 'Following', Icon: Heart },
  { id: 'browse', label: 'Browse', Icon: Compass },
  { id: 'chats', label: 'Chats', Icon: ChatsCircle },
  { id: 'rewards', label: 'Rewards', Icon: Package },
  { id: 'you', label: 'You', Icon: UserCircle },
];

/** The icon canvas, the size the other tabs' glyphs take. */
const CANVAS = 26;
/** A filled circle reads larger than an outline glyph of the same box, so the
 *  plain picture is a touch smaller than the canvas. */
const PLAIN = 24;
/** Selected: a ring at the canvas edge, a gap, the picture inside. */
const RING = 1.5;
const GAP = 1.5;

/**
 * The picture your You card shows: your 7TV still when you have one, else
 * your Twitch picture. Read through the same store the You card fills (the
 * saved identity first, then fresh), so the two never disagree and nothing is
 * fetched twice. Answers null until the identity has had its say, so a cold
 * start does not flash the Twitch picture before the 7TV one.
 */
function useYouPicture(): { userId: string; url: string | null } {
  const userId = useAppStore((s) => s.currentUser?.user_id ?? '');
  const login = useAppStore((s) => s.currentUser?.login || s.currentUser?.username || '');
  const displayName = useAppStore((s) => s.currentUser?.display_name || '');
  const twitchPicture = useAppStore((s) => s.currentUser?.profile_image_url || null);
  const hasIdentity = usePicksStore((s) => !!userId && s.userId === userId && !!s.identity);
  const still = usePicksStore((s) => (userId && s.userId === userId ? s.identity?.picture?.still : undefined));
  const [settledFor, setSettledFor] = useState<string | null>(null);

  useEffect(() => {
    if (!userId) return;
    let live = true;
    void usePicksStore
      .getState()
      .loadIdentity({ userId, login: login || displayName, displayName: displayName || login })
      .finally(() => {
        if (live) setSettledFor(userId);
      });
    return () => {
      live = false;
    };
  }, [userId, login, displayName]);

  // Settled covers the identity read failing (offline, no 7TV): the Twitch
  // picture then stands in.
  const resolved = hasIdentity || settledFor === userId;
  return { userId, url: resolved ? still || twitchPicture : null };
}

/** Your picture in the You tab, or the person glyph until it has loaded. A new
 *  picture replaces the old one only once it is decoded, so the icon never
 *  flashes empty. */
const YouTabIcon: React.FC<{ selected: boolean }> = ({ selected }) => {
  const { userId, url } = useYouPicture();
  const [shown, setShown] = useState<{ userId: string; url: string } | null>(null);

  useEffect(() => {
    if (!url || !userId) return;
    let live = true;
    const img = new Image();
    img.decoding = 'async';
    img.onload = () => {
      if (live) setShown({ userId, url });
    };
    img.src = url;
    return () => {
      live = false;
    };
  }, [url, userId]);

  // Another account's picture never stands in for yours.
  if (!shown || shown.userId !== userId) {
    return <UserCircle size={24} weight={selected ? 'fill' : 'regular'} />;
  }
  const diameter = selected ? CANVAS - 2 * (RING + GAP) : PLAIN;
  return (
    <span
      className="relative flex items-center justify-center rounded-full"
      style={{
        width: CANVAS,
        height: CANVAS,
        boxShadow: selected ? `inset 0 0 0 ${RING}px var(--color-accent)` : undefined,
      }}
    >
      {/* No backing (Brandon: a transparent picture stays transparent in the
          tab bar). The You card and Edit profile keep their black disc. */}
      <span className="block overflow-hidden rounded-full" style={{ width: diameter, height: diameter }}>
        <img src={shown.url} alt="" draggable={false} className="h-full w-full object-cover" />
      </span>
    </span>
  );
};

export const MobileTabBar: React.FC<{
  /** The full-screen player covers the shell: keep the bar out of the
   *  compositor entirely rather than blurring a backdrop nobody can see. */
  hidden?: boolean;
  /** The fade of the page's own background under the bar. Off on the Chats
   *  screen, whose composer sits right above the bar and would fade into it. */
  floor?: boolean;
}> = ({ hidden = false, floor = true }) => {
  const activeTab = useMobileNavStore((s) => s.activeTab);
  const setTab = useMobileNavStore((s) => s.setTab);
  // Unread mentions across your chats, counted by Rust.
  const mentions = useChatTabsStore((s) => s.mentionTotal);

  return (
    <>
      <div
        aria-hidden
        className="sn-tabbar-floor z-[29]"
        style={{ visibility: hidden || !floor ? 'hidden' : undefined }}
      />
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
            return (
              <button
                key={id}
                onClick={() => setTab(id)}
                // The chosen tab reads strongest: primary text, a filled icon
                // and a semibold label in the dark pill. The others are
                // secondary text, which holds its contrast over the floor under
                // the bar. The accent read dimmer than the white tabs beside it
                // (Winters Glass's accent is a soft gray-blue), so the chosen
                // tab looked like the one switched off.
                className={`sn-touch flex-1 flex items-center justify-center transition-colors ${
                  active ? 'text-textPrimary' : 'text-textSecondary'
                }`}
                aria-current={active ? 'page' : undefined}
                aria-label={id === 'chats' && mentions > 0 ? `${label}, ${mentions} unread mentions` : label}
              >
                {/* Icons only (Brandon: with five tabs, "Following" spilled out
                    of the selection's fill). The name stays the button's
                    accessible label. The selected pill is sized to the icon,
                    one per tab at even spacing, and glides between tabs on one
                    shared layoutId, the way the desktop strip's highlight does. */}
                <span className="relative flex items-center justify-center w-[56px] h-[44px]">
                  {active && (
                    <motion.span
                      layoutId="mobileTabHighlight"
                      className="absolute inset-0 glaze-selected"
                      transition={{ type: 'spring', stiffness: 350, damping: 30 }}
                    />
                  )}
                  <span className="relative z-10 flex h-[26px] items-center">
                    {id === 'you' ? (
                      <YouTabIcon selected={active} />
                    ) : (
                      <Icon size={24} weight={active ? 'fill' : 'regular'} />
                    )}
                    {id === 'chats' && mentions > 0 && (
                      <span
                        aria-hidden
                        className="absolute -top-1 left-[15px] min-w-[16px] h-4 px-1 rounded-full bg-error text-white text-[10.5px] font-semibold leading-4 text-center tabular-nums"
                      >
                        {mentions > 99 ? '99+' : mentions}
                      </span>
                    )}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      </nav>
      {/* The keyboard's way down on every screen (it never closes on scroll). */}
      <KeyboardDismiss />
    </>
  );
};
