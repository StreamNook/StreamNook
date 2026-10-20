// The inbox's way in: a tray at one end of the message bar (Settings > Chat >
// Inbox button picks which), the same 40 px glaze control as the chat menu at
// the other end. A small count rides its rim while something is new; with the
// sheet open it is the X that closes it.
import React from 'react';
import { ChevronDown, Inbox, X } from 'lucide-react';
import { fitCount } from '../../utils/fitNumber';
import { toggleInboxSheet, useMentionsStore, type InboxSurface } from './mentionInbox';

export const InboxTray: React.FC<{
  surface: InboxSurface;
  /** The landscape strip's plain variant: the strip's ink, no glaze of its own. */
  plain?: boolean;
  /** The bar's end it sits at: the count rides the corner beside the field. */
  side?: 'left' | 'right';
}> = ({ surface, plain = false, side = 'left' }) => {
  const unread = useMentionsStore((s) => s.unreadTotal);
  const open = useMentionsStore((s) => s.sheetIn === surface);
  return (
    <button
      type="button"
      data-inbox-tray=""
      onClick={() => toggleInboxSheet(surface)}
      aria-label={open ? 'Close the inbox' : unread > 0 ? `Inbox, ${unread} new` : 'Inbox'}
      aria-expanded={open}
      className={
        plain
          ? 'sn-touch relative flex items-center justify-center w-9 h-9 rounded-full active:bg-textPrimary/10'
          : `chrome-glaze chrome-glaze--control relative shrink-0 flex items-center justify-center w-10 h-10 ${
              open ? 'text-textPrimary' : 'text-textSecondary'
            } active:text-textPrimary`
      }
    >
      {/* Both marks in one place, crossfading, so nothing moves. */}
      <span className={`absolute inset-0 flex items-center justify-center transition-opacity duration-150 ${open ? 'opacity-0' : 'opacity-100'}`}>
        <Inbox size={plain ? 18 : 19} strokeWidth={2} />
      </span>
      <span className={`absolute inset-0 flex items-center justify-center transition-opacity duration-150 ${open ? 'opacity-100' : 'opacity-0'}`}>
        {/* On the landscape strip, beside its own hide-chat X, open reads as
            put away: the sheet folds back down into the strip. */}
        {plain ? <ChevronDown size={19} strokeWidth={2.25} /> : <X size={19} strokeWidth={2.25} />}
      </span>
      {!open && unread > 0 && (
        <span
          className={`sn-inbox-badge absolute -top-1 ${side === 'right' ? '-left-1' : '-right-1'} min-w-[17px] h-[17px] px-[5px] rounded-full text-[10.5px] font-bold leading-[17px] text-center tabular-nums pointer-events-none ${
            plain ? 'sn-inbox-badge--plain' : ''
          }`}
        >
          {fitCount(unread)}
        </span>
      )}
    </button>
  );
};
