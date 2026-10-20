// Bridge to Rust `services/moderation.rs`, the one door for chat moderation.
//
// Every surface that bans, times out, lifts or deletes calls `moderateChatter`
// with the chatter and the action; Rust picks the platform API, applies its
// timeout rules (Kick's whole minutes, YouTube's single length, Twitch's 14-day
// cap) and words the result. Nothing here decides any of that. The step cache
// only remembers Rust's answer per provider and list, so hovering a row does
// not ask again.

import { invoke } from '@tauri-apps/api/core';
import { useEffect, useSyncExternalStore } from 'react';

import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';

export type ModAction =
  | { kind: 'timeout'; seconds: number }
  | { kind: 'ban' }
  | { kind: 'unban'; timeout?: boolean }
  /** Twitch only; the request's `reason` is what they see, so one is required. */
  | { kind: 'warn' }
  | { kind: 'delete'; messageId: string };

export interface ModRequest {
  /** 'twitch' (default), 'kick' or 'youtube'. */
  provider?: string | null;
  /** The channel owner's id (Twitch, Kick). */
  broadcasterId?: string | null;
  /** The channel slug (YouTube); a `youtube:` prefix is fine. */
  channel?: string | null;
  targetUserId?: string | null;
  /** For the words only. */
  targetName?: string | null;
  action: ModAction;
  reason?: string | null;
  /** With no reason given, use the first saved ban reason. */
  useSavedReason?: boolean;
  /** The words after the name in a typed `/timeout`; Rust reads the length
   *  and reason from them. */
  typed?: string | null;
}

export interface ModOutcome {
  appliedSeconds: number | null;
  message: string;
  undoable: boolean;
}

export interface ModStep {
  action: 'delete' | 'timeout' | 'ban';
  /** The timeout as the platform applies it; null for Delete and Ban. */
  seconds: number | null;
  label: string;
}

/** Which steps the slider offers; Rust draws them mildest first. */
export interface SliderConfig {
  delete: boolean;
  timeouts: number[];
  ban: boolean;
}

export const DEFAULT_SLIDER_TIMEOUTS = [60, 300, 600, 1800, 3600, 86400, 604800];

/** Fired (bubbling) on a chat row a moderator presses to slide or drag, so
 *  its list holds still; the list lets go itself when the button comes up. */
export const ROW_HOLD_EVENT = 'sn-row-hold';

export function holdChatRow(row: HTMLElement, on: boolean): void {
  row.dispatchEvent(new CustomEvent(ROW_HOLD_EVENT, { bubbles: true, detail: { on } }));
}

/** A timeout length the viewer typed ("45m", "1h30m", "2d", a bare number of
 *  minutes), read by Rust. Rejects with the words Settings shows when it is
 *  not a length Twitch takes (1 s to 14 d). */
export function parseModTimeout(text: string): Promise<ModStep> {
  return invoke<ModStep>('parse_mod_timeout', { text });
}

/** Lengths offered when adding a timeout to the slider, the line buttons or
 *  the hover buttons. The menu's own wording; the chips show Rust's labels. */
export const TIMEOUT_LENGTH_CHOICES: { value: number; label: string }[] = [
  { value: 1, label: '1s' },
  { value: 5, label: '5s' },
  { value: 30, label: '30s' },
  { value: 60, label: '1m' },
  { value: 300, label: '5m' },
  { value: 600, label: '10m' },
  { value: 1800, label: '30m' },
  { value: 3600, label: '1h' },
  { value: 21600, label: '6h' },
  { value: 43200, label: '12h' },
  { value: 86400, label: '1d' },
  { value: 259200, label: '3d' },
  { value: 604800, label: '1w' },
  { value: 1209600, label: '2w' },
];


export function moderateChatter(request: ModRequest): Promise<ModOutcome> {
  return invoke<ModOutcome>('moderate_chatter', { request });
}

/** The request that reverses a ban or timeout. */
export function liftRequest(request: ModRequest): ModRequest {
  return { ...request, action: { kind: 'unban', timeout: request.action.kind === 'timeout' }, reason: null };
}

/**
 * Run an action and say what happened: Rust's words on success (with Undo for
 * a ban or timeout), the platform's reason on failure. Returns the outcome, or
 * null when it failed. A result with Undo shows even with toasts muted, since
 * the toast is the only way back from a slip.
 */
export async function moderateWithToast(
  request: ModRequest,
  opts: { quietSuccess?: boolean; onUndo?: () => void } = {},
): Promise<ModOutcome | null> {
  const { addToast } = useAppStore.getState();
  try {
    const outcome = await moderateChatter(request);
    if (!opts.quietSuccess) {
      const undo = outcome.undoable
        ? {
            label: 'Undo',
            onClick: () => {
              opts.onUndo?.();
              void moderateWithToast(liftRequest(request));
            },
          }
        : undefined;
      addToast(outcome.message, 'success', undo, undo ? { alwaysShow: true } : undefined);
    }
    return outcome;
  } catch (err) {
    Logger.error('[Moderation] action failed:', err);
    addToast(String(err), 'error');
    return null;
  }
}

// ---- Timeout steps ---------------------------------------------------------

const stepsByKey = new Map<string, ModStep[]>();
const inFlight = new Set<string>();
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function requestSteps(key: string, command: string, args: Record<string, unknown>): void {
  if (stepsByKey.has(key) || inFlight.has(key)) return;
  inFlight.add(key);
  invoke<ModStep[]>(command, args)
    .then((steps) => {
      stepsByKey.set(key, steps);
      listeners.forEach((l) => l());
    })
    .catch((err) => Logger.warn('[Moderation] could not load steps:', err))
    .finally(() => inFlight.delete(key));
}

function useSteps(key: string, enabled: boolean, command: string, args: Record<string, unknown>): ModStep[] | null {
  const steps = useSyncExternalStore(subscribe, () => (enabled ? stepsByKey.get(key) ?? null : null));
  useEffect(() => {
    if (enabled) requestSteps(key, command, args);
    // `key` is built from every arg, so it stands in for them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, key]);
  return steps;
}

/**
 * The hover dock's timeouts (then Ban) for `provider`, as Rust says that
 * platform applies them. Null until the first answer lands, or while
 * `enabled` is off.
 */
export function useModTimeoutSteps(provider: string, enabled: boolean): ModStep[] | null {
  const presets = useAppStore((s) => s.settings.moderation?.timeout_presets);
  const key = `dock|${provider}|${presets?.join(',') ?? ''}`;
  return useSteps(key, enabled, 'get_mod_timeout_steps', { provider, presets: presets ?? null });
}

/** The slider's setup as the viewer has it, with its defaults filled in. */
export function useSliderConfig(): SliderConfig {
  const del = useAppStore((s) => s.settings.chat_design?.mod_slider_delete);
  const timeouts = useAppStore((s) => s.settings.chat_design?.mod_slider_timeouts);
  const ban = useAppStore((s) => s.settings.chat_design?.mod_slider_ban);
  return { delete: del !== false, timeouts: timeouts ?? DEFAULT_SLIDER_TIMEOUTS, ban: ban !== false };
}

/** The line buttons' timeouts by default (Rust's `DEFAULT_LINE_TIMEOUTS`). */
export const DEFAULT_LINE_TIMEOUTS = [600];

/** The line buttons' own list as the viewer has it, defaults filled in. Same
 *  shape as the slider's, so Rust builds both with `get_mod_slider_steps`. */
export function useLineConfig(): SliderConfig {
  const del = useAppStore((s) => s.settings.chat_design?.mod_line_delete);
  const timeouts = useAppStore((s) => s.settings.chat_design?.mod_line_timeouts);
  const ban = useAppStore((s) => s.settings.chat_design?.mod_line_ban);
  return { delete: del !== false, timeouts: timeouts ?? DEFAULT_LINE_TIMEOUTS, ban: ban !== false };
}

/**
 * The slider's steps for `provider` and `config` (Delete, timeouts, Ban), as
 * Rust says that platform applies them. Null until the first answer lands.
 * The line buttons ask through here too, with their own list.
 */
export function useModSliderSteps(provider: string, config: SliderConfig, enabled = true): ModStep[] | null {
  const key = `slider|${provider}|${config.delete ? 1 : 0}|${config.timeouts.join(',')}|${config.ban ? 1 : 0}`;
  return useSteps(key, enabled, 'get_mod_slider_steps', { provider, config });
}
