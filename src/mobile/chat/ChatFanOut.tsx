// Hold-and-drag fan-out for a chat message.
//
// Interaction: long-press a message to arm (haptic thump), keep holding and drag
// to a bucket, release to commit. Releasing away from every bucket cancels.
//
// Two properties are load-bearing and both come from the desktop drag layer,
// which was already better suited to touch than to a mouse:
//
//  - MAGNETIC selection. The nearest bucket within an engage radius wins, rather
//    than requiring the finger to be inside a tile. Nobody is pixel-accurate
//    with a thumb.
//  - CONTINUOUS timeout. Once Timeout is engaged, dragging further out dials the
//    duration on the shared ramp (utils/timeoutRamp), so one gesture covers 5
//    seconds through 14 days with no fixed chip list to run off-screen.
//
// Layout follows one rule: DISTANCE IS SEVERITY. Everyday actions sit on a near
// arc, heavier ones on a far arc, so Ban takes a deliberate longer reach than
// Delete. That is deliberately how the destructive actions are protected,
// instead of a confirmation dialog nobody reads.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { motion } from 'framer-motion';
import { Ban, Clock, Copy, Pin, Reply, Trash2, User } from 'lucide-react';
import { BucketTile, type Bucket } from '../../components/chat/ModBucketTile';
import { useAppStore } from '../../stores/AppStore';
import { useChatUserStore } from '../../stores/chatUserStore';
import { computePaintStyle } from '../../services/seventvService';
import { durationTier, timeoutSecsFromDistance } from '../../utils/timeoutRamp';
import { hapticCommit, hapticDestructive, hapticStep, hapticTick } from '../ui/haptics';
import { useWindowShape } from '../ui/useWindowShape';

export type FanAction = 'reply' | 'copy' | 'profile' | 'delete' | 'timeout' | 'ban' | 'pin';

export interface FanTarget {
  messageId: string;
  username: string;
  userId: string;
  content: string;
  /** Anchor point: where the finger went down. */
  originX: number;
  originY: number;
}

/** A fan bucket is a shared `Bucket` plus where on the fan it sits. */
type FanBucket = Bucket & {
  id: FanAction;
  /** 0 = near arc, 1 = far arc. */
  ring: 0 | 1;
};

// Near arc = what you reach for constantly. Far arc = heavier, longer reach.
// `activeTint` is only used by the translucent beside-chat layout; the fan
// renders solid, which keys off the shared SOLID_TINT map by id.
const EVERYONE: FanBucket[] = [
  { id: 'reply', label: 'Reply', icon: Reply, kind: 'neutral', activeTint: '', ring: 0 },
  { id: 'profile', label: 'Profile', icon: User, kind: 'neutral', activeTint: '', ring: 0 },
  { id: 'copy', label: 'Copy', icon: Copy, kind: 'neutral', activeTint: '', ring: 1 },
];

const MOD_ONLY: FanBucket[] = [
  { id: 'delete', label: 'Delete', icon: Trash2, kind: 'danger', activeTint: '', ring: 0 },
  { id: 'timeout', label: 'Timeout', icon: Clock, kind: 'danger', activeTint: '', ring: 1 },
  { id: 'ban', label: 'Ban', icon: Ban, kind: 'danger', activeTint: '', ring: 1 },
  { id: 'pin', label: 'Pin', icon: Pin, kind: 'neutral', activeTint: '', ring: 1 },
];

// Matches the shared tile's own `h-16 w-16`.
const TILE = 64;
// Sized against a ~360px-wide phone viewport: the far arc's full span has to fit
// on screen or the clamp piles tiles on top of each other. 150 x 160deg spans
// ~295px, leaving 4 far tiles ~98px apart centre to centre.
const NEAR_R = 92;
const FAR_R = 150;
const NEAR_SPREAD = 120;
const FAR_SPREAD = 160;
// Vertical room reserved above the far arc for the name chip, so it never lands
// on a tile and the fan is pushed down rather than clipped near the top.
const CHIP_CLEARANCE = 60;
// Generous: a thumb is not precise, and the tiles are far enough apart that a
// wide radius still resolves unambiguously to one of them.
const ENGAGE_R = 60;

interface Props {
  target: FanTarget | null;
  isModerator: boolean;
  /** Pin needs mod powers AND a resolvable message. */
  canPin: boolean;
  onCommit: (action: FanAction, timeoutSecs?: number) => void;
  onCancel: () => void;
}

export const ChatFanOut: React.FC<Props> = ({
  target,
  isModerator,
  canPin,
  onCommit,
  onCancel,
}) => {
  const { fold } = useWindowShape();
  const paintShadowMode = useAppStore((s) => s.settings.cosmetics?.paint_shadows) ?? 'all';
  const paint = useChatUserStore((s) =>
    target ? s.users.get(target.userId)?.paint : undefined,
  );
  const userColor = useChatUserStore((s) =>
    target ? s.users.get(target.userId)?.color : undefined,
  );

  const buckets = useMemo(() => {
    const list = [...EVERYONE, ...(isModerator ? MOD_ONLY : [])];
    return canPin ? list : list.filter((b) => b.id !== 'pin');
  }, [isModerator, canPin]);

  // Tile centres, fanned upward from a fixed origin.
  //
  // The origin is the VIEWPORT CENTRE horizontally, not the press point. Same
  // call the desktop bar layout makes (it centres over the chat panel): pressing
  // near an edge would otherwise clamp half the arc into a pile. Vertically it
  // sits above the pressed row, and is pushed down if the far arc would run off
  // the top of the screen.
  const layout = useMemo(() => {
    if (!target) return null;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    // Centre of the SEGMENT the chat lives in, not of the whole window. On an
    // unfolded Z Fold the hinge runs vertically down the middle, so the window
    // centre is the one x-position guaranteed to put tiles on the crease. When a
    // vertical fold is present the chat pane is the right-hand segment, so the
    // fan centres there instead.
    const originX =
      fold?.vertical && fold.x > 0 && fold.x < vw
        ? fold.x + fold.width + (vw - (fold.x + fold.width)) / 2
        : vw / 2;
    // Keep the whole fan on screen: the far arc reaches FAR_R above the origin,
    // and the name chip sits above that again.
    const minY = FAR_R + TILE / 2 + CHIP_CLEARANCE;
    const originY = Math.min(vh - 8, Math.max(minY, target.originY));

    const rings: Record<0 | 1, FanBucket[]> = {
      0: buckets.filter((b) => b.ring === 0),
      1: buckets.filter((b) => b.ring === 1),
    };
    const out: { bucket: FanBucket; x: number; y: number }[] = [];
    ([0, 1] as const).forEach((ring) => {
      const items = rings[ring];
      if (items.length === 0) return;
      const radius = ring === 0 ? NEAR_R : FAR_R;
      const spread = ring === 0 ? NEAR_SPREAD : FAR_SPREAD;
      const start = -90 - spread / 2;
      const step = items.length === 1 ? 0 : spread / (items.length - 1);
      items.forEach((bucket, i) => {
        const deg = items.length === 1 ? -90 : start + step * i;
        const rad = (deg * Math.PI) / 180;
        out.push({
          bucket,
          x: originX + Math.cos(rad) * radius,
          y: originY + Math.sin(rad) * radius,
        });
      });
    });
    // The chip goes above the far arc, centred with the fan. Anchoring it to the
    // press point instead put it straight on top of the near-arc tiles whenever
    // the press landed near the middle of the screen.
    return {
      tiles: out,
      chipX: originX,
      chipY: Math.max(28, originY - FAR_R - CHIP_CLEARANCE / 2),
    };
  }, [target, buckets, fold]);

  const placed = layout?.tiles ?? [];

  const [active, setActive] = useState<FanAction | null>(null);
  const [secs, setSecs] = useState<number | null>(null);
  // Refs mirror the live values so the window-level pointer handlers, which are
  // installed once, always read current state without re-binding every move.
  const activeRef = useRef<FanAction | null>(null);
  const secsRef = useRef<number | null>(null);
  const tierRef = useRef<number | null>(null);

  // `placed` is a dependency rather than a ref: it only changes when the target
  // or the bucket set changes, never during a drag, so re-binding on it costs
  // nothing and avoids touching a ref during render.
  useEffect(() => {
    if (!target) {
      activeRef.current = null;
      secsRef.current = null;
      tierRef.current = null;
      return;
    }

    const onMove = (e: PointerEvent) => {
      const tiles = placed;
      let best: FanAction | null = null;
      let bestD = Infinity;
      for (const t of tiles) {
        const d = Math.hypot(e.clientX - t.x, e.clientY - t.y);
        if (d < bestD) {
          bestD = d;
          best = t.bucket.id;
        }
      }
      if (bestD > ENGAGE_R) best = null;

      // Timeout stays engaged once entered, and dragging FURTHER from its tile
      // dials the duration up. Without the sticky behaviour the duration would
      // reset the moment the finger left the engage radius.
      let duration: number | null = null;
      const tTile = tiles.find((t) => t.bucket.id === 'timeout');
      if (tTile) {
        const out = Math.hypot(e.clientX - tTile.x, e.clientY - tTile.y);
        if (best === 'timeout' || (activeRef.current === 'timeout' && out > ENGAGE_R)) {
          best = 'timeout';
          duration = timeoutSecsFromDistance(Math.max(0, out - ENGAGE_R / 2));
        }
      }

      if (best !== activeRef.current) {
        activeRef.current = best;
        setActive(best);
        if (best) hapticTick();
        tierRef.current = null;
      }
      if (duration !== secsRef.current) {
        secsRef.current = duration;
        setSecs(duration);
        if (duration !== null) {
          const tier = durationTier(duration);
          if (tierRef.current !== null && tier !== tierRef.current) hapticStep();
          tierRef.current = tier;
        }
      }
    };

    const onUp = () => {
      const chosen = activeRef.current;
      const duration = secsRef.current;
      if (!chosen) {
        onCancel();
        return;
      }
      const bucket = [...EVERYONE, ...MOD_ONLY].find((b) => b.id === chosen);
      if (bucket?.kind === 'danger') hapticDestructive();
      else hapticCommit();
      onCommit(chosen, chosen === 'timeout' ? (duration ?? 600) : undefined);
    };

    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onCancel);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
    };
  }, [target, placed, onCommit, onCancel]);

  if (!target) return null;

  const nameStyle = paint
    ? computePaintStyle(paint, userColor, paintShadowMode)
    : { color: userColor || 'var(--color-accent)' };

  return createPortal(
    <div className="fixed inset-0 z-[9500] touch-none" style={{ pointerEvents: 'none' }}>
      {/* Dim so the fan reads against a busy chat, and so it is obvious the rest
          of the UI is not what you are interacting with. */}
      <motion.div
        className="absolute inset-0 bg-black/45"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ duration: 0.1 }}
      />

      {/* The chatter you grabbed, in the same chip the desktop drag layer uses:
          their 7TV paint on the name, tilted while nothing is selected and
          snapping straight when an action arms. Sits ABOVE the press point so
          the finger is not covering it. */}
      <motion.div
        className="absolute"
        style={{
          left: layout?.chipX ?? target.originX,
          top: layout?.chipY ?? target.originY,
          transform: 'translate(-50%, -50%)',
        }}
      >
        <motion.div
          initial={{ scale: 0.6, opacity: 0 }}
          animate={{ scale: active ? 0.86 : 1, opacity: 1, rotate: active ? 0 : -4 }}
          transition={{ type: 'spring', stiffness: 500, damping: 26 }}
          className="flex items-center gap-1.5 px-3 py-2 rounded-xl border border-white/15 backdrop-blur-md shadow-[0_12px_32px_rgba(0,0,0,0.6)] whitespace-nowrap"
          style={{
            backgroundColor: 'color-mix(in srgb, var(--color-background-tertiary) 90%, transparent)',
          }}
        >
          <span className="text-sm font-bold" style={nameStyle}>
            {target.username}
          </span>
        </motion.div>
      </motion.div>

      {placed.map(({ bucket, x, y }, i) => (
        <motion.div
          key={bucket.id}
          className="absolute"
          style={{ translateX: '-50%', translateY: '-50%' }}
          initial={{ opacity: 0, left: target.originX, top: target.originY }}
          animate={{ opacity: 1, left: x, top: y }}
          transition={{
            type: 'spring',
            stiffness: 520,
            damping: 34,
            // Stagger outward so the fan reads as opening, not as appearing.
            delay: 0.012 * i,
          }}
        >
          {/* Solid fills: translucent tiles over busy chat are hard to focus on,
              which is the same call the desktop above-chat layout makes. */}
          <BucketTile
            bucket={bucket}
            active={active === bucket.id}
            activeDuration={secs}
            solid
          />
        </motion.div>
      ))}
    </div>,
    document.body,
  );
};
