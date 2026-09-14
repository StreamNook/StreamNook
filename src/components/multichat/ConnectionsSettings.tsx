// Connections — the one place to manage platform accounts for MultiChat.
//
// Lists every provider with its connection status and a connect/disconnect
// action. Twitch is the app's native account (managed in the main app), Kick is
// wired to the OAuth flow, and the rest show as "coming soon" until their adapters
// ship. This scales as platforms light up — no more per-composer connect chips
// being the only way in.

import { useCallback } from 'react';
import { ProviderLogo } from '../ProviderLogo';
import { PROVIDERS, PROVIDER_IDS, type ProviderId } from '../../types/providers';
import { useAppStore } from '../../stores/AppStore';
import { useFollowsStore } from '../../stores/followsStore';
import { usePlatformAccountStore } from '../../stores/platformAccountStore';

type Status = 'native' | 'connected' | 'disconnected' | 'anonymous' | 'soon';

const DOT: Record<Status, string> = {
  native: '#53fc18',
  connected: '#53fc18',
  disconnected: 'rgba(255,255,255,0.25)',
  anonymous: '#53fc18',
  soon: 'rgba(245,158,11,0.7)',
};

const LABEL: Record<Status, string> = {
  native: 'Connected · managed in the main app',
  connected: 'Connected',
  disconnected: 'Not connected',
  anonymous: 'Reading anonymously · no sign-in needed',
  soon: 'Coming soon',
};

export default function ConnectionsSettings() {
  const currentUser = useAppStore((s) => s.currentUser);
  // All of this used to be local state kept current by a 5s poll of
  // `kick_is_connected` / `youtube_is_connected`. It now comes from the shared
  // store, which is event-driven, so this panel costs nothing while it is open.
  const kick = usePlatformAccountStore((s) => s.kick);
  const youtube = usePlatformAccountStore((s) => s.youtube);
  const connectPlatform = usePlatformAccountStore((s) => s.connect);
  const disconnectPlatform = usePlatformAccountStore((s) => s.disconnect);
  const resyncYoutube = usePlatformAccountStore((s) => s.resyncYoutube);
  const { connected: kickConnected, name: kickName, busy: kickBusy, step: kickStep } = kick;
  const {
    connected: youtubeConnected,
    name: youtubeName,
    busy: youtubeBusy,
    step: youtubeStep,
  } = youtube;

  // Connecting is ONE action per platform however many credentials it takes, and
  // the flow itself lives in the store so this panel and Settings > Profile >
  // Accounts cannot drift into two different ways of connecting the same account.
  const connectKick = useCallback(() => void connectPlatform('kick'), [connectPlatform]);
  const disconnectKick = useCallback(() => void disconnectPlatform('kick'), [disconnectPlatform]);
  const connectYoutube = useCallback(() => void connectPlatform('youtube'), [connectPlatform]);
  const disconnectYoutube = useCallback(
    () => void disconnectPlatform('youtube'),
    [disconnectPlatform],
  );

  // Connecting Kick brings the follow list with it, so the row can report how
  // many channels came across rather than making that a second thing to do.
  const follows = useFollowsStore((s) => s.follows);
  const kickFollowCount = follows.filter((f) => f.provider === 'kick').length;
  const youtubeFollowCount = follows.filter((f) => f.provider === 'youtube').length;

  const statusFor = (p: ProviderId): Status => {
    if (p === 'twitch') return 'native';
    if (p === 'kick') return kickConnected ? 'connected' : 'disconnected';
    if (p === 'youtube') return youtubeConnected ? 'connected' : 'disconnected';
    // A read-only adapter with no sign-in (TikTok) is working as designed, not
    // "not connected"; only platforms without an adapter are still upcoming.
    if (PROVIDERS[p].chatEnabled) {
      return PROVIDERS[p].send === 'none' ? 'anonymous' : 'disconnected';
    }
    return 'soon';
  };

  // Subtitle, naming the connected account where we know it.
  const subtitleFor = (p: ProviderId, status: Status): string => {
    if (p === 'twitch') {
      return currentUser?.display_name ? `Connected as ${currentUser.display_name}` : LABEL.native;
    }
    if (p === 'kick' && status === 'connected') {
      const who = kickName ? `Connected as ${kickName}` : 'Connected';
      // Naming the follow count here is what tells the user the connection
      // actually brought their channels across, with no second row to explain.
      return kickFollowCount > 0
        ? `${who} · ${kickFollowCount} channel${kickFollowCount === 1 ? '' : 's'}`
        : who;
    }
    if (p === 'youtube' && status === 'connected') {
      const who = youtubeName ? `Connected as ${youtubeName}` : 'Connected';
      // Same reasoning as Kick: naming the count is what tells the user the
      // connection actually brought their channels across.
      return youtubeFollowCount > 0
        ? `${who} · ${youtubeFollowCount} channel${youtubeFollowCount === 1 ? '' : 's'}`
        : who;
    }
    return LABEL[status];
  };

  return (
    <div className="space-y-4">
      <p className="text-xs text-textSecondary">
        Connect your platform accounts to browse, watch, and chat across StreamNook. More platforms
        unlock as their integrations ship.
      </p>

      <div className="hairline-y overflow-hidden rounded-lg border border-borderSubtle">
        {PROVIDER_IDS.map((p) => {
          const meta = PROVIDERS[p];
          const status = statusFor(p);
          return (
            <div
              key={p}
              className="flex items-center gap-3 px-3 py-3"
              style={{ opacity: status === 'soon' ? 0.6 : 1 }}
            >
              <ProviderLogo provider={p} size={22} />
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium text-textPrimary">{meta.label}</div>
                <div className="mt-0.5 flex items-center gap-1.5 text-xs text-textSecondary">
                  <span
                    className="inline-block h-1.5 w-1.5 rounded-full"
                    style={{ backgroundColor: DOT[status] }}
                  />
                  {p === 'kick' && kickStep
                    ? kickStep
                    : p === 'youtube' && youtubeStep
                      ? youtubeStep
                      : subtitleFor(p, status)}
                </div>
              </div>

              {/* Action — Kick + YouTube connect through here. */}
              {p === 'kick' &&
                (kickConnected ? (
                  <button
                    type="button"
                    onClick={disconnectKick}
                    className="glass-button-secondary shrink-0 px-3 py-1 text-xs font-medium text-textSecondary transition-colors hover:text-red-400"
                  >
                    Disconnect
                  </button>
                ) : (
                  <button
                    type="button"
                    onClick={() => void connectKick()}
                    disabled={kickBusy}
                    className="glass-button-secondary shrink-0 px-3 py-1 text-xs font-semibold transition-colors disabled:opacity-60"
                    style={{ color: '#53fc18' }}
                  >
                    {kickBusy ? 'Connecting…' : 'Connect'}
                  </button>
                ))}
              {p === 'youtube' &&
                (youtubeConnected ? (
                  <div className="flex shrink-0 items-center gap-1.5">
                    <button
                      type="button"
                      onClick={() => void resyncYoutube()}
                      disabled={youtubeBusy}
                      className="glass-button-secondary px-3 py-1 text-xs font-medium text-textSecondary transition-colors hover:text-textPrimary disabled:opacity-60"
                    >
                      Sync
                    </button>
                    <button
                      type="button"
                      onClick={disconnectYoutube}
                      className="glass-button-secondary px-3 py-1 text-xs font-medium text-textSecondary transition-colors hover:text-red-400"
                    >
                      Disconnect
                    </button>
                  </div>
                ) : (
                  <button
                    type="button"
                    onClick={() => void connectYoutube()}
                    disabled={youtubeBusy}
                    className="glass-button-secondary shrink-0 px-3 py-1 text-xs font-semibold transition-colors disabled:opacity-60"
                    style={{ color: '#ff4d4d' }}
                  >
                    {youtubeBusy ? 'Connecting…' : 'Connect'}
                  </button>
                ))}

              {status === 'soon' && (
                <span className="shrink-0 rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-500/80">
                  Soon
                </span>
              )}
            </div>
          );
        })}
      </div>

    </div>
  );
}
