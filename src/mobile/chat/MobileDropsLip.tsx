// Drops progress on the phone: the same slim glazed lip desktop draws above
// its message box, rising out of the top of the composer, so nothing covers
// chat. There is no hover on a phone, so a tap opens the drawer's list in a
// bottom sheet. Which reward, how far along and what can be claimed all come
// from Rust (services/watch_drops.rs, through stores/watchDropsStore), the
// same model desktop reads.
import React, { useRef, useState } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { useWatchDrops } from '../../stores/watchDropsStore';
import { barLine, DropsChatBarView, DropsHairline, showsHere, tuckDropsLip } from '../../components/chat/DropsChatBar';
import { useDropClaim } from '../../components/chat/useDropClaim';
import { DrawerBody } from '../../components/chat/DropsDrawer';
import { MobileSheet } from '../ui/MobileSheet';

interface Props {
  /** This chat's Twitch channel id ('' while it is still looked up). */
  channelId: string;
  login: string;
}

// How far down the lip must be pulled to tuck it away.
const TUCK_PX = 22;

export const MobileDropsLip: React.FC<Props> = ({ channelId, login }) => {
  const enabled = useAppStore((s) => s.settings.chat_design?.show_drops_in_chat ?? true);
  const snapshot = useWatchDrops(enabled);
  const view = snapshot?.view ?? null;
  const line = view && showsHere(view, channelId, login) ? barLine(view) : null;
  const [open, setOpen] = useState(false);
  const { claiming, claim } = useDropClaim();
  // The lip follows a finger pulling it down; far enough tucks it away.
  const [pull, setPull] = useState(0);
  const drag = useRef<{ id: number; y: number; x: number; swiped: boolean } | null>(null);
  // A lip that went away (nothing left here, another channel, the setting
  // off) takes its sheet with it, so it never comes back already open.
  if (!line && open) setOpen(false);
  if (!line || !view) return null;
  const tucked = !!view.lip_hidden && !!view.lead;
  return (
    <>
      {tucked ? (
        // Tucked: the thin progress line on the composer's edge; a tap brings
        // the lip back. Rust brings it back itself when something is new.
        <div className="relative z-[1] -mb-px shrink-0">
          <DropsHairline line={line} onShow={() => tuckDropsLip(false)} touch />
        </div>
      ) : (
        // Flush on the composer's top border, so at rest it takes only its
        // own height from the column. Pull it down to tuck it away.
        <div
          className="relative z-[1] -mb-px shrink-0 touch-none px-2.5 pt-1.5"
          onPointerDown={(e) => {
            drag.current = { id: e.pointerId, y: e.clientY, x: e.clientX, swiped: false };
          }}
          onPointerMove={(e) => {
            const d = drag.current;
            if (!d || d.id !== e.pointerId || !view.lead) return;
            const dy = e.clientY - d.y;
            if (dy > 4 && dy > Math.abs(e.clientX - d.x)) {
              d.swiped = true;
              setPull(Math.min(dy, TUCK_PX * 1.6));
            }
          }}
          onPointerUp={() => {
            const d = drag.current;
            drag.current = null;
            if (d?.swiped && pull >= TUCK_PX) tuckDropsLip(true);
            setPull(0);
          }}
          onPointerCancel={() => {
            drag.current = null;
            setPull(0);
          }}
          // A pull is not a tap: it never opens the sheet.
          onClickCapture={(e) => {
            if (pull > 0 || drag.current?.swiped) e.stopPropagation();
          }}
          style={pull > 0 ? { transform: `translateY(${pull * 0.5}px)`, opacity: 1 - pull / (TUCK_PX * 2.4) } : undefined}
        >
          <DropsChatBarView line={line} open={open} onToggle={() => setOpen(true)} touch />
        </div>
      )}
      <MobileSheet open={open} onClose={() => setOpen(false)} maxHeightFraction={0.72}>
        {/* A tile's Claim sized for a thumb rather than a mouse, and every
            tile's last row as tall, so "Claimed" and the times stay level
            with it. */}
        <div className="[&_.drops-tile_button]:!leading-[32px] [&_.drops-tile_.min-h-\[22px\]]:min-h-[34px]">
          <DrawerBody view={view} claimingId={claiming} onClaim={(dropId, instanceId) => void claim(dropId, instanceId)} />
        </div>
      </MobileSheet>
    </>
  );
};
