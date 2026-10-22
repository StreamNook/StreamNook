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
import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ChatsCircle, MonitorPlay, Play, Plus } from 'phosphor-react';
import { ChatsMinimizeButton } from '../chat/ChatsMinimizeButton';
import { useAppStore } from '../../stores/AppStore';
import type { TwitchStream } from '../../types';
import { MobileChatPane } from '../chat/MobileChatPane';
import { ChatTabStrip, useActiveChatChannelId } from '../chat/ChatTabStrip';
import { AddChatSheet } from '../chat/AddChatSheet';
import { ChatPicker } from '../chat/ChatPicker';
import { useChatTabsStore, type ChatTab } from '../chat/chatTabsStore';
import { useRoomPins } from '../chat/useRoomPins';
import { PinnedBanner } from '../watch/PinnedBanner';
import { usePinsOpen } from '../watch/pinsOpen';
import { holdScreenOn } from '../nativeBridge';
import { useMobileNavStore } from '../navStore';
import { useChannelState } from '../../stores/channelStateStore';
import { CollabAvatarStack } from '../../components/SharedViewers';
import { groupFor, isTogether } from '../../utils/sharedViewers';
import { TogetherSheet } from '../player/TogetherSheet';
import { formatViewerCount } from '../../utils/streamStats';

/** The Chats header's room strip: 40 px capsules with 7 px of air above and
 *  below, so they sit clear of the separator under them. */
const CHATS_STRIP_H = 54;

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
    is_live: tab.live === true,
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

/** `shown`: the tab is in front. It stays mounted behind the others (see
 *  MobileApp), and while hidden it holds nothing awake and draws no rows.
 *  `idle`: in front but unseen (PiP, the app in the background): the list
 *  holds still; keep-awake is untouched. */
export const ChatsScreen: React.FC<{ shown?: boolean; idle?: boolean }> = ({ shown = true, idle = false }) => {
  const tabs = useChatTabsStore((s) => s.tabs);
  const activeChannel = useChatTabsStore((s) => s.activeChannel);
  const connection = useChatTabsStore((s) => s.connection);
  const connect = useChatTabsStore((s) => s.connect);
  const idleSecs = useChatTabsStore((s) => s.keepAwakeIdleSecs);
  const keepOn = useAppStore((s) => s.settings.chats_keep_screen_on ?? true);
  const startStream = useAppStore((s) => s.startStream);
  const [addOpen, setAddOpen] = useState(false);
  // A pin opens or folds per channel: each channel starts as Chat > Pinned
  // Messages > Collapsed says, and a tap holds until the chat switches.
  const chatChannelId = useActiveChatChannelId();
  const pins = usePinsOpen();
  const pinsOpen = pins.open;
  // The closed banner's height: the list starts under it. Opened, the banner
  // floats over the rows, so nothing below it moves.
  const pinsRef = useRef<HTMLDivElement | null>(null);
  const [pinsBox, setPinsBox] = useState(0);

  // The first look joins the saved chats; a parked set comes back too.
  useEffect(() => {
    if (shown && connection !== 'connected') connect();
  }, [shown, connection, connect]);

  useReadingAwake(shown && keepOn && tabs.length > 0, idleSecs);

  const pinned = useRoomPins(shown && tabs.length > 0);
  // Folded with Style set to Hidden, a pin draws nothing and holds no room.
  const hasPins = pinned.length > 0 && (pinsOpen || pins.style !== 'hidden');
  useLayoutEffect(() => {
    const el = pinsRef.current;
    if (!hasPins || !el) {
      setPinsBox(0);
      return;
    }
    const read = () => {
      if (!pinsOpen) setPinsBox(el.offsetHeight);
    };
    read();
    const observer = new ResizeObserver(read);
    observer.observe(el);
    return () => observer.disconnect();
  }, [hasPins, pinsOpen]);
  const active = tabs.find((t) => t.channel === activeChannel) ?? null;
  // The stream's own tab is already playing; nothing to start.
  const canWatch = !!active && active.live === true && !active.pinnedToStream;
  // An offline Twitch room's way in: its channel page (latest broadcast, clips),
  // the same room a search opens. Rust starts the stream if it is live after all.
  const canOpenRoom = !!active && active.live !== true && active.provider === 'twitch' && !active.pinnedToStream;

  const watch = useCallback(() => {
    if (active) void startStream(active.login, asStream(active));
  }, [active, startStream]);
  const openRoom = useCallback(() => {
    if (active) void useAppStore.getState().startOfflineChat(active.login, asStream(active));
  }, [active]);
  const openAdd = useCallback(() => setAddOpen(true), []);
  const closeAdd = useCallback(() => setAddOpen(false), []);

  // Leaving: the grabber (a tap), a slide down that starts on the header (the
  // grabber row or the room strip), or back. The chat list is never a handle:
  // a drag there is always a scroll.
  const exitChats = useMobileNavStore((s) => s.exitChats);
  const sheetRef = useRef<HTMLDivElement | null>(null);
  // Where the composer (and any streak banner over it) starts, for the mini
  // player's rest: it follows the banners in and out and the keyboard.
  const setChatsFloor = useMobileNavStore((s) => s.setChatsFloor);
  const hasRooms = tabs.length > 0;
  useEffect(() => {
    const el = shown && hasRooms ? sheetRef.current?.querySelector<HTMLElement>('[data-chat-composer]') : null;
    if (!el) {
      setChatsFloor(null);
      return;
    }
    const read = () => setChatsFloor(Math.round(window.innerHeight - el.getBoundingClientRect().top));
    read();
    const observer = new ResizeObserver(read);
    observer.observe(el);
    window.addEventListener('resize', read);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', read);
      setChatsFloor(null);
    };
  }, [shown, hasRooms, activeChannel, setChatsFloor]);
  const drag = useRef<{
    id: number;
    x: number;
    y: number;
    t: number;
    dy: number;
    vy: number;
    live: boolean;
  } | null>(null);
  const swallowClick = useRef(false);

  const settle = useCallback(
    (from: number, leave: boolean) => {
      const el = sheetRef.current;
      if (!el) return;
      const to = leave ? Math.max(window.innerHeight, 400) : 0;
      el.style.transform = '';
      const anim = el.animate([{ transform: `translateY(${from}px)` }, { transform: `translateY(${to}px)` }], {
        duration: leave ? 200 : 240,
        easing: leave ? 'cubic-bezier(0.4, 0, 1, 1)' : 'cubic-bezier(0.2, 0.8, 0.2, 1)',
        fill: leave ? 'forwards' : 'none',
      });
      if (leave) {
        anim.onfinish = () => {
          exitChats();
          // Hidden now; the next showing starts from rest.
          requestAnimationFrame(() => anim.cancel());
        };
      }
    },
    [exitChats],
  );

  const headerHandlers = {
    onPointerDown: (e: React.PointerEvent) => {
      drag.current = { id: e.pointerId, x: e.clientX, y: e.clientY, t: e.timeStamp, dy: 0, vy: 0, live: false };
    },
    onPointerMove: (e: React.PointerEvent) => {
      const d = drag.current;
      if (!d || d.id !== e.pointerId) return;
      const dx = e.clientX - d.x;
      const dy = e.clientY - d.y;
      if (!d.live) {
        // Sideways belongs to the room row's own scroll.
        if (Math.abs(dx) > 10 && Math.abs(dx) > Math.abs(dy)) {
          drag.current = null;
          return;
        }
        if (dy < 8 || dy < Math.abs(dx)) return;
        d.live = true;
        swallowClick.current = true;
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      }
      const dt = Math.max(1, e.timeStamp - d.t);
      d.vy = (dy - d.dy) / dt;
      d.dy = Math.max(0, dy);
      d.t = e.timeStamp;
      if (sheetRef.current) sheetRef.current.style.transform = `translateY(${d.dy}px)`;
    },
    onPointerUp: (e: React.PointerEvent) => {
      const d = drag.current;
      drag.current = null;
      if (!d || d.id !== e.pointerId || !d.live) return;
      settle(d.dy, d.dy > 120 || d.vy > 0.6);
    },
    onPointerCancel: () => {
      const d = drag.current;
      drag.current = null;
      if (d?.live) settle(d.dy, false);
    },
    // A slide is not a tap on the room it started on.
    onClickCapture: (e: React.MouseEvent) => {
      if (!swallowClick.current) return;
      swallowClick.current = false;
      e.stopPropagation();
      e.preventDefault();
    },
  };

  // The handle: the whole band from the top of the screen (through the status
  // bar's inset) down to the rooms, so it is easy to catch. A tap closes, a
  // slide down leaves, and the pill lights up while it is held. It takes no
  // more layout room than its 16 px row: the rooms below get the air
  // (CHATS_STRIP_H), and the band reaches up into the inset instead.
  const grabber = (
    <button
      onClick={exitChats}
      aria-label="Close chats"
      className="group w-full shrink-0 flex items-start justify-center"
      style={{
        marginTop: 'calc(-1 * var(--sn-safe-t, 0px))',
        paddingTop: 'calc(var(--sn-safe-t, 0px) + 6px)',
        height: 'max(calc(var(--sn-safe-t, 0px) + 16px), 44px)',
      }}
    >
      <span className="block h-[5px] w-10 rounded-full bg-textMuted/60 transition-[width,background-color] duration-150 group-active:w-12 group-active:bg-textSecondary" />
    </button>
  );

  // The room on screen streams together, or shares its chat: said the way the
  // watch screen says it (Rust's channel state for the room, the partners'
  // faces and the group's word), in the strip's empty left side. Tapping it
  // lists the partners, and a partner starts their stream.
  // The room's group comes from the maps the Following cards read (by Twitch id), so the chip
  // and the card always agree; the channel's own pushed state covers a room the maps miss.
  const roomState = useChannelState(active?.provider === 'twitch' ? active.login : null);
  const collabs = useAppStore((s) => s.collaborations);
  const sharedChats = useAppStore((s) => s.sharedChats);
  const group =
    (active ? groupFor(collabs, sharedChats, { provider: active.provider, user_id: active.channelId ?? '' }) : undefined) ??
    roomState?.collab ??
    roomState?.shared_chat ??
    null;
  const [groupOpen, setGroupOpen] = useState(false);
  // Leaving Chats closes its sheets: they draw outside the hidden screen.
  useEffect(() => {
    if (shown) return;
    setAddOpen(false);
    setGroupOpen(false);
  }, [shown]);
  const leading = group ? (
    <button
      onClick={() => setGroupOpen(true)}
      className="min-w-0 h-11 flex items-center active:opacity-70"
      aria-label={group.label}
    >
      <span className="min-w-0 h-10 flex items-center gap-1.5 pl-2 pr-3 rounded-full chrome-glaze chrome-glaze--flat text-textSecondary">
        {!isTogether(group) && <ChatsCircle size={14} weight="fill" className="shrink-0" />}
        <span className="shrink-0">
          <CollabAvatarStack collab={group} size={18} />
        </span>
        <span className="min-w-0 truncate text-[12px] font-medium">{isTogether(group) ? 'Together' : 'Shared chat'}</span>
      </span>
    </button>
  ) : null;

  // The strip's own parts: the same 40 px glass as the rooms capsule, quieter
  // than the room it serves (secondary text, no red; the room's face already
  // says live). Each button keeps a 44 px touch area around its 40 px shape,
  // so the row shares one top and bottom line.
  const trailing = (
    <>
      {canWatch && (
        <button
          onClick={watch}
          // At least the circles' width, so the row keeps its spacing between a
          // live room and an offline one; a viewer count still widens it.
          className="shrink-0 h-11 min-w-11 flex items-center justify-center active:opacity-70"
          aria-label={`Watch ${active?.label ?? ''}`}
        >
          <span className="h-10 min-w-10 flex items-center justify-center gap-1.5 px-3 rounded-full chrome-glaze chrome-glaze--flat text-textSecondary">
            <Play size={12} weight="fill" className="translate-x-[0.5px]" />
            {active?.viewerCount != null && (
              <span className="text-[12px] font-medium tabular-nums">{formatViewerCount(active.viewerCount)}</span>
            )}
          </span>
        </button>
      )}
      {canOpenRoom && (
        <button
          onClick={openRoom}
          className="shrink-0 w-11 h-11 flex items-center justify-center active:opacity-70"
          aria-label={`Open ${active?.label ?? ''}'s channel`}
        >
          <span className="w-10 h-10 flex items-center justify-center rounded-full chrome-glaze chrome-glaze--flat text-textSecondary">
            <MonitorPlay size={14} weight="bold" />
          </span>
        </button>
      )}
      <button
        onClick={openAdd}
        className="shrink-0 w-11 h-11 flex items-center justify-center active:opacity-70"
        aria-label="Add a chat"
      >
        <span className="w-10 h-10 flex items-center justify-center rounded-full chrome-glaze chrome-glaze--flat text-textSecondary">
          <Plus size={15} weight="bold" />
        </span>
      </button>
    </>
  );

  return (
    <div ref={sheetRef} className="flex-1 min-h-0 flex flex-col">
      {/* The header is the only drag handle (touch-action leaves vertical
          moves to it; the room row still scrolls sideways). */}
      <div className="shrink-0 flex flex-col [touch-action:pan-x]" {...headerHandlers}>
        {grabber}
        {tabs.length > 0 && (
          <ChatTabStrip
            always
            trailing={trailing}
            leading={leading}
            edge={<ChatsMinimizeButton onMinimize={exitChats} />}
            height={CHATS_STRIP_H}
          />
        )}
      </div>
      {tabs.length === 0 ? (
        // No chat open: the tab is the picker itself, on the glaze the sheets
        // wear (solid at 0% glassiness). The search field stays put and fully
        // drawn; only the lists under it scroll and fade.
        <div className="flex-1 min-h-0 flex flex-col px-2 pt-1" style={{ paddingBottom: 'calc(var(--sn-safe-b, 0px) + 8px)' }}>
          {/* The glaze is a pill by default; a sheet-sized panel takes the sheets' corner. */}
          <div className="flex-1 min-h-0 flex flex-col pt-3 px-1 chrome-glaze chrome-glaze--flat" style={{ borderRadius: 28 }}>
            <ChatPicker scrolls bottomClearance="16px" onClose={exitChats} />
          </div>
        </div>
      ) : (
        <>
          <div className="flex-1 min-h-0 relative flex flex-col">
            {hasPins && (
              // A grabber's gap under the header, 4 px over the list.
              <div
                ref={pinsRef}
                className="absolute left-0 right-0 top-0 px-2.5 pt-3 pb-1 z-20 pointer-events-none flex flex-col gap-1"
              >
                <PinnedBanner
                  pins={pinned}
                  channelId={chatChannelId ?? undefined}
                  expanded={pinsOpen}
                  onToggle={() => pins.setOpen(!pinsOpen)}
                  collapsedStyle={pins.style}
                />
              </div>
            )}
            {hasPins && <div className="shrink-0" style={{ height: pinsBox }} />}
            <MobileChatPane strip={false} paused={!shown} idle={idle} surface="chats" />
          </div>
        </>
      )}
      <AddChatSheet open={addOpen} onClose={closeAdd} />
      <TogetherSheet collab={group} open={groupOpen} onClose={() => setGroupOpen(false)} />
    </div>
  );
};
