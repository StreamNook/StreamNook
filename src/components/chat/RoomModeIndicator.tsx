// The channel's chat modes, as one chip at the message box's trailing edge.
//
// Rust words the room (services/room_modes.rs) and ships it on the ROOMSTATE
// frame; this only draws it. The chip names one mode ("Slow 30s") with a "+N"
// for the rest, and hovering it lists every mode with what it does and whether
// you can still chat. The box's own prompt never changes, so a mode never
// reads as "you cannot type". The field reserves the chip's width through
// `--sn-room-pad`, measured here, so the typed text never runs under it and a
// mode switching on or off moves nothing outside the box.
import React, { useLayoutEffect, useRef } from 'react';
import { Fingerprint, Heart, Smile, Star, Timer, type LucideIcon } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import type { RoomModeKind, RoomModeView } from '../../stores/chatConnectionStore';

const ICONS: Record<RoomModeKind, LucideIcon> = {
  subs_only: Star,
  emote_only: Smile,
  followers_only: Heart,
  slow: Timer,
  unique: Fingerprint,
};

/** The chip's distance from the field's right edge. */
export const EDGE_PX = 6;
/** Clear space between the typed text and the chip. */
export const CLEAR_PX = 6;

/** Every mode with its sentence, and the "you can chat" line when it holds. */
export const RoomModeDetails: React.FC<{ view: RoomModeView }> = ({ view }) => (
  <div className="flex w-[228px] flex-col gap-2 py-0.5 text-left">
    {view.modes.map((mode) => {
      const Icon = ICONS[mode.kind];
      return (
        <div key={mode.kind} className="flex items-start gap-2">
          <Icon size={13} className="mt-[2px] flex-shrink-0 text-textSecondary" aria-hidden />
          <div className="min-w-0">
            <div className="text-[12px] font-semibold leading-4 text-textPrimary">{mode.label}</div>
            <div className="text-[11px] leading-[15px] text-textSecondary">{mode.detail}</div>
          </div>
        </div>
      );
    })}
    {view.standing && (
      <div className="flex items-center gap-2 text-[11px] leading-[15px] text-textPrimary">
        <span className="flex w-[13px] flex-shrink-0 justify-center" aria-hidden>
          <span className="h-1.5 w-1.5 rounded-full bg-success" />
        </span>
        {view.standing}
      </div>
    )}
  </div>
);

export const RoomModeIndicator: React.FC<{
  view: RoomModeView;
  /** Touch has no hover: a tap opens wherever the surface lists the modes. */
  onActivate?: () => void;
  /** A flex item in the field's own row (the phone) instead of an inset over
   *  the field's padding. */
  inline?: boolean;
}> = ({ view, onActivate, inline = false }) => {
  const ref = useRef<HTMLSpanElement>(null);
  const shown = view.modes.length > 0 && !!view.chip;

  // Hand the field the chip's width. Layout only: the words are Rust's.
  useLayoutEffect(() => {
    const chip = ref.current;
    const field = chip?.parentElement;
    if (inline || !chip || !field) return;
    const reserve = () => field.style.setProperty('--sn-room-pad', `${chip.offsetWidth + EDGE_PX + CLEAR_PX}px`);
    reserve();
    const observer = new ResizeObserver(reserve);
    observer.observe(chip);
    return () => {
      observer.disconnect();
      field.style.removeProperty('--sn-room-pad');
    };
  }, [inline, shown]);

  if (!shown) return null;
  const words = view.modes.map((m) => m.label).join(', ');
  const extra = view.modes.length - 1;
  return (
    <Tooltip content={<RoomModeDetails view={view} />} side="top">
      <span
        ref={ref}
        role={onActivate ? 'button' : 'img'}
        tabIndex={onActivate ? 0 : undefined}
        aria-label={view.standing ? `${words}. ${view.standing}` : words}
        onClick={onActivate}
        // Clicking the chip never takes the caret out of the field.
        onMouseDown={(e) => e.preventDefault()}
        className={`sn-room-chip ${inline ? 'flex-shrink-0 self-center' : 'absolute top-1/2 z-10 -translate-y-1/2'} ${
          view.exempt ? 'text-textMuted' : 'text-textSecondary'
        }`}
        style={inline ? undefined : { right: EDGE_PX }}
      >
        {view.chip}
        {extra > 0 && <span className="sn-room-chip-more">+{extra}</span>}
      </span>
    </Tooltip>
  );
};
