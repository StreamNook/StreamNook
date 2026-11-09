// The tabs' way into the inbox while browsing (Following, Browse): the tray in
// the title row's own 44px glaze, a step quieter than the title at rest. While
// something is new it widens into a capsule of the same glaze tinted the
// accent (still glass, never a solid fill), with the count beside the tray and
// never a mark on its corner. It opens the same sheet as a chat's tray; a
// card over the tab bar says what arrived (MobileApp). The iPhone's twin is
// InboxTitleButton in ios/Sources/StreamNook/Chat/Inbox.swift.
import React from 'react';
import { Inbox } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { fitCount } from '../../utils/fitNumber';
import { toggleInboxSheet, useMentionsStore } from './mentionInbox';

export const InboxTitleButton: React.FC = () => {
  const signedIn = useAppStore((s) => s.isAuthenticated);
  const unread = useMentionsStore((s) => s.unreadTotal);
  if (!signedIn) return null;
  const lit = unread > 0;
  return (
    <button
      type="button"
      onClick={() => toggleInboxSheet('home')}
      aria-label={lit ? `Inbox, ${unread} new` : 'Inbox'}
      className={`chrome-glaze chrome-glaze--control flex h-11 min-w-11 shrink-0 items-center justify-center gap-1.5 transition-[padding] duration-200 ${
        lit ? 'chrome-glaze--tinted px-3.5 text-textPrimary' : 'text-textSecondary'
      } active:text-textPrimary`}
    >
      <Inbox size={20} strokeWidth={2} />
      {lit && <span className="text-[15px] font-semibold leading-none tabular-nums">{fitCount(unread)}</span>}
    </button>
  );
};
