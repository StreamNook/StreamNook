// The main chat header's two-sided switch while a chat is pinned: the pinned
// chat and the stream being watched. It takes the title's place rather than a
// row of its own, and uses the header's segmented-control look and density
// steps. Only the side on screen carries its channel's name; the other side is
// its icon, named by its tooltip. The name keeps a floor of width, so when the
// row runs short the header's density steps drop the uptime and the viewer
// capsule before the name of the chat you are typing into.
//
// A dot on the side not shown says it has new messages since the viewer left
// it. The count is the slice's own live-message counter, so this re-renders
// only when that dot appears or goes.
import { motion } from 'framer-motion';
import { MonitorPlay, PushPin } from 'phosphor-react';
import { Tooltip } from '../ui/Tooltip';
import { useAppStore } from '../../stores/AppStore';
import { useChatConnectionStore } from '../../stores/chatConnectionStore';
import { chatKey, showChatPinSide, type ChatChannel, type ChatPinView } from '../../stores/chatPinStore';
import { usePinPair } from '../../hooks/useChatPin';
import { sliceLookupKey } from '../../utils/providerKey';

// Live-message count of each chat when the viewer last left it, by chat key.
// Transient view state: the chat widget remounts on every flip, so it cannot
// hold this itself.
const leftAt = new Map<string, number>();

function liveCount(c: ChatChannel): number {
  return useChatConnectionStore.getState().channels.get(sliceLookupKey(c.provider, c.login))?.liveMessageCount ?? 0;
}

function useUnread(c: ChatChannel | null, shown: boolean): boolean {
  return useChatConnectionStore((st) => {
    if (!c || shown) return false;
    const n = st.channels.get(sliceLookupKey(c.provider, c.login))?.liveMessageCount ?? 0;
    return n > (leftAt.get(chatKey(c)) ?? 0);
  });
}

export default function ChatPinSwitch() {
  const { pin, live, split, showPinned } = usePinPair();
  const liveName = useAppStore((s) => s.currentStream?.user_name || s.currentStream?.user_login || '');
  const pinnedUnread = useUnread(pin, showPinned);
  const liveUnread = useUnread(live, !showPinned);
  if (!split || !pin || !live) return null;

  const flip = (to: ChatPinView) => {
    if ((to === 'pinned') === showPinned) return;
    const leaving = showPinned ? pin : live;
    leftAt.set(chatKey(leaving), liveCount(leaving));
    void showChatPinSide(to);
  };

  const sides = [
    { key: 'pinned' as const, name: pin.display_name || pin.login, Icon: PushPin, active: showPinned, unread: pinnedUnread, hint: 'Pinned chat' },
    { key: 'live' as const, name: liveName || live.login, Icon: MonitorPlay, active: !showPinned, unread: liveUnread, hint: 'Chat of the stream you are watching' },
  ];

  return (
    <div
      // Never narrower than the name's floor: running short must show up as
      // row overflow, so the header's density steps give way first.
      className="pointer-events-auto relative flex min-w-min items-center rounded-full p-0.5"
      style={{ background: 'rgba(255,255,255,0.06)', boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.07)' }}
    >
      {sides.map((side) => (
        <Tooltip key={side.key} content={`${side.hint}: ${side.name}`} side="bottom">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              flip(side.key);
            }}
            aria-label={`${side.hint}: ${side.name}`}
            aria-pressed={side.active}
            className={`relative inline-flex items-center gap-1.5 rounded-full py-1 text-xs font-semibold transition-colors ${side.active ? 'min-w-0 pl-2 pr-2.5 text-textPrimary' : 'flex-shrink-0 px-2 text-textSecondary hover:text-textPrimary'}`}
          >
            {side.active && (
              <motion.span
                layoutId="chat-pin-pill"
                className="absolute inset-0 rounded-full bg-white/[0.13]"
                style={{ boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.09)' }}
                transition={{ type: 'spring', stiffness: 480, damping: 28 }}
              />
            )}
            <side.Icon
              size={12}
              weight={side.active ? 'fill' : 'regular'}
              className={`relative z-10 flex-shrink-0 ${side.key === 'pinned' && side.active ? 'text-accent' : ''}`}
              aria-hidden
            />
            {side.active && (
              <span data-fit-label className="relative z-10 min-w-[3.5rem] max-w-[10rem] truncate">
                {side.name}
              </span>
            )}
            {side.unread && (
              <span className="relative z-10 h-1.5 w-1.5 flex-shrink-0 rounded-full bg-accent" aria-label="New messages" />
            )}
          </button>
        </Tooltip>
      ))}
    </div>
  );
}
