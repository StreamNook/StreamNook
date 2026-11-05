// A StreamNook member's StreamNook half on their profile sheet, as Rust shaped
// it (services/member_profile.rs: what they hide is already gone): the hours
// hero with its roast, "<Name>'s Twitch" and "Lifetime in StreamNook", the most
// used emotes with their counts, and the accolade wall. Their relics
// (RelicShowcase) and badges ride the sheet's own blocks. The iPhone draws the
// same answer in Cosmetics/MemberSections.swift.
import React, { useState } from 'react';
import { Star } from 'lucide-react';
import { FallbackImage } from '../../components/FallbackImage';
import { TwitchGlyph, TwitchVerifiedMark } from '../../components/ui/TwitchGlyph';
import type {
  HoursHeroView,
  MemberProfileView,
  MemberStatView,
  StatSectionView,
  TopEmoteView,
} from '../../types/profileView';
import { AccoladeWall } from './AccoladeWall';
import { SectionHeader, TapPanel, TapPanelHost } from './IdentityParts';
import { STAT_ICONS } from './memberIcons';
import {
  SECTION_TITLES,
  accentBorder,
  accountGlyph,
  emoteTiles,
  panelFor,
  statPanel,
  statValueSize,
} from './profileSheetParts';

/** One tile: a colored mark and the label, the value and its caption. A tile
 *  with a detail (Twitch age: "Joined March 20, 2014") is a button, and a tap
 *  says the detail in a panel in place. */
const StatTile: React.FC<{ stat: MemberStatView; open?: boolean; onTap?: () => void }> = ({ stat, open, onTap }) => {
  const mark = STAT_ICONS[stat.key];
  const Icon = mark?.icon ?? Star;
  const glyph = stat.key === 'account_type' ? accountGlyph(stat.value) : null;
  const classes = 'glass-panel rounded-xl p-3 min-w-0 h-full';
  const body = (
    <>
      <div className="flex items-center gap-1.5 min-w-0">
        {/* One width for every icon, so the labels start on one line. */}
        <span className="w-3.5 shrink-0 flex justify-center" aria-hidden="true">
          {glyph === 'verified' ? (
            <TwitchVerifiedMark size={13} className="text-[#9146FF]" />
          ) : glyph === 'twitch' ? (
            <TwitchGlyph size={12} className="text-[#9146FF]" />
          ) : (
            <Icon size={11} strokeWidth={2} style={{ color: mark?.color ?? '#aebfca' }} />
          )}
        </span>
        <span className="text-[12px] text-textSecondary truncate">{stat.label}</span>
      </div>
      <div
        className="mt-1 font-bold leading-tight tabular-nums text-textPrimary truncate"
        style={{ fontSize: statValueSize(stat.value) }}
      >
        {stat.value}
      </div>
      {stat.caption && <div className="text-[12px] text-textMuted truncate">{stat.caption}</div>}
      {/* Every tile keeps a note line, so a row of tiles is one even block. */}
      {!stat.caption && (
        <div className="invisible text-[12px]" aria-hidden="true">
          {' '}
        </div>
      )}
    </>
  );
  return onTap ? (
    <button
      type="button"
      onClick={onTap}
      aria-expanded={!!open}
      aria-label={`${stat.label}, ${stat.value}`}
      className={`${classes} block w-full text-left transition-opacity active:opacity-70`}
    >
      {body}
    </button>
  ) : (
    <div className={classes}>{body}</div>
  );
};

/** The hours hero: the number, its unit and the roast under it. */
const HoursHero: React.FC<{ hours: HoursHeroView; accent?: string }> = ({ hours, accent }) => {
  const mark = STAT_ICONS.hours;
  const Icon = mark?.icon ?? Star;
  return (
    <div className="mt-5 glass-panel rounded-xl p-4" style={accentBorder(accent, 0.3)}>
      <div className="mb-2 flex items-center gap-2">
        <Icon size={16} strokeWidth={2.25} style={{ color: mark?.color ?? '#e6c178' }} aria-hidden="true" />
        <span className="text-[11px] font-medium uppercase tracking-[0.14em] text-textMuted">{hours.label}</span>
      </div>
      <div className="flex items-baseline gap-2">
        <span className="text-4xl font-bold tabular-nums text-textPrimary">{hours.value}</span>
        <span className="text-sm text-textMuted">{hours.unit}</span>
      </div>
      {hours.roast && <p className="mt-2 text-[13px] leading-relaxed text-textSecondary">{hours.roast}</p>}
    </div>
  );
};

/** A titled grid of tiles; an odd last tile takes the whole row. A tile with a
 *  detail opens it in a panel under the grid; a tap outside puts it away. */
const StatSection: React.FC<{ section: StatSectionView; accent?: string }> = ({ section, accent }) => {
  const [tapKey, setTapKey] = useState<string | null>(null);
  if (section.tiles.length === 0) return null;
  const odd = section.tiles.length % 2 === 1;
  const tapped = section.tiles.find((s) => s.key === tapKey) ?? null;
  const panel = tapped ? statPanel(tapped) : null;
  return (
    <div className="mt-5 glass-panel rounded-xl p-3.5" style={accentBorder(accent, 0.3)}>
      <SectionHeader title={section.title} className="mb-2.5" />
      <TapPanelHost open={panel !== null} onClose={() => setTapKey(null)}>
        <div className="grid grid-cols-2 gap-2">
          {section.tiles.map((s, i) => (
            <div key={s.key} className={`min-w-0 ${odd && i === section.tiles.length - 1 ? 'col-span-2' : ''}`}>
              <StatTile
                stat={s}
                open={tapKey === s.key}
                onTap={s.detail ? () => setTapKey((cur) => (cur === s.key ? null : s.key)) : undefined}
              />
            </div>
          ))}
        </div>
        {panel && <TapPanel panel={panel} />}
      </TapPanelHost>
    </div>
  );
};

/** The most used emotes with their counts, the first lit, and the totals line.
 *  A tap names an emote and its count in place. */
const EmoteSection: React.FC<{ emotes: TopEmoteView[]; totals?: string; accent?: string }> = ({
  emotes,
  totals,
  accent,
}) => {
  // An emote whose art fails every URL takes itself out, and the next one fills in.
  const [failed, setFailed] = useState<ReadonlySet<string>>(() => new Set());
  const [tapId, setTapId] = useState<string | null>(null);
  const tiles = emoteTiles(emotes, failed);
  if (tiles.length === 0) return null;
  const tapped = tiles.find((e) => e.id === tapId) ?? null;
  return (
    <div className="mt-5 glass-panel rounded-xl p-3.5" style={accentBorder(accent, 0.3)}>
      <SectionHeader
        title={SECTION_TITLES.top_emotes}
        className="mb-2.5"
        trailing={totals ? <span className="text-[11px] tabular-nums text-textMuted">{totals}</span> : undefined}
      />
      <TapPanelHost open={tapped !== null} onClose={() => setTapId(null)}>
        <div className="grid grid-cols-4 gap-2">
          {tiles.map((e, i) => (
            <button
              key={e.id}
              type="button"
              onClick={() => setTapId((cur) => (cur === e.id ? null : e.id))}
              aria-label={`${e.name}, ${e.count_text}`}
              className={`glass-tile flex min-w-0 flex-col items-center gap-1 p-2 ${
                i === 0 ? 'border-accent/30 bg-accent/[0.06]' : ''
              }`}
            >
              <span className="flex h-10 items-center justify-center">
                <FallbackImage
                  src={e.url}
                  alt={e.name}
                  className="max-h-10 max-w-full object-contain"
                  onAllFailed={() => setFailed((prev) => new Set(prev).add(e.id))}
                />
              </span>
              <span className="max-w-full truncate text-[11px] font-semibold tabular-nums text-textPrimary">
                {e.count_text}
              </span>
            </button>
          ))}
        </div>
        {tapped && <TapPanel panel={panelFor('emote', tapped)} />}
      </TapPanelHost>
    </div>
  );
};

export const MemberSections: React.FC<{ profile: MemberProfileView; accent?: string }> = ({ profile, accent }) => (
  <>
    {profile.hours && <HoursHero hours={profile.hours} accent={accent} />}
    {profile.twitch && <StatSection section={profile.twitch} accent={accent} />}
    {profile.lifetime && <StatSection section={profile.lifetime} accent={accent} />}
    <EmoteSection emotes={profile.top_emotes} totals={profile.emote_totals} accent={accent} />
    {profile.accolades && <AccoladeWall wall={profile.accolades} accent={accent} />}
  </>
);
