// The You tab's Accounts card: one branded row per platform, with who is signed
// in, what that account does for you here, and the one action that matters.
//
// Twitch is the account the whole app runs on, so its row only ever offers
// signing out (which signs you out of StreamNook). 7TV is what lets you wear a
// paint and a 7TV badge: Connect in 7TV's own blue, Disconnect with the same
// two-tap confirm. Kick is optional: signed out it offers Kick's own branded
// sign-in; signed in it can re-read your follows and sign out of Kick alone.
//
// Each row shows that account's own picture, never the other's: the Twitch row
// your Twitch picture, the 7TV row your 7TV picture.
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowsClockwise } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { usePlatformAccountStore } from '../../stores/platformAccountStore';
import { useFollowsStore } from '../../stores/followsStore';
import { ProviderMark } from '../../components/ProviderLogo';
import { PROVIDERS, type ProviderId } from '../../types/providers';
import { Logger } from '../../utils/logger';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';
import { Button } from '../ui/Button';
import { usePicksStore } from './picksStore';
import { BrandMark } from './IdentityParts';
import { SEVENTV_BLUE, SEVENTV_INK } from './identity';
import type { AvatarPick } from '../../types/profileView';

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

// glass-exempt: every account picture sits on this disc, pure black like the
// profile pictures, so a picture with transparent parts never floats over the
// card and an empty one is still a circle.
const DISC_STYLE: React.CSSProperties = {
  background: '#000',
};

/** Avatar with the platform's mark pinned to its corner. */
const AccountAvatar: React.FC<{ provider: ProviderId; src: string | null | undefined; sevenTvId?: string | null }> = ({
  provider,
  src,
  sevenTvId,
}) => (
  <div className="relative shrink-0">
    {src ? (
      <SevenTvAvatar
        cosmeticsId={sevenTvId}
        px={44}
        src={src}
        alt=""
        className="w-11 h-11 rounded-full object-cover"
        draggable={false}
      />
    ) : (
      <div className="w-11 h-11 rounded-full flex items-center justify-center" style={DISC_STYLE}>
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

/** The 7TV row's picture: your 7TV picture with 7TV's mark on a disc at its
 *  corner, or the mark alone on the disc when you have none. */
const SevenTvAccountPicture: React.FC<{ picture: AvatarPick | null | undefined }> = ({ picture }) => {
  const [failedFor, setFailedFor] = useState<string | null>(null);
  if (picture && failedFor !== picture.still) {
    return (
      <div className="relative shrink-0">
        <span className="block w-11 h-11 rounded-full overflow-hidden" style={DISC_STYLE}>
          <img
            src={picture.animated ?? picture.still}
            alt=""
            draggable={false}
            className="w-full h-full object-cover"
            onError={(e) => {
              if (picture.animated && e.currentTarget.src !== picture.still) e.currentTarget.src = picture.still;
              else setFailedFor(picture.still);
            }}
          />
        </span>
        <span className="absolute -bottom-0.5 -right-1 w-[17px] h-[17px] rounded-full bg-tertiary flex items-center justify-center">
          <BrandMark brand="seventv" size={9} />
        </span>
      </div>
    );
  }
  return (
    <div className="w-11 h-11 shrink-0 rounded-full flex items-center justify-center" style={DISC_STYLE}>
      <BrandMark brand="seventv" size={13} />
    </div>
  );
};

const SignOutButton: React.FC<{ armed: boolean; disabled?: boolean; onTap: () => void; label?: string }> = ({
  armed,
  disabled,
  onTap,
  label = 'Sign out',
}) =>
  // Armed, the same row capsule in the error color.
  armed ? (
    <Button size="row" kind="brand" onClick={onTap} disabled={disabled} className="whitespace-nowrap bg-error/15 text-error">
      Confirm
    </Button>
  ) : (
    <Button size="row" onClick={onTap} disabled={disabled} className="whitespace-nowrap">
      {label}
    </Button>
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
  const [sevenTvArmed, armSevenTv] = useArmed();
  const [kickArmed, armKick] = useArmed();
  // Read by the identity card above (useOwnPicks), so this row needs no read of
  // its own; null until the 7TV status has answered.
  const sevenTv = usePicksStore((s) => (s.userId === currentUser?.user_id ? s.sevenTv : null));
  const sevenTvPicture = usePicksStore((s) => (s.userId === currentUser?.user_id ? s.identity?.picture : null));
  const sevenTvConnecting = usePicksStore((s) => s.connecting);

  const connectSevenTv = async () => {
    const user = currentUser;
    if (!user?.user_id) return;
    const store = usePicksStore.getState();
    if (!(await store.connectSevenTv(user.user_id))) return;
    // Your paints and 7TV badges become yours to change; read them again.
    void store.loadPicks({
      userId: user.user_id,
      login: user.login || user.username,
      displayName: user.display_name || user.username,
    });
  };

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

      {/* 7TV */}
      <div className="flex items-center gap-3 p-3.5">
        <SevenTvAccountPicture picture={sevenTvPicture} />
        <div className="flex-1 min-w-0">
          <div className="text-[15px] font-semibold text-textPrimary truncate">7TV</div>
          <div className="text-[12.5px] text-textMuted leading-snug truncate">
            {sevenTv?.signedIn
              ? sevenTvArmed
                ? 'Your paint and badge stay on 7TV'
                : 'Connected'
              : 'Paints and badges'}
          </div>
        </div>
        {sevenTv?.signedIn ? (
          <SignOutButton
            armed={sevenTvArmed}
            label="Disconnect"
            onTap={() => {
              if (armSevenTv()) void usePicksStore.getState().disconnectSevenTv();
            }}
          />
        ) : sevenTv ? (
          // 7TV's own colors, so it reads as connecting WITH 7TV.
          <Button
            size="row"
            kind="brand"
            onClick={() => void connectSevenTv()}
            disabled={sevenTvConnecting}
            className="whitespace-nowrap"
            style={{ backgroundColor: SEVENTV_BLUE, color: SEVENTV_INK }}
          >
            {sevenTvConnecting ? 'Connecting' : 'Connect'}
          </Button>
        ) : null}
      </div>

      {/* Kick */}
      <div className="flex items-center gap-3 p-3.5">
        <AccountAvatar
          provider="kick"
          src={kick.connected ? kick.avatarUrl : null}
          sevenTvId={kick.connected ? sevenTvIdForStream({ provider: 'kick', user_id: kick.id }) : null}
        />
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
          // Kick's own colors, so it reads as signing in WITH Kick. The mark is
          // inked black like the label: Kick's green mark vanishes on its green.
          <Button
            size="row"
            kind="brand"
            brand="kick"
            icon={<ProviderMark provider="kick" size={14} className="[&_img]:brightness-0" />}
            onClick={() => void connect('kick')}
            disabled={kick.busy}
            className="whitespace-nowrap"
          >
            {/* The row already names Kick; a short label leaves its status whole. */}
            {kick.busy ? 'Signing in' : 'Sign in'}
          </Button>
        )}
      </div>
    </div>
  );
};
