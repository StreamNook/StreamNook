// Who you are, at the top of the You tab: your Atmosphere glowing behind you,
// your picture on its own (no marks: your accounts show in the rows below),
// your name in your 7TV paint (or your Twitch color), the badges you wear and
// what you have on, and the way into Edit profile, whose chat row previews
// your picks live. The header shows; Edit profile changes.
//
// Rust builds the identity (`own_identity_header`, services/profile_view.rs):
// Twitch's displayed badges minus the channel-only sets, the selected 7TV
// badge, the chat clients and your StreamNook badge, in the one badge order
// every row uses and already capped; your 7TV picture; the Atmosphere you
// wear. It
// saves the answer for the next cold launch, so the saved copy paints the
// first frame and the fresh read follows. Your picks (the chosen theme, paint
// and name color) come from the same two picker answers Edit profile edits,
// held in one store so a change there shows here at once. The iPhone draws the
// same card (You/OwnIdentityHeader.swift).
import React from 'react';
import { ChevronRight, User } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { usePicksStore, useOwnPicks } from './picksStore';
import { AtmosphereGlow, CutoutAvatar, PaintedName, WornBadge } from './IdentityParts';
import { badgeArt, glowAtmosphere, identityNameSize, nameColorOr } from './identity';

const RADIUS = 22;
/** How far down the card the Atmosphere's glow reaches: the picture, the name
 *  and the worn line, fading out above Edit profile. */
export const GLOW = 210;
const AVATAR = 80;

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
  const atmosphere = glowAtmosphere(sn ? sn.theme : identity?.atmosphere?.id);
  // Your color once Edit profile has read it, else the one Rust's identity carries.
  const color = nameColorOr(ext?.chat_color ?? identity?.name_color);
  // Once the 7TV half has answered it is the truth, a paint taken off included;
  // until then, the paint the identity carries.
  const paint = ext ? (ext.paints.find((p) => p.worn)?.def ?? null) : (identity?.paint_def ?? null);
  const atmosphereName = sn
    ? sn.atmospheres.find((a) => a.worn && a.id !== 'tier')?.name
    : identity?.atmosphere?.name;
  const wearing = [atmosphereName, paint?.name ? `${paint.name} paint` : null].filter(Boolean).join(' · ');
  const badges = identity?.badges ?? [];
  const showLogin = !!login && login.toLowerCase() !== displayName.toLowerCase();

  return (
    // Raised over what follows, so the picture note a tap opens is never covered.
    <div className="glass-panel relative z-[1]" style={{ borderRadius: RADIUS }}>
      <GlowLayer>
        <AtmosphereGlow atm={atmosphere} tint={color} height={GLOW} />
      </GlowLayer>
      {/* Raised over the glow, the picture note it opens included. */}
      <div className="relative z-[2] pl-4 pt-4">
        <CutoutAvatar platformUrl={platformUrl} picture={identity?.picture} size={AVATAR} explains />
      </div>
      <div className="relative px-4 pt-3 min-w-0">
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
              <WornBadge key={`${b.id}-${i}`} url={badgeArt(b.url)} fallbacks={b.fallbacks} label={b.title || b.name} />
            ))}
            {badges.length > 0 && wearing && <span className="w-px h-3.5 mx-[3px] bg-borderLight shrink-0" />}
            {wearing && <span className="text-[12.5px] text-textSecondary truncate min-w-0">{wearing}</span>}
          </div>
        )}
      </div>
      <div className="relative mt-4 border-t border-borderSubtle" />
      <button
        onClick={onEdit}
        className="relative w-full flex items-center gap-2.5 px-4 h-[50px] text-left active:bg-surface-active"
        style={{ borderBottomLeftRadius: RADIUS, borderBottomRightRadius: RADIUS }}
      >
        <span className="text-[15px] font-semibold text-textPrimary shrink-0">Edit profile</span>
        <span className="flex-1 min-w-0 text-[12.5px] text-textMuted truncate">
          Pictures, bio, atmosphere, paint, color
        </span>
        <ChevronRight size={16} className="text-textMuted shrink-0" />
      </button>
    </div>
  );
};

/** The glow's clip: the card's own corners, without clipping the card (the
 *  picture note hangs past it). */
const GlowLayer: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div aria-hidden="true" className="absolute inset-0 overflow-hidden pointer-events-none" style={{ borderRadius: RADIUS }}>
    {children}
  </div>
);

/** Signed out, the card keeps its shape: a quiet glow, an empty picture, why
 *  to sign in, and the Twitch sign-in. `waiting`: signed in, with the profile
 *  still on its way (a session kept through a network failure), so the card
 *  says so and offers no sign-in. */
export const SignedOutIdentityCard: React.FC<{ waiting?: boolean }> = ({ waiting = false }) => {
  const loginToTwitch = useAppStore((s) => s.loginToTwitch);
  const isLoading = useAppStore((s) => s.isLoading);
  return (
    <div className="glass-panel relative" style={{ borderRadius: RADIUS }}>
      <GlowLayer>
        <AtmosphereGlow tint="var(--color-accent)" height={GLOW} />
      </GlowLayer>
      <div className="relative pl-4 pt-4">
        <div
          className="rounded-full flex items-center justify-center"
          style={{
            width: AVATAR,
            height: AVATAR,
            background: '#000',
            boxShadow: '0 4px 8px rgba(0,0,0,0.35), inset 0 0 0 0.5px rgba(255,255,255,0.1)',
          }}
        >
          <User size={30} className="text-textMuted" />
        </div>
      </div>
      <div className={`relative px-4 pt-3${waiting ? ' pb-4' : ''}`}>
        <div className="text-[24px] font-bold text-textPrimary leading-tight">{waiting ? 'Signed in' : 'Not signed in'}</div>
        <div className="mt-1 text-[14px] text-textSecondary">
          {waiting
            ? 'Your profile shows once Twitch answers.'
            : 'Sign in to show your badges, Atmosphere and 7TV paint, and to follow channels and chat.'}
        </div>
      </div>
      {!waiting && (
      <div className="relative p-4">
        <button
          onClick={() => void loginToTwitch()}
          disabled={isLoading}
          className="sn-touch w-full h-[50px] rounded-full text-[16px] font-semibold text-white disabled:opacity-60"
          style={{ backgroundColor: '#9146FF' }}
        >
          {isLoading ? 'Signing in' : 'Sign in with Twitch'}
        </button>
      </div>
      )}
    </div>
  );
};

export default OwnIdentityCard;
