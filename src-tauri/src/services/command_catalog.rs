//! The chat commands a composer offers: the built-in table with each row's
//! lock for the viewer's role in the room, plus the viewer's own slash
//! commands from Settings. The table mirrors `COMMAND_DEFINITIONS` in
//! `src/utils/chatCommands.ts`, which the desktop and Android composers still
//! read; a test fails when the two lists drift apart.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const LOCAL: u8 = 1;
const DESKTOP_ONLY: u8 = 2;
const DEV_ONLY: u8 = 4;

/// One built-in command.
pub struct Builtin {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    /// "Everyone" | "Moderator" | "Chat Flow" | "Engagement" | "Broadcaster"
    pub category: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
    pub details: &'static str,
    pub examples: &'static [&'static str],
    flags: u8,
}

impl Builtin {
    /// Runs inside StreamNook only; nothing goes to the chat server.
    pub fn local(&self) -> bool {
        self.flags & LOCAL != 0
    }
    /// Needs a desktop window or file; never listed on a phone.
    pub fn desktop_only(&self) -> bool {
        self.flags & DESKTOP_ONLY != 0
    }
    /// Works only in development builds.
    pub fn dev_only(&self) -> bool {
        self.flags & DEV_ONLY != 0
    }
}

#[allow(clippy::too_many_arguments)]
const fn cmd(
    name: &'static str,
    aliases: &'static [&'static str],
    category: &'static str,
    usage: &'static str,
    description: &'static str,
    details: &'static str,
    examples: &'static [&'static str],
    flags: u8,
) -> Builtin {
    Builtin { name, aliases, category, usage, description, details, examples, flags }
}

pub static BUILTINS: &[Builtin] = &[
    cmd("mods", &[], "Everyone", "/mods", "Display a list of all chat moderators", "", &[], LOCAL),
    cmd("vips", &[], "Everyone", "/vips", "Display a list of VIPs for this channel", "", &[], LOCAL),
    cmd("color", &[], "Everyone", "/color <colorname|hex>", "Change the color of your username", "Twitch accepts the named colors from its own list, and Turbo or Prime users can use any hex value.", &["/color SpringGreen", "/color #ff69b4"], 0),
    cmd("block", &[], "Everyone", "/block <username>", "Block all messages from a specific user", "This is a Twitch block, so it applies on twitch.tv too. Use /ignore for a StreamNook-only hide.", &["/block someuser"], 0),
    cmd("unblock", &[], "Everyone", "/unblock <username>", "Remove user from block list", "", &[], 0),
    cmd("disconnect", &[], "Everyone", "/disconnect", "Disconnect from the chat server", "", &[], 0),
    cmd("w", &["whisper"], "Everyone", "/w <username> <message>", "Send a private whisper", "Whispers go through Twitch and land in their whispers inbox. The other person has to allow whispers from strangers.", &["/w someuser hey, saw your stream earlier"], 0),
    cmd("gift", &[], "Everyone", "/gift [quantity]", "Gift subs to this community", "Opens this channel's sub page, where you choose Gift a sub. You pay on Twitch, never in StreamNook.", &["/gift", "/gift 5"], 0),
    cmd("vote", &[], "Everyone", "/vote <number|option>", "Vote in the running poll", "Pick an option by its number or its name. Works once the poll shows above chat.", &["/vote 1", "/vote Yes"], 0),
    cmd("ban", &["b"], "Moderator", "/ban <username> [reason]", "Permanently ban a user", "The ban lasts until a moderator lifts it with /unban. The reason is only visible to the mod team.", &["/ban someuser", "/ban someuser spamming links"], 0),
    cmd("timeout", &[], "Moderator", "/timeout <username> [seconds] [reason]", "Temporarily ban a user", "Seconds default to 600 (ten minutes). The longest timeout Twitch allows is two weeks, 1209600 seconds.", &["/timeout someuser", "/timeout someuser 600 calm down"], 0),
    cmd("unban", &["u"], "Moderator", "/unban <username>", "Lift a permanent ban or timeout", "Clears both a permanent ban and a running timeout.", &["/unban someuser"], 0),
    cmd("monitor", &[], "Moderator", "/monitor <username>", "Start monitoring a user's messages", "Monitoring is a Twitch mod tool: the user keeps chatting, but their messages are flagged for the mod team.", &[], 0),
    cmd("unmonitor", &[], "Moderator", "/unmonitor <username>", "Stop monitoring a user's messages", "", &[], 0),
    cmd("restrict", &[], "Moderator", "/restrict <username>", "Start restricting a user's messages", "A restricted user still sees chat as normal, but their messages only reach moderators until someone approves them.", &[], 0),
    cmd("unrestrict", &[], "Moderator", "/unrestrict <username>", "Stop restricting a user's messages", "", &[], 0),
    cmd("clear", &[], "Chat Flow", "/clear", "Wipe the chat history", "", &[], 0),
    cmd("slow", &[], "Chat Flow", "/slow <seconds>", "Set limit on how often users can send messages", "Everyone except mods and VIPs has to wait this many seconds between messages. Turn it off with /slowoff.", &["/slow 10", "/slow 120"], 0),
    cmd("slowoff", &[], "Chat Flow", "/slowoff", "Disable slow mode", "", &[], 0),
    cmd("followers", &[], "Chat Flow", "/followers [duration]", "Restrict chat to followers", "With no duration anyone who follows can talk. A duration means they must have followed for at least that long. Turn it off with /followersoff.", &["/followers", "/followers 10m", "/followers 1d"], 0),
    cmd("followersoff", &[], "Chat Flow", "/followersoff", "Disable followers only mode", "", &[], 0),
    cmd("subscribers", &["sub"], "Chat Flow", "/subscribers", "Restrict chat to subscribers", "", &[], 0),
    cmd("subscribersoff", &["suboff"], "Chat Flow", "/subscribersoff", "Disable subscribers only mode", "", &[], 0),
    cmd("uniquechat", &["r9kbeta"], "Chat Flow", "/uniquechat", "Disallow non-unique messages (r9k)", "", &[], 0),
    cmd("uniquechatoff", &["r9kbetaoff"], "Chat Flow", "/uniquechatoff", "Disable uniquechat mode", "", &[], 0),
    cmd("emoteonly", &[], "Chat Flow", "/emoteonly", "Set chat to emotes only", "", &[], 0),
    cmd("emoteonlyoff", &[], "Chat Flow", "/emoteonlyoff", "Disable emote only mode", "", &[], 0),
    cmd("announce", &[], "Engagement", "/announce <message>", "Highlight a message for chat's attention", "The message shows with a colored band in every viewer’s chat, the same as the Twitch /announce.", &["/announce Raffle winners in five minutes"], 0),
    cmd("shoutout", &[], "Engagement", "/shoutout <username>", "Share another streamer's channel", "Posts an official Twitch shoutout with the channel card. Twitch allows one every two minutes, and the same person once per hour.", &["/shoutout someone"], 0),
    cmd("poll", &[], "Broadcaster", "/poll", "Open the poll builder. Channel owner only, because Twitch does not let moderators run polls through the API", "", &[], 0),
    cmd("commercial", &[], "Broadcaster", "/commercial [seconds]", "Run a commercial for all viewers (30-180s)", "Runs an ad break for viewers without ad-free status. Length must be 30, 60, 90, 120, 150 or 180 seconds.", &["/commercial", "/commercial 90"], 0),
    cmd("goal", &[], "Broadcaster", "/goal", "Make or edit a sub or follower goal", "Opens your Creator Goals page on Twitch, the only place goals can be made or changed.", &[], 0),
    cmd("prediction", &[], "Broadcaster", "/prediction", "Open the prediction builder. Channel owner only, because Twitch does not let moderators run predictions through the API", "", &[], 0),
    cmd("raid", &[], "Broadcaster", "/raid <channel>", "Send viewers to another live channel", "Starts the raid countdown. Everyone watching moves to the target when it fires. Cancel with /unraid.", &["/raid someone"], 0),
    cmd("unraid", &["raidcancel"], "Broadcaster", "/unraid", "Cancel the active raid", "", &[], 0),
    cmd("marker", &[], "Broadcaster", "/marker [description]", "Add a stream marker", "Adds a marker at the current point in the VOD so you can find the moment later in the highlighter.", &["/marker", "/marker great play"], 0),
    cmd("clearmessages", &["clearchat"], "Everyone", "/clearmessages", "Hide messages currently shown in this chat. Visual only, not the moderator /clear.", "Clears what you see in this pane. Nothing is deleted for anyone else.", &[], LOCAL),
    cmd("openurl", &[], "Everyone", "/openurl <url>", "Open a link in your default browser", "", &["/openurl https://example.com"], LOCAL),
    cmd("popout", &["popoutchannel"], "Everyone", "/popout [channel]", "Open this (or another) channel's popout chat in your browser", "", &["/popout", "/popout someone"], LOCAL),
    cmd("popup", &[], "Everyone", "/popup [channel]", "Open this (or another) channel in a new StreamNook MultiChat window", "Opens a MultiChat window with that channel, no browser involved.", &["/popup", "/popup someone"], LOCAL | DESKTOP_ONLY),
    cmd("r", &[], "Everyone", "/r <message>", "Reply to the last whisper you received", "Replies to whoever whispered you most recently, so you never have to type their name.", &["/r sure, sounds good"], LOCAL),
    cmd("clip", &["createclip"], "Everyone", "/clip", "Create a clip of the last 30 seconds of the current stream", "Creates a clip through Twitch and copies the link. The clip editor opens in your browser if you want to trim it.", &[], 0),
    cmd("chatters", &[], "Everyone", "/chatters", "Show how many chatters are connected right now", "", &[], LOCAL),
    cmd("ignore", &[], "Everyone", "/ignore <user>", "Hide a user from your chat everywhere (local only)", "Hides that person in every StreamNook chat. Twitch never knows. Undo it with /unignore.", &["/ignore someuser"], LOCAL),
    cmd("settitle", &[], "Broadcaster", "/settitle <title>", "Set the stream title (broadcaster only)", "", &["/settitle Chill Sunday stream"], 0),
    cmd("setgame", &[], "Broadcaster", "/setgame <category>", "Set the stream category by name (broadcaster only)", "The name is matched against the Twitch category list, so spell it the way Twitch does.", &["/setgame Just Chatting", "/setgame Elden Ring"], 0),
    cmd("overlay", &[], "Everyone", "/overlay [channel]", "Float this (or another) channel in a see-through always-on-top chat window", "A see-through, always-on-top chat you can park over a game. Ctrl+Alt+N makes every overlay click-through or clickable again.", &["/overlay", "/overlay someone"], LOCAL | DESKTOP_ONLY),
    cmd("uptime", &[], "Everyone", "/uptime", "Show this channel's current stream uptime", "", &[], LOCAL),
    cmd("song", &[], "Everyone", "/song", "Identify the music currently playing in the stream", "Listens to a few seconds of the stream audio and looks the track up. Nothing is posted to chat unless you paste the result.", &[], LOCAL),
    cmd("remind", &[], "Everyone", "/remind every 20m <message>", "Auto-post reminders to chat: every N min, after a delay, at a time, at uptime, or on a keyword. Manage with /remind list, remove, enable, disable, clear. Add x3 to repeat.", "Reminders are posted by StreamNook while the app is open. Manage them with /remind list, remove, enable, disable and clear.", &["/remind every 20m Drink water", "/remind in 10m Check the queue", "/remind at 21:30 Raid time", "/remind on !discord Discord link is in the panel"], LOCAL),
    cmd("usercard", &["userinfo"], "Everyone", "/usercard <user>", "Open a user's StreamNook profile card", "", &["/usercard someuser"], LOCAL),
    cmd("user", &[], "Everyone", "/user <user>", "Open a user's StreamNook profile card", "", &["/user someuser"], LOCAL),
    cmd("banid", &[], "Moderator", "/banid <userID> [reason]", "Ban by Twitch user ID (works on suspended accounts that /ban no longer reaches)", "", &["/banid 12345678 ban evasion"], 0),
    cmd("nuke", &[], "Moderator", "/nuke <pattern> <action> <past[:future]>", "Mass-action by phrase or /regex/. action = delete | ban | duration. Example: /nuke spam ban 5m:1m. Use /nuke stop to end a running window early.", "Looks back over the past window and acts on every message that matches. A future window keeps catching new matches for that long. The preview above the box shows how many messages and people it would hit before you press Enter.", &["/nuke spam ban 5m:1m", "/nuke /bad\\s*word/ delete 10m", "/nuke stop"], 0),
    cmd("undo", &[], "Moderator", "/undo", "Reverse the most recent /nuke on this channel (bans and timeouts only, deletes are permanent)", "Unbans or untimes-out everyone the last /nuke hit. Deleted messages cannot come back.", &[], 0),
    cmd("endpoll", &[], "Broadcaster", "/endpoll", "End the running poll and leave the result on screen. Channel owner only", "", &[], 0),
    cmd("cancelpoll", &["deletepoll"], "Broadcaster", "/cancelpoll", "Cancel the running poll and hide it entirely. Channel owner only", "", &[], 0),
    cmd("lockprediction", &[], "Broadcaster", "/lockprediction", "Stop taking bets on the running prediction. Channel owner only", "", &[], 0),
    cmd("cancelprediction", &[], "Broadcaster", "/cancelprediction", "Cancel the running prediction and refund everyone. Channel owner only", "", &[], 0),
    cmd("completeprediction", &[], "Broadcaster", "/completeprediction <number|title>", "Pay out the running prediction to a winning outcome, by position or name. Channel owner only", "", &["/completeprediction 1", "/completeprediction Yes"], 0),
    cmd("pin", &[], "Moderator", "/pin <id|@user> [duration] | /pin <message>", "Pin a message by its id, pin that person's most recent one, or send a message and pin it. Length from 30s to 30m for an id or a user; leave it off to pin until the stream ends", "Pinned messages sit above chat for everyone until the timer ends or a mod runs /unpin. A message you send with /pin stays pinned until the stream ends.", &["/pin @someuser", "/pin @someuser 5m", "/pin Giveaway starts at the top of the hour"], 0),
    cmd("unpin", &[], "Moderator", "/unpin", "Remove the currently pinned message", "", &[], 0),
    cmd("blockterm", &[], "Moderator", "/blockterm <phrase>", "Add a phrase to the channel's AutoMod blocked terms", "AutoMod holds any message with that phrase for moderator review.", &["/blockterm free followers"], 0),
    cmd("unblockterm", &[], "Moderator", "/unblockterm <phrase>", "Remove a phrase from the channel's AutoMod blocked terms", "", &[], 0),
    cmd("refresh", &[], "Everyone", "/refresh", "Force re-fetch emotes for this channel (busts the local cache)", "Refetches Twitch, 7TV, BTTV and FFZ emotes for this channel. Use it when a new emote is not showing yet.", &[], LOCAL),
    cmd("reload", &[], "Everyone", "/reload", "Hard refresh: restart the stream and reconnect/reload chat (not just emotes)", "", &[], LOCAL),
    cmd("spam", &[], "Everyone", "/spam <count> <message>", "Send a message several times in a row (up to 1000). /spam stop ends it early.", "Lines go out 1 second apart, or the speed set in Settings, Chat, Spam & Pyramid; add --delay <ms> at the end to change it for one run. Faster than Twitch allows for your role gets lines dropped and can silence you for an hour, and moderators may time you out for spam. One run per channel.", &["/spam 5 Kappa", "/spam 20 GG --delay 200", "/spam stop"], 0),
    cmd("pyramid", &[], "Everyone", "/pyramid <height> <word>", "Stack a word or emote 1, 2, 3... up to the height and back down (height up to 100). /pyramid stop ends it early.", "Height N sends 2N-1 lines: /pyramid 5 Kappa sends 9, up to five and back down. Paced like /spam: the speed set in Settings, Chat, Spam & Pyramid, or --delay <ms> at the end. The widest row must fit the 500-character limit for one message. Going faster than Twitch allows for your role gets lines dropped and can silence you for an hour, and moderators may time you out for spam.", &["/pyramid 5 Kappa", "/pyramid 10 PogChamp --delay 300", "/pyramid stop"], 0),
    cmd("me", &[], "Everyone", "/me <message>", "Send your message as an action, in your name color", "", &["/me waves"], 0),
    cmd("help", &[], "Everyone", "/help [command]", "Show how a command works, or list them all", "", &["/help", "/help timeout"], LOCAL),
    cmd("untimeout", &[], "Moderator", "/untimeout <username>", "End a timeout early", "", &["/untimeout someuser"], 0),
    cmd("warn", &[], "Moderator", "/warn <username> <reason>", "Send a warning they must acknowledge before chatting again", "They see your reason and have to accept it before they can type again. Nothing else happens to them.", &["/warn someuser keep it friendly"], 0),
    cmd("shield", &[], "Moderator", "/shield", "Turn on Shield Mode", "Shield Mode applies the stricter chat settings you set up for raids and harassment, all at once.", &[], 0),
    cmd("shieldoff", &[], "Moderator", "/shieldoff", "Turn off Shield Mode", "", &[], 0),
    cmd("announceblue", &[], "Engagement", "/announceblue <message>", "Announcement with a blue band", "", &["/announceblue Stream starts in 5"], 0),
    cmd("announcegreen", &[], "Engagement", "/announcegreen <message>", "Announcement with a green band", "", &[], 0),
    cmd("announceorange", &[], "Engagement", "/announceorange <message>", "Announcement with an orange band", "", &[], 0),
    cmd("announcepurple", &[], "Engagement", "/announcepurple <message>", "Announcement with a purple band", "", &[], 0),
    cmd("mod", &[], "Broadcaster", "/mod <username>", "Make someone a moderator", "", &["/mod someuser"], 0),
    cmd("unmod", &[], "Broadcaster", "/unmod <username>", "Remove a moderator", "", &[], 0),
    cmd("vip", &[], "Broadcaster", "/vip <username>", "Make someone a VIP", "", &["/vip someuser"], 0),
    cmd("unvip", &[], "Broadcaster", "/unvip <username>", "Remove a VIP", "", &[], 0),
    cmd("rules", &[], "Everyone", "/rules [channel]", "Show the channel's chat rules", "", &["/rules"], LOCAL),
    cmd("followage", &["followed"], "Everyone", "/followage [user] [channel]", "How long you (or someone) have followed this channel", "", &["/followage", "/followage someuser", "/followage someuser otherchannel"], LOCAL),
    cmd("follows", &[], "Everyone", "/follows [channel]", "Show the channel's follower count", "", &[], LOCAL),
    cmd("accountage", &[], "Everyone", "/accountage [user]", "How old your (or someone’s) Twitch account is", "", &["/accountage", "/accountage someuser"], LOCAL),
    cmd("viewers", &[], "Everyone", "/viewers [channel]", "Show how many people are watching", "", &[], LOCAL),
    cmd("founders", &[], "Everyone", "/founders [channel]", "List the channel's founders", "Founders are the first subscribers of a channel that became a Partner or Affiliate.", &[], LOCAL),
    cmd("namehistory", &[], "Everyone", "/namehistory <user>", "Show the names someone used before", "Earlier names come from a public name-history service, so very new renames can be missing.", &[], LOCAL),
    cmd("logs", &[], "Everyone", "/logs <user>", "Show someone's last messages in this chat", "", &["/logs someuser"], LOCAL),
    cmd("sharedchat", &[], "Everyone", "/sharedchat", "See who this chat is shared with", "For the broadcaster it also opens Stream Manager, where a Shared Chat session starts.", &[], LOCAL),
    cmd("reply", &["msgreply"], "Everyone", "/reply <@user> <message>", "Reply to someone's last message", "", &["/reply @someuser good point"], 0),
    cmd("say", &[], "Everyone", "/say <text>", "Send text exactly as typed, skipping your custom commands", "", &["/say !discord"], 0),
    cmd("shrug", &[], "Everyone", "/shrug [message]", "Send your message with ¯\\_(ツ)_/¯ on the end", "", &["/shrug", "/shrug no idea"], 0),
    cmd("lurk", &[], "Everyone", "/lurk [note]", "Tell chat you are lurking", "", &["/lurk", "/lurk making dinner"], 0),
    cmd("squishy", &[], "Everyone", "/squishy", "Post the notsquishY copypasta", "", &[], 0),
    cmd("redeem", &[], "Everyone", "/redeem", "Open this channel's rewards", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("reconnect", &["rejoin"], "Everyone", "/reconnect", "Reconnect this chat without restarting the stream", "", &[], LOCAL),
    cmd("unignore", &[], "Everyone", "/unignore <user>", "Show someone you hid with /ignore again", "", &["/unignore someuser"], LOCAL),
    cmd("setcolor", &[], "Everyone", "/setcolor <user> <color|off>", "Show someone's name in a color you pick (only you see it)", "", &["/setcolor someuser orange", "/setcolor someuser #ff8800", "/setcolor someuser off"], LOCAL),
    cmd("setname", &[], "Everyone", "/setname <user> <nickname|off>", "Give someone a nickname only you see", "", &["/setname someuser Bob", "/setname someuser off"], LOCAL),
    cmd("localmod", &[], "Everyone", "/localmod", "Show only moderators and the broadcaster in this chat (only for you)", "Turn it off with /localmodoff. It lasts until you close StreamNook.", &[], LOCAL | DESKTOP_ONLY),
    cmd("localmodoff", &[], "Everyone", "/localmodoff", "Turn off the moderator-only view", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("localsub", &[], "Everyone", "/localsub", "Show only subscribers and staff in this chat (only for you)", "Turn it off with /localsuboff. It lasts until you close StreamNook.", &[], LOCAL | DESKTOP_ONLY),
    cmd("localsuboff", &[], "Everyone", "/localsuboff", "Turn off the subscriber-only view", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("localascii", &[], "Everyone", "/localascii", "Show only messages in plain ASCII text (only for you)", "Hides messages with emoji or non-Latin text. Turn it off with /localasciioff.", &[], LOCAL | DESKTOP_ONLY),
    cmd("localasciioff", &[], "Everyone", "/localasciioff", "Turn off the ASCII-only view", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("search", &[], "Everyone", "/search", "Search this chat (same as Ctrl+F)", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("favorite", &[], "Everyone", "/favorite [channel]", "Add this (or another) channel to your favorites", "", &[], LOCAL),
    cmd("unfavorite", &[], "Everyone", "/unfavorite [channel]", "Remove a channel from your favorites", "", &[], LOCAL),
    cmd("join", &[], "Everyone", "/join <channel>", "Add a channel's chat to your chat dock", "", &["/join someone"], LOCAL | DESKTOP_ONLY),
    cmd("part", &[], "Everyone", "/part [channel]", "Remove a chat from your chat dock", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("echo", &[], "Everyone", "/echo <text>", "Print a line in this chat that only you see", "", &[], LOCAL),
    cmd("copy", &[], "Everyone", "/copy <text>", "Copy text to your clipboard", "Handy inside your own commands, for example to copy a link you post often.", &[], LOCAL),
    cmd("tts", &["espeak"], "Everyone", "/tts <text>", "Read text aloud on this computer", "", &[], LOCAL),
    cmd("screenshot", &[], "Everyone", "/screenshot", "Save the current stream picture to Pictures\\StreamNook", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("addstreamhighlight", &[], "Everyone", "/addstreamhighlight [note]", "Note this moment of the stream in your highlights file", "Saves the time, the stream uptime, the title and your note. /openstreamhighlights opens the list.", &["/addstreamhighlight", "/addstreamhighlight insane clutch"], LOCAL),
    cmd("openstreamhighlights", &[], "Everyone", "/openstreamhighlights", "Open your stream highlights file", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("emoteinfo", &["ffz", "ffzglobal", "bttv", "bttvemotes", "seventv", "twitchemotes"], "Everyone", "/emoteinfo", "Count the emotes loaded for this channel, per provider", "/ffz, /bttv and /seventv show just that provider.", &[], LOCAL),
    cmd("7tv", &[], "Everyone", "/7tv <search|add|remove|rename> ...", "Find 7TV emotes, or edit this channel's set if you are its 7TV editor", "Adding, removing and renaming need a 7TV account that edits this channel, signed in under Settings.", &["/7tv search peepo", "/7tv add 01F6MQ33FG000FFJ97ZB8MWV52", "/7tv rename Okayeg OKEG", "/7tv remove OKEG"], 0),
    cmd("streamlink", &["livestreamer"], "Everyone", "/streamlink [quality]", "Open this stream in Streamlink", "Needs Streamlink installed and on your PATH.", &["/streamlink", "/streamlink 720p60"], LOCAL | DESKTOP_ONLY),
    cmd("exporttext", &[], "Everyone", "/exporttext <file> <text>", "Add a line of text to a file in StreamNook's exports folder", "Text files only: the name ends in .txt, .csv, .log, .md or .json, and no ending means .txt.", &["/exporttext ideas next stream: speedrun"], LOCAL | DESKTOP_ONLY),
    cmd("openfile", &[], "Everyone", "/openfile [file]", "Show a file from the exports folder, or open the folder", "A file is shown selected in its folder, never run.", &[], LOCAL | DESKTOP_ONLY),
    cmd("proc", &[], "Everyone", "/proc <program> [arguments]", "Run a program on this computer, after you confirm it", "A system dialog shows the exact program first. Only run what you typed yourself.", &["/proc notepad"], LOCAL | DESKTOP_ONLY),
    cmd("setting", &[], "Everyone", "/setting <get|set|reset> <name> [value]", "Read or change a setting by its name", "", &["/setting get chat_design.font_size", "/setting set chat_design.font_size 15", "/setting reset chat_design.font_size"], LOCAL),
    cmd("timer", &[], "Everyone", "/timer <delay> <command or message>", "Run a command or send a message after a delay", "The delay is like 30s, 5m or 1h30m. /timer stop cancels every waiting timer here.", &["/timer 5m Hydrate!", "/timer 30s /slowoff", "/timer stop"], LOCAL),
    cmd("runin", &[], "Everyone", "/runin <channel> <command or message>", "Run a command or send a message in another open chat", "", &["/runin someone hi from the other stream", "/runin someone /uptime"], LOCAL),
    cmd("foreach", &[], "Everyone", "/foreach <a,b,c> <command with $>", "Run one command for each item in a list", "Up to 20 items. Messages are paced to stay inside Twitch’s limits.", &["/foreach alice,bob /shoutout $"], LOCAL),
    cmd("chain", &[], "Everyone", "/chain <command> | <command>", "Run several commands one after another", "", &["/chain /slow 10 | /announce Slow mode for a bit"], LOCAL),
    cmd("barrelroll", &[], "Everyone", "/barrelroll", "Do a barrel roll", "", &[], LOCAL),
    cmd("party", &[], "Everyone", "/party", "Flash party colors over the app for a few seconds", "", &[], LOCAL),
    cmd("raw", &[], "Everyone", "/raw <IRC line>", "Send a raw IRC line on your chat connection", "For people who know Twitch IRC. Twitch ignores most raw commands since 2023. JOIN and PART are refused: use /join and /part.", &["/raw PRIVMSG #someone :hello"], LOCAL),
    cmd("fakemsg", &[], "Everyone", "/fakemsg <IRC line>", "Show a made-up chat message only you see (development builds)", "", &[], LOCAL | DEV_ONLY),
    cmd("debug-env", &[], "Everyone", "/debug-env", "Show the app version, system and data folders", "", &[], LOCAL),
    cmd("debug-chat", &[], "Everyone", "/debug-chat", "Show recent chat connection events", "", &[], LOCAL),
    cmd("debug-eventsub", &[], "Everyone", "/debug-eventsub", "Show whether EventSub is connected", "", &[], LOCAL),
    cmd("debug-logs", &[], "Everyone", "/debug-logs", "Open the log folder", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("debug-logging", &[], "Everyone", "/debug-logging <on|off>", "Turn detailed logging on or off", "", &[], LOCAL),
    cmd("debug-limits", &[], "Everyone", "/debug-limits", "Show your message limits in this chat", "", &[], LOCAL),
    cmd("purge", &["p"], "Moderator", "/purge <username> [reason]", "Clear someone's messages with a one-second timeout", "", &["/purge someuser"], 0),
    cmd("delete", &[], "Moderator", "/delete <message id | @user>", "Delete one message, or someone’s last message", "", &["/delete @someuser"], 0),
    cmd("automod", &["automod_approve", "automod_deny"], "Moderator", "/automod <allow|deny> [message id]", "Allow or deny the newest message AutoMod held", "", &["/automod allow", "/automod deny"], 0),
    cmd("lowtrust", &[], "Moderator", "/lowtrust", "List monitored and restricted users here, with their messages", "", &[], LOCAL),
    cmd("modlogs", &[], "Moderator", "/modlogs", "Show or hide the moderation log", "", &[], LOCAL | DESKTOP_ONLY),
    cmd("requests", &[], "Moderator", "/requests", "Open the reward requests queue", "Opens Twitch's reward queue for this channel in your browser.", &[], 0),
    cmd("raidbrowser", &[], "Broadcaster", "/raidbrowser", "List live channels you follow to raid", "", &[], LOCAL),
    cmd("raidsend", &[], "Broadcaster", "/raidsend", "Send the raid now instead of waiting for the countdown", "", &[], 0),
    cmd("massunban", &["unbanall"], "Broadcaster", "/massunban [stop]", "Unban everyone in your channel", "Lifts every permanent ban, about three a second, and asks you to confirm first. /massunban stop ends it. Timeouts run out by themselves.", &[], 0),
    cmd("leadmod", &[], "Broadcaster", "/leadmod <username>", "Make a moderator a lead moderator", "", &[], 0),
    cmd("unleadmod", &[], "Broadcaster", "/unleadmod <username>", "Remove a lead moderator", "", &[], 0),
    cmd("editor", &[], "Broadcaster", "/editor <username>", "Make someone an editor of your channel", "Editors can change your title, category, ads and more, so it asks you to confirm.", &[], 0),
    cmd("uneditor", &[], "Broadcaster", "/uneditor <username>", "Remove an editor", "", &[], 0),
];

/// A viewer's own slash command (`chat_commands.user_commands` in Settings).
#[derive(Deserialize, Clone, Debug, Default)]
pub struct UserCommand {
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub expansion: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub require_slash: Option<bool>,
    /// A plain-text trigger that also fires as the message's last word.
    #[serde(default)]
    pub also_match_suffix: Option<bool>,
}

/// One row of the command list as a composer shows it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CatalogRow {
    pub name: String,
    pub usage: String,
    pub description: String,
    /// The table's category, "Custom" for the viewer's own commands.
    pub category: String,
    /// The category as the list's section header reads it.
    pub label: String,
    /// What the row needs when the viewer's role does not allow it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<String>,
}

/// The header a category shows under.
pub fn category_label(category: &str) -> &'static str {
    match category {
        "Everyone" => "Everyone",
        "Moderator" => "Moderator",
        "Chat Flow" => "Chat modes",
        "Engagement" => "Engagement",
        "Broadcaster" => "Broadcaster",
        _ => "Your commands",
    }
}

/// Everyone and custom commands never lock; Broadcaster commands need the
/// broadcaster (Twitch takes them only on the channel owner's own token, so a
/// moderator is refused too); the rest need a moderator or the broadcaster.
pub fn lock_for(category: &str, is_moderator: bool, is_broadcaster: bool) -> Option<&'static str> {
    match category {
        "Everyone" | "Custom" => None,
        "Broadcaster" if !is_broadcaster => Some("Needs the broadcaster in this channel"),
        _ if !is_moderator && !is_broadcaster => Some("Needs moderator status in this channel"),
        _ => None,
    }
}

/// Every whitespace-separated token of `query` appears in the haystack.
fn matches(haystack: &str, query: &str) -> bool {
    let hay = haystack.to_lowercase();
    query.split_whitespace().all(|tok| hay.contains(&tok.to_lowercase()))
}

/// The viewer's enabled slash commands as rows: usage from the template's
/// placeholders, description or the expansion cut to 57 characters.
fn custom_rows(commands: &[UserCommand]) -> Vec<(CatalogRow, String)> {
    commands
        .iter()
        .filter(|c| c.enabled && !c.trigger.trim().is_empty() && c.require_slash != Some(false))
        .map(|c| {
            let mut singles: Vec<u32> = Vec::new();
            let mut rests: Vec<u32> = Vec::new();
            let bytes = c.expansion.as_str();
            let mut rest = bytes;
            while let Some(open) = rest.find('{') {
                let after = &rest[open + 1..];
                let Some(close) = after.find('}') else { break };
                let inner = &after[..close];
                if let Some(n) = inner.strip_suffix('+').and_then(|d| d.parse::<u32>().ok()) {
                    if !rests.contains(&n) {
                        rests.push(n);
                    }
                } else if let Ok(n) = inner.parse::<u32>() {
                    if !singles.contains(&n) {
                        singles.push(n);
                    }
                }
                rest = &after[close + 1..];
            }
            singles.sort_unstable();
            rests.sort_unstable();
            let mut args: Vec<String> = singles.iter().map(|i| format!("<arg{i}>")).collect();
            args.extend(rests.iter().map(|i| format!("<arg{i}...>")));
            if c.expansion.contains("{*}") && rests.is_empty() {
                args.push("<...>".to_string());
            }
            let name = c.trigger.to_lowercase();
            let usage = if args.is_empty() { format!("/{}", c.trigger) } else { format!("/{} {}", c.trigger, args.join(" ")) };
            let description = c
                .description
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(String::from)
                .unwrap_or_else(|| {
                    if c.expansion.chars().count() > 60 {
                        format!("{}...", c.expansion.chars().take(57).collect::<String>())
                    } else {
                        c.expansion.clone()
                    }
                });
            let haystack = format!("{name} {usage} {description}");
            (
                CatalogRow {
                    name,
                    usage,
                    description,
                    category: "Custom".to_string(),
                    label: category_label("Custom").to_string(),
                    lock: None,
                },
                haystack,
            )
        })
        .collect()
}

/// The command list in table order, then the viewer's own commands, filtered
/// by `query`. A phone leaves out desktop-only commands; a release build
/// leaves out development-only ones.
pub fn catalog(phone: bool, is_moderator: bool, is_broadcaster: bool, user_commands: &[UserCommand], query: &str) -> Vec<CatalogRow> {
    let dev = cfg!(debug_assertions);
    let mut rows: Vec<CatalogRow> = BUILTINS
        .iter()
        .filter(|b| !(phone && b.desktop_only()) && (dev || !b.dev_only()))
        .filter(|b| {
            let hay = format!("{} {} {} {} {}", b.name, b.usage, b.description, b.details, b.examples.join(" "));
            matches(&hay, query)
        })
        .map(|b| CatalogRow {
            name: b.name.to_string(),
            usage: b.usage.to_string(),
            description: b.description.to_string(),
            category: b.category.to_string(),
            label: category_label(b.category).to_string(),
            lock: lock_for(b.category, is_moderator, is_broadcaster).map(String::from),
        })
        .collect();
    rows.extend(custom_rows(user_commands).into_iter().filter(|(_, hay)| matches(hay, query)).map(|(row, _)| row));
    rows
}

/// The viewer's own commands out of the settings file's `chat_commands` block.
pub fn user_commands(chat_commands: Option<&Value>) -> Vec<UserCommand> {
    chat_commands
        .and_then(|v| v.get("user_commands"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_lists_every_command_the_page_table_does() {
        let page = include_str!("../../../src/utils/chatCommands.ts");
        let start = page.find("export const COMMAND_DEFINITIONS").expect("page table");
        let end = page[start..].find("\n];").map(|e| start + e).expect("page table end");
        let names: Vec<&str> = page[start..end]
            .lines()
            .filter_map(|l| l.trim().strip_prefix("{ name: '"))
            .filter_map(|l| l.split('\'').next())
            .collect();
        assert!(names.len() > 100);
        let ours: Vec<&str> = BUILTINS.iter().map(|b| b.name).collect();
        assert_eq!(names, ours, "the command table drifted from src/utils/chatCommands.ts");
    }

    #[test]
    fn a_phone_never_lists_desktop_only_commands() {
        let phone = catalog(true, false, false, &[], "");
        assert!(!phone.iter().any(|r| r.name == "streamlink" || r.name == "proc"));
        let desktop = catalog(false, false, false, &[], "");
        assert!(desktop.iter().any(|r| r.name == "streamlink"));
    }

    #[test]
    fn locks_follow_the_role_in_the_room() {
        let viewer = catalog(true, false, false, &[], "ban");
        let ban = viewer.iter().find(|r| r.name == "ban").unwrap();
        assert_eq!(ban.lock.as_deref(), Some("Needs moderator status in this channel"));
        // A moderator runs the moderator commands, but Twitch refuses them the owner's.
        let moderator = catalog(true, true, false, &[], "editor");
        assert_eq!(moderator.iter().find(|r| r.name == "editor").unwrap().lock.as_deref(), Some("Needs the broadcaster in this channel"));
        let moderator_ban = catalog(true, true, false, &[], "ban");
        assert_eq!(moderator_ban.iter().find(|r| r.name == "ban").unwrap().lock, None);
        let owner = catalog(true, false, true, &[], "editor");
        assert_eq!(owner.iter().find(|r| r.name == "editor").unwrap().lock, None);
        let raid = catalog(true, false, false, &[], "raidsend");
        assert_eq!(raid[0].lock.as_deref(), Some("Needs the broadcaster in this channel"));
        assert!(catalog(true, false, false, &[], "mods").iter().all(|r| r.lock.is_none() || r.category != "Everyone"));
    }

    #[test]
    fn every_search_token_must_appear() {
        let rows = catalog(true, true, true, &[], "slow messages");
        assert!(rows.iter().any(|r| r.name == "slow"));
        assert!(catalog(true, true, true, &[], "zzzz nothing").is_empty());
    }

    #[test]
    fn own_commands_follow_with_usage_from_their_placeholders() {
        let commands = vec![
            UserCommand { trigger: "Hype".into(), expansion: "{1} is hyped {2+}".into(), enabled: true, ..Default::default() },
            UserCommand { trigger: "off".into(), expansion: "x".into(), enabled: false, ..Default::default() },
            UserCommand {
                trigger: "plain".into(),
                expansion: "x".into(),
                enabled: true,
                require_slash: Some(false),
                ..Default::default()
            },
        ];
        let rows = catalog(true, false, false, &commands, "");
        let last = rows.last().unwrap();
        assert_eq!(last.name, "hype");
        assert_eq!(last.usage, "/Hype <arg1> <arg2...>");
        assert_eq!(last.label, "Your commands");
        assert!(!rows.iter().any(|r| r.name == "off" || r.name == "plain"));
    }
}
