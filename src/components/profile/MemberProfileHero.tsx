// The member's hero band: their frame, avatar, painted name, worn badges,
// applied atmosphere, view count, and rank. Doubles as a drag handle.

import type { PointerEvent as ReactPointerEvent } from 'react';
import { X, User, Eye } from 'lucide-react';
import { motion } from 'framer-motion';
import { ProfileFrame } from './ProfileFrame';
import { StreamNookBadge, getTier } from '../StreamNookBadge';
import { getBadgeImageUrls, getBadgeFallbackUrls } from '../../services/seventvService';
import { FallbackImage } from '../FallbackImage';
import { Tooltip } from '../ui/Tooltip';
import { AtmosphereChip, PaintChip, SevenTvProfileButton } from './IdentityChips';
import type { MemberProfileView } from './memberProfile';

export function MemberProfileHero({
  userId,
  view,
  preview,
  close,
  onDragStart,
}: {
  userId: string;
  view: MemberProfileView;
  preview: boolean;
  close: () => void;
  onDragStart?: (e: ReactPointerEvent) => void;
}) {
  const {
    info,
    namePaint,
    activePaint,
    seventvUserId,
    wornBadges,
    memberNumber,
    profileViews,
    effectiveHiddenSections,
    effectiveTheme,
    cosmeticName,
    headerBorder,
  } = view;
  return (
    <>
    {/* Hero band — identity + rank over the atmosphere (most present here).
        Doubles as the drag handle. */}
    <div
      onPointerDown={onDragStart}
      style={{ borderBottomColor: headerBorder }}
      className="relative z-[2] flex-shrink-0 cursor-grab border-b border-white/[0.06] shadow-[0_12px_24px_-12px_rgba(0,0,0,0.85)] active:cursor-grabbing"
    >
      {/* Readability scrim so the identity reads over a busy backdrop. */}
      <div
        className="pointer-events-none absolute inset-0 z-[1]"
        style={{
          background:
            'linear-gradient(180deg, rgba(10,10,14,0.30) 0%, rgba(10,10,14,0.55) 55%, rgba(10,10,14,0.82) 100%)',
        }}
      />
      {/* 7TV paint accent (supporter tier): a subtle blurred wash of the
          member's paint at the TOP of the hero only, fading out. Deliberately
          restrained vs a full Atmosphere so the paint theme reads as the
          lesser tier. */}
      {effectiveTheme?.paintAura && (
        <motion.div
          className="pointer-events-none absolute inset-x-0 top-0 z-[1] h-full"
          style={{
            ...effectiveTheme.paintAura,
            WebkitMaskImage: 'linear-gradient(to bottom, black, transparent 80%)',
            maskImage: 'linear-gradient(to bottom, black, transparent 80%)',
          }}
          animate={{ opacity: [0.16, 0.24, 0.16] }}
          transition={{ duration: 7, repeat: Infinity, ease: 'easeInOut' }}
        />
      )}
      {/* Top + bottom specular hairlines (catch light, seam into content). */}
      <div className="pointer-events-none absolute inset-x-0 top-0 z-[2] h-px bg-gradient-to-r from-transparent via-white/20 to-transparent" />
      <div className="pointer-events-none absolute inset-x-0 bottom-0 z-[2] h-px bg-gradient-to-r from-transparent via-white/[0.12] to-transparent" />

      {/* The member's equipped Frame, bordering the hero band. */}
      <ProfileFrame userId={userId} />
      {/* Identity — seated and vertically centered in the hero. */}
      <div className="relative z-10 flex items-center gap-3 p-4">
        <div className="flex min-w-0 flex-1 items-center gap-3.5">
          <span className="flex h-16 w-16 flex-shrink-0 items-center justify-center overflow-hidden rounded-full bg-white/[0.04] shadow-[inset_0_1px_0_0_rgba(255,255,255,0.18),0_4px_12px_rgba(0,0,0,0.5)] ring-1 ring-inset ring-white/15">
            {info?.avatar ? (
              <img src={info.avatar} alt="" className="h-full w-full object-cover" draggable={false} />
            ) : (
              <User size={28} className="text-textSecondary" />
            )}
          </span>
          <div className="min-w-0 flex-1">
            {/* Name, then the 7TV paint it wears (as on the chat user card),
                then the Preview pill. */}
            <div className="flex min-w-0 items-center gap-2">
              <span
                className="truncate text-xl font-bold leading-tight text-textPrimary"
                style={namePaint ?? undefined}
              >
                {info?.displayName ?? 'StreamNook member'}
              </span>
              {activePaint && (
                <span className="inline-flex flex-shrink-0" onPointerDown={(e) => e.stopPropagation()}>
                  <PaintChip paint={activePaint} color="#9146FF" side="bottom" />
                </span>
              )}
              {preview && (
                <span className="flex-shrink-0 rounded-full border border-accent/30 bg-accent/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-accent">
                  Preview
                </span>
              )}
            </div>
            {/* Meta line: @handle + the worn badges grouped together, so the
                badges read as part of the identity instead of a stray row
                dangling under the name. */}
            <div className="mt-1.5 flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
              {info?.login && (
                <span className="text-[13px] leading-none text-textMuted">@{info.login}</span>
              )}
              {(() => {
                const { seventv, twitch, thirdParty, bttvPro } = wornBadges;
                const hasAny =
                  !!twitch || !!seventv || memberNumber !== null || thirdParty.length > 0 || !!bttvPro;
                if (!hasAny) return null;
                const imgBadge = (src: string, title: string, key: string) => (
                  <Tooltip key={key} content={title} side="bottom">
                    <img
                      src={src}
                      alt={title}
                      draggable={false}
                      className="h-[18px] w-[18px] flex-shrink-0 object-contain"
                    />
                  </Tooltip>
                );
                // stopPropagation so a badge click opens the nested profile
                // instead of starting a drag.
                return (
                  // Canonical badge order, the same as chat (utils/badgeOrder):
                  // Twitch global, then 7TV, then third-party (BTTV Pro among
                  // them), then StreamNook last. No channel context here, so the
                  // channel-contextual tier (sub/poll) never appears.
                  <div
                    className="flex flex-wrap items-center gap-1.5"
                    onPointerDown={(e) => e.stopPropagation()}
                  >
                    {twitch && imgBadge(twitch.src, `Twitch: ${twitch.title}`, 'tw')}
                    {seventv && (() => {
                      const urls = getBadgeImageUrls(seventv as any);
                      return urls.url4x ? (
                        <Tooltip content={`7TV: ${seventv.tooltip || seventv.name}`} side="bottom">
                          <FallbackImage
                            src={urls.url4x}
                            fallbackUrls={getBadgeFallbackUrls(seventv.id).slice(1)}
                            alt={seventv.tooltip || seventv.name}
                            className="h-[18px] w-[18px] flex-shrink-0"
                          />
                        </Tooltip>
                      ) : null;
                    })()}
                    {thirdParty.map((b: any) =>
                      imgBadge(b.src, `${b.title} (${b.provider.toUpperCase()})`, b.key || b.title),
                    )}
                    {bttvPro && imgBadge(bttvPro.src, 'BTTV Pro', 'bttvpro')}
                    {memberNumber !== null && userId && (
                      <StreamNookBadge userId={userId} side="bottom" />
                    )}
                  </div>
                );
              })()}
              {/* The StreamNook atmosphere behind this profile and their 7TV
                  profile (the paint chip sits beside the name above). */}
              {(effectiveTheme?.atmosphere || seventvUserId) && (
                <div className="flex flex-wrap items-center gap-1.5" onPointerDown={(e) => e.stopPropagation()}>
                  {effectiveTheme?.atmosphere && <AtmosphereChip atmosphere={effectiveTheme.atmosphere} side="bottom" />}
                  {seventvUserId && <SevenTvProfileButton seventvUserId={seventvUserId} side="bottom" />}
                </div>
              )}
              {/* Profile views — a subtle public counter. Hideable via the
                  'views' visibility toggle (honored here so the live preview
                  reflects what others see). */}
              {profileViews != null && !effectiveHiddenSections.includes('views') && (
                <Tooltip content="Profile views" side="bottom">
                  <span className="flex items-center gap-1 text-[11px] leading-none text-textMuted">
                    <Eye size={13} className="opacity-80" />
                    {profileViews.toLocaleString()}
                  </span>
                </Tooltip>
              )}
            </div>
          </div>
        </div>

        {/* Rank identity, lifted from the tier card but WITHOUT its chassis
            so it blends into the hero instead of reading as a card on top.
            The decode cypher still lives on the badge hover. */}
        {memberNumber !== null && (() => {
          const tier = getTier(memberNumber);
          return (
            <div className="flex flex-shrink-0 flex-col items-end pt-0.5 text-right">
              <div className="flex items-baseline gap-1.5">
                <span className="text-[11px] font-light leading-none text-white/35">Nº</span>
                <span className={tier.numberClassName}>{memberNumber.toLocaleString()}</span>
              </div>
              <div className={`mb-2 mt-2.5 h-px w-12 ${tier.hairlineClassName}`} />
              {tier.label && <div className={tier.labelClassName}>{tier.label}</div>}
              {cosmeticName && (
                <div className="mt-2.5 text-[10px] font-light uppercase tracking-[0.22em] text-white/70">
                  {cosmeticName}
                </div>
              )}
            </div>
          );
        })()}

        {/* Close — inline at the top-right so it can't collide with the
            rank number. stopPropagation so it never starts a drag. */}
        <button
          onClick={close}
          aria-label="Close"
          onPointerDown={(e) => e.stopPropagation()}
          className="-mr-1 -mt-1 flex-shrink-0 self-start rounded p-1.5 text-textMuted transition-colors hover:bg-white/[0.10] hover:text-textPrimary"
        >
          <X size={16} />
        </button>
      </div>
    </div>
    </>
  );
}
