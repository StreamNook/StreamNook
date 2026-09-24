// The body of a member's public profile: their relics, then the overview
// (hours, Twitch and lifetime stats, top emotes, accolades), in the compact
// public layout and tinted by their profile accent.

import type { ReactNode } from 'react';
import { RelicStrip } from './RelicStrip';
import ProfileOverview from '../settings/ProfileOverview';
import { ProfileAccentContext, ProfileCompactContext } from '../settings/profileAccentContext';
import type { MemberProfileView } from './memberProfile';

export function MemberProfileSections({
  userId,
  view,
  afterRelics,
}: {
  userId: string;
  view: MemberProfileView;
  /** Placed between the relics and the overview (the chat card's badges). */
  afterRelics?: ReactNode;
}) {
  const { info, memberNumber, counts, effectiveHiddenSections, themeRgb } = view;
  if (!info) return afterRelics ? <div className="mb-3">{afterRelics}</div> : null;
  return (
    <ProfileCompactContext.Provider value={true}>
    <ProfileAccentContext.Provider value={themeRgb}>
      <RelicStrip userId={userId} />
      {afterRelics && <div className="mb-3">{afterRelics}</div>}
      <ProfileOverview
        isOwnProfile={false}
        userId={userId}
        login={info.login}
        displayName={info.displayName}
        broadcasterType=""
        streamNookUserNumber={memberNumber}
        seventvPaintCount={counts.paints}
        seventvBadgeCount={counts.badges}
        ownedCosmeticsCount={counts.sn}
        hiddenSections={effectiveHiddenSections}
      />
    </ProfileAccentContext.Provider>
    </ProfileCompactContext.Provider>
  );
}
