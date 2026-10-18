// Holding a channel's card offers what else you can do with it: open its chat
// without watching (it joins your chats, and the Chats tab shows it) or watch
// it. A tap still watches, as it always has.
import React, { useCallback, useRef, useState } from 'react';
import { ChatsCircle, Play } from 'phosphor-react';
import type { TwitchStream } from '../../types';
import { MobileSheet } from './MobileSheet';
import { useChatTabsStore } from '../chat/chatTabsStore';
import { useMobileNavStore } from '../navStore';
import { streamProvider } from '../../utils/streamProvider';
import { hapticCommit } from './haptics';

/** How long a press must rest before it is a hold. */
const HOLD_MS = 450;
/** A finger that travels this far is scrolling, not holding. */
const SLOP_PX = 10;

/** Adds a channel's chat to your chats and shows the Chats tab. */
export function openChatOnly(stream: TwitchStream): void {
  useChatTabsStore
    .getState()
    .addTab(
      stream.user_login,
      stream.user_id || null,
      stream.user_name || stream.user_login,
      stream.profile_image_url ?? null,
      streamProvider(stream),
    );
  useMobileNavStore.getState().setTab('chats');
}

/** Press handlers for a card, plus the sheet a hold opens. */
export function useChannelHold(stream: TwitchStream, onPress: (stream: TwitchStream) => void) {
  const [open, setOpen] = useState(false);
  const press = useRef<{ x: number; y: number; timer: ReturnType<typeof setTimeout> } | null>(null);
  const held = useRef(false);

  const clear = useCallback(() => {
    if (press.current) clearTimeout(press.current.timer);
    press.current = null;
  }, []);

  const handlers = {
    onPointerDown: (e: React.PointerEvent) => {
      held.current = false;
      clear();
      press.current = {
        x: e.clientX,
        y: e.clientY,
        timer: setTimeout(() => {
          press.current = null;
          held.current = true;
          hapticCommit();
          setOpen(true);
        }, HOLD_MS),
      };
    },
    onPointerMove: (e: React.PointerEvent) => {
      const p = press.current;
      if (p && Math.hypot(e.clientX - p.x, e.clientY - p.y) > SLOP_PX) clear();
    },
    onPointerUp: clear,
    onPointerCancel: clear,
    // The hold owns the long press; the WebView's own menu would open over it.
    onContextMenu: (e: React.MouseEvent) => e.preventDefault(),
    onClick: () => {
      // The finger lifting after a hold is not a tap.
      if (held.current) {
        held.current = false;
        return;
      }
      onPress(stream);
    },
  };

  const close = () => setOpen(false);
  const row = 'sn-touch flex items-center gap-3 px-2 text-[15px] text-textPrimary active:opacity-70';
  const sheet = (
    <MobileSheet open={open} onClose={close} title={stream.user_name || stream.user_login} maxHeightFraction={0.4}>
      <div className="flex flex-col">
        <button
          className={row}
          onClick={() => {
            close();
            openChatOnly(stream);
          }}
        >
          <span className="flex w-6 h-6 items-center justify-center">
            <ChatsCircle size={22} className="text-textSecondary" />
          </span>
          <span className="flex-1 text-left">Open chat</span>
        </button>
        <button
          className={row}
          onClick={() => {
            close();
            onPress(stream);
          }}
        >
          <span className="flex w-6 h-6 items-center justify-center">
            <Play size={22} className="text-textSecondary" />
          </span>
          <span className="flex-1 text-left">Watch</span>
        </button>
      </div>
    </MobileSheet>
  );

  return { handlers, sheet };
}
