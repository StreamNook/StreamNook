// You > Inbox: the inbox (mentions, replies, highlights and whispers), the
// same panel the message bar's tray opens, here as a screen of its own. It
// sits over the watch layer, so opening a row reaches its chat without closing
// the stream, and a whisper row turns the screen into its conversation (back
// returns to the list).
import React from 'react';
import { useMobileNavStore } from '../navStore';
import { DrillInScreen } from '../ui/DrillInScreen';
import { InboxPanel } from '../mentions/InboxPanel';
import { WhisperThread } from '../whispers/WhisperThread';

export const MentionsScreen: React.FC = () => {
  const open = useMobileNavStore((s) => s.mentionsOpen);
  const setOpen = useMobileNavStore((s) => s.setMentionsOpen);
  const thread = useMobileNavStore((s) => s.whisperKey);
  const closeWhisper = useMobileNavStore((s) => s.closeWhisper);

  return (
    <DrillInScreen
      open={open}
      layerKey="mentions"
      className="absolute inset-0 z-50 bg-background flex flex-col"
      style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
    >
      {open &&
        (thread ? (
          // No sheet here to ride the keyboard and clear the gesture bar: the
          // screen pads for both, as the chat composer does.
          <div
            className="flex-1 min-h-0 flex flex-col"
            style={{ paddingBottom: 'calc(var(--sn-kb, 0px) + var(--sn-safe-b, 0px))' }}
          >
            <WhisperThread key={thread} conversationKey={thread} onBack={closeWhisper} />
          </div>
        ) : (
          <InboxPanel variant="screen" onClose={() => setOpen(false)} />
        ))}
    </DrillInScreen>
  );
};
