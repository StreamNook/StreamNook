// The chats of the Sliced layout: under every tile, the chat of that tile, at
// the rectangle the layout engine gave it (nookLayout.ts `chat`).
//
// Each one is an ordinary ChatWidget pointed at the tile's channel through the
// same `channelOverride` seam the chat dock, MultiChat panes and provider
// streams use, so sending, emotes and mod tools work as they do anywhere else.
// The grid already keeps every visible tile's chat connected in the background
// (MultiNookView), so a slice has its messages the moment it mounts.
//
// A chat is a heavy thing to mount, and switching to Sliced asks for one per
// tile. Mounted together in the commit that moves the tiles, eight of them
// held the page for about a second and the tiles jumped instead of gliding. So
// the frames go up with the tiles, and the chats fill them one at a time once
// the tiles have landed, each in a transition the page can interrupt. Leaving
// Sliced is the same care the other way: the chats fade out at once but stay
// mounted, unseen and unclickable, until the tiles have landed (MultiNookView
// holds them in an AnimatePresence), so tearing eight chats down never lands
// in the middle of the glide.
//
// The slice whose tile holds the grid's active chat owns the moderation
// hotkeys; pressing into another slice makes that one active. As in a docked
// chat, EventSub-driven extras stay with the watched stream.
import { memo, startTransition, useEffect, useMemo, useState } from 'react';
import { motion, useIsPresent } from 'framer-motion';
import ChatWidget, { type ChatWidgetChannelOverride } from '../ChatWidget';
import ErrorBoundary from '../ErrorBoundary';
import { useChannelState } from '../../stores/channelStateStore';
import { activeChatSlot, usemultiNookStore } from '../../stores/multiNookStore';
import { makeKey } from '../../utils/providerKey';
import type { MultiNookSlot } from '../../types';
import type { Rect } from './nookLayout';

/** The tiles' own spring (MultiNookCell), so a frame moves with its tile. */
const TILE_SPRING = { type: 'spring', stiffness: 350, damping: 30 } as const;
/** How long the tiles take to land before the first chat mounts. */
const SETTLE_MS = 320;
/** The pause between one chat mounting and the next. */
const STEP_MS = 40;
/** Leaving Sliced: the fade, then the hold until the tiles have landed. */
const FADE_OUT_S = 0.12;
const LEAVE_S = 0.55;

export interface SliceChat {
  slot: MultiNookSlot;
  rect: Rect;
}

export function MultiNookSliceChats({ slices }: { slices: SliceChat[] }) {
  // False while leaving: the chats are on their way out.
  const present = useIsPresent();
  // How many of the chats are mounted so far, in tile order.
  const [mounted, setMounted] = useState(0);
  const wanted = slices.length;
  useEffect(() => {
    if (!present || mounted >= wanted) return;
    const timer = setTimeout(
      () => startTransition(() => setMounted((n) => n + 1)),
      mounted === 0 ? SETTLE_MS : STEP_MS,
    );
    return () => clearTimeout(timer);
  }, [mounted, wanted, present]);

  return (
    // Layout only: the frames inside take the pointer, the gaps between them
    // leave it to the tiles.
    <motion.div
      className="pointer-events-none absolute inset-0"
      // Gone from sight in FADE_OUT_S, then held, unseen, to LEAVE_S.
      exit={{ opacity: [1, 0, 0], transition: { duration: LEAVE_S, times: [0, FADE_OUT_S / LEAVE_S, 1] } }}
    >
      {slices.map(({ slot, rect }, i) => (
        <motion.div
          key={slot.id}
          layout="position"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ layout: TILE_SPRING, opacity: { duration: 0.2, ease: 'easeOut' } }}
          className="pointer-events-auto absolute overflow-hidden rounded-lg border border-white/5 bg-background"
          style={{ left: rect.x, top: rect.y, width: rect.w, height: rect.h }}
          onPointerDownCapture={() => {
            const s = usemultiNookStore.getState();
            if (activeChatSlot(s.slots, s.activeChatChannelId)?.id !== slot.id) {
              s.setActiveChatChannelId(makeKey(slot.provider ?? 'twitch', slot.channelLogin));
            }
          }}
        >
          {i < mounted && (
            <ErrorBoundary componentName="SliceChat" reportToLogService resetKeys={[slot.id]}>
              <SliceChatBody slot={slot} />
            </ErrorBoundary>
          )}
          {/* Leaving, an empty cover takes the clicks meant for the tiles
              arriving underneath. Not pointer-events on the frame: that
              property is inherited, and switching it restyled every element
              of all eight chats in the first frame of the tiles' glide
              (about 100 ms). */}
          {!present && <div className="absolute inset-0 z-10" aria-hidden />}
        </motion.div>
      ))}
    </motion.div>
  );
}

// Memoised on its slot: the grid re-renders for every tile change and for the
// switch out of Sliced, and this file is outside the React Compiler's
// allowlist, so without it each of those re-rendered all eight chats in full
// (about 60 ms, in the first frame of the tiles' glide).
const SliceChatBody = memo(function SliceChatBody({ slot }: { slot: MultiNookSlot }) {
  const provider = slot.provider ?? 'twitch';
  const isTwitch = provider === 'twitch';
  const isActive = usemultiNookStore((s) => activeChatSlot(s.slots, s.activeChatChannelId)?.id === slot.id);
  // Header numbers for a Twitch tile come from Rust's channel state, which
  // ChatWidget already watches; unknown until its first answer.
  const state = useChannelState(isTwitch ? slot.channelLogin : null);
  const answered = state?.viewers_at != null;
  const live = answered && !!state?.started_at;

  const channelOverride = useMemo<ChatWidgetChannelOverride>(
    () => ({
      provider,
      context: 'main',
      user_login: isTwitch ? slot.channelLogin : slot.channelLogin.toLowerCase(),
      user_id: slot.channelId ?? '',
      user_name: slot.channelName || slot.channelLogin,
      profile_image_url: slot.profileImageUrl,
      title: state?.title ?? slot.title,
      game_name: state?.game_name ?? slot.gameName,
      viewer_count: state?.viewer_count ?? undefined,
      started_at: state?.started_at ?? undefined,
      // Rust's grid live check until the channel state answers.
      is_live: isTwitch && answered ? live : slot.offline !== true,
      is_active: isActive,
    }),
    [
      provider,
      isTwitch,
      slot.channelLogin,
      slot.channelId,
      slot.channelName,
      slot.profileImageUrl,
      slot.title,
      slot.gameName,
      slot.offline,
      state?.title,
      state?.game_name,
      state?.viewer_count,
      state?.started_at,
      answered,
      live,
      isActive,
    ],
  );

  return (
    <motion.div
      className="h-full w-full"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.18, ease: 'easeOut' }}
    >
      <ChatWidget channelOverride={channelOverride} />
    </motion.div>
  );
});
