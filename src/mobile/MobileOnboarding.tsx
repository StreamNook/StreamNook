// Logged-out landing for the mobile shell. Sign-in drives the existing
// AppStore mobile login path (native in-app WebView via the Kotlin plugin, with
// DeviceLoginOverlay as the device-code fallback rendered by MobileApp).
import React from 'react';
import { useAppStore } from '../stores/AppStore';

export const MobileOnboarding: React.FC = () => {
  const loginToTwitch = useAppStore((s) => s.loginToTwitch);
  const isLoading = useAppStore((s) => s.isLoading);

  // Stays centred, full width, including on a foldable. An earlier version
  // confined this to one pane to keep the sign-in button off the crease; that
  // was wrong. Unfolding a device is a request for MORE screen, and throwing
  // half of it away to dodge a line that foldable owners already accept is a bad
  // trade. Aligning a split that already exists to the hinge is still worth
  // doing (see AdaptiveGrid and WatchScreen's splitX) because it costs nothing;
  // relocating or shrinking content to avoid the hinge costs the whole point of
  // the device.
  return (
    <div
      className="flex-1 min-h-0 flex flex-col items-center justify-center px-8"
      style={{ paddingTop: 'var(--sn-safe-t, 0px)', paddingBottom: 'var(--sn-safe-b, 0px)' }}
    >
      <div className="text-2xl font-bold text-textPrimary mb-2">StreamNook</div>
      <div className="text-sm text-textMuted text-center mb-10 max-w-[280px]">
        Watch Twitch with your emotes, badges, and cosmetics everywhere.
      </div>
      <button
        onClick={() => void loginToTwitch()}
        disabled={isLoading}
        className="sn-touch glass-button w-full max-w-[320px] px-6 text-[15px] font-semibold text-textPrimary disabled:opacity-60"
        style={{ height: 48 }}
      >
        {isLoading ? 'Signing in…' : 'Sign in with Twitch'}
      </button>
    </div>
  );
};
