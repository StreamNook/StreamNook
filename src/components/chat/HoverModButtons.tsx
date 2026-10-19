import { CaretDown, Clock, Prohibit, Trash, X } from 'phosphor-react';

import { useModTimeoutSteps } from '../../services/moderation';
import { Tooltip } from '../ui/Tooltip';

/** The lengths the arrow beside the quick timeout opens, as Rust says the
 *  platform applies them. */
function DockTimeoutPresets({ provider, onPick }: { provider: string; onPick: (seconds: number, withReason: boolean) => void }) {
  const steps = useModTimeoutSteps(provider, true);
  return (
    <>
      {steps?.map((step) =>
        step.action !== 'timeout' || step.seconds == null ? null : (
          <button
            key={step.seconds}
            aria-label={`Time out for ${step.label}`}
            className="h-[22px] rounded-md px-1.5 text-[11px] font-semibold leading-none text-white/70 hover:text-warning hover:bg-warning/20 transition-colors"
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
              if (step.seconds != null) onPick(step.seconds, e.shiftKey);
            }}
          >
            {step.label}
          </button>
        ),
      )}
    </>
  );
}

interface HoverModButtonsProps {
  provider: string;
  targetName: string;
  /** Ban and timeouts land on this chatter (not you, not the owner, not
   *  another mod unless it is your channel). */
  canPunish: boolean;
  /** The message can be deleted (it has an id). */
  canDelete: boolean;
  /** The other lengths are open in place of the buttons. */
  lengthsOpen: boolean;
  onLengthsOpen: (open: boolean) => void;
  /** A ban (`seconds` null) or a timeout; `withReason` is a shift-click. */
  onBan: (seconds: number | null, withReason: boolean) => void;
  onDelete: () => void;
}

/**
 * The moderator part of a hovered message's corner cluster: Ban, Delete and
 * a quick 10 minute timeout with the other lengths one click away, opening in
 * place. Shared by the chat row and the settings preview, so the preview is
 * the real thing; what a click does is the caller's.
 */
export function HoverModButtons({
  provider,
  targetName,
  canPunish,
  canDelete,
  lengthsOpen,
  onLengthsOpen,
  onBan,
  onDelete,
}: HoverModButtonsProps) {
  if (lengthsOpen) {
    return (
      <>
        <DockTimeoutPresets
          provider={provider}
          onPick={(seconds, withReason) => {
            onLengthsOpen(false);
            onBan(seconds, withReason);
          }}
        />
        <Tooltip content="Close lengths" side="top">
          <button
            aria-label="Close lengths"
            onClick={(e) => {
              e.preventDefault();
              onLengthsOpen(false);
            }}
            className="flex h-[22px] w-4 items-center justify-center rounded-md text-white/50 hover:text-white hover:bg-surface-hover transition-colors"
          >
            <X size={10} weight="bold" />
          </button>
        </Tooltip>
      </>
    );
  }
  return (
    <>
      {/* Ban and timeouts only where they can land; Delete works on any
          message you moderate. */}
      {canPunish && (
        <Tooltip content={`Ban ${targetName}. Shift-click to add a reason.`} side="top">
          <button
            aria-label={`Ban ${targetName}`}
            onClick={(e) => {
              e.preventDefault();
              onBan(null, e.shiftKey);
            }}
            className="p-1 rounded-md text-white/50 hover:text-error hover:bg-error/15 transition-colors"
          >
            <Prohibit size={14} weight="bold" />
          </button>
        </Tooltip>
      )}
      {canDelete && (
        <Tooltip content="Delete this message" side="top">
          <button
            aria-label="Delete this message"
            onClick={(e) => {
              e.preventDefault();
              onDelete();
            }}
            className="p-1 rounded-md text-white/50 hover:text-error hover:bg-error/15 transition-colors"
          >
            <Trash size={14} weight="bold" />
          </button>
        </Tooltip>
      )}
      {canPunish && (
        <>
          <Tooltip content={`Time out ${targetName} for 10m. Shift-click to add a reason.`} side="top">
            <button
              aria-label={`Time out ${targetName} for 10m`}
              onClick={(e) => {
                e.preventDefault();
                onBan(600, e.shiftKey);
              }}
              className="flex h-[22px] items-center gap-1 rounded-md px-1 text-[11px] font-semibold leading-none text-white/50 hover:text-warning hover:bg-warning/20 transition-colors"
            >
              <Clock size={14} weight="bold" />
              10m
            </button>
          </Tooltip>
          <Tooltip content="Other lengths" side="top">
            <button
              aria-label="Other timeout lengths"
              onClick={(e) => {
                e.preventDefault();
                onLengthsOpen(true);
              }}
              className="flex h-[22px] w-4 items-center justify-center rounded-md text-white/50 hover:text-warning hover:bg-warning/20 transition-colors"
            >
              <CaretDown size={10} weight="bold" />
            </button>
          </Tooltip>
        </>
      )}
    </>
  );
}
