// A chatter's sub in the room their profile was opened from, on the desktop
// profile card and the phone's profile sheet. Rust decides the standing and
// writes every word and number (services/sub_standing.rs); this only lays it
// out.
//
// A current sub is a fact worth celebrating, so it reads as one on a glass
// tile: the channel's badge for their tenure, the months as the focal number
// with the tier beside it, each fact with Twitch's own badge for it (founder,
// Prime Gaming, Sub Gifter) or an icon where no badge exists, and how close the
// next badge is. The tier tints the tile's edge, never its fill. Gifting stays
// a quiet secondary action. Anyone else gets the row as a pressable glass
// button, the call to gift, with what is known about them underneath.
import type { CSSProperties } from 'react';
import { Gift } from 'phosphor-react';
import { WatchStreakFlame } from '../icons/WatchStreakFlame';
import type { SubStanding } from '../../types';
import { openExternal } from '../../utils/openExternal';

interface SubStandingRowProps {
  standing: SubStanding;
  /** The worn sub badge art, from `wornSubBadgeArt`; used when Rust could not
   *  read the channel's badge set. */
  badgeArt?: string | null;
  /** Show what is known about a lapsed sub (the card's "Past subscriber" row). */
  showPast?: boolean;
  /** `sheet` is the phone's larger type. */
  size?: 'card' | 'sheet';
}

/** Twitch purple for tier 1 (the lighter brand shade, so it reads as text on
 *  dark glass), its pale tint for tier 2, gold for tier 3, Prime blue for Prime. */
function tierTint(standing: SubStanding): string {
  if (standing.kind === 'prime') return '#00a8e1';
  if (standing.tier === 3) return '#f2c14e';
  if (standing.tier === 2) return '#bf94ff';
  return '#a970ff';
}

type Fact = SubStanding['facts'][number];

/** A fact's mark: Twitch's badge when Rust found one, else the closest icon. */
function FactMark({ fact, px }: { fact: Fact; px: number }) {
  if (fact.art) {
    return <img src={fact.art} alt="" draggable={false} style={{ width: px, height: px }} className="shrink-0 object-contain" />;
  }
  if (fact.kind === 'streak') return <WatchStreakFlame size={px} className="shrink-0 text-highlight-orange" />;
  if (fact.kind === 'gift') return <Gift size={px} weight="duotone" className="shrink-0 text-textSecondary" />;
  return null;
}

export function SubStandingRow({ standing, badgeArt, showPast = true, size = 'card' }: SubStandingRowProps) {
  const sheet = size === 'sheet';
  const titleText = sheet ? 'text-[14px]' : 'text-[12px]';
  const detailText = sheet ? 'text-[12.5px]' : 'text-[11px]';
  const gift = () => {
    if (standing.gift_url) void openExternal(standing.gift_url);
  };

  if (standing.state === 'current') {
    const art = standing.badge_art ?? badgeArt ?? null;
    const next = standing.next_badge;
    // Badges sit at the facts' cap height plus a little, the way chat sets
    // badges against its text: 15 px beside 11 px text, 17 px beside 12.5 px.
    const markPx = sheet ? 17 : 15;
    const giftable = !!standing.gift_label && !!standing.gift_url;
    return (
      <div
        className={`glass-tile relative overflow-hidden ${sheet ? 'px-3.5 pb-3 pt-2.5' : 'px-3 pb-2.5 pt-2'}`}
        style={{ '--sub-tint': tierTint(standing) } as CSSProperties}
      >
        {/* The tier's light on the tile's edge: a wash from the badge corner
            and a lit top rim. A decoration over the tile, which keeps its own
            glass fill underneath. */}
        <span
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 rounded-[inherit]"
          style={{
            backgroundImage:
              'radial-gradient(120% 140% at 0% 0%, color-mix(in srgb, var(--sub-tint) 20%, transparent) 0%, transparent 62%)',
            boxShadow:
              'inset 0 1px 0 0 color-mix(in srgb, var(--sub-tint) 50%, transparent), inset 0 0 0 1px color-mix(in srgb, var(--sub-tint) 18%, transparent)',
          }}
        />

        <div className="relative flex items-center gap-2">
          <span className={`min-w-0 flex-1 truncate ${sheet ? 'text-[11px]' : 'text-[10px]'} font-semibold uppercase tracking-wider text-textSecondary`}>
            {standing.title}
          </span>
          {giftable && (sheet ? (
            // A thumb-sized icon on the phone, so the facts keep the width.
            <button
              type="button"
              onClick={gift}
              aria-label={standing.gift_label ?? undefined}
              className="-my-1.5 -mr-1.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-accent active:scale-90 transition-transform"
            >
              <Gift size={18} weight="duotone" />
            </button>
          ) : (
            <button
              type="button"
              onClick={gift}
              className="inline-flex shrink-0 items-center gap-1 text-[11px] font-medium text-accent decoration-accent/60 underline-offset-[3px] transition-colors hover:text-accent/90 hover:underline"
            >
              <Gift size={13} weight="duotone" />
              <span>{standing.gift_label}</span>
            </button>
          ))}
        </div>

        <div className={`relative flex items-center ${sheet ? 'mt-2 gap-3.5' : 'mt-1.5 gap-3'}`}>
          {art ? (
            <img
              src={art}
              alt=""
              draggable={false}
              className={`${sheet ? 'h-11 w-11' : 'h-9 w-9'} shrink-0 object-contain drop-shadow-[0_2px_6px_rgba(0,0,0,0.45)]`}
            />
          ) : (
            <span aria-hidden="true" className="mx-1.5 h-2.5 w-2.5 shrink-0 rounded-full bg-[color:var(--sub-tint)]" />
          )}
          <div className="min-w-0 flex-1">
            {standing.months != null && standing.months_unit ? (
              <div className="flex items-baseline gap-1.5 leading-none">
                <span className={`${sheet ? 'text-[30px]' : 'text-[26px]'} font-bold tabular-nums tracking-tight text-textPrimary`}>
                  {standing.months}
                </span>
                <span className={`${sheet ? 'text-[13px]' : 'text-[12px]'} font-medium text-textSecondary`}>{standing.months_unit}</span>
                {standing.tier_label && (
                  <span className={`${sheet ? 'text-[13px]' : 'text-[12px]'} ml-1 font-semibold text-[color:var(--sub-tint)]`}>
                    {standing.tier_label}
                  </span>
                )}
              </div>
            ) : standing.tier_label && (
              <div className={`${titleText} font-semibold leading-none text-[color:var(--sub-tint)]`}>{standing.tier_label}</div>
            )}
            {standing.facts.length > 0 && (
              <div className={`${detailText} mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 leading-none text-textSecondary`}>
                {standing.facts.map((fact) => (
                  <span key={fact.kind} className="inline-flex min-w-0 items-center gap-1">
                    <FactMark fact={fact} px={markPx} />
                    <span className={fact.kind === 'prime' || fact.kind === 'founder' ? 'font-semibold text-textPrimary' : ''}>
                      {fact.text}
                    </span>
                  </span>
                ))}
              </div>
            )}
          </div>
        </div>

        {next && (
          <div className={`relative ${sheet ? 'mt-3' : 'mt-2.5'}`}>
            <div className="h-[3px] overflow-hidden rounded-full bg-white/[0.08]">
              {/* One-shot fill on transform only; the Motion setting stills it. */}
              <div
                className="sub-progress-fill h-full w-full rounded-full"
                style={{
                  transform: `scaleX(${Math.max(0.04, next.progress)})`,
                  backgroundImage:
                    'linear-gradient(90deg, color-mix(in srgb, var(--sub-tint) 55%, transparent), var(--sub-tint))',
                }}
              />
            </div>
            <div className={`mt-1.5 flex items-center gap-1.5 ${sheet ? 'text-[11.5px]' : 'text-[10.5px]'} text-textSecondary`}>
              <span className="min-w-0 flex-1 truncate">{next.text}</span>
              {next.art && (
                <img src={next.art} alt="" draggable={false} className={`${sheet ? 'h-4 w-4' : 'h-3.5 w-3.5'} shrink-0 object-contain opacity-80`} />
              )}
            </div>
          </div>
        )}
      </div>
    );
  }

  if (!standing.gift_label || !standing.gift_url) return null;
  const known = standing.state === 'past' && !showPast
    ? null
    : [standing.title, standing.detail].filter(Boolean).join(' · ') || null;
  return (
    <button
      type="button"
      onClick={gift}
      className={`glass-button group flex w-full items-center gap-2.5 text-left ${sheet ? 'px-3 py-2.5' : 'px-2.5 py-2'}`}
    >
      <Gift size={sheet ? 19 : 16} weight="duotone" className="shrink-0 text-accent" />
      <div className="min-w-0 flex-1">
        <div className={`${titleText} truncate font-semibold leading-tight text-textPrimary`}>{standing.gift_label}</div>
        {known && <div className={`${detailText} truncate leading-snug text-textSecondary`}>{known}</div>}
      </div>
      <span aria-hidden="true" className="shrink-0 text-accent transition-transform group-hover:translate-x-0.5">
        →
      </span>
    </button>
  );
}
