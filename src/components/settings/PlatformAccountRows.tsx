import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Loader2, Unlink, Users } from 'lucide-react';
import { ProviderLogo } from '../ProviderLogo';
import { PROVIDERS } from '../../types/providers';
import { usePlatformAccountStore } from '../../stores/platformAccountStore';
import { useFollowsStore } from '../../stores/followsStore';
import type { PlatformId } from '../../services/platformAccountService';
import { Tooltip } from '../ui/Tooltip';

/**
 * Kick and YouTube account rows, in the same visual language as the Twitch
 * accounts above them.
 *
 * These used to live in MultiChat's Connections tab, which was also rendered into
 * Settings → Integrations next to Discord RPC — so "my accounts" meant Twitch in
 * one tab and everything else in another. They are accounts; they belong with the
 * accounts.
 *
 * ONE connect action per platform however many credentials it takes. Kick needs
 * an OAuth token and a kick.com site session; YouTube needs a cookie session and
 * a channel read. The user should never learn that, so nothing here says
 * "import", "sync", "OAuth" or "session".
 */

const PLATFORMS: PlatformId[] = ['kick', 'youtube', 'tiktok'];

export default function PlatformAccountRows() {
  return (
    <div className="space-y-1.5">
      <p className="text-[11px] font-medium uppercase tracking-wide text-textMuted">
        Other platforms
      </p>
      {PLATFORMS.map((p) => (
        <PlatformRow key={p} provider={p} />
      ))}
      <p className="text-xs text-textMuted">
        Watching on one of these uses that account — for following, chatting and
        anything it unlocks.
      </p>
      <p className="text-xs text-textMuted">
        TikTok is the exception. Signing in only lets age-restricted LIVEs play,
        and TikTok chat stays read-only either way.
      </p>
    </div>
  );
}

function PlatformRow({ provider }: { provider: PlatformId }) {
  const state = usePlatformAccountStore((s) => s[provider]);
  const connect = usePlatformAccountStore((s) => s.connect);
  const disconnect = usePlatformAccountStore((s) => s.disconnect);
  const channelCount = useFollowsStore(
    (s) => s.follows.filter((f) => f.provider === provider).length,
  );
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);

  const meta = PROVIDERS[provider];
  const { connected, name, avatarUrl, busy, step } = state;

  // Connected: lead with WHO, the way the Twitch rows above do, and let the mark
  // on the avatar say which platform. Disconnected: there is no "who" yet, so the
  // platform name is the title.
  const title = connected ? (name ?? meta.label) : meta.label;
  // A step message replaces the subtitle while something is happening, so the row
  // says what it is doing instead of freezing on a stale line.
  // TikTok's session only plays age-restricted LIVEs and imports nothing, so
  // its row says that rather than counting channels it never read.
  const subtitle =
    step ??
    (provider === 'tiktok'
      ? connected
        ? `${meta.label} · age-restricted LIVEs play`
        : 'Sign in to watch age-restricted LIVEs'
      : connected
        ? channelCount > 0
          ? `${meta.label} · ${channelCount} channel${channelCount === 1 ? '' : 's'}`
          : meta.label
        : 'Not connected');

  return (
    <div className="flex items-center gap-3 rounded-lg px-3 py-2.5 bg-white/[0.03]">
      {/* Picture and platform in one glance. The mark floats bare on the avatar
          — no plate behind it — matching how the sidebar marks a mixed list; a
          background disc reads as a chip stuck to the photo. */}
      <div className="relative flex-shrink-0">
        {avatarUrl ? (
          <img
            src={avatarUrl}
            alt=""
            className="w-9 h-9 rounded-full object-cover"
            // A dead avatar URL should cost the picture, not leave a broken icon.
            onError={(e) => {
              (e.target as HTMLImageElement).style.visibility = 'hidden';
            }}
          />
        ) : (
          <div
            className="w-9 h-9 rounded-full flex items-center justify-center"
            style={{ backgroundColor: `${meta.color}1f` }}
          >
            <ProviderLogo provider={provider} size={17} />
          </div>
        )}
        {/* Only when there IS an avatar: without one the circle already shows the
            logo, and stamping a second copy on it would be noise. */}
        {avatarUrl && (
          <div className="absolute -bottom-0.5 -right-0.5 flex items-center justify-center drop-shadow-[0_1px_2px_rgba(0,0,0,0.85)]">
            <ProviderLogo provider={provider} size={13} />
          </div>
        )}
      </div>

      <div className="min-w-0 flex-1">
        <div className="text-sm text-textPrimary truncate">{title}</div>
        <div className="text-xs text-textSecondary truncate">{subtitle}</div>
      </div>

      {busy ? (
        <Loader2 size={15} className="animate-spin text-textMuted flex-shrink-0" />
      ) : connected ? (
        confirmDisconnect ? (
          <div className="flex items-center gap-1 flex-shrink-0">
            <button
              onClick={() => setConfirmDisconnect(false)}
              className="text-[11px] text-textMuted hover:text-textPrimary px-2 py-1 rounded-md hover:bg-white/[0.05] transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={() => {
                setConfirmDisconnect(false);
                void disconnect(provider);
              }}
              className="text-[11px] font-medium text-red-400 hover:bg-red-500/10 px-2 py-1 rounded-md transition-colors"
            >
              Disconnect
            </button>
          </div>
        ) : (
          <div className="flex items-center gap-0.5 flex-shrink-0">
            {/* A YouTube account can own several channels, and each one has its own
                subscriptions. Sign-in can only ever land on the default one, so
                without this the other channels are unreachable. Quiet and secondary:
                most people have exactly one and will never press it. */}
            {provider === 'youtube' && (
              <Tooltip content="Switch channel (each one has its own subscriptions)">
                <button
                  onClick={() => {
                    void invoke('open_youtube_channel_switcher').catch(() => {});
                  }}
                  aria-label="Switch YouTube channel"
                  className="p-1.5 text-textMuted hover:text-textPrimary hover:bg-white/[0.05] rounded-md transition-colors"
                >
                  <Users size={15} />
                </button>
              </Tooltip>
            )}
            <Tooltip content={`Disconnect ${meta.label}`}>
              <button
                onClick={() => setConfirmDisconnect(true)}
                aria-label={`Disconnect ${meta.label}`}
                className="p-1.5 text-textMuted hover:text-red-400 hover:bg-red-500/10 rounded-md transition-colors"
              >
                <Unlink size={15} />
              </button>
            </Tooltip>
          </div>
        )
      ) : (
        <button
          onClick={() => void connect(provider)}
          className="px-3 py-1.5 text-sm font-medium glass-button flex-shrink-0"
        >
          Connect
        </button>
      )}
    </div>
  );
}
