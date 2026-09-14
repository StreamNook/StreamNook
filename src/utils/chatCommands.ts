import type { UserSlashCommand } from '../types';

export interface CommandDefinition {
  name: string;
  description: string;
  category: 'Everyone' | 'Moderator' | 'Chat Flow' | 'Engagement' | 'Broadcaster' | 'Custom';
  usage: string;
  // Optional fields used by the guided multi-step /remind flow autocomplete.
  insertText?: string; // text inserted on Tab/Enter (defaults to `name`)
  hint?: boolean; // a non-completing guidance row (Tab/Enter does nothing)
  badgeLabel?: string; // overrides the category text shown in the badge
  // Used by the browsable command menu (the button left of the emote picker).
  // The slash autocomplete ignores these: it only has one line per command.
  examples?: string[]; // full example invocations, shown as click-to-insert chips
  details?: string; // a longer plain-language explanation, one or two sentences
  local?: boolean; // runs inside StreamNook only; nothing is sent to the chat server
}

export const COMMAND_DEFINITIONS: CommandDefinition[] = [
  // Everyone
  { name: 'mods', usage: '/mods', description: 'Display a list of all chat moderators', category: 'Everyone' },
  { name: 'vips', usage: '/vips', description: 'Display a list of VIPs for this channel', category: 'Everyone' },
  { name: 'color', usage: '/color <colorname|hex>', description: 'Change the color of your username', category: 'Everyone' , examples: ['/color SpringGreen', '/color #ff69b4'], details: 'Twitch accepts the named colors from its own list, and Turbo or Prime users can use any hex value.' },
  { name: 'block', usage: '/block <username>', description: 'Block all messages from a specific user', category: 'Everyone' , examples: ['/block someuser'], details: 'This is a Twitch block, so it applies on twitch.tv too. Use /ignore for a StreamNook-only hide.' },
  { name: 'unblock', usage: '/unblock <username>', description: 'Remove user from block list', category: 'Everyone' },
  { name: 'disconnect', usage: '/disconnect', description: 'Disconnect from the chat server', category: 'Everyone' },
  { name: 'w', usage: '/w <username> <message>', description: 'Send a private whisper', category: 'Everyone' , examples: ['/w someuser hey, saw your stream earlier'], details: 'Whispers go through Twitch and land in their whispers inbox. The other person has to allow whispers from strangers.' },
  { name: 'gift', usage: '/gift <quantity>', description: 'Gift Subs to the community', category: 'Everyone' , examples: ['/gift 5'], details: 'Opens the Twitch gifting flow for this channel. You pay on twitch.tv, not in StreamNook.' },
  { name: 'vote', usage: '/vote', description: 'Vote in the active poll', category: 'Everyone' },
  
  // Moderator / Utility
  { name: 'ban', usage: '/ban <username> [reason]', description: 'Permanently ban a user', category: 'Moderator' , examples: ['/ban someuser', '/ban someuser spamming links'], details: 'The ban lasts until a moderator lifts it with /unban. The reason is only visible to the mod team.' },
  { name: 'timeout', usage: '/timeout <username> [seconds] [reason]', description: 'Temporarily ban a user', category: 'Moderator' , examples: ['/timeout someuser', '/timeout someuser 600 calm down'], details: 'Seconds default to 600 (ten minutes). The longest timeout Twitch allows is two weeks, 1209600 seconds.' },
  { name: 'unban', usage: '/unban <username>', description: 'Lift a permanent ban or timeout', category: 'Moderator' , examples: ['/unban someuser'], details: 'Clears both a permanent ban and a running timeout.' },
  { name: 'monitor', usage: '/monitor <username>', description: 'Start monitoring a user\'s messages', category: 'Moderator' , details: 'Monitoring is a Twitch mod tool: the user keeps chatting, but their messages are flagged for the mod team.' },
  { name: 'unmonitor', usage: '/unmonitor <username>', description: 'Stop monitoring a user\'s messages', category: 'Moderator' },
  { name: 'restrict', usage: '/restrict <username>', description: 'Start restricting a user\'s messages', category: 'Moderator' , details: 'A restricted user still sees chat as normal, but their messages only reach moderators until someone approves them.' },
  { name: 'unrestrict', usage: '/unrestrict <username>', description: 'Stop restricting a user\'s messages', category: 'Moderator' },
  // Chat Flow
  { name: 'clear', usage: '/clear', description: 'Wipe the chat history', category: 'Chat Flow' },
  { name: 'slow', usage: '/slow <seconds>', description: 'Set limit on how often users can send messages', category: 'Chat Flow' , examples: ['/slow 10', '/slow 120'], details: 'Everyone except mods and VIPs has to wait this many seconds between messages. Turn it off with /slowoff.' },
  { name: 'slowoff', usage: '/slowoff', description: 'Disable slow mode', category: 'Chat Flow' },
  { name: 'followers', usage: '/followers [duration]', description: 'Restrict chat to followers', category: 'Chat Flow' , examples: ['/followers', '/followers 10m', '/followers 1d'], details: 'With no duration anyone who follows can talk. A duration means they must have followed for at least that long. Turn it off with /followersoff.' },
  { name: 'followersoff', usage: '/followersoff', description: 'Disable followers only mode', category: 'Chat Flow' },
  { name: 'subscribers', usage: '/subscribers', description: 'Restrict chat to subscribers', category: 'Chat Flow' },
  { name: 'subscribersoff', usage: '/subscribersoff', description: 'Disable subscribers only mode', category: 'Chat Flow' },
  { name: 'uniquechat', usage: '/uniquechat', description: 'Disallow non-unique messages (r9k)', category: 'Chat Flow' },
  { name: 'uniquechatoff', usage: '/uniquechatoff', description: 'Disable uniquechat mode', category: 'Chat Flow' },
  { name: 'emoteonly', usage: '/emoteonly', description: 'Set chat to emotes only', category: 'Chat Flow' },
  { name: 'emoteonlyoff', usage: '/emoteonlyoff', description: 'Disable emote only mode', category: 'Chat Flow' },
  
  // Engagement
  { name: 'announce', usage: '/announce <message>', description: 'Highlight a message for chat\'s attention', category: 'Engagement' , examples: ['/announce Raffle winners in five minutes'], details: 'The message shows with a colored band in every viewer’s chat, the same as the Twitch /announce.' },
  { name: 'shoutout', usage: '/shoutout <username>', description: 'Share another streamer\'s channel', category: 'Engagement' , examples: ['/shoutout someone'], details: 'Posts an official Twitch shoutout with the channel card. Twitch allows one every two minutes, and the same person once per hour.' },
  { name: 'poll', usage: '/poll', description: 'Open the poll builder. Channel owner only, because Twitch does not let moderators run polls through the API', category: 'Broadcaster' },
  { name: 'deletepoll', usage: '/deletepoll', description: 'Delete the active poll', category: 'Engagement' },
  
  // Broadcaster
  { name: 'commercial', usage: '/commercial [seconds]', description: 'Run a commercial for all viewers (30-180s)', category: 'Broadcaster' , examples: ['/commercial', '/commercial 90'], details: 'Runs an ad break for viewers without ad-free status. Length must be 30, 60, 90, 120, 150 or 180 seconds.' },
  { name: 'goal', usage: '/goal', description: 'Manage a sub or follower goal', category: 'Broadcaster' },
  { name: 'prediction', usage: '/prediction', description: 'Open the prediction builder. Channel owner only, because Twitch does not let moderators run predictions through the API', category: 'Broadcaster' },
  { name: 'raid', usage: '/raid <channel>', description: 'Send viewers to another live channel', category: 'Broadcaster' , examples: ['/raid someone'], details: 'Starts the raid countdown. Everyone watching moves to the target when it fires. Cancel with /unraid.' },
  { name: 'unraid', usage: '/unraid', description: 'Cancel the active raid', category: 'Broadcaster' },
  { name: 'marker', usage: '/marker [description]', description: 'Add a stream marker', category: 'Broadcaster' , examples: ['/marker', '/marker great play'], details: 'Adds a marker at the current point in the VOD so you can find the moment later in the highlighter.' },

  // StreamNook-native QoL commands (no Twitch-server effect; client-side only)
  { name: 'clearmessages', usage: '/clearmessages', description: 'Hide messages currently shown in this chat. Visual only, not the moderator /clear.', category: 'Everyone' , local: true, details: 'Clears what you see in this pane. Nothing is deleted for anyone else.' },
  { name: 'openurl', usage: '/openurl <url>', description: 'Open a link in your default browser', category: 'Everyone' , local: true, examples: ['/openurl https://example.com'] },
  { name: 'popout', usage: '/popout [channel]', description: "Open this (or another) channel's popout chat in your browser", category: 'Everyone' , local: true, examples: ['/popout', '/popout someone'] },
  { name: 'popup', usage: '/popup [channel]', description: 'Open this (or another) channel in a new StreamNook MultiChat window', category: 'Everyone' , local: true, examples: ['/popup', '/popup someone'], details: 'Opens a MultiChat window with that channel, no browser involved.' },
  { name: 'r', usage: '/r <message>', description: 'Reply to the last whisper you received', category: 'Everyone' , local: true, examples: ['/r sure, sounds good'], details: 'Replies to whoever whispered you most recently, so you never have to type their name.' },
  { name: 'clip', usage: '/clip', description: 'Create a clip of the last 30 seconds of the current stream', category: 'Everyone' , details: 'Creates a clip through Twitch and copies the link. The clip editor opens in your browser if you want to trim it.' },
  { name: 'chatters', usage: '/chatters', description: 'Show how many chatters are connected right now', category: 'Everyone' , local: true },
  { name: 'ignore', usage: '/ignore <user>', description: 'Hide a user from your chat everywhere (local only)', category: 'Everyone' , local: true, examples: ['/ignore someuser'], details: 'Hides that person in every StreamNook chat. Twitch never knows. Undo it in Settings, Chat, Ignored users.' },
  { name: 'settitle', usage: '/settitle <title>', description: 'Set the stream title (broadcaster only)', category: 'Broadcaster' , examples: ['/settitle Chill Sunday stream'] },
  { name: 'setgame', usage: '/setgame <category>', description: 'Set the stream category by name (broadcaster only)', category: 'Broadcaster' , examples: ['/setgame Just Chatting', '/setgame Elden Ring'], details: 'The name is matched against the Twitch category list, so spell it the way Twitch does.' },
  { name: 'overlay', usage: '/overlay [channel]', description: 'Float this (or another) channel in a see-through always-on-top chat window', category: 'Everyone' , local: true, examples: ['/overlay', '/overlay someone'], details: 'A see-through, always-on-top chat you can park over a game. Ctrl+Alt+N makes every overlay click-through or clickable again.' },
  { name: 'uptime', usage: '/uptime', description: "Show this channel's current stream uptime", category: 'Everyone' , local: true },
  { name: 'song', usage: '/song', description: 'Identify the music currently playing in the stream', category: 'Everyone' , local: true, details: 'Listens to a few seconds of the stream audio and looks the track up. Nothing is posted to chat unless you paste the result.' },
  { name: 'remind', usage: '/remind every 20m <message>', description: 'Auto-post reminders to chat: every N min, after a delay, at a time, at uptime, or on a keyword. Manage with /remind list, remove, enable, disable, clear. Add x3 to repeat.', category: 'Everyone' , local: true, examples: ['/remind every 20m Drink water', '/remind in 10m Check the queue', '/remind at 21:30 Raid time', '/remind on !discord Discord link is in the panel'], details: 'Reminders are posted by StreamNook while the app is open. Manage them with /remind list, remove, enable, disable and clear.' },
  { name: 'usercard', usage: '/usercard <user>', description: "Open a user's StreamNook profile card", category: 'Everyone' , local: true, examples: ['/usercard someuser'] },
  { name: 'user', usage: '/user <user>', description: "Open a user's StreamNook profile card", category: 'Everyone' , local: true, examples: ['/user someuser'] },
  { name: 'banid', usage: '/banid <userID> [reason]', description: 'Ban by Twitch user ID (works on suspended accounts that /ban no longer reaches)', category: 'Moderator' , examples: ['/banid 12345678 ban evasion'] },
  { name: 'nuke', usage: '/nuke <pattern> <action> <past[:future]>', description: 'Mass-action by phrase or /regex/. action = delete | ban | duration. Example: /nuke spam ban 5m:1m. Use /nuke stop to end a running window early.', category: 'Moderator' , examples: ['/nuke spam ban 5m:1m', '/nuke /bad\\s*word/ delete 10m', '/nuke stop'], details: 'Looks back over the past window and acts on every message that matches. A future window keeps catching new matches for that long. The preview above the box shows how many messages and people it would hit before you press Enter.' },
  { name: 'undo', usage: '/undo', description: 'Reverse the most recent /nuke on this channel (bans and timeouts only, deletes are permanent)', category: 'Moderator' , details: 'Unbans or untimes-out everyone the last /nuke hit. Deleted messages cannot come back.' },
  { name: 'endpoll', usage: '/endpoll', description: 'End the running poll and leave the result on screen. Channel owner only', category: 'Broadcaster' },
  { name: 'cancelpoll', usage: '/cancelpoll', description: 'Cancel the running poll and hide it entirely. Channel owner only', category: 'Broadcaster' },
  { name: 'lockprediction', usage: '/lockprediction', description: 'Stop taking bets on the running prediction. Channel owner only', category: 'Broadcaster' },
  { name: 'cancelprediction', usage: '/cancelprediction', description: 'Cancel the running prediction and refund everyone. Channel owner only', category: 'Broadcaster' },
  { name: 'completeprediction', usage: '/completeprediction <number|title>', description: 'Pay out the running prediction to a winning outcome, by position or name. Channel owner only', category: 'Broadcaster' , examples: ['/completeprediction 1', '/completeprediction Yes'] },
  { name: 'pin', usage: '/pin <id|@user> [duration]', description: 'Pin a message by its id, or pin that person\'s most recent one. Duration like 60s or 5m; leave it off to pin until removed', category: 'Moderator' , examples: ['/pin @someuser', '/pin @someuser 5m'], details: 'Pinned messages sit above chat for everyone until the timer ends or a mod runs /unpin.' },
  { name: 'unpin', usage: '/unpin', description: 'Remove the currently pinned message', category: 'Moderator' },
  { name: 'blockterm', usage: '/blockterm <phrase>', description: 'Add a phrase to the channel\'s AutoMod blocked terms', category: 'Moderator' , examples: ['/blockterm free followers'], details: 'AutoMod holds any message with that phrase for moderator review.' },
  { name: 'unblockterm', usage: '/unblockterm <phrase>', description: 'Remove a phrase from the channel\'s AutoMod blocked terms', category: 'Moderator' },
  { name: 'refresh', usage: '/refresh', description: 'Force re-fetch emotes for this channel (busts the local cache)', category: 'Everyone' , local: true, details: 'Refetches Twitch, 7TV, BTTV and FFZ emotes for this channel. Use it when a new emote is not showing yet.' },
  { name: 'reload', usage: '/reload', description: 'Hard refresh: restart the stream and reconnect/reload chat (not just emotes)', category: 'Everyone' , local: true },
];

// Reserved trigger names — user-defined commands matching these never fire
// because the built-in switch in commandHandler.ts intercepts first. We still
// expose them here so the settings UI can warn the user before they save.
export const RESERVED_TRIGGERS: ReadonlySet<string> = new Set(
  COMMAND_DEFINITIONS.map((c) => c.name.toLowerCase())
);
// `me` falls through to native IRC and isn't in COMMAND_DEFINITIONS but still
// shouldn't be overridable; same for help/user which are also handled.
['me', 'help', 'mods', 'vips'].forEach((extra) => (RESERVED_TRIGGERS as Set<string>).add(extra));

// Template grammar (full reference is in the help block of the settings UI):
//
//   {N}             1-indexed positional arg (e.g. {1}, {2})
//   {N+}            rest of the args starting at position N joined with space
//   {*}             alias for {1+}
//   {{              literal "{"
//   }}              literal "}"
//   {user.name}     invoker's display name (alias: {user})
//   {user.id}       invoker's Twitch user ID
//   {channel.name}  current channel's display name (alias: {channel})
//   {channel.id}    current channel's broadcaster ID
//   {stream.title}  current stream title (empty when offline)
//   {stream.game}   current stream category / game name (empty when offline)
//   {stream.uptime} formatted uptime, e.g. "2h 14m" (empty when offline)
//
// Implementation strategy: walk the template once with a single regex that
// captures every kind of placeholder, including literal-brace escapes. This
// avoids the chained-.replace() ambiguity where an inserted value could
// itself look like a placeholder and get expanded again.

const PLACEHOLDER_RE = /\{\{|\}\}|\{(\d+)\+\}|\{(\d+)\}|\{\*\}|\{([a-zA-Z][a-zA-Z0-9_.]*)\}/g;

export interface TemplateContext {
  user_name: string;
  user_id: string;
  channel_name: string;
  channel_id: string;
  stream_title: string;
  stream_game: string;
  stream_uptime: string;
  args: string[];
}

export interface TemplateExpansion {
  text: string;
  missing_args: number[]; // 1-indexed positions that the template needs but the user didn't provide
}

function dottedFieldValue(key: string, ctx: TemplateContext): string | null {
  switch (key.toLowerCase()) {
    case 'user':
    case 'user.name':
      return ctx.user_name;
    case 'user.id':
      return ctx.user_id;
    case 'channel':
    case 'channel.name':
      return ctx.channel_name;
    case 'channel.id':
      return ctx.channel_id;
    case 'stream.title':
      return ctx.stream_title;
    case 'stream.game':
      return ctx.stream_game;
    case 'stream.uptime':
      return ctx.stream_uptime;
    default:
      return null; // Unknown placeholder — left as literal in the output
  }
}

export function expandUserCommand(template: string, ctx: TemplateContext): TemplateExpansion {
  const missing: number[] = [];

  const text = template.replace(PLACEHOLDER_RE, (match, positionalRest: string | undefined, positional: string | undefined, dotted: string | undefined) => {
    // Literal brace escapes
    if (match === '{{') return '{';
    if (match === '}}') return '}';

    // {N+} — rest of args starting at position N
    if (positionalRest !== undefined) {
      const start = parseInt(positionalRest, 10);
      if (start < 1 || start > ctx.args.length) {
        // Treat as missing if N is past the supplied args
        missing.push(start);
        return '';
      }
      return ctx.args.slice(start - 1).join(' ');
    }

    // {N} — single positional arg
    if (positional !== undefined) {
      const i = parseInt(positional, 10);
      const value = ctx.args[i - 1];
      if (value === undefined || value === '') {
        missing.push(i);
        return '';
      }
      return value;
    }

    // {*} — all args (alias for {1+})
    if (match === '{*}') {
      return ctx.args.join(' ');
    }

    // {field} or {field.subfield}
    if (dotted !== undefined) {
      const value = dottedFieldValue(dotted, ctx);
      if (value === null) return match; // Leave unknown placeholder literal
      return value;
    }

    return match;
  });

  // Collapse consecutive whitespace caused by missing args ("hi  there" -> "hi there")
  const collapsed = text.replace(/[ \t]{2,}/g, ' ').trim();

  return { text: collapsed, missing_args: Array.from(new Set(missing)).sort() };
}

// Format a started_at ISO timestamp into "Hh Mm" / "Mm" — for {stream.uptime}.
export function formatStreamUptime(startedAt: string | null | undefined): string {
  if (!startedAt) return '';
  const start = Date.parse(startedAt);
  if (!Number.isFinite(start)) return '';
  const elapsedMs = Date.now() - start;
  if (elapsedMs <= 0) return '0m';
  const totalMinutes = Math.floor(elapsedMs / 60000);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
}

// Build CommandDefinition entries for the autocomplete from the user's
// user-defined commands. Inferred usage scans the template for placeholders
// and renders them as `<argN>` segments alongside the trigger. Commands that
// have `require_slash: false` aren't shown in the slash autocomplete (they
// fire from plain messages, not /-prefixed input).
const POSITIONAL_SINGLE_RE = /\{(\d+)\}/g;
const POSITIONAL_REST_RE = /\{(\d+)\+\}/g;
const ALL_REST_RE = /\{\*\}/g;

export function buildUserCommandDefinitions(commands: UserSlashCommand[] | undefined): CommandDefinition[] {
  if (!commands || commands.length === 0) return [];
  return commands
    .filter((c) => c.enabled && c.trigger.trim().length > 0 && c.require_slash !== false)
    .map((c) => {
      const positionalIndices = new Set<number>();
      let m: RegExpExecArray | null;
      const single = new RegExp(POSITIONAL_SINGLE_RE.source, 'g');
      while ((m = single.exec(c.expansion)) !== null) {
        positionalIndices.add(parseInt(m[1], 10));
      }
      const restIndices = new Set<number>();
      const rest = new RegExp(POSITIONAL_REST_RE.source, 'g');
      while ((m = rest.exec(c.expansion)) !== null) {
        restIndices.add(parseInt(m[1], 10));
      }
      const hasAllRest = ALL_REST_RE.test(c.expansion);
      const usageArgs: string[] = [];
      Array.from(positionalIndices)
        .sort((a, b) => a - b)
        .forEach((i) => usageArgs.push(`<arg${i}>`));
      Array.from(restIndices)
        .sort((a, b) => a - b)
        .forEach((i) => usageArgs.push(`<arg${i}...>`));
      if (hasAllRest && restIndices.size === 0) usageArgs.push('<...>');

      const usage = `/${c.trigger}${usageArgs.length > 0 ? ' ' + usageArgs.join(' ') : ''}`;
      const description = c.description?.trim() || (c.expansion.length > 60 ? c.expansion.slice(0, 57) + '...' : c.expansion);

      return {
        name: c.trigger.toLowerCase(),
        description,
        category: 'Custom' as const,
        usage,
      };
    });
}

// Lookup an enabled user command by trigger (case-insensitive). Honors the
// require_slash flag — pass `requireSlash: true` when the caller is in the
// slash-command branch, `false` when it's a plain-text scan.
export function findUserCommand(
  trigger: string,
  commands: UserSlashCommand[] | undefined,
  requireSlash: boolean = true,
): UserSlashCommand | undefined {
  if (!commands) return undefined;
  const target = trigger.toLowerCase();
  return commands.find((c) => {
    if (!c.enabled) return false;
    if (c.trigger.toLowerCase() !== target) return false;
    const cmdRequiresSlash = c.require_slash !== false;
    return cmdRequiresSlash === requireSlash;
  });
}

// Scan a plain-text (non-slash) message for any enabled user command whose
// trigger matches at the start (and optionally end) of the message. Returns
// the matched command + the args extracted from the surrounding text, or null.
export interface PlainTextMatch {
  command: UserSlashCommand;
  args: string[];
}

export function matchPlainTextUserCommand(
  message: string,
  commands: UserSlashCommand[] | undefined,
): PlainTextMatch | null {
  if (!commands || commands.length === 0) return null;
  const trimmed = message.trim();
  if (!trimmed) return null;
  const lowered = trimmed.toLowerCase();
  const words = trimmed.split(/\s+/);
  const firstWord = words[0]?.toLowerCase() ?? '';
  const lastWord = words[words.length - 1]?.toLowerCase() ?? '';

  for (const cmd of commands) {
    if (!cmd.enabled) continue;
    if (cmd.require_slash !== false) continue; // slash commands handled elsewhere
    const t = cmd.trigger.toLowerCase();
    if (!t) continue;
    if (firstWord === t) {
      return { command: cmd, args: words.slice(1) };
    }
    if (cmd.also_match_suffix && lastWord === t && lowered !== t /* avoid double-fire when trigger is the only word */) {
      return { command: cmd, args: words.slice(0, -1) };
    }
  }
  return null;
}
