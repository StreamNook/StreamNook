import type { ModAction, ModRequest, ModStep } from '../../services/moderation';
import { stepTint } from './modStepTint';

// Line buttons: one-click mod actions at the start of every line a moderator
// can act on. They sit on EVERY row, so a row carries a single empty element
// (the strip) and nothing else: the list turns Rust's steps
// (`get_mod_slider_steps` with the line list) into `LineButton`s once, draws
// them once into a picture (`lineArt.ts`) that every strip shows, and reads a
// hover or click back from where the pointer is inside the strip. Measured:
// three real buttons per row halved the frame rate of a 400 msg/s chat,
// because every flush restyles and repaints every row; one box per row costs
// what no buttons cost.

/** Marks the strip; its data attributes name the chatter and channel. */
export const LINE_STRIP_ATTR = 'data-mod-line';

export interface LineButton {
  /** `d` (delete), `b` (ban) or `t<seconds>` (a timeout as the platform applies it). */
  key: string;
  kind: ModStep['action'];
  /** Rust's label: "10m", "Delete", "Ban". */
  label: string;
  /** What the settings preview names, and the strip's accessible summary. */
  name: string;
  /** The step's color, shared with the slider's track. */
  tint: string;
}

function lineKey(step: ModStep): string {
  if (step.action === 'delete') return 'd';
  if (step.action === 'ban') return 'b';
  return `t${step.seconds ?? 0}`;
}

function lineName(step: ModStep): string {
  if (step.action === 'delete') return 'Delete message';
  if (step.action === 'ban') return 'Ban';
  return `Time out ${step.label}`;
}

/** The buttons for Rust's steps, mildest first. */
export function lineButtons(steps: ModStep[]): LineButton[] {
  return steps.map((step, i) => ({
    key: lineKey(step),
    kind: step.action,
    label: step.label,
    name: lineName(step),
    tint: stepTint(steps, i),
  }));
}

/** The action a button key stands for; Delete needs the message. */
export function lineAction(key: string, messageId: string | undefined): ModAction | null {
  if (key === 'd') return messageId ? { kind: 'delete', messageId } : null;
  if (key === 'b') return { kind: 'ban' };
  if (key.startsWith('t')) {
    const seconds = Number(key.slice(1));
    return Number.isInteger(seconds) && seconds > 0 ? { kind: 'timeout', seconds } : null;
  }
  return null;
}

/** The strip's data attributes for a chatter (`data-p`, `data-b`, ...). */
export function lineTargetAttrs(target: Omit<ModRequest, 'action'>, messageId: string | undefined) {
  return {
    [LINE_STRIP_ATTR]: '',
    'data-p': target.provider ?? undefined,
    'data-b': target.broadcasterId ?? undefined,
    'data-c': target.channel ?? undefined,
    'data-u': target.targetUserId ?? undefined,
    'data-n': target.targetName ?? undefined,
    'data-m': messageId,
  };
}

// ---- Geometry --------------------------------------------------------------

/** Proportions, in ems of the chat text: the old CSS buttons' sizes. */
export const LINE_GEOMETRY = {
  height: 1.2,
  /** Delete and Ban are square-ish icon buttons. */
  iconWidth: 1.3,
  icon: 0.8,
  /** A timeout is its label plus this on each side, at least iconWidth. */
  pad: 0.32,
  label: 0.7,
  radius: 0.3,
  /** Between two buttons, in px (as the CSS had it). */
  gapPx: 2,
} as const;

export interface LineLayout {
  fontPx: number;
  /** Each button's left edge and width in px, in order. */
  boxes: { x: number; w: number }[];
  width: number;
  height: number;
}

/** Where each button sits for `fontPx` text; `measure` gives a label's width. */
export function lineLayout(buttons: LineButton[], fontPx: number, measure: (label: string) => number): LineLayout {
  const g = LINE_GEOMETRY;
  const boxes: { x: number; w: number }[] = [];
  let x = 0;
  for (const b of buttons) {
    const w =
      b.kind === 'timeout'
        ? Math.max(g.iconWidth * fontPx, measure(b.label) + 2 * g.pad * fontPx)
        : g.iconWidth * fontPx;
    const rounded = Math.round(w);
    boxes.push({ x, w: rounded });
    x += rounded + g.gapPx;
  }
  return { fontPx, boxes, width: Math.max(0, x - g.gapPx), height: Math.round(g.height * fontPx) };
}

/** The button under `x` px from the strip's left edge, or -1 between them. */
export function lineHit(layout: LineLayout, x: number): number {
  return layout.boxes.findIndex((b) => x >= b.x && x < b.x + b.w);
}

/** The request for button `index` of a strip whose data attributes are `d`. */
export function lineRequest(
  buttons: LineButton[],
  index: number,
  d: DOMStringMap,
): { request: ModRequest; name: string; label: string } | null {
  const button = buttons[index];
  if (!button) return null;
  const action = lineAction(button.key, d.m);
  if (!action) return null;
  return {
    label: button.label,
    request: {
      provider: d.p,
      broadcasterId: d.b,
      channel: d.c,
      targetUserId: d.u,
      targetName: d.n,
      action,
      useSavedReason: action.kind !== 'delete',
    },
    name: button.name,
  };
}
