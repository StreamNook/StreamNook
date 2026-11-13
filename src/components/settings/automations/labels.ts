// The words the automations page uses for roles, events and actions, shared by
// the editor and Try it.
import type { AutomationAction, AutomationEventKind, AutomationSenderRole } from '../../../types';

export const ROLE_ITEMS: { id: AutomationSenderRole; label: string }[] = [
  { id: 'broadcaster', label: 'The streamer' },
  { id: 'moderator', label: 'Moderators' },
  { id: 'vip', label: 'VIPs' },
  { id: 'subscriber', label: 'Subscribers' },
  { id: 'viewer', label: 'Everyone else' },
];

export const EVENT_ITEMS: { id: AutomationEventKind; label: string }[] = [
  { id: 'sub', label: 'New subs' },
  { id: 'resub', label: 'Resubs' },
  { id: 'gift', label: 'Gifted subs' },
  { id: 'gift_bomb', label: 'Gift bombs' },
  { id: 'raid', label: 'Raids' },
  { id: 'cheer', label: 'Cheers' },
  { id: 'announcement', label: 'Announcements' },
];

/** An action's name in the editor. */
export const ACTION_NAMES: Record<AutomationAction['kind'], string> = {
  reply: 'Reply',
  send: 'Post a message',
  notify: 'Notify me',
};

/** An action's word before its text in a preview or a Try it result. */
export const ACTION_WORDS: Record<AutomationAction['kind'], string> = {
  reply: 'Reply',
  send: 'Post',
  notify: 'Notify',
};
