// Drops progress on the phone: the same slim glazed lip desktop draws above
// its message box, rising out of the top of the composer, so nothing covers
// chat. There is no hover on a phone, so a tap opens the drawer's list in a
// bottom sheet. Which reward, how far along and what can be claimed all come
// from Rust (services/watch_drops.rs, through stores/watchDropsStore), the
// same model desktop reads.
import React, { useState } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { useWatchDrops } from '../../stores/watchDropsStore';
import { barLine, DropsChatBarView, showsHere, useDropClaim } from '../../components/chat/DropsChatBar';
import { DrawerBody } from '../../components/chat/DropsDrawer';
import { MobileSheet } from '../ui/MobileSheet';

interface Props {
  /** This chat's Twitch channel id ('' while it is still looked up). */
  channelId: string;
  login: string;
}

export const MobileDropsLip: React.FC<Props> = ({ channelId, login }) => {
  const enabled = useAppStore((s) => s.settings.chat_design?.show_drops_in_chat ?? true);
  const snapshot = useWatchDrops(enabled);
  const view = snapshot?.view ?? null;
  const line = view && showsHere(view, channelId, login) ? barLine(view) : null;
  const [open, setOpen] = useState(false);
  const { claiming, claim } = useDropClaim();
  // A lip that went away (nothing left here, another channel, the setting
  // off) takes its sheet with it, so it never comes back already open.
  if (!line && open) setOpen(false);
  if (!line || !view) return null;
  return (
    <>
      {/* Flush on the composer's top border, so at rest it takes only its own
          height from the column. */}
      <div className="relative z-[1] -mb-px shrink-0 px-2.5 pt-1.5">
        <DropsChatBarView line={line} open={open} onToggle={() => setOpen(true)} touch />
      </div>
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
