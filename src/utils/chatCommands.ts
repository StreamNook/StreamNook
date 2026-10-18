// The command table the desktop chat box's slash autocomplete and command menu
// read on every keystroke. Rust runs every command (services/slash_dispatch.rs)
// from its own copy of this table (services/command_catalog.rs), which serves
// the phone's command sheet; a Rust test fails when the two drift apart.

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
  aliases?: string[]; // other names that run the same command (`/b` for `/ban`)
  desktopOnly?: boolean; // needs a desktop window or file; hidden in the phone app
  devOnly?: boolean; // works only in development builds; release builds never list it
}

export const COMMAND_DEFINITIONS: CommandDefinition[] = [
  // Everyone
  { name: 'mods', usage: '/mods', description: 'Display a list of all chat moderators', category: 'Everyone' , local: true },
  { name: 'vips', usage: '/vips', description: 'Display a list of VIPs for this channel', category: 'Everyone' , local: true },
  { name: 'color', usage: '/color <colorname|hex>', description: 'Change the color of your username', category: 'Everyone' , examples: ['/color SpringGreen', '/color #ff69b4'], details: 'Twitch accepts the named colors from its own list, and Turbo or Prime users can use any hex value.' },
  { name: 'block', usage: '/block <username>', description: 'Block all messages from a specific user', category: 'Everyone' , examples: ['/block someuser'], details: 'This is a Twitch block, so it applies on twitch.tv too. Use /ignore for a StreamNook-only hide.' },
  { name: 'unblock', usage: '/unblock <username>', description: 'Remove user from block list', category: 'Everyone' },
  { name: 'disconnect', usage: '/disconnect', description: 'Disconnect from the chat server', category: 'Everyone' },
  { name: 'w', aliases: ['whisper'], usage: '/w <username> <message>', description: 'Send a private whisper', category: 'Everyone' , examples: ['/w someuser hey, saw your stream earlier'], details: 'Whispers go through Twitch and land in their whispers inbox. The other person has to allow whispers from strangers.' },
  { name: 'gift', usage: '/gift [quantity]', description: 'Gift subs to this community', category: 'Everyone' , examples: ['/gift', '/gift 5'], details: "Opens this channel's sub page, where you choose Gift a sub. You pay on Twitch, never in StreamNook." },
  { name: 'vote', usage: '/vote <number|option>', description: 'Vote in the running poll', category: 'Everyone' , examples: ['/vote 1', '/vote Yes'], details: 'Pick an option by its number or its name. Works once the poll shows above chat.' },
  
  // Moderator / Utility
  { name: 'ban', aliases: ['b'], usage: '/ban <username> [reason]', description: 'Permanently ban a user', category: 'Moderator' , examples: ['/ban someuser', '/ban someuser spamming links'], details: 'The ban lasts until a moderator lifts it with /unban. The reason is only visible to the mod team.' },
  { name: 'timeout', usage: '/timeout <username> [seconds] [reason]', description: 'Temporarily ban a user', category: 'Moderator' , examples: ['/timeout someuser', '/timeout someuser 600 calm down'], details: 'Seconds default to 600 (ten minutes). The longest timeout Twitch allows is two weeks, 1209600 seconds.' },
  { name: 'unban', aliases: ['u'], usage: '/unban <username>', description: 'Lift a permanent ban or timeout', category: 'Moderator' , examples: ['/unban someuser'], details: 'Clears both a permanent ban and a running timeout.' },
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
  { name: 'subscribers', aliases: ['sub'], usage: '/subscribers', description: 'Restrict chat to subscribers', category: 'Chat Flow' },
  { name: 'subscribersoff', aliases: ['suboff'], usage: '/subscribersoff', description: 'Disable subscribers only mode', category: 'Chat Flow' },
  { name: 'uniquechat', aliases: ['r9kbeta'], usage: '/uniquechat', description: 'Disallow non-unique messages (r9k)', category: 'Chat Flow' },
  { name: 'uniquechatoff', aliases: ['r9kbetaoff'], usage: '/uniquechatoff', description: 'Disable uniquechat mode', category: 'Chat Flow' },
  { name: 'emoteonly', usage: '/emoteonly', description: 'Set chat to emotes only', category: 'Chat Flow' },
  { name: 'emoteonlyoff', usage: '/emoteonlyoff', description: 'Disable emote only mode', category: 'Chat Flow' },
  
  // Engagement
  { name: 'announce', usage: '/announce <message>', description: 'Highlight a message for chat\'s attention', category: 'Engagement' , examples: ['/announce Raffle winners in five minutes'], details: 'The message shows with a colored band in every viewer’s chat, the same as the Twitch /announce.' },
  { name: 'shoutout', usage: '/shoutout <username>', description: 'Share another streamer\'s channel', category: 'Engagement' , examples: ['/shoutout someone'], details: 'Posts an official Twitch shoutout with the channel card. Twitch allows one every two minutes, and the same person once per hour.' },
  { name: 'poll', usage: '/poll', description: 'Open the poll builder. Channel owner only, because Twitch does not let moderators run polls through the API', category: 'Broadcaster' },
  
  // Broadcaster
  { name: 'commercial', usage: '/commercial [seconds]', description: 'Run a commercial for all viewers (30-180s)', category: 'Broadcaster' , examples: ['/commercial', '/commercial 90'], details: 'Runs an ad break for viewers without ad-free status. Length must be 30, 60, 90, 120, 150 or 180 seconds.' },
  { name: 'goal', usage: '/goal', description: 'Make or edit a sub or follower goal', category: 'Broadcaster' , details: 'Opens your Creator Goals page on Twitch, the only place goals can be made or changed.' },
  { name: 'prediction', usage: '/prediction', description: 'Open the prediction builder. Channel owner only, because Twitch does not let moderators run predictions through the API', category: 'Broadcaster' },
  { name: 'raid', usage: '/raid <channel>', description: 'Send viewers to another live channel', category: 'Broadcaster' , examples: ['/raid someone'], details: 'Starts the raid countdown. Everyone watching moves to the target when it fires. Cancel with /unraid.' },
  { name: 'unraid', aliases: ['raidcancel'], usage: '/unraid', description: 'Cancel the active raid', category: 'Broadcaster' },
  { name: 'marker', usage: '/marker [description]', description: 'Add a stream marker', category: 'Broadcaster' , examples: ['/marker', '/marker great play'], details: 'Adds a marker at the current point in the VOD so you can find the moment later in the highlighter.' },

  // StreamNook-native QoL commands (no Twitch-server effect; client-side only)
  { name: 'clearmessages', aliases: ['clearchat'], usage: '/clearmessages', description: 'Hide messages currently shown in this chat. Visual only, not the moderator /clear.', category: 'Everyone' , local: true, details: 'Clears what you see in this pane. Nothing is deleted for anyone else.' },
  { name: 'openurl', usage: '/openurl <url>', description: 'Open a link in your default browser', category: 'Everyone' , local: true, examples: ['/openurl https://example.com'] },
  { name: 'popout', aliases: ['popoutchannel'], usage: '/popout [channel]', description: "Open this (or another) channel's popout chat in your browser", category: 'Everyone' , local: true, examples: ['/popout', '/popout someone'] },
  { name: 'popup', usage: '/popup [channel]', description: 'Open this (or another) channel in a new StreamNook MultiChat window', category: 'Everyone' , local: true, desktopOnly: true, examples: ['/popup', '/popup someone'], details: 'Opens a MultiChat window with that channel, no browser involved.' },
  { name: 'r', usage: '/r <message>', description: 'Reply to the last whisper you received', category: 'Everyone' , local: true, examples: ['/r sure, sounds good'], details: 'Replies to whoever whispered you most recently, so you never have to type their name.' },
  { name: 'clip', aliases: ['createclip'], usage: '/clip', description: 'Create a clip of the last 30 seconds of the current stream', category: 'Everyone' , details: 'Creates a clip through Twitch and copies the link. The clip editor opens in your browser if you want to trim it.' },
  { name: 'chatters', usage: '/chatters', description: 'Show how many chatters are connected right now', category: 'Everyone' , local: true },
  { name: 'ignore', usage: '/ignore <user>', description: 'Hide a user from your chat everywhere (local only)', category: 'Everyone' , local: true, examples: ['/ignore someuser'], details: 'Hides that person in every StreamNook chat. Twitch never knows. Undo it with /unignore.' },
  { name: 'settitle', usage: '/settitle <title>', description: 'Set the stream title (broadcaster only)', category: 'Broadcaster' , examples: ['/settitle Chill Sunday stream'] },
  { name: 'setgame', usage: '/setgame <category>', description: 'Set the stream category by name (broadcaster only)', category: 'Broadcaster' , examples: ['/setgame Just Chatting', '/setgame Elden Ring'], details: 'The name is matched against the Twitch category list, so spell it the way Twitch does.' },
  { name: 'overlay', usage: '/overlay [channel]', description: 'Float this (or another) channel in a see-through always-on-top chat window', category: 'Everyone' , local: true, desktopOnly: true, examples: ['/overlay', '/overlay someone'], details: 'A see-through, always-on-top chat you can park over a game. Ctrl+Alt+N makes every overlay click-through or clickable again.' },
  { name: 'uptime', usage: '/uptime', description: "Show this channel's current stream uptime", category: 'Everyone' , local: true },
  { name: 'song', usage: '/song', description: 'Identify the music currently playing in the stream', category: 'Everyone' , local: true, details: 'Listens to a few seconds of the stream audio and looks the track up. Nothing is posted to chat unless you paste the result.' },
  { name: 'remind', usage: '/remind every 20m <message>', description: 'Auto-post reminders to chat: every N min, after a delay, at a time, at uptime, or on a keyword. Manage with /remind list, remove, enable, disable, clear. Add x3 to repeat.', category: 'Everyone' , local: true, examples: ['/remind every 20m Drink water', '/remind in 10m Check the queue', '/remind at 21:30 Raid time', '/remind on !discord Discord link is in the panel'], details: 'Reminders are posted by StreamNook while the app is open. Manage them with /remind list, remove, enable, disable and clear.' },
  { name: 'usercard', aliases: ['userinfo'], usage: '/usercard <user>', description: "Open a user's StreamNook profile card", category: 'Everyone' , local: true, examples: ['/usercard someuser'] },
  { name: 'user', usage: '/user <user>', description: "Open a user's StreamNook profile card", category: 'Everyone' , local: true, examples: ['/user someuser'] },
  { name: 'banid', usage: '/banid <userID> [reason]', description: 'Ban by Twitch user ID (works on suspended accounts that /ban no longer reaches)', category: 'Moderator' , examples: ['/banid 12345678 ban evasion'] },
  { name: 'nuke', usage: '/nuke <pattern> <action> <past[:future]>', description: 'Mass-action by phrase or /regex/. action = delete | ban | duration. Example: /nuke spam ban 5m:1m. Use /nuke stop to end a running window early.', category: 'Moderator' , examples: ['/nuke spam ban 5m:1m', '/nuke /bad\\s*word/ delete 10m', '/nuke stop'], details: 'Looks back over the past window and acts on every message that matches. A future window keeps catching new matches for that long. The preview above the box shows how many messages and people it would hit before you press Enter.' },
  { name: 'undo', usage: '/undo', description: 'Reverse the most recent /nuke on this channel (bans and timeouts only, deletes are permanent)', category: 'Moderator' , details: 'Unbans or untimes-out everyone the last /nuke hit. Deleted messages cannot come back.' },
  { name: 'endpoll', usage: '/endpoll', description: 'End the running poll and leave the result on screen. Channel owner only', category: 'Broadcaster' },
  { name: 'cancelpoll', aliases: ['deletepoll'], usage: '/cancelpoll', description: 'Cancel the running poll and hide it entirely. Channel owner only', category: 'Broadcaster' },
  { name: 'lockprediction', usage: '/lockprediction', description: 'Stop taking bets on the running prediction. Channel owner only', category: 'Broadcaster' },
  { name: 'cancelprediction', usage: '/cancelprediction', description: 'Cancel the running prediction and refund everyone. Channel owner only', category: 'Broadcaster' },
  { name: 'completeprediction', usage: '/completeprediction <number|title>', description: 'Pay out the running prediction to a winning outcome, by position or name. Channel owner only', category: 'Broadcaster' , examples: ['/completeprediction 1', '/completeprediction Yes'] },
  { name: 'pin', usage: '/pin <id|@user> [duration] | /pin <message>', description: 'Pin a message by its id, pin that person\'s most recent one, or send a message and pin it. Length from 30s to 30m for an id or a user; leave it off to pin until the stream ends', category: 'Moderator' , examples: ['/pin @someuser', '/pin @someuser 5m', '/pin Giveaway starts at the top of the hour'], details: 'Pinned messages sit above chat for everyone until the timer ends or a mod runs /unpin. A message you send with /pin stays pinned until the stream ends.' },
  { name: 'unpin', usage: '/unpin', description: 'Remove the currently pinned message', category: 'Moderator' },
  { name: 'blockterm', usage: '/blockterm <phrase>', description: 'Add a phrase to the channel\'s AutoMod blocked terms', category: 'Moderator' , examples: ['/blockterm free followers'], details: 'AutoMod holds any message with that phrase for moderator review.' },
  { name: 'unblockterm', usage: '/unblockterm <phrase>', description: 'Remove a phrase from the channel\'s AutoMod blocked terms', category: 'Moderator' },
  { name: 'refresh', usage: '/refresh', description: 'Force re-fetch emotes for this channel (busts the local cache)', category: 'Everyone' , local: true, details: 'Refetches Twitch, 7TV, BTTV and FFZ emotes for this channel. Use it when a new emote is not showing yet.' },
  { name: 'reload', usage: '/reload', description: 'Hard refresh: restart the stream and reconnect/reload chat (not just emotes)', category: 'Everyone' , local: true },
  { name: 'spam', usage: '/spam <count> <message>', description: 'Send a message several times in a row (up to 1000). /spam stop ends it early.', category: 'Everyone' , examples: ['/spam 5 Kappa', '/spam 20 GG --delay 200', '/spam stop'], details: 'Lines go out 1 second apart, or the speed set in Settings, Chat, Spam & Pyramid; add --delay <ms> at the end to change it for one run. Faster than Twitch allows for your role gets lines dropped and can silence you for an hour, and moderators may time you out for spam. One run per channel.' },
  { name: 'pyramid', usage: '/pyramid <height> <word>', description: 'Stack a word or emote 1, 2, 3... up to the height and back down (height up to 100). /pyramid stop ends it early.', category: 'Everyone' , examples: ['/pyramid 5 Kappa', '/pyramid 10 PogChamp --delay 300', '/pyramid stop'], details: 'Height N sends 2N-1 lines: /pyramid 5 Kappa sends 9, up to five and back down. Paced like /spam: the speed set in Settings, Chat, Spam & Pyramid, or --delay <ms> at the end. The widest row must fit the 500-character limit for one message. Going faster than Twitch allows for your role gets lines dropped and can silence you for an hour, and moderators may time you out for spam.' },
  // Listed so they autocomplete and show in the menu (they always worked).
  { name: 'me', usage: '/me <message>', description: 'Send your message as an action, in your name color', category: 'Everyone' , examples: ['/me waves'] },
  { name: 'help', usage: '/help [command]', description: 'Show how a command works, or list them all', category: 'Everyone' , local: true, examples: ['/help', '/help timeout'] },
  { name: 'untimeout', usage: '/untimeout <username>', description: 'End a timeout early', category: 'Moderator' , examples: ['/untimeout someuser'] },
  { name: 'warn', usage: '/warn <username> <reason>', description: 'Send a warning they must acknowledge before chatting again', category: 'Moderator' , examples: ['/warn someuser keep it friendly'], details: 'They see your reason and have to accept it before they can type again. Nothing else happens to them.' },
  { name: 'shield', usage: '/shield', description: 'Turn on Shield Mode', category: 'Moderator' , details: 'Shield Mode applies the stricter chat settings you set up for raids and harassment, all at once.' },
  { name: 'shieldoff', usage: '/shieldoff', description: 'Turn off Shield Mode', category: 'Moderator' },
  { name: 'announceblue', usage: '/announceblue <message>', description: 'Announcement with a blue band', category: 'Engagement' , examples: ['/announceblue Stream starts in 5'] },
  { name: 'announcegreen', usage: '/announcegreen <message>', description: 'Announcement with a green band', category: 'Engagement' },
  { name: 'announceorange', usage: '/announceorange <message>', description: 'Announcement with an orange band', category: 'Engagement' },
  { name: 'announcepurple', usage: '/announcepurple <message>', description: 'Announcement with a purple band', category: 'Engagement' },
  { name: 'mod', usage: '/mod <username>', description: 'Make someone a moderator', category: 'Broadcaster' , examples: ['/mod someuser'] },
  { name: 'unmod', usage: '/unmod <username>', description: 'Remove a moderator', category: 'Broadcaster' },
  { name: 'vip', usage: '/vip <username>', description: 'Make someone a VIP', category: 'Broadcaster' , examples: ['/vip someuser'] },
  { name: 'unvip', usage: '/unvip <username>', description: 'Remove a VIP', category: 'Broadcaster' },

  // Viewer info
  { name: 'rules', usage: '/rules [channel]', description: "Show the channel's chat rules", category: 'Everyone' , local: true, examples: ['/rules'] },
  { name: 'followage', aliases: ['followed'], usage: '/followage [user] [channel]', description: 'How long you (or someone) have followed this channel', category: 'Everyone' , local: true, examples: ['/followage', '/followage someuser', '/followage someuser otherchannel'] },
  { name: 'follows', usage: '/follows [channel]', description: "Show the channel's follower count", category: 'Everyone' , local: true },
  { name: 'accountage', usage: '/accountage [user]', description: 'How old your (or someone’s) Twitch account is', category: 'Everyone' , local: true, examples: ['/accountage', '/accountage someuser'] },
  { name: 'viewers', usage: '/viewers [channel]', description: 'Show how many people are watching', category: 'Everyone' , local: true },
  { name: 'founders', usage: '/founders [channel]', description: "List the channel's founders", category: 'Everyone' , local: true, details: 'Founders are the first subscribers of a channel that became a Partner or Affiliate.' },
  { name: 'namehistory', usage: '/namehistory <user>', description: 'Show the names someone used before', category: 'Everyone' , local: true, details: 'Earlier names come from a public name-history service, so very new renames can be missing.' },
  { name: 'logs', usage: '/logs <user>', description: "Show someone's last messages in this chat", category: 'Everyone' , local: true, examples: ['/logs someuser'] },
  { name: 'sharedchat', usage: '/sharedchat', description: 'See who this chat is shared with', category: 'Everyone' , local: true, details: 'For the broadcaster it also opens Stream Manager, where a Shared Chat session starts.' },

  // Chatting
  { name: 'reply', aliases: ['msgreply'], usage: '/reply <@user> <message>', description: "Reply to someone's last message", category: 'Everyone' , examples: ['/reply @someuser good point'] },
  { name: 'say', usage: '/say <text>', description: 'Send text exactly as typed, skipping your custom commands', category: 'Everyone' , examples: ['/say !discord'] },
  { name: 'shrug', usage: '/shrug [message]', description: 'Send your message with ¯\\_(ツ)_/¯ on the end', category: 'Everyone' , examples: ['/shrug', '/shrug no idea'] },
  { name: 'lurk', usage: '/lurk [note]', description: 'Tell chat you are lurking, with a random line each time',category: 'Everyone' , examples: ['/lurk', '/lurk making dinner'] },
  { name: 'squishy', usage: '/squishy', description: 'Post the notsquishY copypasta', category: 'Everyone' },
  { name: 'redeem', usage: '/redeem', description: "Open this channel's rewards", category: 'Everyone' , local: true, desktopOnly: true },
  { name: 'reconnect', aliases: ['rejoin'], usage: '/reconnect', description: 'Reconnect this chat without restarting the stream', category: 'Everyone' , local: true },

  // Your view
  { name: 'unignore', usage: '/unignore <user>', description: 'Show someone you hid with /ignore again', category: 'Everyone' , local: true, examples: ['/unignore someuser'] },
  { name: 'setcolor', usage: '/setcolor <user> <color|off>', description: "Show someone's name in a color you pick (only you see it)", category: 'Everyone' , local: true, examples: ['/setcolor someuser orange', '/setcolor someuser #ff8800', '/setcolor someuser off'] },
  { name: 'setname', usage: '/setname <user> <nickname|off>', description: 'Give someone a nickname only you see', category: 'Everyone' , local: true, examples: ['/setname someuser Bob', '/setname someuser off'] },
  { name: 'localmod', usage: '/localmod', description: 'Show only moderators and the broadcaster in this chat (only for you)', category: 'Everyone' , local: true, desktopOnly: true, details: 'Turn it off with /localmodoff. It lasts until you close StreamNook.' },
  { name: 'localmodoff', usage: '/localmodoff', description: 'Turn off the moderator-only view', category: 'Everyone' , local: true, desktopOnly: true },
  { name: 'localsub', usage: '/localsub', description: 'Show only subscribers and staff in this chat (only for you)', category: 'Everyone' , local: true, desktopOnly: true, details: 'Turn it off with /localsuboff. It lasts until you close StreamNook.' },
  { name: 'localsuboff', usage: '/localsuboff', description: 'Turn off the subscriber-only view', category: 'Everyone' , local: true, desktopOnly: true },
  { name: 'localascii', usage: '/localascii', description: 'Show only messages in plain ASCII text (only for you)', category: 'Everyone' , local: true, desktopOnly: true, details: 'Hides messages with emoji or non-Latin text. Turn it off with /localasciioff.' },
  { name: 'localasciioff', usage: '/localasciioff', description: 'Turn off the ASCII-only view', category: 'Everyone' , local: true, desktopOnly: true },
  { name: 'search', usage: '/search', description: 'Search this chat (same as Ctrl+F)', category: 'Everyone' , local: true, desktopOnly: true },
  { name: 'favorite', usage: '/favorite [channel]', description: 'Add this (or another) channel to your favorites', category: 'Everyone' , local: true },
  { name: 'unfavorite', usage: '/unfavorite [channel]', description: 'Remove a channel from your favorites', category: 'Everyone' , local: true },
  { name: 'join', usage: '/join <channel>', description: "Add a channel's chat to your chat dock", category: 'Everyone' , local: true, desktopOnly: true, examples: ['/join someone'] },
  { name: 'part', usage: '/part [channel]', description: 'Remove a chat from your chat dock', category: 'Everyone' , local: true, desktopOnly: true },

  // Tools
  { name: 'echo', usage: '/echo <text>', description: 'Print a line in this chat that only you see', category: 'Everyone' , local: true },
  { name: 'copy', usage: '/copy <text>', description: 'Copy text to your clipboard', category: 'Everyone' , local: true, details: 'Handy inside your own commands, for example to copy a link you post often.' },
  { name: 'tts', aliases: ['espeak'], usage: '/tts <text>', description: 'Read text aloud on this computer', category: 'Everyone' , local: true },
  { name: 'screenshot', usage: '/screenshot', description: 'Save the current stream picture to Pictures\\StreamNook', category: 'Everyone' , local: true, desktopOnly: true },
  { name: 'addstreamhighlight', usage: '/addstreamhighlight [note]', description: 'Note this moment of the stream in your highlights file', category: 'Everyone' , local: true, examples: ['/addstreamhighlight', '/addstreamhighlight insane clutch'], details: 'Saves the time, the stream uptime, the title and your note. /openstreamhighlights opens the list.' },
  { name: 'openstreamhighlights', usage: '/openstreamhighlights', description: 'Open your stream highlights file', category: 'Everyone' , local: true, desktopOnly: true },
  { name: 'emoteinfo', aliases: ['ffz', 'ffzglobal', 'bttv', 'bttvemotes', 'seventv', 'twitchemotes'], usage: '/emoteinfo', description: 'Count the emotes loaded for this channel, per provider', category: 'Everyone' , local: true, details: '/ffz, /bttv and /seventv show just that provider.' },
  { name: '7tv', usage: '/7tv <search|add|remove|rename> ...', description: "Find 7TV emotes, or edit this channel's set if you are its 7TV editor", category: 'Everyone' , examples: ['/7tv search peepo', '/7tv add 01F6MQ33FG000FFJ97ZB8MWV52', '/7tv rename Okayeg OKEG', '/7tv remove OKEG'], details: 'Adding, removing and renaming need a 7TV account that edits this channel, signed in under Settings.' },
  { name: 'streamlink', aliases: ['livestreamer'], usage: '/streamlink [quality]', description: 'Open this stream in Streamlink', category: 'Everyone' , local: true, desktopOnly: true, examples: ['/streamlink', '/streamlink 720p60'], details: 'Needs Streamlink installed and on your PATH.' },
  { name: 'exporttext', usage: '/exporttext <file> <text>', description: "Add a line of text to a file in StreamNook's exports folder", category: 'Everyone' , local: true, desktopOnly: true, examples: ['/exporttext ideas next stream: speedrun'], details: 'Text files only: the name ends in .txt, .csv, .log, .md or .json, and no ending means .txt.' },
  { name: 'openfile', usage: '/openfile [file]', description: 'Show a file from the exports folder, or open the folder', category: 'Everyone' , local: true, desktopOnly: true, details: 'A file is shown selected in its folder, never run.' },
  { name: 'proc', usage: '/proc <program> [arguments]', description: 'Run a program on this computer, after you confirm it', category: 'Everyone' , local: true, desktopOnly: true, examples: ['/proc notepad'], details: 'A system dialog shows the exact program first. Only run what you typed yourself.' },
  { name: 'setting', usage: '/setting <get|set|reset> <name> [value]', description: 'Read or change a setting by its name', category: 'Everyone' , local: true, examples: ['/setting get chat_design.font_size', '/setting set chat_design.font_size 15', '/setting reset chat_design.font_size'] },
  { name: 'timer', usage: '/timer <delay> <command or message>', description: 'Run a command or send a message after a delay', category: 'Everyone' , local: true, examples: ['/timer 5m Hydrate!', '/timer 30s /slowoff', '/timer stop'], details: 'The delay is like 30s, 5m or 1h30m. /timer stop cancels every waiting timer here.' },
  { name: 'runin', usage: '/runin <channel> <command or message>', description: 'Run a command or send a message in another open chat', category: 'Everyone' , local: true, examples: ['/runin someone hi from the other stream', '/runin someone /uptime'] },
  { name: 'foreach', usage: '/foreach <a,b,c> <command with $>', description: 'Run one command for each item in a list', category: 'Everyone' , local: true, examples: ['/foreach alice,bob /shoutout $'], details: 'Up to 20 items. Messages are paced to stay inside Twitch’s limits.' },
  { name: 'chain', usage: '/chain <command> | <command>', description: 'Run several commands one after another', category: 'Everyone' , local: true, examples: ['/chain /slow 10 | /announce Slow mode for a bit'] },
  { name: 'barrelroll', usage: '/barrelroll', description: 'Do a barrel roll', category: 'Everyone' , local: true },
  { name: 'party', usage: '/party', description: 'Flash party colors over the app for a few seconds', category: 'Everyone' , local: true },
  { name: 'raw', usage: '/raw <IRC line>', description: 'Send a raw IRC line on your chat connection', category: 'Everyone' , local: true, examples: ['/raw PRIVMSG #someone :hello'], details: 'For people who know Twitch IRC. Twitch ignores most raw commands since 2023. JOIN and PART are refused: use /join and /part.' },
  { name: 'fakemsg', usage: '/fakemsg <IRC line>', description: 'Show a made-up chat message only you see (development builds)', category: 'Everyone' , local: true, devOnly: true },
  { name: 'debug-env', usage: '/debug-env', description: 'Show the app version, system and data folders', category: 'Everyone' , local: true },
  { name: 'debug-chat', usage: '/debug-chat', description: 'Show recent chat connection events', category: 'Everyone' , local: true },
  { name: 'debug-eventsub', usage: '/debug-eventsub', description: 'Show whether EventSub is connected', category: 'Everyone' , local: true },
  { name: 'debug-logs', usage: '/debug-logs', description: 'Open the log folder', category: 'Everyone' , local: true, desktopOnly: true },
  { name: 'debug-logging', usage: '/debug-logging <on|off>', description: 'Turn detailed logging on or off', category: 'Everyone' , local: true },
  { name: 'debug-limits', usage: '/debug-limits', description: 'Show your message limits in this chat', category: 'Everyone' , local: true },

  // Moderation
  { name: 'purge', aliases: ['p'], usage: '/purge <username> [reason]', description: "Clear someone's messages with a one-second timeout", category: 'Moderator' , examples: ['/purge someuser'] },
  { name: 'delete', usage: '/delete <message id | @user>', description: 'Delete one message, or someone’s last message', category: 'Moderator' , examples: ['/delete @someuser'] },
  { name: 'automod', aliases: ['automod_approve', 'automod_deny'], usage: '/automod <allow|deny> [message id]', description: 'Allow or deny the newest message AutoMod held', category: 'Moderator' , examples: ['/automod allow', '/automod deny'] },
  { name: 'lowtrust', usage: '/lowtrust', description: 'List monitored and restricted users here, with their messages', category: 'Moderator' , local: true },
  { name: 'modlogs', usage: '/modlogs', description: 'Show or hide the moderation log', category: 'Moderator' , local: true, desktopOnly: true },
  { name: 'requests', usage: '/requests', description: 'Open the reward requests queue', category: 'Moderator' , details: "Opens Twitch's reward queue for this channel in your browser." },

  // Broadcaster
  { name: 'raidbrowser', usage: '/raidbrowser', description: 'List live channels you follow to raid', category: 'Broadcaster' , local: true },
  { name: 'raidsend', usage: '/raidsend', description: 'Send the raid now instead of waiting for the countdown', category: 'Broadcaster' },
  { name: 'massunban', aliases: ['unbanall'], usage: '/massunban [stop]', description: 'Unban everyone in your channel', category: 'Broadcaster' , details: 'Lifts every permanent ban, about three a second, and asks you to confirm first. /massunban stop ends it. Timeouts run out by themselves.' },
  { name: 'leadmod', usage: '/leadmod <username>', description: 'Make a moderator a lead moderator', category: 'Broadcaster' },
  { name: 'unleadmod', usage: '/unleadmod <username>', description: 'Remove a lead moderator', category: 'Broadcaster' },
  { name: 'editor', usage: '/editor <username>', description: 'Make someone an editor of your channel', category: 'Broadcaster' , details: 'Editors can change your title, category, ads and more, so it asks you to confirm.' },
  { name: 'uneditor', usage: '/uneditor <username>', description: 'Remove an editor', category: 'Broadcaster' },
];

/** Alias -> the command it runs (`b` -> `ban`). */
export const COMMAND_ALIASES: ReadonlyMap<string, string> = new Map(
  COMMAND_DEFINITIONS.flatMap((c) => (c.aliases ?? []).map((a) => [a.toLowerCase(), c.name] as [string, string])),
);

/** The command a typed name runs: itself, or what its alias points at. */
export function resolveCommandAlias(name: string): string {
  const n = name.toLowerCase();
  return COMMAND_ALIASES.get(n) ?? n;
}

// /fakemsg only works in development builds, so a release build never lists it.
const DEV_BUILD = !!import.meta.env?.DEV;
for (let i = COMMAND_DEFINITIONS.length - 1; i >= 0; i--) {
  if (COMMAND_DEFINITIONS[i].devOnly && !DEV_BUILD) COMMAND_DEFINITIONS.splice(i, 1);
}

// Every built-in name and alias, so the settings UI can say when a custom
// command shares a name with a built-in. Who wins is customCommandWins().
export const RESERVED_TRIGGERS: ReadonlySet<string> = new Set([
  ...COMMAND_DEFINITIONS.map((c) => c.name.toLowerCase()),
  ...COMMAND_ALIASES.keys(),
]);
['mods', 'vips'].forEach((extra) => (RESERVED_TRIGGERS as Set<string>).add(extra));

// The built-ins custom commands have always yielded to: a custom command with
// one of these names never fires. Every other built-in name and alias came
// later, after people had saved custom commands, so a saved custom command of
// the same name keeps running in its place. Adding a built-in must never take
// over a command someone already saved (a moderator's custom /p would
// otherwise start purging users).
const BUILT_IN_FIRST_TRIGGERS: ReadonlySet<string> = new Set([
  'mods', 'vips', 'color', 'block', 'unblock', 'disconnect', 'w', 'gift', 'vote', 'ban', 'timeout', 'unban',
  'monitor', 'unmonitor', 'restrict', 'unrestrict', 'clear', 'slow', 'slowoff', 'followers', 'followersoff',
  'subscribers', 'subscribersoff', 'uniquechat', 'uniquechatoff', 'emoteonly', 'emoteonlyoff', 'announce',
  'shoutout', 'poll', 'deletepoll', 'commercial', 'goal', 'prediction', 'raid', 'unraid', 'marker',
  'clearmessages', 'openurl', 'popout', 'popup', 'r', 'clip', 'chatters', 'ignore', 'settitle', 'setgame',
  'overlay', 'uptime', 'song', 'remind', 'usercard', 'user', 'banid', 'nuke', 'undo', 'endpoll', 'cancelpoll',
  'lockprediction', 'cancelprediction', 'completeprediction', 'pin', 'unpin', 'blockterm', 'unblockterm',
  'refresh', 'reload', 'me', 'help',
]);

/** Whether a saved custom command named `typed` runs instead of the built-in
 *  of that name (always true when no built-in has it). Same answer on every
 *  platform's chat. */
export function customCommandWins(typed: string): boolean {
  return !BUILT_IN_FIRST_TRIGGERS.has(typed.toLowerCase());
}

/**
 * The slash autocomplete's list for what was typed after `/`: every command
 * whose name starts with it, with the command that typed name actually runs
 * first, so Enter picks it. That is a custom command of exactly that name
 * when it wins (customCommandWins), else the built-in of that name or the one
 * the name is an alias of (`/b` puts `/ban` first). Otherwise list order.
 */
export function rankCommandMatches(commands: CommandDefinition[], typed: string): CommandDefinition[] {
  const query = typed.toLowerCase();
  const aliasOf = COMMAND_ALIASES.get(query);
  const rank = (c: CommandDefinition) => {
    const name = c.name.toLowerCase();
    const custom = c.category === 'Custom';
    if (name === query && custom) return customCommandWins(query) ? 0 : 2;
    if (name === query || (!custom && name === aliasOf)) return 1;
    return 3;
  };
  return commands
    .filter((c) => c.name.toLowerCase().startsWith(query) || (c.category !== 'Custom' && c.name.toLowerCase() === aliasOf))
    .map((c, i) => ({ c, i, r: rank(c) }))
    .sort((a, b) => a.r - b.r || a.i - b.i)
    .map((x) => x.c);
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
