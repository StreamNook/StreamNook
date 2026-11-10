// The tall layout's chat chrome (SN-40). The chat itself is the one chat pane
// the watch layer always shows, in its vertical mode (MobileChatPane
// `vertical`); these are only the pieces around it: the chat button beside the
// message box, the grown chat's handle and the lower third's mention strip.
// Their rules live in verticalChatParts.ts and verticalView.ts. The iPhone's
// twin is Watch/VerticalChat.swift.
import React from 'react';
import { At, ChatTeardropText } from 'phosphor-react';
import { useChatTabsStore } from '../chat/chatTabsStore';
import { useMissedCount } from './verticalChatParts';
import { missedSpoken, missedWords, type VerticalChatState } from './verticalView';

/**
 * The chat button beside the message box: hides chat (pure video) or brings it
 * back. While chat is cleared it carries how many rows arrived since, inside the
 * button on tinted glaze, never as a mark on its corner. A VOD's replay counts
 * nothing.
 */
export const VerticalChatButton: React.FC<{
  state: VerticalChatState;
  /** The chat shows a VOD's recorded replay. */
  replay: boolean;
  onPress: () => void;
}> = ({ state, replay, onPress }) => {
  const room = useChatTabsStore((s) => s.activeChannel);
  const cleared = state === 'cleared';
  const count = useMissedCount(room, cleared && !replay);
  const words = missedWords(count);
  return (
    <button
      type="button"
      data-vertical-chrome=""
      onClick={(e) => {
        e.stopPropagation();
        onPress();
      }}
      aria-label={cleared ? missedSpoken(count) : 'Hide chat'}
      aria-pressed={!cleared}
      className={`chrome-glaze chrome-glaze--control ${words ? 'chrome-glaze--tinted px-3' : ''} pointer-events-auto flex h-10 min-w-10 items-center justify-center gap-1.5 rounded-full text-textPrimary transition-[padding] duration-150`}
    >
      <ChatTeardropText size={18} weight={cleared ? 'regular' : 'fill'} />
      {words && <span className="text-[13px] font-bold leading-none tabular-nums">{words}</span>}
    </button>
  );
};

/** The grown chat's handle along its top edge: a grabber in a strip the whole
 *  width. A tap or a drag down folds the chat back to the lower third (the
 *  watch layer owns the drag, through the props it hands in). */
export const VerticalChatHandle: React.FC<React.HTMLAttributes<HTMLDivElement>> = (props) => (
  <div
    {...props}
    data-tall-region="handle"
    data-vertical-chrome=""
    role="button"
    tabIndex={0}
    aria-label="Collapse chat"
    className="relative flex h-[22px] w-full shrink-0 items-center justify-center"
    style={{ touchAction: 'none' }}
  >
    {/* glass-exempt: a grabber mark, not a surface */}
    <span aria-hidden className="pointer-events-none h-[5px] w-9 rounded-full bg-textMuted/55" />
  </div>
);

/** A mention's card, folded to one line in the lower third; a tap grows the
 *  chat with the card in it. */
export const VerticalMentionStrip: React.FC<{ onOpen: () => void }> = ({ onOpen }) => (
  <button
    type="button"
    onClick={onOpen}
    aria-label="You were mentioned. Opens the chat"
    className="sn-popover sn-popover--static pointer-events-auto flex h-7 w-fit max-w-full items-center gap-1.5 rounded-full px-2.5 text-[12px] font-medium text-textPrimary"
  >
    <At size={11} weight="bold" className="shrink-0 text-accent" />
    <span className="truncate">You were mentioned</span>
  </button>
);
