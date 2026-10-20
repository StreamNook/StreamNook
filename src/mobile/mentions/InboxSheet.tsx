// The inbox in the bottom sheet the chat menu uses (MobileSheet), so the phone's
// sheets all behave alike: its grabber and slide down, a tap on the dimmed
// chat above it, or back closes it. The content is the inbox panel.
import React, { useCallback } from 'react';
import { MobileSheet } from '../ui/MobileSheet';
import { closeInboxSheet, useMentionsStore, type InboxSurface } from './mentionInbox';
import { InboxPanel } from './InboxPanel';

export const InboxSheet: React.FC<{ surface: InboxSurface }> = ({ surface }) => {
  const open = useMentionsStore((s) => s.sheetIn === surface);
  const close = useCallback(() => closeInboxSheet(), []);
  return (
    // `fill`: the rows arrive after it opens, so it never grows under the finger.
    <MobileSheet open={open} onClose={close} title="Inbox" fill>
      {/* Edge to edge inside the sheet's padding: the panel lays out its own rows. */}
      <div className="-mx-4 -mb-4 h-[calc(100%+16px)]">
        <InboxPanel variant="sheet" onClose={close} />
      </div>
    </MobileSheet>
  );
};
