// The words (and marks) the automations page uses for roles, events, triggers
// and actions, shared by the list, the editor and Try it.
import {
  AlarmClock,
  AtSign,
  Award,
  Bell,
  ChartBar,
  Clock,
  Eraser,
  Gift,
  Hand,
  Highlighter,
  Hourglass,
  LogIn,
  LogOut,
  Mail,
  Megaphone,
  MessageSquare,
  MessageSquarePlus,
  MessageSquareText,
  NotebookPen,
  Pin,
  Radio,
  Reply,
  ShieldBan,
  Sparkles,
  Terminal,
  Timer,
  Trophy,
  UserPlus,
  Volume2,
  type LucideIcon,
} from 'lucide-react';
import type {
  AutomationAction,
  AutomationEventKind,
  AutomationPlatform,
  AutomationSenderRole,
  AutomationTrigger,
} from '../../../types';

/** The mark leading a rule's row: what it waits for. */
export const TRIGGER_ICONS: Record<AutomationTrigger['kind'], LucideIcon> = {
  mention: AtSign,
  message: MessageSquareText,
  first_message: MessageSquarePlus,
  channel_event: Gift,
  badge_drop: Award,
  follow: UserPlus,
  reward_redeemed: Sparkles,
  poll_started: ChartBar,
  prediction_started: Trophy,
  live_change: Radio,
  whisper: Mail,
  timer: Timer,
  clock: AlarmClock,
  uptime: Hourglass,
  delay: Clock,
};

/** The mark of each step in the Add a step menu. */
export const ACTION_ICONS: Record<AutomationAction['kind'], LucideIcon> = {
  reply: Reply,
  send: MessageSquare,
  whisper: Mail,
  announce: Megaphone,
  command: Terminal,
  shoutout: Hand,
  pin: Pin,
  timeout: ShieldBan,
  delete: Eraser,
  notify: Bell,
  speak: Volume2,
  highlight: Highlighter,
  log: NotebookPen,
  wait: Hourglass,
  join_chat: LogIn,
  leave_chat: LogOut,
};

/** Where each step sits in the Add a step menu, and the line under its name. */
export const ACTION_MENU: Record<AutomationAction['kind'], { section: string; help: string }> = {
  reply: { section: 'In chat', help: 'Answer the message that set it off' },
  send: { section: 'In chat', help: 'A line in this chat or another' },
  whisper: { section: 'In chat', help: 'A private message' },
  announce: { section: 'In chat', help: 'A colored announcement' },
  command: { section: 'In chat', help: 'Title, category, marker and more' },
  shoutout: { section: 'In chat', help: 'Shout out who set it off' },
  pin: { section: 'Moderate', help: 'Pin the message' },
  timeout: { section: 'Moderate', help: 'Time out who sent it' },
  delete: { section: 'Moderate', help: 'Delete the message' },
  notify: { section: 'Just for you', help: 'A card here, or a system notification' },
  speak: { section: 'Just for you', help: 'Read out loud on this device' },
  highlight: { section: 'Just for you', help: 'Tint the message in your chat' },
  log: { section: 'Just for you', help: 'Write it to Activity, or a file' },
  wait: { section: 'More', help: 'Pause before the next step' },
  join_chat: { section: 'More', help: 'Keep a chat joined for rules' },
  leave_chat: { section: 'More', help: 'Stop keeping a chat joined' },
};

/** The order of the menu's groups. */
export const ACTION_MENU_SECTIONS = ['In chat', 'Moderate', 'Just for you', 'More'];

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
  { id: 'super_chat', label: 'Super Chats' },
  { id: 'super_sticker', label: 'Super Stickers' },
  { id: 'membership', label: 'New members' },
  { id: 'member_gift', label: 'Gifted memberships' },
  { id: 'tiktok_gift', label: 'Gifts' },
  { id: 'tiktok_follow', label: 'Follows' },
  { id: 'tiktok_share', label: 'Shares' },
];

/** The platforms, in the order the editor shows them (a plain switch beside the platform's own logo). */
export const PLATFORM_ITEMS: { id: AutomationPlatform; label: string }[] = [
  { id: 'twitch', label: 'Twitch' },
  { id: 'kick', label: 'Kick' },
  { id: 'youtube', label: 'YouTube' },
  { id: 'tiktok', label: 'TikTok' },
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
