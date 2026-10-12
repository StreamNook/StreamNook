// The channel's chat modes, drawn inside the message box at its trailing edge.
//
// Rust words the room (services/room_modes.rs) and ships it on the ROOMSTATE
// frame; this only draws it. The field's placeholder carries the words
// ("Emote-only chat"), these icons stay while you type, and hovering them
// lists every mode with what it does and whether you can still chat. The
// field reserves the icons' width through `--sn-room-pad` (globals.css), so a
// mode switching on or off never moves anything outside the box.
import React from 'react';
import { Fingerprint, Heart, Smile, Star, Timer, type LucideIcon } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import type { RoomModeKind, RoomModeView } from '../../stores/chatConnectionStore';
import { COUNT_PX, EDGE_PX, GAP_PX, ICON_PX } from './roomModeLayout';

const ICONS: Record<RoomModeKind, LucideIcon> = {
  subs_only: Star,
  emote_only: Smile,
  followers_only: Heart,
  slow: Timer,
  unique: Fingerprint,
};

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
   *  the field's padding; it keeps the first icon and a "+N" count. */
  inline?: boolean;
}> = ({ view, onActivate, inline = false }) => {
  if (view.modes.length === 0) return null;
  // Every mode spares you (a moderator, say): the icons step back a shade.
  const exempt = view.modes.every((m) => m.applies_to_you === false);
  const words = view.modes.map((m) => m.label).join(', ');
  const extra = view.modes.length - 1;
  return (
    <Tooltip content={<RoomModeDetails view={view} />} side="top">
      <span
        role={onActivate ? 'button' : 'img'}
        tabIndex={onActivate ? 0 : undefined}
        aria-label={view.standing ? `${words}. ${view.standing}` : words}
        onClick={onActivate}
        // Clicking the icons never takes the caret out of the field.
        onMouseDown={(e) => e.preventDefault()}
        className={`sn-room-modes flex items-center ${
          inline ? 'h-9 flex-shrink-0 self-end px-1' : 'absolute top-1/2 z-10 -translate-y-1/2'
        } ${exempt ? 'text-textMuted' : 'text-textSecondary'}`}
        style={inline ? { gap: GAP_PX } : { right: EDGE_PX, gap: GAP_PX }}
      >
        {(inline ? view.modes.slice(0, 1) : view.modes).map((mode, i) => {
          const Icon = ICONS[mode.kind];
          return (
            <Icon
              key={mode.kind}
              size={ICON_PX}
              strokeWidth={2.2}
              aria-hidden
              className="flex-shrink-0"
              data-room-mode-rest={i > 0 ? '' : undefined}
            />
          );
        })}
        {extra > 0 && (
          <span
            data-room-mode-count={inline ? undefined : ''}
            className="text-[10px] font-semibold leading-none tabular-nums"
            style={{ width: COUNT_PX }}
          >
            +{extra}
          </span>
        )}
      </span>
    </Tooltip>
  );
};
