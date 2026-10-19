// A notification the main window announced, as plain data, for a MultiChat
// window to show while the main window is hidden or minimized. Rust picks the
// window (commands/notification_mirror.rs); a click comes back to the main
// window by id, which runs the notification's own action.

/** To a MultiChat window: show this card. */
export const MIRRORED_NOTIFICATION_EVENT = 'mirrored-notification';
/** To the main window: the card with this id was clicked. */
export const MIRRORED_NOTIFICATION_ACTIVATE_EVENT = 'mirrored-notification-activate';

/** The notification centre's own type names, so the glyphs match it. */
export type MirroredNoticeKind =
  | 'live'
  | 'whisper'
  | 'update'
  | 'drops'
  | 'channel_points'
  | 'badge'
  | 'gift_sub'
  | 'twitch_reward'
  | 'membership_gift'
  | 'mention';

/** To the main window: a notification's action is waiting in `take_notice_action`. */
export const NOTICE_ACTION_PENDING_EVENT = 'notice-action-pending';

/**
 * What clicking a notification does, as data, for a Windows notification used
 * after a restart (commands/notification_mirror.rs `NoticeAction`).
 */
export type NoticeAction =
  | { type: 'watch'; channel: string }
  | { type: 'whisper'; userId: string; login: string; name: string; avatar: string | null }
  | { type: 'drops' }
  | { type: 'badges' }
  | { type: 'changelog'; version: string | null }
  /** Open this channel with its rewards showing. */
  | { type: 'rewards'; channel: string }
  /** Open Settings on this tab. */
  | { type: 'settings'; tab: string }
  /** Open this mention: `channel` is its room key (a `startStream` key). */
  | { type: 'mention'; id: string; channel: string };

/** Where a reply typed into a Windows notification goes (`NoticeReply`). */
export type NoticeReply =
  | { type: 'whisper'; userId: string; login: string; name: string; avatar: string | null }
  | { type: 'chat'; channel: string; broadcasterId: string | null; parentId: string; toName: string };

export interface MirroredNotice {
  id: string;
  kind: MirroredNoticeKind;
  title: string;
  body: string;
  imageUrl?: string | null;
  /** A person's picture (round) rather than box art or a badge (square). */
  roundImage?: boolean;
  /** What a click does, in words. Absent = the card is not clickable. */
  actionLabel?: string | null;
  /** The click's action as data. */
  action?: NoticeAction | null;
  /** A reply box's target (Windows notifications). */
  reply?: NoticeReply | null;
}

/** How long a card stays up: as long as the matching toast would. */
export const mirroredNoticeHoldMs = (kind: MirroredNoticeKind): number =>
  kind === 'live' ? 8000 : 5000;
