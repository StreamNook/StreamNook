// You > Mentions: the inbox, the same panel the message bar's tray opens, here
// as a screen of its own. It sits over the watch layer, so opening a row
// reaches its chat without closing the stream.
import React from 'react';
import { useMobileNavStore } from '../navStore';
import { DrillInScreen } from '../ui/DrillInScreen';
import { InboxPanel } from '../mentions/InboxPanel';

export const MentionsScreen: React.FC = () => {
  const open = useMobileNavStore((s) => s.mentionsOpen);
  const setOpen = useMobileNavStore((s) => s.setMentionsOpen);

  return (
    <DrillInScreen
      open={open}
      layerKey="mentions"
      className="absolute inset-0 z-50 bg-background flex flex-col"
      style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
    >
      {open && <InboxPanel variant="screen" onClose={() => setOpen(false)} />}
    </DrillInScreen>
  );
};
