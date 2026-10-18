// Who you are, at the top of the You tab: your Atmosphere as the card's band,
// your picture cut into its edge with your StreamNook badge on it as a medal,
// your name in your 7TV paint (or your Twitch color) below the art, the badges
// you wear and what you have on, a line of chat as everyone sees it, and the
// way into Edit profile. The header shows; Edit profile changes.
//
// Rust builds the identity (`own_identity_header`, services/profile_view.rs):
// Twitch's displayed badges minus the channel-only sets, the selected 7TV
// badge, your StreamNook badge, your 7TV picture, the Atmosphere you wear. It
// saves the answer for the next cold launch, so the saved copy paints the
// first frame and the fresh read follows. Your picks (the chosen theme, paint
// and name color) come from the same two picker answers Edit profile edits,
// held in one store so a change there shows here at once. The iPhone draws the
// same card (You/OwnIdentityHeader.swift).
import React from 'react';
import { ChevronRight, User } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { usePicksStore, useOwnPicks } from './picksStore';
import { AtmosphereBand, CutoutAvatar, PaintedName, ProfilePreviewRow, WornBadge } from './IdentityParts';
import {
  bandSwatch,
  chatAtmosphereId,
  identityNameSize,
  medalUrl,
  nameColorOr,
  wornPreviewBadges,
} from './identity';

const RADIUS = 22;
const BAND = 104;
const AVATAR = 80;
/** The cutout ring at this avatar size (CutoutAvatar's own rule). */
const RING = Math.max(3, Math.round(AVATAR * 0.05));

interface Props {
  userId: string;
  displayName: string;
  login?: string;
  /** Your Twitch picture, drawn when you have no 7TV picture. */
  platformUrl?: string | null;
  onEdit: () => void;
}

export const OwnIdentityCard: React.FC<Props> = ({ userId, displayName, login, platformUrl, onEdit }) => {
  useOwnPicks({ userId, login: login || displayName, displayName });
  const identity = usePicksStore((s) => (s.userId === userId ? s.identity : null));
  const sn = usePicksStore((s) => (s.userId === userId ? s.sn : null));
  const ext = usePicksStore((s) => (s.userId === userId ? s.ext : null));

  // The theme you just picked once Edit profile has read it, else the one Rust
  // says you wear (Rust's copy also covers a catalog the page has not read yet).
  const rustAtmosphere = identity?.atmosphere;
  const swatch = sn
    ? (bandSwatch(sn.theme) ?? (rustAtmosphere?.id === sn.theme ? rustAtmosphere.swatch : null))
    : (rustAtmosphere?.swatch ?? null);
  const color = nameColorOr(ext?.chat_color);
  // Once the 7TV half has answered it is the truth, a paint taken off included;
  // until then, the paint the identity carries.
  const paint = ext ? (ext.paints.find((p) => p.worn)?.def ?? null) : (identity?.paint_def ?? null);
  const atmosphereName = sn
    ? sn.atmospheres.find((a) => a.worn && a.id !== 'tier')?.name
    : identity?.atmosphere?.name;
  const wearing = [atmosphereName, paint?.name ? `${paint.name} paint` : null].filter(Boolean).join(' · ');
  const badges = (identity?.badges ?? []).slice(0, 6);
  const showLogin = !!login && login.toLowerCase() !== displayName.toLowerCase();

  return (
    // Raised over what follows, so the picture note a tap opens is never covered.
    <div className="glass-panel relative z-[1]" style={{ borderRadius: RADIUS }}>
      <AtmosphereBand swatch={swatch} tint={color} height={BAND} radius={RADIUS} />
      <div className="px-3" style={{ marginTop: -(AVATAR / 2 + RING) }}>
        <CutoutAvatar
          platformUrl={platformUrl}
          picture={identity?.picture}
          size={AVATAR}
          medal={medalUrl(identity?.streamnook)}
          medalLabel={identity?.streamnook?.label}
          explains
        />
      </div>
      <div className="px-4 pt-1 min-w-0">
        <PaintedName
          text={displayName}
          paint={paint}
          color={color}
          className="block font-bold leading-tight break-words"
          style={{ fontSize: identityNameSize(displayName) }}
        />
        {showLogin && <div className="text-[13px] text-textMuted truncate mt-px">@{login}</div>}
        {(badges.length > 0 || wearing) && (
          <div className="mt-1.5 flex items-center gap-1.5 min-w-0">
            {badges.map((b, i) => (
              <WornBadge key={`${b.id}-${i}`} url={b.url} fallbacks={b.fallbacks} label={b.title || b.name} />
            ))}
            {badges.length > 0 && wearing && <span className="w-px h-3.5 mx-[3px] bg-borderLight shrink-0" />}
            {wearing && <span className="text-[12.5px] text-textSecondary truncate min-w-0">{wearing}</span>}
          </div>
        )}
      </div>
      {(sn || ext) && (
        <div className="px-3 pt-4">
          <ProfilePreviewRow
            name={displayName}
            paint={paint}
            colorHex={ext?.chat_color}
            badges={wornPreviewBadges(sn, ext)}
            atmosphereId={chatAtmosphereId(sn)}
          />
        </div>
      )}
      <div className="mt-3 border-t border-borderSubtle" />
      <button
        onClick={onEdit}
        className="w-full flex items-center gap-2.5 px-4 h-[50px] text-left active:bg-surface-active"
        style={{ borderBottomLeftRadius: RADIUS, borderBottomRightRadius: RADIUS }}
      >
        <span className="text-[15px] font-semibold text-textPrimary shrink-0">Edit profile</span>
        <span className="flex-1 min-w-0 text-[12.5px] text-textMuted truncate">
          Pictures, bio, Atmosphere, paint, color
        </span>
        <ChevronRight size={16} className="text-textMuted shrink-0" />
      </button>
    </div>
  );
};

/** Signed out, the card keeps its shape: the quiet band, an empty picture, why
 *  to sign in, and the Twitch sign-in. */
export const SignedOutIdentityCard: React.FC = () => {
  const loginToTwitch = useAppStore((s) => s.loginToTwitch);
  const isLoading = useAppStore((s) => s.isLoading);
  return (
    <div className="glass-panel" style={{ borderRadius: RADIUS }}>
      <AtmosphereBand tint="var(--color-accent)" height={BAND} radius={RADIUS} />
      <div className="px-3" style={{ marginTop: -(AVATAR / 2 + RING) }}>
        <div className="rounded-full bg-secondary w-fit" style={{ padding: RING }}>
          <div
            className="rounded-full bg-surface flex items-center justify-center"
            style={{ width: AVATAR, height: AVATAR }}
          >
            <User size={30} className="text-textMuted" />
          </div>
        </div>
      </div>
      <div className="px-4 pt-2">
        <div className="text-[24px] font-bold text-textPrimary leading-tight">Not signed in</div>
        <div className="mt-1 text-[14px] text-textSecondary">
          Sign in to show your badges, Atmosphere and 7TV paint, and to follow channels and chat.
        </div>
      </div>
      <div className="p-4">
        <button
          onClick={() => void loginToTwitch()}
          disabled={isLoading}
          className="sn-touch w-full h-[50px] rounded-full text-[16px] font-semibold text-white disabled:opacity-60"
          style={{ backgroundColor: '#9146FF' }}
        >
          {isLoading ? 'Signing in' : 'Sign in with Twitch'}
        </button>
      </div>
    </div>
  );
};

export default OwnIdentityCard;
