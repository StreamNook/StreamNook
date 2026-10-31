// Holding a channel's card offers what else you can do with it: open its chat
// without watching (it joins your chats, and the Chats tab shows it), watch
// it, or favorite it. A tap still watches, as it always has. The sheet is
// hosted once (ChannelHoldSheet, mounted in MobileApp), so a card that moves
// after a press or unmounts never takes the sheet with it.
import React, { useCallback, useMemo, useRef, useState } from 'react';
import { ChatsCircle, Play } from 'phosphor-react';
import type { TwitchStream } from '../../types';
import { MobileSheet } from './MobileSheet';
import { useChatTabsStore } from '../chat/chatTabsStore';
import { useMobileNavStore } from '../navStore';
import { streamProvider } from '../../utils/streamProvider';
import { hapticCommit } from './haptics';
import { createHoldFlag } from './holdFlag';
import { useChannelHoldSheet } from './channelHoldStore';
import { FavoriteStar, favoriteLabel } from './menuIcons';
import { favoriteRefOf, setFavoriteChannel, useIsFavorite } from '../../utils/favoriteDoor';
import { useAppStore } from '../../stores/AppStore';

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

/** Press handlers for a card. A hold opens the one hold sheet for this card. */
export function useChannelHold(stream: TwitchStream, onPress: (stream: TwitchStream) => void) {
  const press = useRef<{ x: number; y: number; timer: ReturnType<typeof setTimeout> } | null>(null);
  const [held] = useState(createHoldFlag);

  const clear = useCallback(() => {
    if (press.current) clearTimeout(press.current.timer);
    press.current = null;
  }, []);

  const handlers = {
    onPointerDown: (e: React.PointerEvent) => {
      held.clear();
      clear();
      press.current = {
        x: e.clientX,
        y: e.clientY,
        timer: setTimeout(() => {
          press.current = null;
          held.set();
          hapticCommit();
          useChannelHoldSheet.getState().open({ stream, onPress });
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
      if (held.consume()) return;
      onPress(stream);
    },
  };

  return { handlers, consumeHeld: held.consume };
}

const ROW = 'sn-touch flex items-center gap-3 px-2 text-[15px] text-textPrimary active:opacity-70';

/** The hold sheet, for whichever card was held last. Mounted once, in MobileApp. */
export const ChannelHoldSheet: React.FC = () => {
  const held = useChannelHoldSheet((s) => s.held);
  const close = useChannelHoldSheet((s) => s.close);
  // Until Rust's lists land the row holds its place and ignores a press.
  const followingApplied = useAppStore((s) => s.followingApplied);
  const stream = held?.stream ?? null;
  const provider = stream ? streamProvider(stream) : null;
  // Favorites on the phone are Twitch and Kick channels, named by an id or a login.
  const favoriteRef = useMemo(
    () =>
      stream && (provider === 'twitch' || provider === 'kick') && (stream.user_id || stream.user_login)
        ? favoriteRefOf(stream)
        : null,
    [stream, provider],
  );
  const isFavorite = useIsFavorite(favoriteRef);

  return (
    <MobileSheet
      open={held !== null}
      onClose={close}
      title={stream ? stream.user_name || stream.user_login : ''}
      maxHeightFraction={0.4}
    >
      {held && stream && (
        <div className="flex flex-col">
          <button
            className={ROW}
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
            className={ROW}
            onClick={() => {
              close();
              held.onPress(stream);
            }}
          >
            <span className="flex w-6 h-6 items-center justify-center">
              <Play size={22} className="text-textSecondary" />
            </span>
            <span className="flex-1 text-left">Watch</span>
          </button>
          {favoriteRef && (
            <button
              className={ROW}
              aria-disabled={followingApplied ? undefined : true}
              onClick={() => {
                if (!followingApplied) return;
                close();
                void setFavoriteChannel(favoriteRef, !isFavorite);
              }}
            >
              <span className="flex w-6 h-6 items-center justify-center">
                <FavoriteStar on={isFavorite} size={22} />
              </span>
              <span className="flex-1 text-left">{favoriteLabel(isFavorite)}</span>
            </button>
          )}
        </div>
      )}
    </MobileSheet>
  );
};
