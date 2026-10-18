// Chats: your chats with no stream playing. The same tabs as the strip under
// the player (Rust keeps one list), shown as this screen's header with the
// controls beside them: the play pill starts the room on screen when it is
// live, and + adds a chat. Composer, emotes, mentions and the points wallet are
// the ordinary chat pane's.
//
// Opening this screen is the first look that joins the saved chats (Rust
// restores them at launch without joining). While it is in front it keeps the
// phone awake, until Rust's idle span passes with no touch, so a phone left on
// a desk still sleeps.
import React, { useCallback, useEffect, useState } from 'react';
import { Play, Plus } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import type { TwitchStream } from '../../types';
import { MobileChatPane } from '../chat/MobileChatPane';
import { ChatTabStrip, useActiveChatChannelId } from '../chat/ChatTabStrip';
import { AddChatSheet } from '../chat/AddChatSheet';
import { ChatPicker } from '../chat/ChatPicker';
import { useChatTabsStore, type ChatTab } from '../chat/chatTabsStore';
import { useRoomPins } from '../chat/useRoomPins';
import { PinnedBanner } from '../watch/PinnedBanner';
import { holdScreenOn } from '../nativeBridge';
import { formatViewerCount } from '../../utils/streamStats';

/** The room on screen as the player's start reads it; Rust fills what a tab
 *  does not know (title, category, the live row). */
function asStream(tab: ChatTab): TwitchStream {
  return {
    id: tab.channel,
    user_id: tab.channelId ?? '',
    user_login: tab.login,
    user_name: tab.label,
    title: '',
    viewer_count: tab.viewerCount ?? 0,
    game_name: '',
    thumbnail_url: '',
    started_at: '',
    profile_image_url: tab.avatar ?? undefined,
    provider: tab.provider,
    is_live: true,
  } as TwitchStream;
}

/** Keeps the screen on while this screen is in front, until `idleSecs` pass
 *  with no touch. Any touch wakes the hold again. */
function useReadingAwake(on: boolean, idleSecs: number): void {
  const [idle, setIdle] = useState(false);
  useEffect(() => {
    if (!on) return;
    let timer = setTimeout(() => setIdle(true), idleSecs * 1000);
    const touched = () => {
      clearTimeout(timer);
      setIdle(false);
      timer = setTimeout(() => setIdle(true), idleSecs * 1000);
    };
    window.addEventListener('pointerdown', touched, { passive: true });
    window.addEventListener('keydown', touched);
    return () => {
      clearTimeout(timer);
      window.removeEventListener('pointerdown', touched);
      window.removeEventListener('keydown', touched);
    };
  }, [on, idleSecs]);
  const hold = on && !idle;
  useEffect(() => {
    holdScreenOn('chats', hold);
    return () => holdScreenOn('chats', false);
  }, [hold]);
}

export const ChatsScreen: React.FC = () => {
  const tabs = useChatTabsStore((s) => s.tabs);
  const activeChannel = useChatTabsStore((s) => s.activeChannel);
  const connection = useChatTabsStore((s) => s.connection);
  const connect = useChatTabsStore((s) => s.connect);
  const idleSecs = useChatTabsStore((s) => s.keepAwakeIdleSecs);
  const keepOn = useAppStore((s) => s.settings.chats_keep_screen_on ?? true);
  const startStream = useAppStore((s) => s.startStream);
  const [addOpen, setAddOpen] = useState(false);
  const [pinsOpen, setPinsOpen] = useState(false);

  // The first look joins the saved chats; a parked set comes back too.
  useEffect(() => {
    if (connection !== 'connected') connect();
  }, [connection, connect]);

  useReadingAwake(keepOn && tabs.length > 0, idleSecs);

  const pinned = useRoomPins(tabs.length > 0);
  const chatChannelId = useActiveChatChannelId();
  const active = tabs.find((t) => t.channel === activeChannel) ?? null;
  // The stream's own tab is already playing; nothing to start.
  const canWatch = !!active && active.live === true && !active.pinnedToStream;

  const watch = useCallback(() => {
    if (active) void startStream(active.login, asStream(active));
  }, [active, startStream]);
  const openAdd = useCallback(() => setAddOpen(true), []);
  const closeAdd = useCallback(() => setAddOpen(false), []);

  const trailing = (
    <>
      {canWatch && (
        <button
          onClick={watch}
          className="sn-touch shrink-0 h-8 flex items-center gap-1.5 pl-2.5 pr-3 rounded-full bg-live/15 text-textPrimary active:opacity-70"
          aria-label={`Watch ${active?.label ?? ''}`}
        >
          <span className="w-1.5 h-1.5 rounded-full bg-live" />
          {active?.viewerCount != null && (
            <span className="text-[12px] font-semibold tabular-nums">{formatViewerCount(active.viewerCount)}</span>
          )}
          <Play size={12} weight="fill" />
        </button>
      )}
      <button
        onClick={openAdd}
        className="sn-touch shrink-0 w-8 h-8 flex items-center justify-center rounded-full chrome-glaze chrome-glaze--flat text-textSecondary active:opacity-70"
        aria-label="Add a chat"
      >
        <Plus size={15} weight="bold" />
      </button>
    </>
  );

  return (
    <div
      className="flex-1 min-h-0 flex flex-col"
      style={
        {
          // The composer sits clear of the floating tab bar.
          '--sn-composer-floor': 'calc(var(--sn-safe-b, 0px) + 14px + var(--sn-tabbar-h, 64px) + 10px)',
        } as React.CSSProperties
      }
    >
      {tabs.length === 0 ? (
        // No chat open: the tab is the picker itself.
        <div className="flex-1 min-h-0 overflow-y-auto px-2 pt-2 [scrollbar-width:none]">
          <ChatPicker bottomClearance="calc(var(--sn-safe-b, 0px) + 14px + var(--sn-tabbar-h, 64px) + 16px)" />
        </div>
      ) : (
        <>
          <ChatTabStrip always trailing={trailing} />
          <div className="flex-1 min-h-0 relative flex flex-col">
            {pinned.length > 0 && (
              <div className="absolute left-0 right-0 top-0 px-2.5 pt-1.5 pb-2 z-20 pointer-events-none flex flex-col gap-1">
                <PinnedBanner
                  pins={pinned}
                  channelId={chatChannelId ?? undefined}
                  expanded={pinsOpen}
                  onToggle={() => setPinsOpen((v) => !v)}
                />
              </div>
            )}
            <MobileChatPane strip={false} />
          </div>
        </>
      )}
      <AddChatSheet open={addOpen} onClose={closeAdd} />
    </div>
  );
};
