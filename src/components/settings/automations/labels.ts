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
  speak: 'Say it out loud',
  highlight: 'Highlight the message',
  command: 'Run a command',
  whisper: 'Whisper',
  timeout: 'Time them out',
  delete: 'Delete the message',
  pin: 'Pin the message',
  announce: 'Announce',
  shoutout: 'Shout them out',
  join_chat: 'Keep a chat joined',
  leave_chat: 'Stop keeping a chat joined',
  wait: 'Wait',
  log: 'Log it',
};

/** An action's word before its text in a preview or a Try it result. */
export const ACTION_WORDS: Record<AutomationAction['kind'], string> = {
  reply: 'Reply',
  send: 'Post',
  notify: 'Notify',
  speak: 'Say',
  highlight: 'Highlight',
  command: 'Run',
  whisper: 'Whisper',
  timeout: 'Time out',
  delete: 'Delete',
  pin: 'Pin',
  announce: 'Announce',
  shoutout: 'Shout out',
  join_chat: 'Join',
  leave_chat: 'Leave',
  wait: 'Wait',
  log: 'Log',
};

export const P2_TRIGGERS: { value: 'whisper' | 'timer' | 'clock' | 'uptime' | 'delay'; label: string }[] = [
  { value: 'whisper', label: 'A whisper' },
  { value: 'timer', label: 'Every so often' },
  { value: 'clock', label: 'At a time' },
  { value: 'uptime', label: 'When the stream reaches' },
  { value: 'delay', label: 'After the chat opens' },
];

export const ANNOUNCE_COLORS: { value: 'primary' | 'blue' | 'green' | 'orange' | 'purple'; label: string }[] = [
  { value: 'primary', label: 'Default' },
  { value: 'blue', label: 'Blue' },
  { value: 'green', label: 'Green' },
  { value: 'orange', label: 'Orange' },
  { value: 'purple', label: 'Purple' },
];
