import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { motion, useMotionValue } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { Trash2, Clock, Ban, RotateCcw, Pin, PinOff, type LucideIcon } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { useChatUserStore } from '../../stores/chatUserStore';
import { computePaintStyle } from '../../services/seventvService';
import { useDragModerationStore } from '../../stores/dragModerationStore';
import { usePinStore } from '../../stores/pinStore';
import { Logger } from '../../utils/logger';
import { BucketTile, SOLID_TINT, type Bucket } from './ModBucketTile';
import {
  MAX_TIMEOUT_SECS,
  formatDuration,
  timeoutSecsFromDistance,
} from '../../utils/timeoutRamp';
import { kickAppliedSeconds, kickTimeoutMinutes } from '../../utils/kickTimeout';


// The timeout ramp now lives in utils/timeoutRamp so the mobile fan-out dials
// on the identical curve. Behaviour here is unchanged.

/**
 * Global drag-to-moderate overlay. A grab handle in the chat dock "lifts" a
 * chatter into a chip that follows the cursor; a dock of action buckets animates
 * in, reacts as you drag over them, and fires the action on drop. Mod buckets
 * (delete/timeout/ban) only appear for moderators; profile/whisper are universal.
 * Hit-testing uses elementFromPoint, so the chip + overlay are pointer-events:none
 * and only the bucket tiles capture the pointer.
 */
export default function ModerationDragLayer() {
  const dragged = useDragModerationStore((s) => s.dragged);
  const paintShadowMode = useAppStore((s) => s.settings.cosmetics?.paint_shadows) ?? 'all';
  const paint = useChatUserStore((s) => (dragged ? s.users.get(dragged.userId)?.paint : undefined));

  const x = useMotionValue(0);
  const y = useMotionValue(0);
  const [activeBucket, setActiveBucket] = useState<string | null>(null);
  const [activeDuration, setActiveDuration] = useState<number | null>(null);
  const activeBucketRef = useRef<string | null>(null);
  const activeDurationRef = useRef<number | null>(null);
  const chatPlacement = useAppStore((s) => s.chatPlacement);
  const dragLayoutRaw = useAppStore((s) => s.settings.chat_design?.mod_drag_layout);
  // Two layouts: 'column' (beside chat) and 'bar' (above chat). The legacy
  // 'slider' mode was removed; fall back to the beside-chat column.
  const dragLayout: 'column' | 'bar' = dragLayoutRaw === 'bar' ? 'bar' : 'column';
  const modPinStyle = useAppStore((s) => s.settings.chat_design?.mod_pin_style) ?? 'both';
  // Whether the lifted message is the currently-pinned one (flips the Pin bucket
  // into Unpin).
  const draggedPinned = usePinStore((s) => (dragged?.messageId ? s.pinnedIds.includes(dragged.messageId) : false));
  const [anchorStyle, setAnchorStyle] = useState<CSSProperties | null>(null);
  const columnRef = useRef<HTMLDivElement>(null);

  // Spawn the column vertically near the message you grabbed (centered on the
  // pickup point, clamped to the viewport using the column's real height), and
  // horizontally just left of the chat panel when chat is right-docked (else
  // screen-centered). Measured in a layout effect so it never flashes elsewhere.
  useLayoutEffect(() => {
    // Nothing to measure without a drag; the render below hides the stale
    // anchor itself, and the next drag re-measures before paint.
    if (!dragged) return;
    const panel = document.querySelector('[data-chat-panel]') as HTMLElement | null;
    const rect = panel?.getBoundingClientRect();

    // Bar: a horizontal row centered above the grabbed MESSAGE. Anchor to the
    // message row's TOP edge (not the pickup point's Y) so the bar always opens
    // fully clear above the whole message — never overlapping it or sitting
    // right next to the cursor, no matter where on the row you grabbed.
    // Horizontally centered over chat.
    if (dragLayout === 'bar') {
      const { origin } = useDragModerationStore.getState();
      const cx = rect && rect.width > 0 ? rect.left + rect.width / 2 : origin.x;
      const msgEl = dragged.messageId
        ? (document.querySelector(`[data-message-id="${dragged.messageId}"]`) as HTMLElement | null)
        : null;
      const msgRect = msgEl?.getBoundingClientRect();
      const anchorY = msgRect && msgRect.height > 0 ? msgRect.top : origin.y;
      // Layout measurement: read the message row's position after layout and
      // commit the anchor before paint. The synchronous second render is the
      // point of useLayoutEffect here. (This keeps the layer out of the
      // compiler; it only renders while a message is being dragged.)
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setAnchorStyle({ top: Math.max(8, anchorY - 10), left: cx, transform: 'translate(-50%, -100%)' });
      return;
    }

    // Column: vertical, near the grabbed message, just left of the chat panel.
    const { origin } = useDragModerationStore.getState();
    const margin = 12;
    const h = columnRef.current?.offsetHeight ?? 220;
    // Never let the stack reach down into the player's control bar. The controls
    // are pinned to the player's bottom edge, so reserve their height up from
    // there. Use offsetHeight (layout height), not getBoundingClientRect: Plyr
    // translates the bar fully off-screen while hidden, which is exactly when a
    // drag begins, so its on-screen rect can't be trusted.
    let lowerBound = window.innerHeight;
    const player = document.querySelector('.video-player-container') as HTMLElement | null;
    const playerRect = player?.getBoundingClientRect();
    if (playerRect && playerRect.height > 0) {
      const controls = document.querySelector('.plyr__controls') as HTMLElement | null;
      const controlsH = controls && controls.offsetHeight > 0 ? controls.offsetHeight : 88;
      lowerBound = playerRect.bottom - controlsH;
    }
    const bottomLimit = Math.max(margin, lowerBound - h - margin);
    const top = Math.min(Math.max(origin.y - h / 2, margin), bottomLimit);
    if (rect && rect.width > 0 && chatPlacement === 'right') {
      setAnchorStyle({ top, right: Math.max(8, window.innerWidth - rect.left + 12) });
    } else {
      setAnchorStyle({ top, left: '50%', transform: 'translateX(-50%)' });
    }
  }, [dragged, chatPlacement, dragLayout]);

  useEffect(() => {
    if (!dragged) return;
    const { origin } = useDragModerationStore.getState();
    x.set(origin.x);
    y.set(origin.y);
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'grabbing';

    const setActive = (bucket: string | null, duration: number | null) => {
      if (bucket !== activeBucketRef.current) {
        activeBucketRef.current = bucket;
        setActiveBucket(bucket);
      }
      if (duration !== activeDurationRef.current) {
        activeDurationRef.current = duration;
        setActiveDuration(duration);
      }
    };

    const onMove = (e: PointerEvent) => {
      x.set(e.clientX);
      y.set(e.clientY);

      // Magnetic selection: snap to the NEAREST bucket within an engage radius,
      // so you never have to be pixel-precise. Distance to each tile center, not
      // exact elementFromPoint hover. The above-chat layout uses a tighter radius
      // so it's easy to move off a bucket and release to cancel.
      const ENGAGE = dragLayout === 'bar' ? 60 : 120;
      let bucketId: string | null = null;
      let bestD = Infinity;
      document.querySelectorAll<HTMLElement>('[data-bucket-id]').forEach((tile) => {
        const r = tile.getBoundingClientRect();
        const d = Math.hypot(e.clientX - (r.left + r.width / 2), e.clientY - (r.top + r.height / 2));
        if (d < bestD) {
          bestD = d;
          bucketId = tile.getAttribute('data-bucket-id');
        }
      });
      if (bestD > ENGAGE) bucketId = null;

      // Timeout duration is continuous: once Timeout is engaged, dragging FURTHER
      // LEFT (Timeout sits on the left of the triangle, into the open player area)
      // dials a longer timeout — no fixed chip list to clip off-screen. Same in
      // both layouts. It stays engaged while you drag out past the tile.
      let duration: number | null = null;
      const tTile = document.querySelector<HTMLElement>('[data-bucket-id="timeout"]');
      if (tTile) {
        const r = tTile.getBoundingClientRect();
        // Drag further left of the tile = longer timeout.
        const out = r.left - e.clientX;
        if (bucketId === 'timeout' || (activeBucketRef.current === 'timeout' && out > 0)) {
          bucketId = 'timeout';
          duration = timeoutSecsFromDistance(Math.max(0, out));
        }
      }
      setActive(bucketId, duration);
    };

    // One place for every action: every bucket drop dispatches here.
    const runAction = (kind: string, secs?: number) => {
      const { userId, login, displayName, broadcasterId, messageId } = dragged;
      const app = useAppStore.getState();
      // Kick routes ban/timeout/unban to its own moderation API (numeric ids,
      // duration in minutes). Delete is provider-branched below; pin is Twitch
      // only and its bucket is hidden elsewhere for providers.
      const isKick = dragged.provider === 'kick';
      const isTwitchDrag = (dragged.provider ?? 'twitch') === 'twitch';
      // YouTube routes to the webview-session mod commands (the chatter's channel id +
      // the source slug carried on the drag). Timeout uses YouTube's fixed length.
      const isYouTube = dragged.provider === 'youtube';
      const kickBan = (durationMinutes: number | null) =>
        invoke('kick_ban_user', {
          broadcasterUserId: Number(broadcasterId),
          targetUserId: Number(userId),
          durationMinutes,
          reason: null,
        });
      const kickUnban = () =>
        invoke('kick_unban_user', {
          broadcasterUserId: Number(broadcasterId),
          targetUserId: Number(userId),
        });
      const youtubeBan = (durationSeconds: number | null) =>
        invoke('youtube_ban_user', {
          channel: dragged.channel,
          targetChannelId: userId,
          durationSeconds,
        });
      const youtubeUnban = () =>
        invoke('youtube_unban_user', { channel: dragged.channel, targetChannelId: userId });
      const undo = {
        label: 'Undo',
        onClick: () => {
          (isKick
            ? kickUnban()
            : isYouTube
            ? youtubeUnban()
            : invoke('unban_user', { broadcasterId, targetUserId: userId })
          ).catch((err) => Logger.error('[DragMod] Undo failed:', err));
        },
      };
      switch (kind) {
        case 'profile':
          app.openProfileViewer(userId);
          break;
        case 'whisper':
          app.openWhisperWithUser({ id: userId, login, display_name: displayName });
          break;
        case 'delete':
          if (messageId) {
            (isKick
              ? invoke('kick_delete_message', { messageId })
              : isYouTube
              ? invoke('youtube_delete_message', { channel: dragged.channel, messageId })
              : invoke('delete_chat_message', { broadcasterId, messageId })
            )
              .then(() => app.addToast(`Deleted a message from ${displayName}`, 'success'))
              .catch((err) => {
                Logger.error('[DragMod] Delete failed:', err);
                app.addToast(`Couldn't delete that message`, 'error');
              });
          }
          break;
        case 'timeout': {
          const s = secs ?? 600;
          // Kick only accepts whole minutes, so report the duration it will
          // actually apply rather than the one the ramp was sitting on.
          const applied = isKick ? kickAppliedSeconds(s) : s;
          (isKick
            ? kickBan(kickTimeoutMinutes(s))
            : isYouTube
            ? youtubeBan(s)
            : invoke('ban_user', { broadcasterId, targetUserId: userId, duration: s, reason: null })
          )
            .then(() => app.addToast(`Timed out ${displayName} for ${formatDuration(applied)}`, 'success', undo))
            .catch((err) => {
              Logger.error('[DragMod] Timeout failed:', err);
              app.addToast(`Couldn't time out ${displayName}`, 'error');
            });
          break;
        }
        case 'ban':
          (isKick
            ? kickBan(null)
            : isYouTube
            ? youtubeBan(null)
            : invoke('ban_user', { broadcasterId, targetUserId: userId, duration: null, reason: null })
          )
            .then(() => app.addToast(`Banned ${displayName}`, 'success', undo))
            .catch((err) => {
              Logger.error('[DragMod] Ban failed:', err);
              app.addToast(`Couldn't ban ${displayName}`, 'error');
            });
          break;
        case 'pin':
          if (messageId && isTwitchDrag) {
            invoke('pin_chat_message', { broadcasterId, messageId, durationSeconds: null })
              .then(() => {
                app.addToast(`Pinned a message from ${displayName}`, 'success');
                usePinStore.getState().requestRefresh();
              })
              .catch((err) => {
                Logger.error('[DragMod] Pin failed:', err);
                app.addToast(`Couldn't pin that message`, 'error');
              });
          }
          break;
        case 'unpin':
          if (messageId && isTwitchDrag) {
            invoke('unpin_chat_message', { broadcasterId, messageId })
              .then(() => {
                app.addToast(`Unpinned the message`, 'success');
                usePinStore.getState().requestRefresh();
              })
              .catch((err) => {
                Logger.error('[DragMod] Unpin failed:', err);
                app.addToast(`Couldn't unpin that message`, 'error');
              });
          }
          break;
        case 'unban':
          (isKick
            ? kickUnban()
            : isYouTube
            ? youtubeUnban()
            : invoke('unban_user', { broadcasterId, targetUserId: userId }))
            .then(() =>
              app.addToast(
                dragged.moderationState === 'timeout'
                  ? `Removed timeout on ${displayName}`
                  : `Unbanned ${displayName}`,
                'success',
              ),
            )
            .catch((err) => {
              Logger.error('[DragMod] Unban failed:', err);
              app.addToast(
                `Couldn't lift the ${dragged.moderationState === 'timeout' ? 'timeout' : 'ban'} on ${displayName}`,
                'error',
              );
            });
          break;
      }
    };

    const execute = () => {
      const bucket = activeBucketRef.current;
      if (bucket) runAction(bucket, activeDurationRef.current ?? undefined);
    };

    const cleanup = () => {
      activeBucketRef.current = null;
      activeDurationRef.current = null;
      setActiveBucket(null);
      setActiveDuration(null);
      useDragModerationStore.getState().endDrag();
    };
    const onUp = () => {
      execute();
      cleanup();
    };
    // pointercancel (gesture taken over by the browser) fires instead of
    // pointerup — cancel without executing so the overlay never gets stuck.
    const onCancel = () => cleanup();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') cleanup();
    };

    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onCancel);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
      window.removeEventListener('keydown', onKey);
      document.body.style.userSelect = '';
      document.body.style.cursor = '';
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dragged]);

  if (!dragged) return null;

  const nameStyle = paint
    ? computePaintStyle(paint, dragged.color, paintShadowMode)
    : { color: dragged.color || 'var(--color-accent)' };

  const moderated = dragged.moderationState;
  const timeoutBucket = { id: 'timeout', label: 'Timeout', icon: Clock, kind: 'danger', activeTint: 'bg-amber-500/25 border-amber-400/70 text-amber-200' } as Bucket;
  const banBucket = { id: 'ban', label: 'Ban', icon: Ban, kind: 'danger', activeTint: 'bg-red-500/25 border-red-400/70 text-red-200' } as Bucket;
  const deleteBucket = { id: 'delete', label: 'Delete', icon: Trash2, kind: 'danger', activeTint: 'bg-orange-500/25 border-orange-400/70 text-orange-200' } as Bucket;
  const unbanBucket = {
    id: 'unban',
    label: moderated === 'timeout' ? 'Untimeout' : 'Unban',
    icon: RotateCcw,
    kind: 'danger',
    activeTint: 'bg-emerald-500/25 border-emerald-400/70 text-emerald-200',
  } as Bucket;
  const pinBucket = (draggedPinned
    ? { id: 'unpin', label: 'Unpin', icon: PinOff, kind: 'neutral', activeTint: 'bg-sky-500/25 border-sky-400/70 text-sky-200' }
    : { id: 'pin', label: 'Pin', icon: Pin, kind: 'neutral', activeTint: 'bg-sky-500/25 border-sky-400/70 text-sky-200' }) as Bucket;
  // Mod-only PUNITIVE buckets (profile/whisper removed). The triangle puts the
  // primary action at the apex and Timeout (left) + Delete (right) at the base:
  //   normal    -> Timeout (left), Ban (apex), Delete (right)
  //   timed out -> Timeout (left), Untimeout (apex), Delete (right)
  //   banned    -> Unban (+ Delete) in a single row
  // So an accidental ban/timeout is always reversible via the apex Unban/Untimeout.
  // Order matters: the triangle reads index 0 = base-left, 1 = apex, 2 = base-right.
  const core: Bucket[] =
    moderated === 'ban'
      ? [unbanBucket]
      : moderated === 'timeout'
        ? [timeoutBucket, unbanBucket]
        : [timeoutBucket, banBucket];
  const punitive: Bucket[] = !dragged.isModerator
    ? []
    : [...core, ...(dragged.messageId ? [deleteBucket] : [])];
  // Pin is NON-punitive, so it sits as its own tile ABOVE the triangle (not in
  // it) when the pin style includes 'drag' and there's a message to pin.
  // Inline pin is always available (ChatMessage); the drag tile is the optional
  // extra — shown unless the setting is 'inline' (button only). Legacy 'drag'
  // and the default 'both' both enable it.
  // Twitch-only: pinning is Helix, and provider messages DO carry a messageId,
  // so without this the bucket would appear on Kick/YouTube and pin against a
  // Twitch channel that happens to share the id.
  const showDragPin =
    dragged.isModerator &&
    !!dragged.messageId &&
    (dragged.provider ?? 'twitch') === 'twitch' &&
    modPinStyle !== 'inline';
  const buckets: Bucket[] = [...(showDragPin ? [pinBucket] : []), ...punitive];
  // Triangle is the ABOVE-CHAT layout only: 3 = apex (Ban / Untimeout) on top,
  // Timeout (left) + Delete (right) as the base; fewer just sit in one centered
  // row. Beside chat stacks every bucket vertically instead (see render).
  const triTop = punitive.length === 3 ? [punitive[1]] : punitive;
  const triBottom = punitive.length === 3 ? [punitive[0], punitive[2]] : [];
  // Wider spacing in the above-chat layout (paired with the tighter engage
  // radius) so it's easier to land on, and move off, a tile; the beside-chat
  // column uses a roomier gap to match its bigger tiles.
  const gapCls = dragLayout === 'bar' ? 'gap-4' : 'gap-2.5';

  // `armed` (a real action is selected) drives the chip's "snapped in" feedback.
  const armed = !!activeBucket;

  return createPortal(
    <div className="fixed inset-0 z-[9998] pointer-events-none overflow-hidden">
      {/* Chip held by the cursor. z-50 keeps it ABOVE the bucket column so the
          dragged name floats over the drop target instead of clipping under it.
          The static wrapper carries the up-left offset so it doesn't fight
          framer-motion's animated transform on the chip. */}
      <motion.div className="fixed top-0 left-0 z-50 pointer-events-none" style={{ x, y }}>
        <div style={{ transform: 'translate(-50%, calc(-100% - 16px))' }}>
          <motion.div
            initial={{ scale: 0.6, opacity: 0 }}
            // When an action is armed the chip shrinks + straightens: a little
            // "snapped in" cue that pairs with the bucket pop.
            animate={{ scale: armed ? 0.86 : 1, opacity: 1, rotate: armed ? 0 : -4 }}
            transition={{ type: 'spring', stiffness: 500, damping: 26 }}
            className="flex items-center gap-1.5 px-3 py-2 rounded-xl border border-white/15 backdrop-blur-md shadow-[0_12px_32px_rgba(0,0,0,0.6)] whitespace-nowrap"
            // Themed (was hardcoded zinc-900/90).
            style={{ backgroundColor: 'color-mix(in srgb, var(--color-background-tertiary) 90%, transparent)' }}
          >
            <span className="text-sm font-bold" style={nameStyle}>
              {dragged.displayName}
            </span>
          </motion.div>
        </div>
      </motion.div>

      {/* Floating action dock (no panel/blur). Beside chat: one vertical stack of
          bigger, translucent tiles, clamped to sit above the player controls.
          Above chat: a compact triangle of opaque tiles where room is tight. */}
      {buckets.length > 0 && (
      <div className="fixed pointer-events-none" style={(dragged ? anchorStyle : null) ?? undefined}>
        <motion.div
          ref={columnRef}
          initial={{ scale: 0.92, opacity: 0 }}
          animate={{ scale: 1, opacity: 1 }}
          transition={{ type: 'spring', stiffness: 400, damping: 30 }}
          className={`pointer-events-auto flex flex-col items-center ${gapCls}`}
        >
          {dragLayout === 'column' ? (
            // Beside chat: every action in one vertical column. Bigger + a touch
            // translucent so they're easy to hit when dragging out to the side.
            buckets.map((b) => (
              <BucketTile key={b.id} bucket={b} active={activeBucket === b.id} activeDuration={activeDuration} solid={false} big />
            ))
          ) : (
            <>
              {showDragPin && (
                <div className={`flex ${gapCls}`}>
                  <BucketTile bucket={pinBucket} active={activeBucket === pinBucket.id} activeDuration={null} solid />
                </div>
              )}
              <div className={`flex ${gapCls}`}>
                {triTop.map((b) => (
                  <BucketTile key={b.id} bucket={b} active={activeBucket === b.id} activeDuration={activeDuration} solid />
                ))}
              </div>
              {triBottom.length > 0 && (
                <div className={`flex ${gapCls}`}>
                  {triBottom.map((b) => (
                    <BucketTile key={b.id} bucket={b} active={activeBucket === b.id} activeDuration={activeDuration} solid />
                  ))}
                </div>
              )}
            </>
          )}
        </motion.div>
      </div>
      )}
    </div>,
    document.body,
  );
}
