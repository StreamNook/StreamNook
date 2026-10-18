import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion, useMotionValue } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { IconContext } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import type { ProviderId } from '../../types/providers';
import { useChatUserStore } from '../../stores/chatUserStore';
import { computePaintStyle } from '../../services/seventvService';
import { useDragModerationStore } from '../../stores/dragModerationStore';
import { usePinStore } from '../../stores/pinStore';
import { Logger } from '../../utils/logger';
import { BucketTile, type Bucket } from './ModBucketTile';
import { BAN_GAP_PX, BAN_TILE, DELETE_TILE, PIN_TILE, TIMEOUT_TILE, UNPIN_TILE, liftTile, pickTile } from './modTiles';
import { DEFAULT_BUCKET_TIMEOUT_SECS, moderateWithToast } from '../../services/moderation';

/** The tile glyphs' weight: the bold line matches the tile labels. */
const TILE_ICONS = { weight: 'bold' } as const;

/**
 * Global drag-to-moderate overlay. Pressing a message "lifts" its chatter into
 * a chip that follows the cursor; a dock of action tiles animates in, reacts as
 * you drag over them, and fires the action on drop. Every action is one drop
 * (Timeout applies the length set in Moderation settings), a tile is picked
 * only while the cursor is on it, and letting go anywhere else cancels. Mod
 * tiles only appear for moderators.
 */
export default function ModerationDragLayer() {
  const dragged = useDragModerationStore((s) => s.dragged);
  const paintShadowMode = useAppStore((s) => s.settings.cosmetics?.paint_shadows) ?? 'all';
  const paint = useChatUserStore((s) => (dragged ? s.users.get(dragged.userId)?.paint : undefined));

  const x = useMotionValue(0);
  const y = useMotionValue(0);
  const [activeBucket, setActiveBucket] = useState<string | null>(null);
  const activeBucketRef = useRef<string | null>(null);
  // Dropping on Timeout applies this one length; other lengths are the row
  // slider's job. (A second drag to dial the length was the confusing part.)
  const bucketTimeout = useAppStore((s) => s.settings.chat_design?.mod_bucket_timeout) ?? DEFAULT_BUCKET_TIMEOUT_SECS;
  const bucketTimeoutRef = useRef(bucketTimeout);
  useEffect(() => {
    bucketTimeoutRef.current = bucketTimeout;
  }, [bucketTimeout]);
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

  // Spawn the column vertically near the message you grabbed (Timeout level
  // with the pickup point, clamped to the viewport using the column's real
  // height), and
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
        ? (document.querySelector(`[data-message-id="${CSS.escape(dragged.messageId)}"]`) as HTMLElement | null)
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
    const column = columnRef.current;
    const h = column?.offsetHeight ?? 220;
    // The pickup height meets Timeout, the common action, so a straight move
    // left lands on it: Delete sits above it and Ban below, past a wider gap.
    // offsetTop, not the on-screen box: the column opens scaled down.
    const anchorTile = column?.querySelector<HTMLElement>('[data-bucket-id="timeout"]');
    const anchorY = anchorTile ? anchorTile.offsetTop + anchorTile.offsetHeight / 2 : h / 2;
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
    const top = Math.min(Math.max(origin.y - anchorY, margin), bottomLimit);
    // Beside a right-docked chat only when the whole column fits left of it;
    // a chat nearly as wide as the window would push it past the window's
    // left edge, so it centers over the chat instead.
    const w = columnRef.current?.offsetWidth ?? 96;
    if (rect && rect.width > 0 && chatPlacement === 'right') {
      setAnchorStyle(
        rect.left - 12 - w >= margin
          ? { top, right: window.innerWidth - rect.left + 12 }
          : { top, left: rect.left + rect.width / 2, transform: 'translateX(-50%)' },
      );
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

    const setActive = (bucket: string | null) => {
      if (bucket !== activeBucketRef.current) {
        activeBucketRef.current = bucket;
        setActiveBucket(bucket);
      }
    };

    const onMove = (e: PointerEvent) => {
      x.set(e.clientX);
      y.set(e.clientY);

      // A tile is picked only while the cursor is ON it (plus a small margin).
      // The old nearest-tile-within-120px rule kept something picked almost
      // everywhere around the cluster, so letting go to cancel acted instead.
      // The picked tile keeps priority until the cursor leaves its own box
      // (it scales up, so it does not flicker at the edge); otherwise the
      // nearer center wins, never whichever tile comes last in the page.
      setActive(
        pickTile(
          document.querySelectorAll<HTMLElement>('[data-bucket-id]'),
          e.clientX,
          e.clientY,
          activeBucketRef.current,
        ),
      );
    };

    // One place for every action: every bucket drop dispatches here. Ban,
    // timeout, lift and delete go through Rust `moderate_chatter`, which picks
    // the platform API, applies its timeout rules and words the toast.
    const runAction = (kind: string) => {
      const { userId, login, displayName, broadcasterId, messageId } = dragged;
      const app = useAppStore.getState();
      const isTwitchDrag = (dragged.provider ?? 'twitch') === 'twitch';
      const target = {
        provider: dragged.provider ?? 'twitch',
        broadcasterId,
        channel: dragged.channel,
        targetUserId: userId,
        targetName: displayName,
      };
      switch (kind) {
        case 'profile':
          // The same card a click on their name opens.
          void import('../../utils/openProfilePopup').then(({ openProfilePopup }) =>
            openProfilePopup({ userId, username: login, displayName, provider: dragged.provider as ProviderId | undefined }),
          );
          break;
        case 'whisper':
          app.openWhisperWithUser({ id: userId, login, display_name: displayName });
          break;
        case 'delete':
          if (messageId) void moderateWithToast({ ...target, action: { kind: 'delete', messageId } });
          break;
        case 'timeout':
          void moderateWithToast({ ...target, action: { kind: 'timeout', seconds: bucketTimeoutRef.current } });
          break;
        case 'ban':
          void moderateWithToast({ ...target, action: { kind: 'ban' } });
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
          void moderateWithToast({
            ...target,
            action: { kind: 'unban', timeout: dragged.moderationState === 'timeout' },
          });
          break;
      }
    };

    const execute = () => {
      const bucket = activeBucketRef.current;
      if (bucket) runAction(bucket);
    };

    const cleanup = () => {
      activeBucketRef.current = null;
      setActiveBucket(null);
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
  const timeoutBucket = TIMEOUT_TILE;
  const banBucket = BAN_TILE;
  const deleteBucket = DELETE_TILE;
  const unbanBucket = liftTile(moderated === 'timeout');
  const pinBucket = draggedPinned ? UNPIN_TILE : PIN_TILE;
  // Mod-only PUNITIVE buckets (profile/whisper removed). The triangle puts the
  // primary action at the apex and Timeout (left) + Delete (right) at the base:
  //   normal    -> Timeout (left), Ban (apex), Delete (right)
  //   timed out -> Timeout (left), Untimeout (apex), Delete (right)
  //   banned    -> Unban (+ Delete) in a single row
  // So an accidental ban/timeout is always reversible via the apex Unban/Untimeout.
  // Order matters: the triangle reads index 0 = base-left, 1 = apex, 2 = base-right.
  // Yourself, the owner, another mod: no ban or timeout can land, so only
  // Delete (and Pin) are offered.
  const canPunish = dragged.canPunish !== false;
  const core: Bucket[] = !canPunish
    ? []
    : moderated === 'ban'
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
  // Beside chat: the same tiles in one column, mild to harsh, so Ban is the
  // farthest from the pickup (which meets Timeout) and sits past a wider gap:
  //   normal    -> Pin?, Delete?, Timeout, Ban
  //   timed out -> Pin?, Untimeout, Delete?, Timeout
  //   banned    -> Pin?, Unban, Delete?
  const columnBuckets: Bucket[] = dragged.isModerator
    ? [
        ...(showDragPin ? [pinBucket] : []),
        ...(moderated && canPunish ? [unbanBucket] : []),
        ...(dragged.messageId ? [deleteBucket] : []),
        ...(moderated === 'ban' || !canPunish ? [] : [timeoutBucket]),
        ...(moderated || !canPunish ? [] : [banBucket]),
      ]
    : [];
  // Triangle is the ABOVE-CHAT layout only: 3 = apex (Ban / Untimeout) on top,
  // Timeout (left) + Delete (right) as the base; fewer just sit in one centered
  // row. Pin ends the bottom row, nearest the message, so reaching it never
  // crosses Ban and the apex stays the farthest tile. Beside chat stacks every
  // bucket vertically instead (see render).
  const triTop = punitive.length === 3 ? [punitive[1]] : punitive;
  const triBottom = [...(punitive.length === 3 ? [punitive[0], punitive[2]] : []), ...(showDragPin ? [pinBucket] : [])];
  // Wider spacing in the above-chat layout so it's easier to land on, and
  // move off, a tile; the beside-chat column uses a roomier gap to match its
  // bigger tiles.
  const gapCls = dragLayout === 'bar' ? 'gap-4' : 'gap-2.5';

  // `armed` (a real action is selected) drives the chip's "snapped in" feedback.
  const armed = !!activeBucket;
  // The Timeout tile names its length while you are on it.
  const activeDuration = activeBucket === 'timeout' ? bucketTimeout : null;

  return createPortal(
    <div className="fixed inset-0 z-[9998] pointer-events-none overflow-hidden">
      {/* Chip held by the cursor. z-50 keeps it ABOVE the bucket column so the
          dragged name floats over the drop target instead of clipping under it.
          The static wrapper carries the up-left offset so it doesn't fight
          framer-motion's animated transform on the chip. */}
      <motion.div className="fixed top-0 left-0 z-50 pointer-events-none" style={{ x, y }}>
        {/* Anchored by its left side so the name never shifts when the hint
            beside it comes and goes. */}
        <div style={{ transform: 'translate(-24px, calc(-100% - 16px))' }}>
          <motion.div
            initial={{ scale: 0.6, opacity: 0 }}
            // When an action is armed the chip shrinks + straightens: a little
            // "snapped in" cue that pairs with the bucket pop.
            animate={{ scale: armed ? 0.86 : 1, opacity: 1, rotate: armed ? 0 : -4 }}
            transition={{ type: 'spring', stiffness: 500, damping: 26 }}
            className="flex items-center gap-1.5 px-3 py-2 rounded-xl border border-white/15 backdrop-blur-md shadow-[0_12px_32px_rgba(0,0,0,0.6)] whitespace-nowrap"
            // Themed (was hardcoded zinc-900/90). Scaled from its left edge so
            // the armed shrink does not move the name either.
            style={{
              originX: 0,
              backgroundColor: 'color-mix(in srgb, var(--color-background-tertiary) calc(90% + (1 - var(--glass-strength, 1)) * 10%), transparent)',
            }}
          >
            <span className="text-sm font-bold" style={nameStyle}>
              {dragged.displayName}
            </span>
            {/* Off every tile, letting go does nothing; say so. */}
            <AnimatePresence initial={false}>
              {!armed && buckets.length > 0 && (
                <motion.span
                  key="cancel-hint"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.12 }}
                  className="text-[11px] font-medium text-textSecondary"
                >
                  Let go to cancel
                </motion.span>
              )}
            </AnimatePresence>
          </motion.div>
        </div>
      </motion.div>

      {/* Floating action dock (no panel/blur). Beside chat: one vertical stack of
          bigger, translucent tiles, clamped to sit above the player controls.
          Above chat: a compact triangle of opaque tiles where room is tight. */}
      {buckets.length > 0 && (
      <div className="fixed pointer-events-none" style={(dragged ? anchorStyle : null) ?? undefined}>
        <IconContext.Provider value={TILE_ICONS}>
        <motion.div
          ref={columnRef}
          initial={{ scale: 0.92, opacity: 0 }}
          animate={{ scale: 1, opacity: 1 }}
          transition={{ type: 'spring', stiffness: 400, damping: 30 }}
          // Positioned, so a tile's offsetTop (the anchor above) is measured
          // from the column.
          className={`pointer-events-auto relative flex flex-col items-center ${gapCls}`}
        >
          {dragLayout === 'column' ? (
            // Beside chat: every action in one vertical column. Bigger + a touch
            // translucent so they're easy to hit when dragging out to the side.
            columnBuckets.map((b) => (
              <div key={b.id} style={b.id === 'ban' ? { marginTop: BAN_GAP_PX } : undefined}>
                <BucketTile bucket={b} active={activeBucket === b.id} activeDuration={activeDuration} solid={false} big />
              </div>
            ))
          ) : (
            <>
              {triTop.length > 0 && (
                <div className={`flex ${gapCls}`}>
                  {triTop.map((b) => (
                    <BucketTile key={b.id} bucket={b} active={activeBucket === b.id} activeDuration={activeDuration} solid />
                  ))}
                </div>
              )}
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
        </IconContext.Provider>
      </div>
      )}
    </div>,
    document.body,
  );
}
