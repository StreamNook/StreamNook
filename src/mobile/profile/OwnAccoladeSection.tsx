// Your accolade wall on the You tab, under the identity card: the wall Rust
// builds for you (earned and locked, a locked secret as "???", your numbers in
// the hints), drawn by the same AccoladeWall the profile sheet uses. Loaded with
// the tab; Rust paints its saved copy first, then the fresh one, and the wall is
// read again when Rust says an accolade was saved or the points walk finished.
//
// The slot is held while you are known to be a member (or the member list has not
// loaded) and nothing has answered, as invisible space exactly as tall as a
// collapsed wall, so the wall arriving moves nothing below it. If it turns out
// there is no wall (not a member), the slot collapses by animation (height and
// opacity only, which Interface > Motion collapses to instant).
import React, { useSyncExternalStore } from 'react';
import { useOwnAccolades } from '../../hooks/useOwnAccolades';
import {
  getStreamNookRegistryVersion,
  getStreamNookUserNumber,
  isStreamNookRegistryLoaded,
  subscribeStreamNookRegistryVersion,
} from '../../services/supabaseService';
import { AccoladeWall } from './AccoladeWall';
import { collapsedWallHeight } from './accoladeWallLayout';
import { ownWallSlot } from './ownWallSlot';

export const OwnAccoladeSection: React.FC<{ userId: string }> = ({ userId }) => {
  // The third argument is the server snapshot a static render (the test) needs.
  useSyncExternalStore(subscribeStreamNookRegistryVersion, getStreamNookRegistryVersion, getStreamNookRegistryVersion);
  const { wall, answered } = useOwnAccolades(userId);
  const slot = ownWallSlot(isStreamNookRegistryLoaded(), getStreamNookUserNumber(userId) !== null, wall, answered);
  const open = slot !== 'hidden';
  return (
    <div
      aria-hidden={slot === 'shown' ? undefined : true}
      inert={slot !== 'shown'}
      className="grid"
      style={{
        gridTemplateRows: open ? '1fr' : '0fr',
        opacity: open ? 1 : 0,
        transition: 'grid-template-rows 180ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 140ms linear',
      }}
    >
      <div className="min-h-0 overflow-hidden">
        <div className="px-4 pb-5">
          {slot === 'shown' && wall ? (
            <AccoladeWall wall={wall} className="" />
          ) : slot === 'reserved' ? (
            // The held slot: nothing to see, the height of a collapsed wall.
            <div style={{ height: collapsedWallHeight() }} />
          ) : null}
        </div>
      </div>
    </div>
  );
};
