// The inbox in the bottom sheet the chat menu uses (MobileSheet), so the phone's
// sheets all behave alike: its grabber and slide down, a tap on the dimmed
// chat above it, or back closes it. The content is the inbox panel; a whisper
// turns the same sheet into its conversation, and back (the arrow or Android's
// Back) returns to the list.
import React, { useCallback } from 'react';
import { MobileSheet } from '../ui/MobileSheet';
import { useMobileNavStore } from '../navStore';
import { WhisperThread } from '../whispers/WhisperThread';
import { closeInboxSheet, useMentionsStore, type InboxSurface } from './mentionInbox';
import { InboxPanel } from './InboxPanel';

export const InboxSheet: React.FC<{ surface: InboxSurface }> = ({ surface }) => {
  const open = useMentionsStore((s) => s.sheetIn === surface);
  const thread = useMobileNavStore((s) => (open ? s.whisperKey : null));
  const closeWhisper = useMobileNavStore((s) => s.closeWhisper);
  const close = useCallback(() => closeInboxSheet(), []);
  return (
    // `fill`: the rows arrive after it opens, so it never grows under the finger.
    // `flush`: the panel (or the thread) is the scroller, so the list scrolls and pages.
    <MobileSheet
      open={open}
      onClose={close}
      onBack={thread ? closeWhisper : undefined}
      title={thread ? undefined : 'Inbox'}
      fill
      flush
    >
      {thread ? (
        <WhisperThread key={thread} conversationKey={thread} onBack={closeWhisper} />
      ) : (
        <InboxPanel variant="sheet" onClose={close} />
      )}
    </MobileSheet>
  );
};
