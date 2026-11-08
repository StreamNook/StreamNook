// The message box's settings-driven look, shared by chat's composer
// (ChatWidget) and Settings > Chat's preview, so the preview draws the box
// from the same code chat does: which prompt shows, whether the emote button
// and the wallet balance show, the room modes as chip or prompt, and the
// field's and Send button's looks.
import type { CSSProperties } from 'react';
import type { ChatInputSettings } from '../../types';
import type { RoomModeView } from '../../stores/chatConnectionStore';

/** What the composer's situation is, beyond your settings. */
export interface ComposerSituation {
  /** You can send here (signed in on this platform). */
  canSend: boolean;
  provider: string;
  /** Twitch wants you to sign in again before you can send. */
  signInOwed: boolean;
  watchStreak: boolean;
  resub: boolean;
  /** Sub-only, and you cannot bypass it. */
  subOnlyBlocked: boolean;
  inputDisabled: boolean;
  /** The room's modes as Rust words them (null when it reports none). */
  roomModes: RoomModeView | null;
}

export interface ComposerLook {
  showEmoteButton: boolean;
  showPointsBalance: boolean;
  pointsBalanceInline: boolean;
  walletGains: 'chest' | 'all';
  /** The room's modes for the chip at the field's end (null: none, or shown
   *  as the prompt instead). */
  roomModesChip: RoomModeView | null;
  /** The prompt in the empty field, full and for a narrow row. */
  placeholder: string;
  placeholderNarrow: string;
}

/** Your Chat Input settings applied to the composer's situation. */
export function composerLook(prefs: ChatInputSettings | undefined, s: ComposerSituation): ComposerLook {
  const roomModes = (prefs?.show_room_modes ?? true) && s.roomModes && s.roomModes.modes.length > 0 ? s.roomModes : null;
  const roomModesAsPrompt = prefs?.room_modes_style !== 'chip';
  const roomModePrompt = roomModesAsPrompt ? (roomModes?.prompt ?? null) : null;
  const roomModePromptShort = roomModesAsPrompt ? (roomModes?.prompt_short ?? null) : null;
  const full = !s.canSend
    ? s.provider === 'kick'
      ? 'Connect your Kick account to send'
      : s.provider === 'youtube'
      ? 'Connect your YouTube account to send'
      : 'Connect your TikTok account to send'
    : s.signInOwed
    ? 'Sign in again to chat'
    : s.watchStreak
    ? 'Add a message to your watch streak'
    : s.resub
    ? 'Add a message to your resub'
    : roomModePrompt ?? (s.subOnlyBlocked ? 'Subscriber-Only Mode' : 'Send a message');
  // The same prompt in a word or two for a narrow composer row, where the
  // full one is cut mid-word: Chromium draws no ellipsis on a textarea
  // placeholder. utils/composerNarrow.ts swaps between the two.
  const narrow = !s.canSend
    ? 'Sign in'
    : s.signInOwed
    ? 'Sign in'
    : s.watchStreak || s.resub
    ? 'Message'
    : roomModePromptShort ?? (s.subOnlyBlocked ? 'Sub-only' : 'Message');
  // The placeholder keeps its text whenever it is saying something the user
  // can act on (connect your account, sub-only, a room mode), so hiding it
  // only ever drops the idle "Send a message" prompt.
  const placeholder = prefs?.hide_placeholder && !s.inputDisabled && !s.watchStreak && !roomModePrompt ? '' : full;
  return {
    showEmoteButton: !prefs?.hide_emote_button,
    showPointsBalance: !prefs?.hide_points_balance,
    pointsBalanceInline: !!prefs?.show_points_balance_inline,
    walletGains: prefs?.wallet_gains === 'chest' ? 'chest' : 'all',
    roomModesChip: roomModes && !roomModesAsPrompt ? roomModes : null,
    placeholder,
    placeholderNarrow: placeholder ? narrow : '',
  };
}

/** 36px clears the inset emote button; 12px is the plain inset once it is
 *  hidden; the send-as picker takes 74px. */
export const composerPaddingLeft = (sendAsPicker: boolean, emoteButton: boolean) =>
  `${sendAsPicker ? 74 : emoteButton ? 36 : 12}px`;

/** The message field's look for what it holds. */
export function composerInputLook(o: {
  remindOverlay: boolean;
  watchStreak: boolean;
  commandValid: boolean;
  isCommand: boolean;
  paddingLeft: string;
}): { className: string; style: CSSProperties } {
  const className = `sn-composer-input relative w-full text-sm placeholder-textSecondary resize-none overflow-hidden scrollbar-thin leading-[1.4] self-center transition-all duration-300 ${o.remindOverlay ? '' : 'glass-input'} ${
    o.watchStreak
      ? 'ring-2 ring-amber-500/50 bg-amber-500/5 shadow-[0_0_15px_color-mix(in_srgb,var(--color-warning)_15%,transparent)] placeholder-amber-500/60 text-textPrimary'
      : o.commandValid && !o.remindOverlay
      ? 'font-bold tracking-wide'
      : ''
  }`;
  const style: CSSProperties = {
    minHeight: '36px',
    maxHeight: '120px',
    paddingTop: '8px',
    paddingBottom: '8px',
    paddingLeft: o.paddingLeft,
    paddingRight: 'var(--sn-room-pad, 12px)',
    ...(o.remindOverlay
      ? {
          // The backdrop renders the surface and chipped text; the textarea
          // goes transparent and only carries the caret and editing. A 1px
          // transparent border keeps its box model on the backdrop's.
          color: 'transparent',
          caretColor: 'var(--color-text-primary)',
          background: 'transparent',
          border: '1px solid transparent',
          boxShadow: 'none',
          backdropFilter: 'none',
          WebkitBackdropFilter: 'none',
          wordSpacing: '0.4em',
          // No fade: transparent at once so the text never ghosts over the chips.
          transition: 'none',
        }
      : o.commandValid && !o.watchStreak
      ? {
          backgroundColor: 'rgba(200, 224, 232, 0.1)',
          borderColor: 'var(--color-accent)',
          boxShadow: '0 0 16px color-mix(in srgb, var(--color-accent-neon) 25%, transparent), inset 4px 4px 10px -3px rgba(0, 0, 0, 0.6)',
          color: 'var(--color-accent)',
        }
      : o.isCommand && !o.watchStreak
      ? {
          backgroundColor: 'rgba(255, 255, 255, 0.04)',
          borderColor: 'rgba(255, 255, 255, 0.2)',
          color: 'var(--color-text-primary)',
        }
      : { color: 'var(--color-text-primary)' }),
  };
  return { className, style };
}

/** The Send button's look. */
export const sendButtonClass = (watchStreak: boolean) =>
  `sn-composer-tool flex-shrink-0 flex items-center justify-center self-center w-9 h-9 text-white rounded transition-all duration-300 disabled:opacity-50 disabled:cursor-not-allowed ${
    watchStreak ? 'bg-amber-500 hover:bg-amber-400 shadow-[0_0_12px_color-mix(in_srgb,var(--color-warning)_30%,transparent)]' : 'glass-button'
  }`;

