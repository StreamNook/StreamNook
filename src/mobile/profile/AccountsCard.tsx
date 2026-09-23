// The You tab's Accounts card: one branded row per platform, with who is signed
// in, what that account does for you here, and the one action that matters.
//
// Twitch is the account the whole app runs on, so its row only ever offers
// signing out (which signs you out of StreamNook). Kick is optional: signed out
// it offers Kick's own branded sign-in; signed in it can re-read your follows
// and sign out of Kick alone.
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowsClockwise } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { usePlatformAccountStore } from '../../stores/platformAccountStore';
import { useFollowsStore } from '../../stores/followsStore';
import { ProviderMark } from '../../components/ProviderLogo';
import { PROVIDERS, type ProviderId } from '../../types/providers';
import { Logger } from '../../utils/logger';

/** Two-tap confirm that disarms itself, shared by both sign-out buttons. */
function useArmed(ms = 3000): [boolean, () => boolean] {
  const [armed, setArmed] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);
  const tap = () => {
    if (armed) {
      setArmed(false);
      return true;
    }
    setArmed(true);
    timer.current = setTimeout(() => setArmed(false), ms);
    return false;
  };
  return [armed, tap];
}

/** Avatar with the platform's mark pinned to its corner. */
const AccountAvatar: React.FC<{ provider: ProviderId; src: string | null | undefined }> = ({ provider, src }) => (
  <div className="relative shrink-0">
    {src ? (
      <img
        src={src}
        alt=""
        className="w-11 h-11 rounded-full object-cover"
        draggable={false}
      />
    ) : (
      <div className="w-11 h-11 rounded-full bg-surface flex items-center justify-center">
        <ProviderMark provider={provider} size={20} />
      </div>
    )}
    {src && (
      // The bare mark, lifted off the photo by a shadow rather than a disc.
      <span className="absolute -bottom-0.5 -right-1 flex [filter:drop-shadow(0_1px_2px_rgba(0,0,0,0.7))]">
        <ProviderMark provider={provider} size={15} />
      </span>
    )}
  </div>
);

const SignOutButton: React.FC<{ armed: boolean; disabled?: boolean; onTap: () => void }> = ({
  armed,
  disabled,
  onTap,
}) => (
  <button
    onClick={onTap}
    disabled={disabled}
    className="sn-touch shrink-0 flex items-center disabled:opacity-50"
  >
    {/* The tap target stays full size; the pill inside is what you see. */}
    <span
      className={`px-3 py-1.5 rounded-full text-[12.5px] font-semibold whitespace-nowrap transition-colors ${
        armed ? 'bg-error/15 text-error' : 'glass-button text-textSecondary'
      }`}
    >
      {armed ? 'Confirm' : 'Sign out'}
    </span>
  </button>
);

export const AccountsCard: React.FC = () => {
  const currentUser = useAppStore((s) => s.currentUser);
  const signOutActiveAccount = useAppStore((s) => s.signOutActiveAccount);
  const addToast = useAppStore((s) => s.addToast);
  const kick = usePlatformAccountStore((s) => s.kick);
  const connect = usePlatformAccountStore((s) => s.connect);
  const disconnect = usePlatformAccountStore((s) => s.disconnect);
  const kickFollows = useFollowsStore((s) => s.follows.filter((f) => f.provider === 'kick').length);

  const [twitchArmed, armTwitch] = useArmed();
  const [kickArmed, armKick] = useArmed();
  const [syncing, setSyncing] = useState(false);

  const resyncKick = async () => {
    if (syncing) return;
    setSyncing(true);
    try {
      // A session still in the embedded browser reads silently; a cleared one
      // (a Twitch sign-out wipes that browser) needs the sign-in window again.
      const synced = await invoke<boolean>('kick_account_is_synced').catch(() => false);
      const { imported } = await useFollowsStore.getState().syncKick(!synced);
      addToast(`Found ${imported} Kick channel${imported === 1 ? '' : 's'} you follow`, 'success');
    } catch (e) {
      Logger.warn('[Accounts] kick resync failed:', e);
      addToast('Could not load your Kick follows. Try again in a moment.', 'error');
    } finally {
      setSyncing(false);
    }
  };

  const kickStatus =
    kick.step ??
    (kick.connected
      ? kickFollows > 0
        ? `${kickFollows} follow${kickFollows === 1 ? '' : 's'}`
        : 'Signed in'
      : 'Not signed in');

  return (
    <div className="glass-panel divide-y divide-borderSubtle">
      {/* Twitch */}
      <div className="flex items-center gap-3 p-3.5">
        <AccountAvatar provider="twitch" src={currentUser?.profile_image_url} />
        <div className="flex-1 min-w-0">
          <div className="text-[15px] font-semibold text-textPrimary truncate">
            {currentUser?.display_name || currentUser?.login || PROVIDERS.twitch.label}
          </div>
          <div className="text-[12.5px] text-textMuted leading-snug">
            {twitchArmed ? 'Signs you out of StreamNook' : 'Your StreamNook account'}
          </div>
        </div>
        <SignOutButton
          armed={twitchArmed}
          onTap={() => {
            if (armTwitch()) void signOutActiveAccount();
          }}
        />
      </div>

      {/* Kick */}
      <div className="flex items-center gap-3 p-3.5">
        <AccountAvatar provider="kick" src={kick.connected ? kick.avatarUrl : null} />
        <div className="flex-1 min-w-0">
          <div className="text-[15px] font-semibold text-textPrimary truncate">
            {kick.connected ? (kick.name ?? PROVIDERS.kick.label) : PROVIDERS.kick.label}
          </div>
          <div className="text-[12.5px] text-textMuted leading-snug truncate">{kickStatus}</div>
        </div>
        {kick.connected ? (
          <>
            <button
              onClick={() => void resyncKick()}
              disabled={syncing || kick.busy}
              className="sn-touch shrink-0 flex items-center justify-center text-textMuted disabled:opacity-50"
              aria-label="Update your Kick follows"
            >
              <ArrowsClockwise size={18} className={syncing ? 'animate-spin' : ''} />
            </button>
            <SignOutButton
              armed={kickArmed}
              disabled={kick.busy}
              onTap={() => {
                if (armKick()) void disconnect('kick');
              }}
            />
          </>
        ) : (
          // Kick's own colours, so it reads as signing in WITH Kick.
          <button
            onClick={() => void connect('kick')}
            disabled={kick.busy}
            className="sn-touch shrink-0 flex items-center disabled:opacity-60"
          >
            <span
              className="px-3.5 py-1.5 rounded-full text-[12.5px] font-bold text-black whitespace-nowrap"
              style={{ backgroundColor: PROVIDERS.kick.color }}
            >
              {kick.busy ? 'Signing in' : 'Sign in with Kick'}
            </span>
          </button>
        )}
      </div>
    </div>
  );
};
