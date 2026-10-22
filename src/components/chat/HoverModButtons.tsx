import { useState } from 'react';
import type React from 'react';
import { CaretDown, Clock, Prohibit, Trash, X } from 'phosphor-react';

import { armedFor, moderateWithToast, useModTimeoutSteps, type ModOutcome, type ModRequest } from '../../services/moderation';
import { useAppStore } from '../../stores/AppStore';
import { Tooltip } from '../ui/Tooltip';
import { ReasonPicker } from './ReasonPicker';
import { pickerReasons } from './reasonPick';

/** The lengths the arrow beside the quick timeout opens, as Rust says the
 *  platform applies them. */
function DockTimeoutPresets({
  provider,
  onPick,
}: {
  provider: string;
  onPick: (seconds: number, label: string, e: React.MouseEvent<HTMLElement>) => void;
}) {
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
              if (step.seconds != null) onPick(step.seconds, step.label, e);
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
  /** A ban (`seconds` null) or a timeout; `withReason` is a shift-click.
   *  The outcome, when there is one, lets Ban show it is waiting for a
   *  second press (Ban confirmation). */
  onBan: (seconds: number | null, withReason: boolean) => void | Promise<ModOutcome | null>;
  onDelete: () => void;
  /** Who a ban or timeout lands on. With it, an Alt-click on Ban or a timeout
   *  opens the reason picker and the pick runs here; without it (the settings
   *  preview) an Alt-click is a plain click. */
  target?: Omit<ModRequest, 'action'> | null;
}

/** The ban or timeout a reason is being picked for, and the dock it hangs from. */
interface Picking {
  anchor: HTMLElement;
  seconds: number | null;
  label: string | null;
}

/**
 * The moderator part of a hovered message's corner cluster: Ban, Delete and
 * a quick 10 minute timeout with the other lengths one click away, opening in
 * place. Shared by the chat row and the settings preview, so the preview is
 * the real thing; what a click does is the caller's, except the reason picker
 * an Alt-click opens.
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
  target,
}: HoverModButtonsProps) {
  const [picking, setPicking] = useState<Picking | null>(null);
  // Ban waiting for its second press (Ban confirmation on).
  const [banArmed, setBanArmed] = useState(false);
  const altHint = target ? ' Alt-click to pick a reason.' : '';

  // A plain click acts (Shift hands it to the composer, the caller's part);
  // Alt opens the picker under the dock, and Alt on the same button again
  // puts it away.
  const press = (e: React.MouseEvent<HTMLElement>, seconds: number | null, label: string | null) => {
    e.preventDefault();
    if (e.altKey && target) {
      const anchor = e.currentTarget.closest<HTMLElement>('[data-row-dock]') ?? e.currentTarget.parentElement ?? e.currentTarget;
      setPicking(picking?.seconds === seconds ? null : { anchor, seconds, label });
      return;
    }
    setPicking(null);
    const done = onBan(seconds, e.shiftKey);
    if (seconds == null && done) {
      void done.then((o) => {
        if (!o?.confirmMs) {
          setBanArmed(false);
          return;
        }
        setBanArmed(true);
        window.setTimeout(() => setBanArmed(false), armedFor(o));
      });
    }
  };

  const pick = (reason: string | null) => {
    const p = picking;
    setPicking(null);
    if (!p || !target) return;
    void moderateWithToast({
      ...target,
      action: p.seconds == null ? { kind: 'ban' } : { kind: 'timeout', seconds: p.seconds },
      reason,
      useSavedReason: false,
    });
  };

  const picker = picking && (
    <ReasonPicker
      anchor={picking.anchor}
      title={picking.seconds == null ? `Ban ${targetName}` : `Time out ${targetName} for ${picking.label}`}
      reasons={pickerReasons(useAppStore.getState().settings.moderation?.saved_ban_reasons)}
      onPick={pick}
      onClose={() => setPicking(null)}
    />
  );

  if (lengthsOpen) {
    return (
      <>
        <DockTimeoutPresets
          provider={provider}
          onPick={(seconds, label, e) => {
            onLengthsOpen(false);
            press(e, seconds, label);
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
        {picker}
      </>
    );
  }
  return (
    <>
      {/* Ban and timeouts only where they can land; Delete works on any
          message you moderate. */}
      {canPunish && (
        <Tooltip
          // Keyed by the armed state, so a tooltip already up swaps its words.
          key={banArmed ? 'ban-armed' : 'ban'}
          content={banArmed ? `Press again to ban ${targetName}` : `Ban ${targetName}. Shift-click to add a reason.${altHint}`}
          side="top"
          disabled={picking != null}
        >
          <button
            aria-label={banArmed ? `Press again to ban ${targetName}` : `Ban ${targetName}`}
            onClick={(e) => press(e, null, null)}
            className={`p-1 rounded-md transition-colors ${
              banArmed ? 'bg-error text-white' : 'text-white/50 hover:text-error hover:bg-error/15'
            }`}
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
              setPicking(null);
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
          <Tooltip
            content={`Time out ${targetName} for 10m. Shift-click to add a reason.${altHint}`}
            side="top"
            disabled={picking != null}
          >
            <button
              aria-label={`Time out ${targetName} for 10m`}
              onClick={(e) => press(e, 600, '10m')}
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
                setPicking(null);
                onLengthsOpen(true);
              }}
              className="flex h-[22px] w-4 items-center justify-center rounded-md text-white/50 hover:text-warning hover:bg-warning/20 transition-colors"
            >
              <CaretDown size={10} weight="bold" />
            </button>
          </Tooltip>
        </>
      )}
      {picker}
    </>
  );
}
