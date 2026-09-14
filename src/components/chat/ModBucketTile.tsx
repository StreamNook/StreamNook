// The drop-target tile for drag-to-moderate, shared by the desktop drag layer
// and the mobile fan-out so both render the identical thing.
//
// Lifted out of ModerationDragLayer unchanged. The mobile fan originally drew
// its own plain circles and looked visibly worse than the desktop client for no
// reason other than that the tile was a local component nobody could import.
import { motion } from 'framer-motion';
import type { LucideIcon } from 'lucide-react';
import { formatDuration } from '../../utils/timeoutRamp';

export type BucketKind = 'neutral' | 'danger';

export interface Bucket {
  id: string;
  label: string;
  icon: LucideIcon;
  kind: BucketKind;
  /** Tailwind classes applied when this bucket is the active drop target. */
  activeTint: string;
}

// Fully-opaque per-action fills for the above-chat layout, where translucent
// tiles over busy chat are hard to focus on.
export const SOLID_TINT: Record<string, string> = {
  delete: 'bg-orange-600 border-orange-400 text-white',
  timeout: 'bg-amber-600 border-amber-300 text-white',
  ban: 'bg-red-600 border-red-400 text-white',
  unban: 'bg-emerald-600 border-emerald-400 text-white',
  pin: 'bg-sky-600 border-sky-400 text-white',
  unpin: 'bg-sky-600 border-sky-400 text-white',
  reply: 'bg-violet-600 border-violet-400 text-white',
  copy: 'bg-zinc-600 border-zinc-300 text-white',
  profile: 'bg-indigo-600 border-indigo-400 text-white',
};

export function BucketTile({
  bucket,
  active,
  activeDuration,
  solid,
  big = false,
}: {
  bucket: Bucket;
  active: boolean;
  activeDuration: number | null;
  solid: boolean;
  /** Beside-chat tiles render larger (easier to hit when dragging to the side). */
  big?: boolean;
}) {
  const Icon = bucket.icon;
  const isDanger = bucket.kind === 'danger';
  // Each tile floats on its own (no panel). The above-chat ('solid') layout uses
  // fully opaque fills so they're easy to focus on over busy chat; beside-chat
  // keeps a lighter translucent tint that lets the stream show through a little.
  const tint = solid
    ? active
      ? SOLID_TINT[bucket.id] ?? 'bg-zinc-900 border-white/25 text-white'
      : 'bg-zinc-900 border-white/15 text-white/80'
    : active
      ? bucket.activeTint
      : 'bg-zinc-900/70 border-white/10 text-white/75';
  return (
    <motion.div
      data-bucket-id={bucket.id}
      // Scale only (no y-shift) so the tile's center stays put: the magnetic
      // nearest-tile selection keys off these centers and must not drift.
      animate={{ scale: active ? 1.18 : 1 }}
      transition={{ type: 'spring', stiffness: 500, damping: 17 }}
      className={`relative flex ${big ? 'h-20 w-20' : 'h-16 w-16'} flex-col items-center justify-center gap-1 rounded-2xl border shadow-[0_8px_22px_rgba(0,0,0,0.5)] transition-colors ${tint}`}
    >
      <motion.div
        animate={active && isDanger ? { rotate: [-10, 6, 0] } : { rotate: 0 }}
        transition={{ duration: 0.3 }}
      >
        <Icon size={big ? 24 : 20} strokeWidth={2} />
      </motion.div>
      <span
        className={`px-0.5 text-center ${big ? 'text-xs' : 'text-[11px]'} font-semibold leading-tight`}
      >
        {bucket.id === 'timeout' && active && activeDuration != null
          ? formatDuration(activeDuration)
          : bucket.label}
      </span>
    </motion.div>
  );
}
