// The sheet an emote opens when tapped in a phone chat row (SN-38): the phone's own sheet, the one
// the chat menu uses, at a fixed height from its first frame so nothing grows under the finger.
// Rust answers who made the emote and says the words (`emote_credit` with `detail`), cached per
// emote. A tap outside, a drag down on the grabber, or Back closes it.
import { useEffect, useState } from 'react';

import { MobileSheet } from '../ui/MobileSheet';
import { useAppStore } from '../../stores/AppStore';
import type { EmoteCardTarget } from '../../stores/emoteCardStore';
import { fetchEmoteCredit, type EmoteCredit, type EmoteCreator } from '../../services/emoteCredit';
import { EmoteSheetBody } from './EmoteSheetBody';

interface Props {
  open: boolean;
  target: EmoteCardTarget | null;
  stack: EmoteCardTarget[];
  onClose: () => void;
  /** The sheet has gone (its exit played): a hand-off to another sheet starts here. */
  onExited: () => void;
  onSwitch: (target: EmoteCardTarget) => void;
  onOpenTwitch: (creator: EmoteCreator) => void;
}

export function EmoteSheet({ open, target, stack, onClose, onExited, onSwitch, onOpenTwitch }: Props) {
  return (
    <MobileSheet open={open} onClose={onClose} onExited={onExited} maxHeightFraction={0.62} fill>
      {target && (
        <Lookup
          key={`${target.provider}:${target.id}`}
          target={target}
          stack={stack}
          onClose={onClose}
          onSwitch={onSwitch}
          onOpenTwitch={onOpenTwitch}
        />
      )}
    </MobileSheet>
  );
}

function copyName(name: string): void {
  void navigator.clipboard.writeText(name).then(
    () => useAppStore.getState().addToast(`Copied ${name}`, 'success'),
    () => useAppStore.getState().addToast('Could not copy the name', 'error'),
  );
}

/** One emote's lookup: keyed by the emote, so a switch within the stack starts a fresh one. */
function Lookup({
  target,
  stack,
  onClose,
  onSwitch,
  onOpenTwitch,
}: {
  target: EmoteCardTarget;
  stack: EmoteCardTarget[];
  onClose: () => void;
  onSwitch: (target: EmoteCardTarget) => void;
  onOpenTwitch: (creator: EmoteCreator) => void;
}) {
  const [answer, setAnswer] = useState<{ credit: EmoteCredit | null; done: boolean }>({ credit: null, done: false });
  useEffect(() => {
    let live = true;
    void fetchEmoteCredit({
      provider: target.provider,
      id: target.id,
      name: target.name,
      channel: target.channelLogin,
      channelId: target.channelId,
      detail: true,
    }).then((credit) => {
      if (live) setAnswer({ credit, done: true });
    });
    return () => {
      live = false;
    };
  }, [target]);
  return (
    <EmoteSheetBody
      target={target}
      stack={stack}
      credit={answer.credit}
      done={answer.done}
      onClose={onClose}
      onSwitch={onSwitch}
      onOpenTwitch={onOpenTwitch}
      onCopy={copyName}
    />
  );
}
