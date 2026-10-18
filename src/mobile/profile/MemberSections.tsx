// A StreamNook member's overview, top emotes and accolades on their profile
// sheet, as Rust shaped them (services/member_profile.rs: what they hide is
// already gone). Their StreamNook badges and relics ride the sheet's badge
// groups. The iPhone draws the same answer in Cosmetics/MemberSections.swift.
import React, { useState } from 'react';
import { Star } from 'lucide-react';
import { FallbackImage } from '../../components/FallbackImage';
import type { AccoladeMedalView, MemberProfileView, MemberStatView } from '../../types/profileView';
import { SectionHeader } from './IdentityParts';
import { ACCOLADE_ICONS, STAT_ICONS } from './memberIcons';

/** The desktop medallion's bevel: inset light and shade, no outer glow. */
const MEDALLION_BEVEL = 'inset 1.5px 1.5px 1px rgba(255,255,255,0.35), inset -1.5px -1.5px 3px rgba(0,0,0,0.30)';

/** One overview tile: a colored mark and the label, the value, its caption. */
const StatTile: React.FC<{ stat: MemberStatView }> = ({ stat }) => {
  const mark = STAT_ICONS[stat.key];
  const Icon = mark?.icon ?? Star;
  return (
    <div className="glass-panel rounded-xl p-3 min-w-0">
      <div className="flex items-center gap-1.5 min-w-0">
        {/* One width for every icon, so the labels start on one line. */}
        <span className="w-3.5 shrink-0 flex justify-center" aria-hidden="true">
          <Icon size={11} strokeWidth={2} style={{ color: mark?.color ?? '#aebfca' }} />
        </span>
        <span className="text-[12px] text-textSecondary truncate">{stat.label}</span>
      </div>
      <div className="mt-1 text-[17px] leading-tight font-bold tabular-nums text-textPrimary truncate">{stat.value}</div>
      {/* Every tile keeps the caption line, so the grid is one even block;
          the caption hugs its value. */}
      <div className={`text-[12px] text-textMuted truncate ${stat.caption ? '' : 'invisible'}`} aria-hidden={!stat.caption}>
        {stat.caption ?? ' '}
      </div>
    </div>
  );
};

/** An earned accolade: its two-stop gradient lit from the top left, the icon
 *  in white, the name under it. */
const Medallion: React.FC<{ medal: AccoladeMedalView }> = ({ medal }) => {
  const Icon = ACCOLADE_ICONS[medal.icon] ?? Star;
  const [from, to] = medal.colors;
  return (
    <div className="flex flex-col items-center gap-1 min-w-0" role="img" aria-label={medal.label.replace(/\u00ad/g, '')}>
      <span
        className="flex w-11 h-11 items-center justify-center rounded-full"
        style={{ backgroundImage: `linear-gradient(140deg, ${from}, ${to ?? from})`, boxShadow: MEDALLION_BEVEL }}
      >
        <Icon size={18} strokeWidth={2} className="text-white drop-shadow-[0_1px_1px_rgba(0,0,0,0.35)]" aria-hidden="true" />
      </span>
      <span className="w-full text-center text-[11px] font-medium leading-tight text-textSecondary line-clamp-2 [hyphens:manual]">
        {medal.label}
      </span>
    </div>
  );
};

export const MemberSections: React.FC<{ profile: MemberProfileView }> = ({ profile }) => {
  // An emote whose art fails every URL takes itself out, as a badge does.
  const [failed, setFailed] = useState<ReadonlySet<string>>(() => new Set());
  const emotes = profile.top_emotes.filter((e) => !failed.has(e.id));
  return (
    <>
      {profile.stats.length > 0 && (
        <div className="mt-5">
          <SectionHeader title="Overview" className="mb-2" />
          <div className="grid grid-cols-2 gap-2">
            {profile.stats.map((s) => (
              <StatTile key={s.key} stat={s} />
            ))}
          </div>
        </div>
      )}

      {emotes.length > 0 && (
        <div className="mt-5">
          <SectionHeader title="Top emotes" className="mb-2" />
          <div className="flex flex-wrap gap-2">
            {emotes.map((e) => (
              <span key={e.id} className="w-8 h-8 flex items-center justify-center" title={e.name}>
                <FallbackImage
                  src={e.url}
                  alt={e.name}
                  className="w-7 h-7 object-contain"
                  onAllFailed={() => setFailed((prev) => new Set(prev).add(e.id))}
                />
              </span>
            ))}
          </div>
        </div>
      )}

      {profile.accolades.length > 0 && (
        <div className="mt-5">
          <SectionHeader title="Accolades" detail={String(profile.accolades.length)} className="mb-2" />
          <div className="grid grid-cols-5 gap-x-2 gap-y-3">
            {profile.accolades.map((a) => (
              <Medallion key={a.id} medal={a} />
            ))}
          </div>
        </div>
      )}
    </>
  );
};
