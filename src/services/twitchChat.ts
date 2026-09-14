import { parseBadges } from './twitchBadges';
import type { SongMatch } from '../utils/songId';
import type { ProviderId } from '../types/providers';
import { kickBadgeImage } from '../utils/kickBadges';

// Message segment types - matches Rust MessageSegment enum
export type MessageSegment =
  | { type: 'text'; content: string }
  | { type: 'emote'; content: string; emote_id?: string; emote_url: string; is_zero_width?: boolean; modifier_flags?: number; is_personal?: boolean }
  | { type: 'emoji'; content: string; emoji_url: string }
  | { type: 'link'; content: string; url: string }
  // Cheermote segment for animated bits (e.g., Cheer500)
  | { type: 'cheermote'; content: string; prefix: string; bits: number; tier: string; color: string; cheermote_url: string }
  // Twitch chat GIF (GIPHY-backed). `content` is the bracketed description the
  // message text carries at the GIF's span; the URL must be used as sent.
  | { type: 'gif'; content: string; gif_id: string; gif_url: string };

// Reply information parsed from IRC tags - matches Rust ReplyInfo
export interface BackendReplyInfo {
  parent_msg_id: string;
  parent_display_name: string;
  parent_msg_body: string;
  parent_user_id: string;
  parent_user_login: string;
}

// Pre-computed message metadata - THE ENDGAME
// All these fields are computed in Rust to eliminate frontend processing
export interface MessageMetadata {
  /** Whether this is an ACTION message (/me command) */
  is_action: boolean;
  /** Whether this message mentions the current user (set by frontend context) */
  is_mentioned: boolean;
  /** Whether this is the user's first message in the channel */
  is_first_message: boolean;
  /** Pre-formatted timestamp string (e.g., "3:45 PM") */
  formatted_timestamp?: string;
  /** Pre-formatted timestamp with seconds (e.g., "3:45:30 PM") */
  formatted_timestamp_with_seconds?: string;
  /** Reply information if this is a reply message */
  reply_info?: BackendReplyInfo;
  /** Source room ID for shared chat messages */
  source_room_id?: string;
  /** Whether this message is from shared chat (different from current room) */
  is_from_shared_chat: boolean;
  /** Message type for special messages (sub, resub, subgift, etc.) */
  msg_type?: string;
  /** Bits amount if this is a cheer message */
  bits_amount?: number;
  /** System message for subscriptions/donations */
  system_message?: string;
  /** True when the Rust rule engine evaluated this message: the fields below
   *  are authoritative and the row runs no matcher of its own. */
  rules_evaluated?: boolean;
  /** The message replies to one of ours. */
  is_reply_to_me?: boolean;
  /** First matching highlight rule (phrase, user, or badge), if any. */
  highlight?: HighlightStamp;
  /** Built-in event tint (raid, returning chatter, first message, self). */
  built_in?: BuiltInStamp;
  /** Ids of the user's saved filters this message satisfies. */
  filter_ids?: string[];
  /** Sender's low-trust status in this channel (moderators only). */
  suspicious?: 'monitored' | 'restricted' | string;
  /** Row came from a history backfill rather than live delivery. */
  from_backfill?: boolean;
}

/** A matched highlight rule, stamped by the Rust rule engine. */
export interface HighlightStamp {
  rule_id: string;
  kind: 'phrase' | 'user' | 'badge';
  color: string;
  sound_id?: string | null;
  cooldown_ms: number;
}

/** A built-in event highlight, stamped by the Rust rule engine. */
export interface BuiltInStamp {
  kind: 'raider' | 'returning' | 'first_time' | 'self';
  color: string;
  label: string;
}

export interface BackendChatMessage {
  id: string;
  username: string;
  display_name: string;
  color?: string;
  user_id: string;
  timestamp: string;
  content: string;
  /** Source platform; absent means twitch (back-compat with pre-multi-platform messages). */
  provider?: ProviderId;
  /** Composite source key the message was published on ("youtube:slug", "kick:slug",
   *  or a bare Twitch login). */
  channel?: string;
  badges: Array<{
    name: string;
    version: string;
    image_url_1x?: string;
    image_url_2x?: string;
    image_url_4x?: string;
    title?: string;
    description?: string;
  }>;
  /** No longer serialized by the backend (the `emotes` tag and `segments`
   *  carry everything); optional so old cached objects still typecheck. */
  emotes?: Array<{ id: string; start: number; end: number; url: string }>;
  /** No longer serialized by the backend - the ResizeObserver measurement is
   *  authoritative and nothing reads this. */
  layout?: { height: number; width: number; has_reply?: boolean; is_first_message?: boolean };
  tags: { [key: string]: string };
  // Pre-parsed segments from Rust (Phase 3.1 - The Endgame)
  segments?: MessageSegment[];
  // Pre-computed metadata - THE ENDGAME
  // All message analysis done in Rust, frontend just renders
  metadata?: MessageMetadata;
  // Local-only: the "/song" result card (album art + clickable service links).
  songCard?: SongMatch;
}

// Frontend-facing reply info (camelCase for consistency)
export interface ReplyInfo {
  parentMsgId: string;
  parentDisplayName: string;
  parentMsgBody: string;
  parentUserId: string;
  parentUserLogin: string;
}

// Structured backend rows are parsed at least twice per message today (the
// ChatWidget side-effect loop and the row's own render memo), and both derive
// channelId from the row's own tags - so one cache entry per object is
// correct. Keyed by object identity: upgrades replace the object, so a stale
// entry can never be served. Raw IRC strings stay uncached (cheap, and their
// channelId can vary by caller).
const parsedObjCache = new WeakMap<object, ReturnType<typeof parseMessageUncached>>();

export const parseMessage = (raw: string | BackendChatMessage, channelId?: string) => {
  if (typeof raw !== 'string') {
    const hit = parsedObjCache.get(raw);
    if (hit) return hit;
    const parsed = parseMessageUncached(raw, channelId);
    parsedObjCache.set(raw, parsed);
    return parsed;
  }
  return parseMessageUncached(raw, channelId);
};

const parseMessageUncached = (raw: string | BackendChatMessage, channelId?: string) => {
  // Check if it's a backend message object
  if (typeof raw !== 'string') {
    const tags = new Map<string, string>(Object.entries(raw.tags));

    // Reconstruct badge string from the array
    const badgeStr = raw.badges.map(b => `${b.name}/${b.version}`).join(',');
    
    // Use parseBadges to enrich with metadata from cache
    const sourceRoomId = raw.metadata?.source_room_id || tags.get('source-room-id');
    const effectiveChannelId = sourceRoomId || channelId;
    // Non-Twitch providers carry their own badge images (baked URLs, or for Kick
    // global types a bundled icon). Bypass the Twitch badge cache, which is keyed
    // by Twitch badge names and would mis-resolve or drop them.
    let badgeData: ReturnType<typeof parseBadges>;
    if (raw.provider && raw.provider !== 'twitch') {
      // Non-Twitch providers carry their own already-resolved badge images
      // (Kick: baked badges_v2 art + custom/default subscriber art; YouTube:
      // real per-tier member badge art), so build badgeData directly instead of
      // going through the Twitch badge cache. Only Kick has bundled fallback art
      // for its icon-only role badges; YouTube role-badge art is a later pass, so
      // YouTube only renders badges that ship a real image_url (member tiers).
      badgeData = [];
      for (const b of raw.badges) {
        const url = b.image_url_1x || (raw.provider === 'kick' ? kickBadgeImage(b.name) : undefined);
        if (url) {
          badgeData.push({
            key: `${b.name}/${b.version}`,
            info: { localUrl: url, image_url_1x: url, image_url_4x: url, title: b.title },
          });
        }
      }
    } else {
      badgeData = parseBadges(badgeStr, effectiveChannelId);
    }

    // PHASE 3.1 - THE ENDGAME: Use pre-computed reply info from Rust
    let replyInfo: ReplyInfo | undefined;
    if (raw.metadata?.reply_info) {
      // Convert from Rust snake_case to frontend camelCase
      const ri = raw.metadata.reply_info;
      replyInfo = {
        parentMsgId: ri.parent_msg_id,
        parentDisplayName: ri.parent_display_name,
        parentMsgBody: ri.parent_msg_body,
        parentUserId: ri.parent_user_id,
        parentUserLogin: ri.parent_user_login,
      };
    } else {
      // Fallback: parse from tags (legacy support)
      const replyParentMsgId = tags.get('reply-parent-msg-id');
      if (replyParentMsgId) {
        replyInfo = {
          parentMsgId: replyParentMsgId,
          parentDisplayName: tags.get('reply-parent-display-name') || '',
          parentMsgBody: tags.get('reply-parent-msg-body')?.replace(/\\s/g, ' ') || '',
          parentUserId: tags.get('reply-parent-user-id') || '',
          parentUserLogin: tags.get('reply-parent-user-login') || '',
        };
      }
    }

    // PHASE 3.1 - Use pre-computed is_action from Rust metadata
    let isAction = raw.metadata?.is_action ?? false;
    let content = raw.content;
    
    // Fallback check if metadata not available
    if (!raw.metadata && content.startsWith('\u0001ACTION ') && content.endsWith('\u0001')) {
      isAction = true;
      content = content.slice(8, -1);
    }

    // Remove redundant @mention from reply messages
    // The UI already shows reply context, so the leading @username is redundant
    if (replyInfo && replyInfo.parentUserLogin) {
      const mentionPattern = new RegExp(`^@${replyInfo.parentUserLogin}\\s*`, 'i');
      content = content.replace(mentionPattern, '').trim();
    }

    return {
      tags,
      username: raw.username,
      // The structured backend message carries the cased display name in its own
      // field; the renderer prefers it over the lowercase login. (For Twitch the
      // display-name tag already covers this; for Kick the login is the lowercase
      // slug, so this is what shows the cased "Stone916" instead of "stone916".)
      displayName: raw.display_name,
      provider: raw.provider,
      // The chatter's numeric platform user id (Kick puts it here; Twitch carries
      // it in the user-id tag). Used provider-namespaced for 7TV cosmetics lookup.
      providerUserId: raw.user_id,
      content,
      color: raw.color || tags.get('color') || '#9147FF',
      badges: badgeData,
      emotes: tags.get('emotes') || '',
      replyInfo,
      isAction,
      layout: raw.layout, // Pass layout through
      segments: raw.segments, // Pass pre-parsed segments through (Phase 3.1)
      // Pass pre-computed metadata through (Phase 3.1 - THE ENDGAME)
      metadata: raw.metadata,
      // The composite source key ("youtube:slug", "kick:slug", or a bare Twitch
      // login). The non-Twitch mod commands need the slug to resolve the channel.
      channel: raw.channel || '',
    };
  }

  // Legacy string parsing logic
  // Parse IRC tags, username, content, etc.
  const tags = new Map<string, string>();
  let message = (raw as string).trim();

  // Parse tags if present
  if (raw.startsWith('@')) {
    const tagEnd = raw.indexOf(' ');
    const tagStr = raw.slice(1, tagEnd);
    tagStr.split(';').forEach(pair => {
      const [key, value] = pair.split('=');
      if (key && value !== undefined) {
        tags.set(key, value);
      }
    });
    message = raw.slice(tagEnd + 1).trim();
  }

  // Check if this is a USERNOTICE (subscription, etc.)
  const isUserNotice = message.includes('USERNOTICE');

  // Parse IRC message format: :user!user@user.tmi.twitch.tv PRIVMSG #channel :message
  // Extract username from prefix or tags
  let username = tags.get('display-name') || tags.get('login') || 'unknown';
  if (message.startsWith(':')) {
    const prefixEnd = message.indexOf(' ');
    const prefix = message.slice(1, prefixEnd);
    const userMatch = prefix.match(/^([^!]+)!/);
    if (userMatch && username === 'unknown') {
      username = userMatch[1];
    }
    message = message.slice(prefixEnd + 1);
  }

  // Extract the actual message content
  let content = '';
  let isAction = false;
  if (isUserNotice) {
    // For USERNOTICE, content comes after USERNOTICE #channel [:]
    const contentMatch = message.match(/USERNOTICE\s+#\S+\s+:?(.*)$/);
    content = contentMatch ? contentMatch[1] : message;
  } else {
    // For PRIVMSG, content comes after PRIVMSG #channel [:]
    const contentMatch = message.match(/PRIVMSG\s+#\S+\s+:?(.*)$/);
    content = contentMatch ? contentMatch[1] : message;

    // Check if this is an ACTION message (like /me command)
    if (content.startsWith('\u0001ACTION ') && content.endsWith('\u0001')) {
      isAction = true;
      // Strip the ACTION wrapper
      content = content.slice(8, -1);
    }
  }

  // Parse badges using the badge service
  // For shared chat messages, prefer source-badges over badges
  const sourceBadgeStr = tags.get('source-badges');
  const badgeStr = sourceBadgeStr || tags.get('badges') || '';

  // For shared chat, use source-room-id for badge lookup
  const sourceRoomId = tags.get('source-room-id');
  const effectiveChannelId = sourceRoomId || channelId;

  const badgeData = parseBadges(badgeStr, effectiveChannelId);

  // Parse reply information if present
  let replyInfo: ReplyInfo | undefined;
  const replyParentMsgId = tags.get('reply-parent-msg-id');
  if (replyParentMsgId) {
    const parentDisplayName = tags.get('reply-parent-display-name') || '';
    const parentMsgBody = tags.get('reply-parent-msg-body')?.replace(/\\s/g, ' ') || '';
    const parentUserId = tags.get('reply-parent-user-id') || '';
    const parentUserLogin = tags.get('reply-parent-user-login') || '';

    replyInfo = {
      parentMsgId: replyParentMsgId,
      parentDisplayName,
      parentMsgBody,
      parentUserId,
      parentUserLogin,
    };

    // Remove redundant @mention from reply messages
    // The UI already shows reply context, so the leading @username is redundant
    if (parentUserLogin) {
      const mentionPattern = new RegExp(`^@${parentUserLogin}\\s*`, 'i');
      content = content.replace(mentionPattern, '').trim();
    }
  }

  return {
    tags,
    username,
    displayName: tags.get('display-name'),
    provider: undefined as ProviderId | undefined,
    content,
    color: tags.get('color') || '#9147FF', // Twitch purple as default
    badges: badgeData,
    emotes: tags.get('emotes') || '',
    replyInfo,
    isAction,
    channel: '', // raw IRC path is Twitch-only; channel slug isn't tracked here
  };
};
