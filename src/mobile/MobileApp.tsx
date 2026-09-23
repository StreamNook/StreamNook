// Mobile shell root. The phone counterpart of App.tsx: same stores, same
// services, same theme system, phone-shaped chrome (bottom tabs, sheets,
// safe-area frame). Boot orchestration is deliberately replicated, not shared;
// see boot/useMobileBoot.ts for the contract with App.tsx.
import React, { lazy, Suspense, useEffect, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import './mobile.css';
import { useAppStore } from '../stores/AppStore';
import { applyNativeInsetsOnce } from './nativeInsets';
import { useThemeBoot } from '../boot/useThemeBoot';
import { useMobileBoot } from './boot/useMobileBoot';
import { useKeyboardInsets } from './ui/useKeyboardInsets';
import { useCurrentStreamStats } from '../utils/useCurrentStreamStats';
import { installBackHandler, useMobileNavStore } from './navStore';
import { installLifecycle } from './lifecycle';
import { installVideoCap } from './videoCap';
import { MobileTabBar } from './MobileTabBar';
import { MobileOnboarding } from './MobileOnboarding';
// The real wizard, not a phone-shaped rewrite of it. SetupWizard is already
// mobile-adapted from the in-place-shell days: swipe navigation (framer `drag`),
// mobile padding and safe areas, a mobile back control, the pagination dots as a
// swipeable carousel, and MOBILE_SKIPPED_STEPS to drop steps that configure
// surfaces a phone does not have. Lazy so its ~1100 lines stay out of the main
// chunk for everyone past first run.
const SetupWizard = lazy(() => import('../components/SetupWizard'));
import { FollowingScreen } from './screens/FollowingScreen';
import { BrowseScreen } from './screens/BrowseScreen';
import { YouScreen } from './screens/YouScreen';
import { WatchScreen } from './screens/WatchScreen';
import { SettingsScreen } from './screens/SettingsScreen';
import { CategoryStreamsScreen } from './screens/CategoryStreamsScreen';
import { RewardsScreen } from './screens/RewardsScreen';
import { CosmeticsScreen } from './screens/CosmeticsScreen';
import { MobileClipSheet } from './watch/MobileClipSheet';
import LoadingWidget from '../components/LoadingWidget';
import DeviceLoginOverlay from '../components/DeviceLoginOverlay';
import ToastManager from '../components/ToastManager';
import { usePlatformAccountSync } from '../hooks/usePlatformAccountSync';

const TAB_ORDER = ['following', 'browse', 'rewards', 'you'] as const;

const MobileApp: React.FC = () => {
  useThemeBoot();
  useMobileBoot();
  // Kick account state (connected, name, busy) for the composer and Accounts.
  // Mounted exactly once; the hook warns on a second mount.
  usePlatformAccountSync();
  useKeyboardInsets();
  // Shared with the desktop shell: the watched stream's viewer count is written
  // once at playback start and never refreshed without this.
  useCurrentStreamStats();

  const isBooting = useAppStore((s) => s.isBooting);
  const isAuthenticated = useAppStore((s) => s.isAuthenticated);
  const setupComplete = useAppStore((s) => s.settings.setup_complete);
  const [wizardDismissed, setWizardDismissed] = useState(false);
  const activeTab = useMobileNavStore((s) => s.activeTab);

  // Is the tab shell completely hidden behind the full-screen watch layer?
  //
  // The shell stays MOUNTED while you watch (so the back gesture and scroll
  // position survive), which means its stream cards keep animating their live
  // dots and hype-train shine forever, on a list nobody can see. Measured on
  // device: 29 `animate-ping` halos plus a hype-train badge, all running with
  // `iteration-count: infinite`, 24 of them off-screen, costing ~4.6% of a CPU
  // core. Each animating element is its own composited layer, so the compositor
  // services all of them every frame.
  //
  // Only in `full`. In `mini` the shell is genuinely visible behind the small
  // player, and the dots have to keep moving there.
  const playerMode = useMobileNavStore((s) => s.playerMode);
  const streamUrl = useAppStore((s) => s.streamUrl);
  const shellCovered = playerMode === 'full' && !!streamUrl && streamUrl !== 'offline';

  // Same backstop the desktop gate uses, and it matters MORE here: the wizard's
  // own comment records that `setup_complete` has been seen reverting to false
  // between launches ON ANDROID specifically, which reopens the wizard forever
  // and makes the app unusable. The wizard writes this key itself on finish, so
  // reading it is all that is needed. Key must match SETUP_COMPLETE_MARKER in
  // App.tsx.
  const setupMarked = (() => {
    try {
      return localStorage.getItem('streamnook-setup-complete') === 'true';
    } catch {
      return false;
    }
  })();
  const needsSetup = !setupComplete && !setupMarked && !wizardDismissed;

  // Slide direction for the tab transition: toward the newly selected tab.
  // Adjust-state-during-render idiom (no refs in render, no extra effect pass).
  const tabIndex = TAB_ORDER.indexOf(activeTab);
  const [prevTabIndex, setPrevTabIndex] = React.useState(tabIndex);
  const [direction, setDirection] = React.useState(1);
  if (prevTabIndex !== tabIndex) {
    setDirection(tabIndex >= prevTabIndex ? 1 : -1);
    setPrevTabIndex(tabIndex);
  }

  useEffect(() => installBackHandler(), []);
  useEffect(() => installLifecycle(), []);
  // Teach the resolver what this screen can show, before any stream resolves.
  useEffect(() => installVideoCap(), []);
  useEffect(() => {
    applyNativeInsetsOnce();
  }, []);

  return (
    <div className="sn-mobile bg-background text-textPrimary">
      {isBooting ? (
        <div className="flex-1 min-h-0 flex items-center justify-center">
          <LoadingWidget fullScreen={false} message="Loading StreamNook" size={56} />
        </div>
      ) : needsSetup ? (
        /* FIRST RUN WINS OVER AUTH, and the order here is the whole point: the
           wizard performs the Twitch sign-in as one of its own steps (it calls
           loginToTwitch directly). Gating it behind `isAuthenticated` — which is
           what this did at first — means a signed-out first-run user can never
           reach it, and lands on the bare sign-in screen instead. Desktop gates
           purely on setup_complete for exactly this reason.
           `wizardDismissed` is the escape hatch so closing without finishing
           does not trap you here. */
        <Suspense
          fallback={
            <div className="flex-1 min-h-0 flex items-center justify-center">
              <LoadingWidget fullScreen={false} size={56} />
            </div>
          }
        >
          <SetupWizard isOpen onClose={() => setWizardDismissed(true)} />
        </Suspense>
      ) : !isAuthenticated ? (
        /* Signed out AFTER setup: a sign-out, a token expiry, a forced re-auth.
           Setup is already done, so re-running the wizard would be wrong; this
           is just the way back in. */
        <MobileOnboarding />
      ) : (
        <>
          <div
            className="flex-1 min-h-0 flex flex-col relative overflow-hidden"
            data-sn-covered={shellCovered ? 'true' : undefined}
            style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
          >
            <AnimatePresence mode="popLayout" initial={false} custom={direction}>
              <motion.div
                key={activeTab}
                className="flex-1 min-h-0 flex flex-col"
                initial={{ opacity: 0, x: 28 * direction }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: -28 * direction }}
                transition={{ duration: 0.16, ease: [0.2, 0.8, 0.2, 1] }}
              >
                {activeTab === 'following' && <FollowingScreen />}
                {activeTab === 'browse' && <BrowseScreen />}
                {activeTab === 'rewards' && <RewardsScreen />}
                {activeTab === 'you' && <YouScreen />}
              </motion.div>
            </AnimatePresence>
          </div>
          {/* Floating pill bar (fixed) and the full-screen layers above it. */}
          <MobileTabBar hidden={shellCovered} />
          <CategoryStreamsScreen />
          <WatchScreen />
          <CosmeticsScreen />
          <SettingsScreen />
          {/* Consumes the same `clipModal` store field the desktop <ClipModal />
              does. Without a mobile consumer, tapping a clip in chat flipped
              that state and rendered nothing. Portals to z-[9000] via
              MobileSheet, so it clears the watch layer regardless of position
              here. */}
          <MobileClipSheet />
        </>
      )}

      {/* Always mounted: device-code login fallback + toasts. */}
      <DeviceLoginOverlay />
      <ToastManager />
    </div>
  );
};

export default MobileApp;
