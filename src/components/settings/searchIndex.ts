export interface SettingsIndexEntry {
  tab: string;
  section: string;
  sectionId?: string;
  title: string;
  /** The row this setting is nested under, for a sub-setting drawn with
   *  SubControl. Search shows it in the trail and lands inside that row. */
  parent?: string;
  description?: string;
}

const tokenize = (s: string): string[] =>
  s.toLowerCase().split(/\s+/).filter(Boolean);

export const searchSettings = (
  query: string,
  limit = 50,
  // Optional whitelist of source tabs. Lets a scoped surface (e.g. the MultiChat
  // settings, which only has the Chat panel) search just its own settings.
  allowTabs?: string[],
): SettingsIndexEntry[] => {
  const tokens = tokenize(query);
  if (tokens.length === 0) return [];

  const scored: { entry: SettingsIndexEntry; score: number }[] = [];

  for (const entry of SETTINGS_INDEX) {
    if (allowTabs && !allowTabs.includes(entry.tab)) continue;
    const title = entry.title.toLowerCase();
    const description = entry.description?.toLowerCase() ?? '';
    const section = entry.section.toLowerCase();
    const parent = entry.parent?.toLowerCase() ?? '';
    const tab = entry.tab.toLowerCase();
    const haystack = `${title} ${parent} ${description} ${section} ${tab}`;

    let allMatch = true;
    let score = 0;
    for (const token of tokens) {
      if (!haystack.includes(token)) {
        allMatch = false;
        break;
      }
      if (title.startsWith(token)) score += 100;
      else if (title.includes(token)) score += 50;
      else if (parent.includes(token)) score += 30;
      else if (section.includes(token)) score += 20;
      else if (description.includes(token)) score += 10;
      else if (tab.includes(token)) score += 5;
    }

    if (allMatch) scored.push({ entry, score });
  }

  scored.sort((a, b) => b.score - a.score);
  return scored.slice(0, limit).map((s) => s.entry);
};

// Manual index for the in-Settings search box. Mirrors the rendered
// <SettingsSection>/<SettingsRow> tree in src/components/settings/. `sectionId`
// must equal the DOM id on the matching <SettingsSection> (or its wrapper) so a
// hit scrolls to it; sections with no id just switch tabs. `description` is part
// of the haystack, so pack synonyms in. Keep this in sync with the command
// palette catalog in src/utils/commandPaletteSources.ts.
export const SETTINGS_INDEX: SettingsIndexEntry[] = [
  // === Player ===
  {
    tab: 'Player',
    section: 'Player Buttons',
    title: 'Player Buttons',
    description: 'Choose which action buttons (follow, subscribe, create clip, identify song, clips & vods, add to multinook, refresh, close) appear in the top-right of the video player. Previously "Player Overlay Buttons".'
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Scroll volume',
    description: 'Scroll the mouse wheel over the video to change volume up or down, louder quieter, one handed mouse only control. Previously "Scroll to change volume".'
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Scroll for About',
    description: "Scroll down over the player (with Shift when the wheel is set to volume) to slide the channel's About panel up over the stream. Previously \"Scroll to open Channel About\"."
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Middle-click mute',
    description: 'Click the scroll wheel or middle mouse button over the player to mute and unmute the stream without the keyboard. Previously "Middle-click to mute".'
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Middle-click opens MultiNook',
    description: 'Click the scroll wheel or middle mouse button on a stream card in Home or a sidebar channel to open it in MultiNook, multi view, watch several streams side by side, alongside the stream you are watching.'
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Step',
    parent: 'Scroll volume',
    description: 'How much one mouse wheel notch changes the volume, from 1 to 25 percent. Also used by the volume up and volume down keyboard shortcuts. Previously "Volume Step".'
  },
  {
    tab: 'Player',
    section: 'Playback',
    sectionId: 'settings-section-video-player',
    title: 'Resume VODs',
    description: 'Reopening a past broadcast picks up at your last position, and Home keeps a Continue Watching row. Previously "Resume VODs where you left off".'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Auto-Switch',
    description: 'When a stream goes offline, automatically switch to another stream.'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Stream switching',
    description: 'When the channel you are watching goes offline, StreamNook picks a new live stream and starts it for you. Previously "Move to another stream when this one ends".'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Destination',
    parent: 'Stream switching',
    description: 'The most-watched live stream in the same category, or one of your followed channels that is live right now. Previously "Where to go next".'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Notice',
    parent: 'Stream switching',
    description: 'A toast names the new channel each time StreamNook switches for you. Previously "Tell me when it switches".'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Follow raids',
    description: 'When the streamer raids another channel, StreamNook jumps there with them (you need to be signed in). Previously "Follow raids automatically".'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Stay in chat',
    description: "Keeps you in the channel's chat when the stream goes offline instead of switching you away. Previously \"Stay in chat after the stream ends\"."
  },
  {
    tab: 'Player',
    section: 'Streaming',
    sectionId: 'settings-section-streaming',
    title: 'Streaming',
    description: 'Codec preferences and stream resolve timing: connection timeout and auto-retry delay.'
  },
  {
    tab: 'Player',
    section: 'Streaming',
    sectionId: 'settings-section-streaming',
    title: 'AV1 & h265',
    description: 'Asks Twitch for AV1 and h265 (HEVC) versions of the stream alongside h264, which some channels offer at better quality for the same bandwidth. Previously "Allow AV1 and h265 streams".'
  },
  {
    tab: 'Player',
    section: 'Streaming',
    sectionId: 'settings-section-streaming',
    title: 'Start timeout',
    description: 'How long StreamNook keeps trying to open a stream before giving up, which helps when a channel has only just gone live. Previously "Keep trying for a set time".'
  },
  {
    tab: 'Player',
    section: 'Streaming',
    sectionId: 'settings-section-streaming',
    title: 'Retry delay',
    description: 'How long to wait between attempts while a stream is not available yet (0 means a single attempt). Previously "Pause between attempts".'
  },
  {
    tab: 'Player',
    section: 'Playback',
    sectionId: 'settings-section-video-player',
    title: 'Playback',
    description: 'Playback behavior: autoplay, live edge, low latency, buffer, quality, volume, aspect ratio, mute. Previously "Video Player".'
  },
  {
    tab: 'Player',
    section: 'Playback',
    sectionId: 'settings-section-video-player',
    title: 'Autoplay',
    description: 'The stream starts playing the moment it loads, with no need to press play. Previously "Play as soon as a stream opens".'
  },
  {
    tab: 'Player',
    section: 'Latency & Buffering',
    sectionId: 'settings-section-latency',
    title: 'Target latency',
    description: 'How far behind the live edge the player rides; lower is closer to live (reopen the stream to apply). Previously "How close to live to stay".'
  },
  {
    tab: 'Player',
    section: 'Latency & Buffering',
    sectionId: 'settings-section-latency',
    title: 'Low latency',
    description: 'Uses the low-latency engine to hold a tight live edge gap smoothly on channels that support it. Previously "Low Latency".'
  },
  {
    tab: 'Player',
    section: 'Latency & Buffering',
    sectionId: 'settings-section-latency',
    title: 'Buffer',
    description: 'How much video the player keeps loaded ahead of playback; more is steadier on a shaky connection but adds delay. Previously "Buffer up to a set length".'
  },
  {
    tab: 'Player',
    section: 'Playback',
    sectionId: 'settings-section-video-player',
    title: 'Start quality',
    description: 'Every stream opens at this quality, and you can change it anytime from the player controls. Previously "Quality to start streams at".'
  },
  {
    tab: 'Player',
    section: 'Picture',
    title: '16:9 window',
    description: "Resizing the window snaps to the video's shape, so you never see black bars around the picture. Previously \"Keep the window at 16:9\"."
  },
  {
    tab: 'Player',
    section: 'Picture',
    title: 'Cinema mode',
    description: 'Letterbox bar color. Cinema Mode uses classic black bars; off matches the bars to your theme background so the video floats. Black bars, color-matched, immersive, pillarbox. Previously "Cinema Mode".'
  },
  {
    tab: 'Player',
    section: 'Playback',
    sectionId: 'settings-section-video-player',
    title: 'Start muted',
    description: 'Every stream opens silent until you unmute it. Previously "Start streams muted".'
  },
  {
    tab: 'Player',
    section: 'Playback',
    sectionId: 'settings-section-video-player',
    title: 'Start volume',
    description: 'The volume every stream opens at before you adjust it. Previously "Starting volume".'
  },
  {
    tab: 'Player',
    section: 'Audio Boost',
    sectionId: 'settings-section-audio-boost',
    title: 'Audio Boost',
    description: 'Compressor and makeup gain to even out loud and quiet moments and make the stream louder without clipping.'
  },
  {
    tab: 'Player',
    section: 'Audio Boost',
    sectionId: 'settings-section-audio-boost',
    title: 'Leveling',
    description: "Evens out the stream's loudness and lifts it, on top of the normal volume slider. Previously \"Turn on Audio Boost\"."
  },
  {
    tab: 'Player',
    section: 'Audio Boost',
    sectionId: 'settings-section-audio-boost',
    title: 'Boost',
    parent: 'Leveling',
    description: 'How much louder to make the stream after compression (volume boost / gain).'
  },
  {
    tab: 'Player',
    section: 'Audio Boost',
    sectionId: 'settings-section-audio-boost',
    title: 'Compressor',
    parent: 'Leveling',
    description: 'Threshold, ratio, knee, attack and release controls for the audio compressor. Previously "Advanced Compressor Controls".'
  },
  {
    tab: 'Player',
    section: 'Song Identification',
    sectionId: 'settings-section-song-id',
    title: 'Song Identification',
    description: 'Identify the music playing in a stream (what song is this). Powers the /song chat command and the player music button; names the track and links it on Spotify, Apple Music, and song.link. Shazam, recognize, now playing.'
  },
  {
    tab: 'Player',
    section: 'Song Identification',
    sectionId: 'settings-section-song-id',
    title: 'Listen time',
    description: 'How many seconds of audio to fingerprint; longer matches more reliably over talking or noise, but the result takes a little longer to appear.'
  },
  {
    tab: 'Player',
    section: 'Song Identification',
    sectionId: 'settings-section-song-id',
    title: 'Retries',
    description: 'If the first listen finds nothing, StreamNook listens again this many times. Previously "Retries when nothing matches".'
  },
  {
    tab: 'Player',
    section: 'Picture',
    title: 'Ambient glow',
    description: 'The player glows with the stream, picking up the color of whatever is on screen, so a neon game and a talk show look different. Glow with the stream, ambient light, backdrop color, bias lighting.'
  },
  {
    tab: 'Player',
    section: 'Latency & Buffering',
    sectionId: 'settings-section-latency',
    title: 'Latency & Buffering',
    description: 'How close to live the player rides, low latency, and how much video it keeps loaded ahead. Delay, live edge, buffering, stutter.'
  },
  {
    tab: 'Player',
    section: 'Picture',
    title: 'Picture',
    description: 'The window shape and what surrounds the video: 16:9 window, cinema mode letterbox bars, ambient glow.'
  },
  // === Theme ===
  {
    tab: 'Theme',
    section: 'Theme',
    title: 'Theme',
    description: 'Pick a color theme or build your own. Themes set the palette only; font and glassiness are chosen separately, so you can use any font with any theme. Signature themes: Frosted Glass, Standard Issue, OLED, Prism (spectral dispersion, refracted light, iridescent, optical).'
  },
  {
    tab: 'Theme',
    section: 'Glassiness',
    title: 'Glassiness',
    description: 'How see-through and frosted every surface is, for every theme. 100% is the signature glass; 0% removes all transparency and blur for a completely flat, solid, opaque look.'
  },
  {
    tab: 'Theme',
    section: 'Font',
    sectionId: 'settings-section-font',
    title: 'Font',
    description: 'Interface font, independent of the theme. Choose any font with any theme. Satoshi, Twitch (Inter), Geist, Manrope, Outfit, Space Grotesk, Serif, System, or a custom font of your own.'
  },
  {
    tab: 'Theme',
    section: 'Font',
    sectionId: 'settings-section-font',
    title: 'Custom font',
    description: 'Use any font you want for the app. Type a name like Poppins, Bebas Neue, or Rubik and it loads automatically, or type the name of a font already installed on this PC.'
  },
  // === Chat ===
  {
    tab: 'Chat',
    section: 'Combined Chat',
    sectionId: 'settings-section-combined-chat',
    title: 'Merged feed',
    description: 'Merge the Twitch, YouTube, Kick and TikTok chat of one streamer into a single feed. Combined chat, multi platform chat, multistream chat, see youtube chat in twitch chat, kick chat together, tiktok chat together, unified chat, cross platform chat, merged chat. Previously "Combine chat across platforms".'
  },
  {
    tab: 'Chat',
    section: 'Combined Chat',
    sectionId: 'settings-section-combined-chat',
    title: 'Suggestions',
    parent: 'Merged feed',
    description: 'Look for a Kick or YouTube channel of the same name when you open a stream; anything found waits behind the + in the chat header. Link channels, same streamer on another platform, connect kick to twitch channel, find youtube channel, auto detect, multistreamer. Previously "Suggest links".'
  },
  {
    tab: 'Chat',
    section: 'Combined Chat',
    sectionId: 'settings-section-combined-chat',
    title: 'Platform marks',
    parent: 'Merged feed',
    description: 'Show a small platform logo on messages from another platform. Platform badge, source icon, which platform, provider logo in chat. Previously "Mark where a message came from".'
  },
  {
    tab: 'Chat',
    section: 'Combined Chat',
    sectionId: 'settings-section-combined-chat',
    title: 'Platforms',
    parent: 'Merged feed',
    description: 'Choose which platforms may join a combined feed. Twitch, YouTube, Kick, TikTok, turn off a platform, exclude platform. Previously "Platforms to include".'
  },
  {
    tab: 'Chat',
    section: 'Chat Placement',
    title: 'Chat Placement',
    description: 'Choose where to display the chat window (right, bottom) or hide it completely.'
  },
  {
    tab: 'Chat',
    section: 'Chat Placement',
    title: 'Position',
    description: 'Dock chat to the left, right, or bottom of the player, or hide it to give the video the whole window. Previously "Where chat sits".'
  },
  {
    tab: 'Chat',
    section: 'Channel Points',
    title: 'Channel Points',
    description: 'Auto-claim the bonus chest on the stream you are watching. Channel points, bonus claim, points automation is a separate opt-in plugin.'
  },
  {
    tab: 'Chat',
    section: 'Channel Points',
    title: 'Bonus chests',
    description: 'Collects the bonus chest on the stream you are watching the moment it appears. Previously "Auto-claim bonus chests".'
  },
  {
    tab: 'Chat',
    section: 'YouTube Chat',
    sectionId: 'settings-section-youtube-chat',
    title: 'Feed',
    description: "Live chat shows everything, while Top chat is YouTube's own filtered view that keeps a very fast chat readable. Previously \"Which chat to read\"."
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Chat Events',
    description: 'What live channel activity shows while you watch: polls, predictions, and channel point redemptions. Turn any off to keep chat clean.'
  },
  {
    tab: 'Chat',
    section: 'Polls & Predictions',
    title: 'Polls',
    description: 'Show a live poll card at the top of chat when the streamer runs one, with the running vote tally.'
  },
  {
    tab: 'Chat',
    section: 'Polls & Predictions',
    title: 'Predictions',
    description: 'Show a live prediction card at the top of chat, with the outcomes and how points are stacking up.'
  },
  {
    tab: 'Chat',
    section: 'Polls & Predictions',
    title: 'Starting a poll or prediction',
    description: 'On your own channel, the chart button beside the message box opens a builder for polls and predictions, with outcomes, a duration, channel-point voting and a live preview. Also reachable with /poll and /prediction.'
  },
  {
    tab: 'Chat',
    section: 'Polls & Predictions',
    title: 'Top card',
    parent: 'Predictions',
    description: 'Pick which card sits on top when a poll and a prediction run at the same time. Previously "When both are running".'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Redemptions',
    description: 'Shows a chat row when someone redeems a reward that does not post its own message, like a no-input reward. Previously "Channel point redemptions".'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Gift sub batches',
    description: "Shows one 'gifting N subs' row with the recipients attached when someone gifts a batch, instead of a row per gift. Previously \"Collapse gift-sub floods\"."
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Clip chat replay',
    description: 'Shows the chat that was live while a clip was recorded, beside the clip. Previously "Chat replay on clips".'
  },
  {
    tab: 'Chat',
    section: 'Chat Logging',
    title: 'Chat Logging',
    description: 'Save chat to plain text files as you watch: one folder per channel, one file per day. Log folder, per-channel filter, timestamps, events and moderation.'
  },
  {
    tab: 'Chat',
    section: 'Chat Logging',
    title: 'Save logs',
    description: 'Writes chat to plain text files as you watch: one folder per channel, one file per day. Previously "Save chat logs".'
  },
  {
    tab: 'Chat',
    section: 'Chat Logging',
    title: 'Folder',
    parent: 'Save logs',
    description: 'Where the files are written. Browse to pick your own folder, Reset to go back to the default. Previously "Log folder".'
  },
  {
    tab: 'Chat',
    section: 'Chat Logging',
    title: 'Channels',
    parent: 'Save logs',
    description: 'Restrict logging to specific channels. Leave empty to log every channel you open. Previously "Only log these channels".'
  },
  {
    tab: 'Chat',
    section: 'Chat Logging',
    title: 'Events',
    parent: 'Save logs',
    description: 'Also log subscriptions, raids, announcements, timeouts, and deleted messages. Previously "Events and moderation".'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Message Layout',
    description: 'How chat messages are laid out: dividers, striped rows, spacing, text size and weight, timestamps, message animation, history opacity.'
  },
  {
    tab: 'Chat',
    section: 'Pinned Messages',
    title: 'Pinned Messages',
    description: 'How a pinned message shows at the top of chat: open or collapsed when you arrive, and whether a collapsed pin is a thin bar or hidden.'
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Names & Badges',
    description: 'How chatter names look: name prefix and style, readable colors, badges and add-on badges, profile pictures, the @ before names, 7TV paints and cosmetics.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Mentions & Replies',
    description: 'How a message that mentions you stands out, the mention and reply colors, the mention sound, how @mentions look (weight, italic, pill, color), inline paint on @mentions, and how a reply shows its parent.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Mention sound',
    description: 'A sound when someone @s you or replies to you, in any chat you have open. Ping, notification sound for mentions, with its own volume.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: '@name style',
    description: 'How @mentions look in chat: weight (regular, medium, bold), italic, plain or pill shape, and their own name color or one color. Bold mentions, italic mentions.'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Dividers',
    description: 'Draws a thin line between messages so a fast chat is easier to scan. Previously "Lines between messages".'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Message animation',
    description: 'Chat animation for how a new message arrives: instant, fade, slide or rise. The entrance motion as each message lands.'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Striped rows',
    description: "Gives every other message a slightly different background, in your theme's colors, so rows are easier to follow. Previously \"Striped message rows\"."
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Spacing',
    description: 'Blank space between one message and the next; more room means fewer messages on screen. Previously "Message spacing".'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Text size',
    description: 'Size of message text, with room to go large when MultiChat fills a whole monitor.'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Activity feed size',
    description: 'Text size for the MultiChat activity feed, where subs, raids, and gifts land.'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Text weight',
    description: 'How heavy the message text is, from light to bold.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Flash',
    parent: 'Highlight',
    description: 'Briefly flashes any message that mentions or replies to you, so you spot it in a fast chat. Previously "Flash when you are mentioned".'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Timestamps',
    description: 'Shows the time each message was sent, next to the name. Previously "Show timestamps".'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Seconds',
    parent: 'Timestamps',
    description: 'Shows seconds too, so 7:42 PM reads 7:42:30 PM. Previously "Include seconds".'
  },
  {
    tab: 'Chat',
    section: 'Pinned Messages',
    title: 'Collapsed',
    description: 'Shows the pinned message as a compact one-line bar when you enter a channel. Previously "Pins start collapsed".'
  },
  {
    tab: 'Chat',
    section: 'Pinned Messages',
    title: 'Style',
    parent: 'Collapsed',
    description: 'Shrinks a collapsed pin to a thin one-line bar you can click to expand, or hides it completely. Previously "Collapsed pin style".'
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Separator',
    parent: 'Name style',
    description: 'The mark between a name and its message, like a colon or an arrow. Previously "Name separator".'
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Name style',
    description: 'How names stand out from the message: plain, or with a bar, chip, brackets, or dot.'
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Prefix color',
    parent: 'Name style',
    description: "Colors the separator, bar, dot, brackets, or chip with the chatter's own color or your theme accent."
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Highlight',
    description: 'The highlight color on messages that mention you. Previously "Mention color".'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Thread color',
    description: 'The color that marks replies in a thread. Previously "Reply thread color".'
  },
  {
    tab: 'Chat',
    section: 'Links',
    title: 'Links',
    description: 'Show rich preview cards when links are posted in chat. Unfurl, embed, trusted sources, shorten links. Previously "Link Previews".'
  },
  {
    tab: 'Chat',
    section: 'Links',
    title: 'Style',
    description: 'Off keeps links as plain text, Card + Link adds a preview card under the link, and Clean shows only the card. Previously "How links show".'
  },
  {
    tab: 'Chat',
    section: 'Links',
    title: 'Short links',
    description: 'Shows each link as a compact label, the site plus a short path, instead of the full raw URL. Previously "Shorten links".'
  },
  {
    tab: 'Chat',
    section: 'Links',
    title: 'Trusted sites',
    parent: 'Style',
    description: 'Links from trusted sites expand into a preview on their own; every other link shows a Load preview button instead.'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Emotes',
    description: 'Customize emote display: inline size, hover preview size, and spacing.'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Size',
    description: 'Scales emotes in chat relative to the text, with 1.00x being the default size. Previously "Emote size".'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Hover size',
    description: 'How large an emote grows when you hover it, in chat and in the emote menu. Previously "Emote hover size".'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Spacing',
    description: 'Space on each side of an emote; go negative to let neighboring emotes overlap. Previously "Emote spacing".'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Chat Input',
    description: 'Quality-of-life behavior for the message composer: duplicate-message bypass and quick send.'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Duplicate sends',
    description: 'Adds an invisible character when you repeat a message, so Twitch does not reject the second send. Previously "Send the same message twice".'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Ctrl+Enter',
    description: 'Sends the message and leaves it in the box, so you can send it again straight away. Previously "Ctrl+Enter sends and keeps the text".'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Spellcheck',
    description: 'Underline misspelled words in the message box and offer corrections when you right-click one. Emotes, chatters, commands and links are left alone. Previously "Check spelling as you type".'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Dictionary',
    parent: 'Spellcheck',
    description: 'Words you have taught the spell checker so it stops flagging them. Add or remove entries here. Previously "Spell check dictionary".'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Hide placeholder',
    description: 'Leave the message box empty instead of prompting you to send a message. Notices you can act on, like read-only or subscriber-only mode, still show. Previously "Hide the placeholder text".'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Hide command button',
    description: 'Removes the slash button from inside the message box. It opens a browsable menu of every command you can run here, with examples; typing / still opens the quick list. Previously "Hide the command button".'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Hide emote button',
    description: 'Remove the smiley from inside the message box. The emote picker is still reachable from its keyboard shortcut and from tab completion. Previously "Hide the emote button".'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Points balance',
    description: 'Show your channel points beside the button next to the message box always, only on hover, or hide the button. Hidden still brings it back whenever a bonus chest is waiting. Previously "Channel points balance".'
  },
  {
    tab: 'Chat',
    section: 'Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'Tab Completion',
    description: 'Tab cycles forward through matching emotes in the chat input, Shift+Tab cycles back. Autocomplete. Previously "Emote Tab Completion".'
  },
  {
    tab: 'Chat',
    section: 'Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'Emotes',
    description: 'Press Tab to complete the emote you are typing, in a carousel or a list. Previously "Complete emote names with Tab".'
  },
  {
    tab: 'Chat',
    section: 'Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'Style',
    parent: 'Emotes',
    description: 'Carousel completes in place and cycles; List shows every emote you can use and narrows as you type. Previously "What Tab opens".'
  },
  {
    tab: 'Chat',
    section: 'Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'Colon list',
    description: 'Type a colon and two letters to see every emote you can use and where it comes from. Previously "Show the emote list when you type :".'
  },
  {
    tab: 'Chat',
    section: 'Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'Matching',
    parent: 'Emotes',
    description: 'Starts With needs the emote to begin with what you typed; Contains matches it anywhere in the name. Previously "How names match".'
  },
  {
    tab: 'Chat',
    section: 'Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'Chatter names',
    description: 'Also cycles through the names of people currently in chat. Previously "Complete chatter names too".'
  },
  {
    tab: 'Chat',
    section: 'Chat Behavior',
    title: 'Chat Behavior',
    description: 'Deleted messages, shared chat messages, the smooth scroll on Resume, and how many messages chat keeps.'
  },
  {
    tab: 'Chat',
    section: 'Docked Chats',
    title: 'Docked Chats',
    description: 'Chats you keep open beside the stream, and how you switch between them.'
  },
  {
    tab: 'Chat',
    section: 'Docked Chats',
    title: 'Menu style',
    description: 'Switch docked chats from a list or a row of tabs under the chat header. Dock tabs, one click per chat.'
  },
  {
    tab: 'Chat',
    section: 'Chat Behavior',
    title: 'Deleted messages',
    description: 'How a message looks once a moderator deletes it or times out or bans its sender: crossed out, dimmed, in italics with a reason tag, left as is, or removed.'
  },
  {
    tab: 'Chat',
    section: 'Chat Behavior',
    title: 'Hide shared chat',
    description: "Hides messages that came from the other channel in a Twitch Shared Chat, so you only see this channel's own chatters. Previously \"Hide shared chat messages\"."
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: '7TV paint',
    description: "Draws a mentioned name in that person's 7TV paint instead of a flat color. Previously \"Paint @mentions inline\"."
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Compact',
    parent: 'Hover size',
    description: 'Show just the emote name on hover instead of the full hint. Previously "Compact emote tooltips".'
  },
  {
    tab: 'Chat',
    section: 'Emote Effects',
    title: 'FFZ effects',
    description: 'Applies FrankerFaceZ modifiers (wide, flips, rainbow, shake) to the emote before them, the way FFZ does. Previously "FFZ emote effects".'
  },
  {
    tab: 'Chat',
    section: 'Emote Effects',
    title: 'BetterTTV modifiers',
    description: 'Applies BetterTTV modifiers (w! wide, h! and v! flips, c! cursed, p! party, s! shake) to the emote after them, the way BetterTTV does. Previously "BetterTTV emote modifiers".'
  },
  {
    tab: 'Chat',
    section: 'Emote Effects',
    title: 'Giant emotes',
    description: 'Draws the last emote of a "Gigantify an Emote" power-up message at 4x below the message, like Twitch does.'
  },
  {
    tab: 'Chat',
    section: 'Hidden Messages',
    sectionId: 'settings-section-chat-filters',
    title: 'Known bots',
    description: 'Hide chat messages from StreamElements, Nightbot, Moobot and other well-known bots in every channel. Local only; nothing is sent to the platform. Previously "Hide known bots".'
  },
  {
    tab: 'Chat',
    section: 'Hidden Messages',
    sectionId: 'settings-section-chat-filters',
    title: 'Everywhere',
    description: 'Users whose messages never appear in your chat, on any platform. Add names here or from a user card in chat. Previously "Hidden everywhere".'
  },
  {
    tab: 'Chat',
    section: 'Hidden Messages',
    sectionId: 'settings-section-chat-filters',
    title: 'Per channel',
    description: 'Users hidden only in a single channel, added from their user card while watching. Remove a name to see their messages again. Previously "Hidden in one channel".'
  },
  {
    tab: 'Chat',
    section: 'Fullscreen Chat',
    title: 'Overlay',
    description: 'Float the chat panel over the video while the player is fullscreen, as a translucent column. Opacity, width, side, and hide-with-controls options. Previously "Chat over fullscreen video".'
  },
  {
    tab: 'Chat',
    section: 'Hidden Messages',
    sectionId: 'settings-section-chat-filters',
    title: 'Ignored phrases',
    description: 'Hide any message containing a word, phrase or regular expression, in every channel. Evaluated before the message reaches chat.'
  },
  {
    tab: 'Chat',
    section: 'Filters & Search',
    sectionId: 'settings-section-chat-query',
    title: 'Filters',
    description: 'Saved message filters (mods only, subs only, mentions, redemptions, links, custom expressions). Apply one to a chat pane from its header. Previously "Saved filters".'
  },
  {
    tab: 'Chat',
    section: 'Filters & Search',
    sectionId: 'settings-section-chat-query',
    title: 'Search depth',
    description: 'How many recent messages per channel Ctrl+F can search. Kept in the Rust backend, not in the chat view. Previously "How far back search reaches".'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'Mode',
    description: 'Fold a run of the same message into one row with a count like x12, just number them in place, or leave repeats alone. Helps when a copypasta wave or one emote floods chat. Previously "When a message repeats".'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'Match',
    parent: 'Mode',
    description: 'Whether nearly-identical messages count as repeats, ignoring capitals, extra spaces and trailing punctuation, or only exactly identical ones. Previously "How closely they must match".'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'Minimum',
    parent: 'Mode',
    description: 'How many copies before the counter shows, how long copies keep joining the same run, and what colour the counter is. Previously "Repeat counter threshold, window and colour".'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'Mod & VIP exemption',
    parent: 'Mode',
    description: 'Keep messages from the broadcaster, moderators and VIPs on their own rows, and optionally show everything in channels you moderate so nothing you might need to action is hidden. Previously "Never fold mods, VIPs or the streamer".'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Messages first',
    description: 'Whether clicking someone in chat opens their recent messages first or their profile first. The card switches between the two either way. Previously "Open on their messages".'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Join date',
    description: 'Pick the rows the user card displays: joined Twitch, following since, channels they follow, chatters, past subscriber, last live, how long ago, and the 7TV profile link. Hide fields you never read. Previously "Which details show on the card".'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: '7TV notices',
    description: 'Shows a chat notice when a mod adds, removes, or renames a 7TV emote in the channel. Previously "7TV emote update notices".'
  },
  {
    tab: 'Chat',
    section: 'Chat Behavior',
    title: 'Smooth resume',
    description: 'Animates the scroll back to the bottom when you click Resume; auto-scroll for new messages stays instant. Previously "Smooth scroll on Resume".'
  },
  {
    tab: 'Chat',
    section: 'Chat Behavior',
    title: 'Scrollback',
    description: 'How many messages each chat keeps on screen to scroll back through; more history uses more memory. Previously "Message buffer".'
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Paint shadows',
    description: 'Some paints stack several drop shadows for readability; keep them all, just one, or none if names look too noisy. Previously "Paint drop shadows". Visual controls for 7TV-rendered usernames (paints), including drop shadows.'
  },
  {
    tab: 'Chat',
    section: 'Highlight Appearance',
    title: 'Highlight Appearance',
    description: 'How highlights look across every highlight type: phrases, usernames, badges, and built-in events. Display style, tint opacity, flash window title.'
  },
  {
    tab: 'Chat',
    section: 'Highlight Appearance',
    title: 'Style',
    description: 'How a highlighted message is emphasized (standard tint and other styles). Previously "Display style".'
  },
  {
    tab: 'Chat',
    section: 'Highlight Appearance',
    title: 'Opacity',
    parent: 'Style',
    description: 'Strength of the highlight tint behind a matched message. Previously "Tint opacity".'
  },
  {
    tab: 'Chat',
    section: 'Highlight Appearance',
    title: 'Title flash',
    description: 'Flash the window title bar when a highlight fires while the app is in the background. Previously "Flash window title when unfocused".'
  },
  {
    tab: 'Chat',
    section: 'Highlight Phrases',
    title: 'Highlight Phrases',
    description: 'Flash chat messages that match specific words, names, or patterns. Mentions of your own name and replies to you are always highlighted; these are extra.'
  },
  {
    tab: 'Chat',
    section: 'Event Highlights',
    title: 'Event Highlights',
    description: 'Auto-highlight messages from event types: first-time chatters, returning chatters, your own messages, and raid announcements. Previously "Built-in Event Highlights".'
  },
  {
    tab: 'Chat',
    section: 'Event Highlights',
    title: 'First-time chatters',
    description: "Highlight a chatter's very first message in the channel."
  },
  {
    tab: 'Chat',
    section: 'Event Highlights',
    title: 'Returning chatters',
    description: 'Highlight the first message from a returning chatter.'
  },
  {
    tab: 'Chat',
    section: 'Event Highlights',
    title: 'Your own messages',
    description: 'Highlight messages you send.'
  },
  {
    tab: 'Chat',
    section: 'Event Highlights',
    title: 'Raid announcements',
    description: 'Highlight raid announcement messages.'
  },
  {
    tab: 'Chat',
    section: 'Username Highlights',
    title: 'Username Highlights',
    description: 'Always highlight messages from specific users by login. Match is case-insensitive.'
  },
  {
    tab: 'Chat',
    section: 'Badge Highlights',
    title: 'Badge Highlights',
    description: 'Highlight every message from users carrying a specific Twitch badge. Use name/version (e.g. moderator/1) or name/* to match any version.'
  },
  {
    tab: 'Chat',
    section: 'Custom Commands',
    title: 'Custom Commands',
    description: 'Define your own chat commands with expansions and auto-fill.'
  },
  {
    tab: 'Chat',
    section: 'Reminders',
    sectionId: 'reminders',
    title: 'Reminders',
    description: 'Auto-post a message into chat to remind the streamer: every N minutes, after a delay, at a clock time, at a stream uptime, or when a keyword appears. Repeat it several times so it lands. Also settable from chat with /remind.'
  },
  {
    tab: 'Chat',
    section: 'Reminders',
    sectionId: 'reminders',
    title: 'Auto message timer',
    description: 'Schedule a recurring or one-off chat message with the /remind command.'
  },
  {
    tab: 'Chat',
    section: 'Nicknames',
    title: 'Nicknames',
    description: "Nicknames you've set for individual chatters. Only visible to you. Set or clear a nickname from the user's profile card in chat. Previously \"User Overrides\"."
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'Clock',
    parent: 'Timestamps',
    description: '12-hour or 24-hour timestamps next to chat messages.'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Animation',
    description: 'Play animated emotes always, only on hover, or never (first frame). Never is lightest on the GPU. Previously "Animate emotes".'
  },
  {
    tab: 'Chat',
    section: 'Message Layout',
    title: 'History opacity',
    description: 'Dim the scrollback loaded on join so live messages stand out.'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    title: 'Pronouns',
    description: 'Show pronouns from pronouns.alejo.io on the user card. Off by default; one small third-party request per person.'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    title: 'Private notes',
    description: 'A private note on each user, kept across renames, shown on the user card.'
  },
  {
    tab: 'Chat',
    section: 'Custom Sounds',
    sectionId: 'settings-section-custom-sounds',
    title: 'Custom highlight sounds',
    description: 'Use your own audio files as highlight sounds; they appear in every highlight sound picker.'
  },
  {
    tab: 'Chat',
    section: 'Image Uploads',
    sectionId: 'settings-section-image-uploads',
    title: 'Paste upload',
    description: 'Paste a screenshot into the chat box and StreamNook uploads it to a host you pick (nuuls, catbox, Litterbox, uguu or your own) and inserts the link. Previously "Paste images to upload".'
  },
  {
    tab: 'Chat',
    section: 'Image Uploads',
    sectionId: 'settings-section-image-uploads',
    title: 'Image host',
    description: 'Choose where pasted images are uploaded: i.nuuls.com, catbox.moe, Litterbox (72 h), uguu.se (3 h), or a custom multipart uploader. Includes a one-click test upload.'
  },
  {
    tab: 'Chat',
    section: 'Chat Placement',
    title: 'Hover reveal',
    parent: 'Position',
    description: 'Keep chat tucked against its left or right edge and slide it out when you move toward that side. Reveal on hover, auto hide chat, peek, slide out.'
  },
  {
    tab: 'Chat',
    section: 'Fullscreen Chat',
    title: 'Fullscreen Chat',
    description: 'Keep chatting while the stream fills the screen: chat floats over fullscreen video as a translucent column.'
  },
  {
    tab: 'Chat',
    section: 'Fullscreen Chat',
    title: 'Auto-hide',
    parent: 'Overlay',
    description: 'The fullscreen chat column fades out with the player controls and comes back when you move the mouse. Hide with the player controls.'
  },
  {
    tab: 'Chat',
    section: 'Fullscreen Chat',
    title: 'Opacity',
    parent: 'Overlay',
    description: 'How solid the fullscreen chat column is; lower lets more video show through. Overlay opacity, transparency, see-through.'
  },
  {
    tab: 'Chat',
    section: 'Fullscreen Chat',
    title: 'Width',
    parent: 'Overlay',
    description: 'Fullscreen chat column width, 240 to 640 pixels. Overlay width.'
  },
  {
    tab: 'Chat',
    section: 'Fullscreen Chat',
    title: 'Side',
    parent: 'Overlay',
    description: 'Which side the fullscreen chat column floats on; Auto follows the chat placement. Overlay side, left, right.'
  },
  {
    tab: 'Chat',
    section: 'YouTube Chat',
    sectionId: 'settings-section-youtube-chat',
    title: 'Currency',
    description: 'Show Super Chat amounts converted to one currency, or as sent. Super Chat currency, money, dollars, euros, convert.'
  },
  {
    tab: 'Chat',
    section: 'Polls & Predictions',
    title: 'Polls & Predictions',
    description: 'The live poll and prediction cards at the top of chat: show or hide each, start polls collapsed, and which card sits on top.'
  },
  {
    tab: 'Chat',
    section: 'Polls & Predictions',
    title: 'Collapsed',
    parent: 'Polls',
    description: 'Opens live polls as their header bar instead of expanded, so a poll never takes over the top of chat. Polls start collapsed, minimize poll.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Style',
    description: 'How subs, gifts, bits and milestones look: tinted cards, a plain row with a ring (outline), or a plain row. How event rows look, event style.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Outline color',
    parent: 'Style',
    description: 'The ring color for outlined event rows. Leave it on the default to follow the theme accent.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Glint',
    description: 'A short highlight when an event row lands: a sheen, a pulse, or a spark around the edge. Event glint, shimmer, animation.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Loop',
    parent: 'Glint',
    description: 'Keep the event glint going instead of playing it once. Keep the glint going, repeat, loop animation.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Cheers',
    description: 'Bits cheers as their own card with the cheer gem, or as an ordinary message with the cheermotes inline. Bits cheers, cheer display.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Wording',
    description: 'Your own sentence for each kind of event, with tokens like {username} and {months}. Event wording, custom text, template.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Platforms',
    description: 'Turn event kinds off per platform: Twitch, YouTube, Kick, TikTok. Events by platform, hide subs, hide gifts.'
  },
  {
    tab: 'Chat',
    section: 'Chat Logging',
    title: 'Timestamps',
    parent: 'Save logs',
    description: 'Start each chat log line with the time it was sent. Log timestamps.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Volume',
    parent: 'Mention sound',
    description: 'How loud the mention sound plays. Mention sound volume, quieter, louder.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Replies to you',
    parent: 'Mention sound',
    description: 'A reply to one of your messages plays the mention sound too, even without an @. Reply sound.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Weight',
    parent: '@name style',
    description: 'How heavy an @name reads inside a message: regular, medium or bold. @name weight, bold mentions.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Italic',
    parent: '@name style',
    description: 'Whether an @name slants: like the message, never, or always. @name italic, /me.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Shape',
    parent: '@name style',
    description: 'Plain colored text, or a soft pill behind the name. @name shape, pill mention.'
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Color',
    parent: '@name style',
    description: "Each person's own name color, or one color for every @name. @name color, mention text color."
  },
  {
    tab: 'Chat',
    section: 'Mentions & Replies',
    title: 'Context',
    description: 'How a reply shows the message it answers: a context line above it, an @name at the start, or nothing. How replies show their parent, reply style, replying to.'
  },
  {
    tab: 'Chat',
    section: 'Links',
    title: 'Color',
    description: 'The color of links in chat; leave it on the default to follow the theme. Link color, hyperlink, url color.'
  },
  {
    tab: 'Chat',
    section: 'Links',
    title: 'Underline',
    description: 'Underline links in chat, or leave them colored without a line. Underline links.'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Emoji style',
    description: 'Which set draws the emoji in messages: Apple, Google, Twitter, Facebook, or your system emoji.'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Personal emotes',
    description: "Emotes from a chatter's own 7TV personal set; off shows the text they typed. 7TV personal emotes."
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'GIFs',
    description: 'Show Twitch GIFs posted by Tier 2 and Tier 3 subscribers, or swap each for a chip you click to reveal. Show GIFs in chat.'
  },
  {
    tab: 'Chat',
    section: 'Emote Effects',
    title: 'Emote Effects',
    description: 'Emote modifiers from FFZ and BetterTTV, and Twitch giant power-up emotes.'
  },
  {
    tab: 'Chat',
    section: 'Emote Effects',
    title: 'Position',
    parent: 'Giant emotes',
    description: 'Where a giant emote sits: under the message on the left, centered, on the right, or in the text. Where the giant emote sits, align.'
  },
  {
    tab: 'Chat',
    section: 'Hidden Messages',
    sectionId: 'settings-section-chat-filters',
    title: 'Bot commands',
    description: 'Hide messages that are bot commands, so a chat full of !drops and !uptime reads as a chat. Hide commands, exclamation mark.'
  },
  {
    tab: 'Chat',
    section: 'Hidden Messages',
    sectionId: 'settings-section-chat-filters',
    title: 'Patterns',
    parent: 'Bot commands',
    description: 'Which commands get hidden: a prefix hides every command starting with it, an exact pattern only that word. Command patterns.'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'Window',
    parent: 'Mode',
    description: 'How long copies keep joining the same run before the next one starts fresh. Group copies sent within, seconds.'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'Counter color',
    parent: 'Mode',
    description: 'The color of the little x12 counter next to a folded message. Counter colour.'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'Moderated channels',
    parent: 'Mode',
    description: "Turns folding off wherever you're a mod, so a hidden copy is never a message you needed to action. Show everything in channels you moderate."
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Following since',
    description: 'Show when they followed this channel on the user card. Followage, follow age.'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Follow count',
    description: 'Show how many channels this person follows on the user card. Channels they follow.'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Chatters',
    description: "Show how many people are in this person's own chat right now. Chatter count."
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Past subscriber',
    description: 'Show total months subscribed for people who are not subscribed now.'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Last live',
    description: 'Show when they last streamed on the user card.'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Relative dates',
    description: 'Adds a plain-English age next to dates, so "Mar 3, 2019" also reads "(6y ago)". Show how long ago.'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: '7TV profile link',
    description: 'A 7TV chip next to their name that opens their 7TV profile.'
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Readable colors',
    description: "Nudges a chatter's color lighter on a dark theme or darker on a light one so it stands out. Keep name colors readable, contrast, dark names."
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Badges',
    description: "The platform's own badges next to names: moderator, subscriber, VIP and the rest. Show badges."
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Badge size',
    description: 'How big badges draw, relative to the text. Badge scale.'
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Add-on badges',
    description: 'Badges from 7TV, FFZ, Chatterino, Homies and the other badge services, plus StreamNook membership badges. Third-party badges.'
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Services',
    parent: 'Add-on badges',
    description: 'Turn individual badge services off: 7TV, FFZ, Chatterino, Homies and the rest. Badge services, badge providers.'
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: 'Profile pictures',
    description: "On YouTube and TikTok, the chatter's picture leads their message. Profile pictures beside names, avatars, pfp."
  },
  {
    tab: 'Chat',
    section: 'Names & Badges',
    title: '@ prefix',
    description: 'Writes every name as @name. @ before names, at sign.'
  },
  {
    tab: 'Chat',
    section: 'Event Highlights',
    title: 'Style',
    parent: 'First-time chatters',
    description: "A tinted wash with a bar down the left, or a ring around a first-time chatter's message. First-time look."
  },
  {
    tab: 'Chat',
    section: 'Event Highlights',
    title: 'Ring fill',
    parent: 'First-time chatters',
    description: 'A faint color-matched fill inside the first-time ring. Fill inside the ring.'
  },
  {
    tab: 'Chat',
    section: 'Event Highlights',
    title: 'Glint',
    parent: 'First-time chatters',
    description: 'A short highlight as a first-time message lands: a sheen, a pulse, or a spark around the edge. First-time glint.'
  },
  {
    tab: 'Chat',
    section: 'Event Highlights',
    title: 'Loop',
    parent: 'First-time chatters',
    description: 'Keep the first-time glint going instead of playing it once. Keep the glint going.'
  },
  {
    tab: 'Chat',
    section: 'Image Uploads',
    sectionId: 'settings-section-image-uploads',
    title: 'Custom host',
    parent: 'Paste upload',
    description: 'Upload address, file field name, extra form fields and where the link sits in the reply, for your own image host. Host details, self-hosted uploader.'
  },
  {
    tab: 'Chat',
    section: 'Image Uploads',
    sectionId: 'settings-section-image-uploads',
    title: 'Test upload',
    parent: 'Paste upload',
    description: 'Sends a 1-pixel test image to the host you picked and shows the link it came back with. Try it.'
  },
  // === Moderation ===
  {
    tab: 'Moderation',
    section: 'Moderation Actions',
    title: 'Timeout presets',
    description: 'Custom timeout durations on the message hover dock and drag dial.'
  },
  {
    tab: 'Moderation',
    section: 'Streamer Mode',
    sectionId: 'settings-section-streamer-mode',
    title: 'Activation',
    description: 'Hide viewer counts, link previews, restricted users and highlight sounds while live. Auto-detects OBS, Streamlabs, XSplit, Twitch Studio and vMix. Previously "Streamer mode".'
  },
  {
    tab: 'Moderation',
    section: 'AutoMod',
    title: 'AutoMod held messages',
    description: 'Messages AutoMod is holding show in a strip above the chat input for moderators, with Allow and Deny. Restricted and monitored chatters are labelled in chat.'
  },
  {
    tab: 'Moderation',
    section: 'Reasons',
    sectionId: 'settings-section-mod-reasons',
    title: '/nuke reason',
    description: 'The reason written against every ban and timeout that /nuke issues, shown in the channel mod view. Defaults to "/nuke". Previously "Reason for /nuke".'
  },
  {
    tab: 'Moderation',
    section: 'Reasons',
    sectionId: 'settings-section-mod-reasons',
    title: 'Saved reasons',
    description: 'Your list of ban and timeout reasons, offered when you moderate from a user card or a message. The first one is prefilled for you.'
  },
  {
    tab: 'Moderation',
    section: 'Moderation Actions',
    title: 'Moderation Actions',
    description: 'Choose how to moderate: classic click buttons, drag a chat message into an action bucket (ban/timeout/delete/whisper/profile), or both. Also called Action Style. Includes Drag Style and Pin Action placement.'
  },
  {
    tab: 'Moderation',
    section: 'Moderation Actions',
    title: 'Method',
    description: 'Buttons show delete, timeout, and ban when you hover a message; Drag lets you pick a message up and drop it on a color-coded action bucket; Both gives you both. Previously "How you act on a message".'
  },
  {
    tab: 'Moderation',
    section: 'Moderation Actions',
    title: 'Bucket position',
    parent: 'Method',
    description: 'Beside chat puts a column of bigger tiles to the left of chat, clear of the player controls; Above chat puts a compact cluster right above the message for when space is tight. Previously "Where the drop buckets appear".'
  },
  {
    tab: 'Moderation',
    section: 'Moderation Actions',
    title: 'Pin bucket',
    parent: 'Method',
    description: 'Moderators always get a Pin button beside Copy on a message; this adds a Pin tile to the drag buckets as well. Previously "Pin from the drag gesture too".'
  },
  {
    tab: 'Moderation',
    section: 'Mod Rooms',
    title: 'Mod Rooms',
    description: 'Private, encrypted chat rooms for the mod teams of channels you moderate. Manage the one-time Twitch consent: see which account is connected, connect, or disconnect.'
  },
  {
    tab: 'Moderation',
    section: 'Mod Rooms',
    title: 'Connection',
    description: 'Which account mod rooms are connected as. Connect the one-time consent or disconnect to switch accounts or revoke access.'
  },
  {
    tab: 'Moderation',
    section: 'Mod Logs',
    title: 'Mod Logs',
    description: 'Control moderation action visibility.'
  },
  {
    tab: 'Moderation',
    section: 'Mod Logs',
    title: 'Log panel',
    description: 'Adds a panel inside chat that lists recent timeouts, bans, and deleted messages as they happen. Previously "Show the mod log beside chat".'
  },
  {
    tab: 'Moderation',
    section: 'Removed Messages',
    title: 'Removed Messages',
    description: 'Control how removed messages are shown in chat. Previously "Message Visibility".'
  },
  {
    tab: 'Moderation',
    section: 'Removed Messages',
    title: 'Action notices',
    description: 'Add an extra system row to chat when a mod times someone out, bans, or deletes a message (on top of the strikethrough you already see). Previously "Announce mod actions inline".'
  },
  {
    tab: 'Moderation',
    section: 'Removed Messages',
    title: 'Hide strikethrough',
    description: 'Banned, timed-out, and deleted messages stay exactly as they were, with no line through them. Previously "Hide strikethrough on removed messages".'
  },
  {
    tab: 'Moderation',
    section: 'Log Highlights',
    title: 'Log Highlights',
    description: 'Color-code mod-log entries by severity. Choose how the highlight shows, then customize any category color.'
  },
  {
    tab: 'Moderation',
    section: 'Log Highlights',
    title: 'Style',
    description: 'How each mod-log entry is emphasized by severity. Previously "Highlight style".'
  },
  {
    tab: 'Moderation',
    section: 'Mass Actions',
    title: 'Mass Actions',
    description: 'Mods can sweep a phrase or pattern across the current channel using these commands in the chat input.'
  },
  {
    tab: 'Moderation',
    section: 'Mass Actions',
    title: '/nuke',
    description: 'Bans, times out, or deletes every recent message that matches a word or /regex/flags, typed as /nuke <pattern> <action> <past[:future]>.'
  },
  {
    tab: 'Moderation',
    section: 'Mass Actions',
    title: '/undo',
    description: 'Reverses the most recent /nuke in this channel. Bans and timeouts are lifted; deleted messages stay gone because Twitch cannot restore them.'
  },
  // === Overlay ===
  {
    tab: 'Overlay',
    section: 'Stream Overlay',
    title: 'Stream Overlay',
    description: 'Design a chat overlay for OBS, StreamElements, and Streamlabs browser sources. Put your multi-platform stream chat on screen with emotes, 7TV paints, badges, and cosmetics. On-stream chat widget, alerts, chat box.'
  },
  {
    tab: 'Overlay',
    section: 'Stream Overlay',
    title: 'Overlay profiles',
    description: 'Run multiple overlays in different styles, each with its own OBS link. Create, duplicate, rename, and delete overlay profiles.'
  },
  {
    tab: 'Overlay',
    section: 'Sources',
    title: 'Sources',
    description: 'Choose which platforms feed the overlay (Twitch, Kick, YouTube, TikTok) and whether to tag each message with its source platform. Add a channel from any platform, or paste its link.'
  },
  {
    tab: 'Overlay',
    section: 'Sources',
    title: 'Platforms',
    description: "Hide a platform's messages without removing its source: Twitch, Kick, YouTube, TikTok. Previously \"Platform filter\"."
  },
  {
    tab: 'Overlay',
    section: 'Sources',
    title: 'Source tag',
    description: 'Shows which platform each message came from, as a dot, an icon, or the platform name.'
  },
  {
    tab: 'Overlay',
    section: 'Layout',
    title: 'Presets',
    description: 'Common sizes to start from, then fine-tune below. Overlay size, 1080p, vertical, OBS browser source size.'
  },
  {
    tab: 'Overlay',
    section: 'Layout',
    title: 'Width',
    description: 'How wide the overlay is; long messages wrap sooner in a narrow one.'
  },
  {
    tab: 'Overlay',
    section: 'Layout',
    title: 'Height',
    description: 'Taller fits more chat on screen at once. Overlay height.'
  },
  {
    tab: 'Overlay',
    section: 'Layout',
    title: 'Background',
    description: 'Transparent lets your scene show through; Solid draws a panel behind the chat. Overlay background.'
  },
  {
    tab: 'Overlay',
    section: 'Layout',
    title: 'Color',
    parent: 'Background',
    description: 'The solid background color behind the overlay chat. Previously "Background color".'
  },
  {
    tab: 'Overlay',
    section: 'Layout',
    title: 'Opacity',
    parent: 'Background',
    description: 'How see-through the solid overlay background is. Previously "Background opacity".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Text',
    description: 'Overlay font family, font size, line height, and spacing between messages. Make overlay message text bold, light, italic, or strikethrough. Font weight, slant, crossed out, line through.'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Font',
    description: 'Overlay font family, from the list or your own. Typeface, Google Fonts.'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Custom font',
    parent: 'Font',
    description: 'Any free font from fonts.google.com by its exact name, or one installed on your streaming PC. Poppins, Bebas Neue, own font.'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Size',
    description: 'Overlay font size. Previously "Font size".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Line height',
    description: 'Spacing within a wrapped overlay message.'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Spacing',
    description: 'Gap between overlay messages. Previously "Message spacing".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Alignment',
    description: 'Left, center, or right; event cards line up the same way. Previously "Text alignment".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Weight',
    description: 'How heavy the overlay text is: light, regular, bold. Usernames stay bold either way. Previously "Text weight".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Italic',
    description: 'Slant overlay message text. Actions (/me) are italic either way.'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Strikethrough',
    description: 'Draw a line through overlay message text. Crossed out, line through.'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Color',
    description: 'The overlay message text color. Previously "Text color".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Shadow',
    description: 'Shadow behind overlay text for legibility over any scene: color, size, blur, spread, opacity, strength. Outline, stroke, drop shadow, contrast, readable. Previously "Text shadow".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Color',
    parent: 'Shadow',
    description: 'The overlay text shadow color. Previously "Shadow color".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Size',
    parent: 'Shadow',
    description: 'How far the overlay text shadow spreads; 0 turns it off. Previously "Shadow size".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Strength',
    parent: 'Shadow',
    description: 'How solid the overlay text shadow is. Previously "Shadow strength".'
  },
  {
    tab: 'Overlay',
    section: 'Text',
    title: 'Emoji style',
    description: 'One consistent emoji set across every platform on the overlay, or your system emoji.'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'Emotes & Badges',
    description: 'Emote size on the overlay and whether chatter badges are shown: platform badges, third-party badges (7TV, FFZ, Chatterino), the StreamNook member badge, 7TV paints, and atmospheres.'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'Emote size',
    description: 'How big emotes draw on the overlay. Emote scale.'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'Giant emotes',
    description: 'Render the last emote of a Gigantify power-up message at 4x below the message on the overlay.'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'Placement',
    parent: 'Giant emotes',
    description: 'Where a Gigantify power-up emote lands on the overlay: left, centered, or right below the message, or inline next to the username. Previously "Giant emote placement".'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'GIFs',
    description: 'GIFs that Tier 2 and Tier 3 subscribers post in chat, drawn big on the overlay like a gigantified emote. Previously "Chat GIFs".'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'Personal emotes',
    description: "Show or hide 7TV personal emotes on the overlay. A subscriber's own set works in every channel, so chatters show emotes your channel never added. Unknown emotes, random emotes, emotes not in my channel. Previously \"7TV personal emotes\"."
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'Badges',
    description: 'Badges the platform sends on the overlay: subscriber, moderator, VIP, and the rest. Previously "Show badges".'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'Badge size',
    description: 'How big badges draw on the overlay. Badge scale.'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'Add-on badges',
    description: 'Third-party badges on the overlay: 7TV, FFZ, Chatterino, and more, plus the StreamNook member badge. Previously "Third-party badges".'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    title: 'Services',
    parent: 'Add-on badges',
    description: 'Pick which badge services show on the overlay: StreamNook, 7TV, FFZ, Chatterino and the rest. Previously "Badge providers".'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: 'Profile pictures',
    description: 'Show or hide chatter avatars (profile pictures) on the overlay. YouTube and TikTok send them. Pfp, user photo, author image.'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: '@ prefix',
    description: 'Show or strip the leading @ on usernames on the overlay. YouTube handles arrive as @name; turn off to remove the at sign. Previously "@ before usernames".'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: 'Readable colors',
    description: 'Brighten chatter name colors that are too dark to read on the overlay. Dark names, navy, maroon, contrast, legibility, lighten. Previously "Readable name colors".'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: 'Paints',
    description: '7TV paints: colored and animated username gradients on the overlay.'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: 'Atmospheres',
    description: "A StreamNook member's animated wash behind their own messages on the overlay."
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: 'First-time chatters',
    description: 'Mark the first message someone ever sends in the channel on the overlay: Twitch style (pink outline like Twitch chat) or StreamNook style (purple highlight like the app chat). First message highlight, new chatter, first time chat border.'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: 'Color',
    parent: 'First-time chatters',
    description: 'Custom accent color for the first-time chatter highlight on the overlay (outline, fill, bar, and label together). Default is Twitch pink or StreamNook purple. Previously "Highlight color".'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: 'Fill',
    parent: 'First-time chatters',
    description: 'Nearly transparent color-matched tint inside the first-time chatter outline on the overlay. Fill, background tint, highlight. Previously "Fill the highlight".'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: 'Animation',
    parent: 'First-time chatters',
    description: "Border accent when a first-time chatter's message lands on the overlay: Sheen (glint sweep), Pulse (border breathes), or Chase (spark orbits the ring). Plays once, or repeats every 5 seconds with the repeat toggle. Animation, sweep, shimmer, border flash, loop."
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    title: 'Loop',
    parent: 'First-time chatters',
    description: 'Keep the first-time animation going while the message is on screen. Previously "Repeat the animation".'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Replies',
    description: 'How a reply renders on the overlay: the "Replying to" context line, just the @username in front of the message the way old Twitch chat did, or nothing. Reply thread, reply preview, reply context.'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Links',
    description: 'Give links on the overlay their own accent color or leave them in the body text color, and turn the underline on or off. Blue links, url color, hyperlink styling.'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Color',
    parent: 'Links',
    description: 'The accent color for links on the overlay. Previously "Link color".'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Underline',
    parent: 'Links',
    description: 'Underline links on the overlay, or leave them plain. Previously "Underline links".'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Timestamps',
    description: 'Show the time beside each overlay message. Previously "Show timestamps".'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Bubbles',
    description: 'Each overlay chat message sits in its own bubble with adjustable shape (rounded, pill, speech), corner radius, color, and opacity. Chat bubble, pill, messenger style, message background. Previously "Message bubbles".'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Shape',
    parent: 'Bubbles',
    description: 'Rounded, pill, or speech bubble for overlay messages. Previously "Bubble shape".'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Corner radius',
    parent: 'Bubbles',
    description: 'How round the overlay message bubbles are.'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Color',
    parent: 'Bubbles',
    description: 'The overlay message bubble color. Previously "Bubble color".'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Opacity',
    parent: 'Bubbles',
    description: 'How see-through the overlay message bubbles are. Previously "Bubble opacity".'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    title: 'Max lines',
    description: "Clamp long overlay messages to a number of lines with an ellipsis so walls of text and copypasta can't fill the canvas. Truncate, line limit. Previously \"Max lines per message\"."
  },
  {
    tab: 'Overlay',
    section: 'Behavior',
    title: 'Behavior',
    description: 'Whether new messages appear at the bottom or top, message entrance animation (fade, slide, drift, rise, pop, stamp), and the maximum messages kept on screen.'
  },
  {
    tab: 'Overlay',
    section: 'Behavior',
    title: 'Lifetime',
    description: 'Takes a message off the overlay once it has been up this long, so a quiet stream never shows stale chat. Expire, auto clear, hide after inactivity. Previously "Remove messages after".'
  },
  {
    tab: 'Overlay',
    section: 'Behavior',
    title: 'Restore on reload',
    description: 'Bring back the last on-screen messages after an OBS browser source reload instead of clearing. Off by default: clear on reload, OBS refresh, restart, stream start, keep buffer, persistence, blank overlay. Previously "Restore chat on reload".'
  },
  {
    tab: 'Overlay',
    section: 'Behavior',
    title: 'Recent chat',
    description: 'Fill the overlay with the Twitch channel’s recent messages when it starts, instead of an empty overlay. Chat history, backlog, previous messages, load history. Previously "Recent chat on start".'
  },
  {
    tab: 'Overlay',
    section: 'Behavior',
    title: 'Mod commands',
    description: 'Let the broadcaster and moderators type !refreshoverlay or !clearoverlay in chat to reload or clear the overlay without opening OBS. Chat command, refresh overlay, reload chat, clear overlay.'
  },
  {
    tab: 'Overlay',
    section: 'Behavior',
    title: 'Direction',
    description: 'Whether new overlay messages appear at the bottom or the top. Previously "New messages".'
  },
  {
    tab: 'Overlay',
    section: 'Behavior',
    title: 'Entrance',
    description: 'How each new overlay message arrives: fade, slide, drift, rise, pop, or stamp. Entrance animation.'
  },
  {
    tab: 'Overlay',
    section: 'Filters',
    title: 'Hide bots',
    description: 'Keep Nightbot, StreamElements, other known bots, and anyone with a bot badge off the overlay. Previously "Hide bot messages".'
  },
  {
    tab: 'Overlay',
    section: 'Filters',
    title: 'Hide commands',
    description: 'Keeps chat commands like !title off the overlay; choose which ones below. Previously "Hide command messages".'
  },
  {
    tab: 'Overlay',
    section: 'Filters',
    title: 'Commands',
    parent: 'Hide commands',
    description: 'Which chat commands stay off the overlay. Previously "Commands to hide".'
  },
  {
    tab: 'Overlay',
    section: 'Filters',
    title: 'Phrases',
    description: 'Hide overlay messages containing chosen words or phrases, case-insensitive. Profanity filter, banned words, phrase blocklist, spoiler shield. Previously "Hide messages containing".'
  },
  {
    tab: 'Overlay',
    section: 'Hidden Accounts',
    title: 'Hidden Accounts',
    description: 'Hide specific people on each overlay source, by username or display name. Block a user, ignore, bot by name.'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    title: 'Cheers',
    description: 'Show a Twitch cheer on the overlay inline like a normal message, or promote it to an event card like subs and raids. Bits, cheer, gem, tier. Previously "Bits messages".'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    title: 'Style',
    description: 'How subs, gifts, raids, and other events look on the overlay: Plain per-platform tint, Outline thin ring in the platform color, or the StreamNook signature gradient wash. Previously "Event style".'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    title: 'Outline color',
    parent: 'Style',
    description: "One fixed ring color for Outline-style events on the overlay, or the default where each event uses its platform's color."
  },
  {
    tab: 'Overlay',
    section: 'Events',
    title: 'Fill',
    parent: 'Style',
    description: 'Nearly transparent color-matched tint inside the Outline event ring on the overlay. Fill, background tint. Previously "Fill the outline".'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    title: 'Animation',
    parent: 'Style',
    description: 'Border accent when an Outline-style event lands on the overlay: Sheen (glint sweep), Pulse (border breathes), or Chase (spark orbits the ring). Plays once, or repeats every 5 seconds with the repeat toggle. Animation, sweep, shimmer, border flash, loop.'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    title: 'Loop',
    parent: 'Style',
    description: 'Keep the event outline animation going while the event is on screen. Previously "Repeat the animation".'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    title: 'Wording',
    description: 'Write your own wording for subs, gifts, raids, bits, milestones, follows and announcements on the overlay, using {username}, {months}, {streak}, {tier}, {recipient}, {count}, {bits} and {viewers} tokens. Custom message, event template, resub message, welcome message. Previously "Custom event text".'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    title: 'Platforms',
    description: 'Per-source event filter: choose which event types each platform shows on the overlay, separately for Twitch, YouTube, TikTok, and Kick. Hide subs, gifts, raids, bits, follows, milestones, or announcements per platform. Previously "Show events".'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    title: 'Super Chat currency',
    description: 'Convert every YouTube Super Chat on the overlay into one currency, or show each as it was sent.'
  },
  // === Interface ===
  {
    tab: 'Interface',
    section: 'Sidebar',
    sectionId: 'settings-section-sidebar',
    title: 'Sidebar',
    description: 'Control the appearance of the stream list sidebar: display mode, expand on hover, recommended streams.'
  },
  {
    tab: 'Interface',
    section: 'Sidebar',
    sectionId: 'settings-section-sidebar',
    title: 'Style',
    description: 'Choose Expanded, Compact (profile pictures only, hover for details), Hidden (slides in from the left edge), or Disabled. Previously "How the sidebar appears".'
  },
  {
    tab: 'Interface',
    section: 'Sidebar',
    sectionId: 'settings-section-sidebar',
    title: 'Hover expand',
    parent: 'Style',
    description: 'Move your cursor over the compact sidebar to open it fully, and it folds back when you leave. Previously "Expand when you hover".'
  },
  {
    tab: 'Interface',
    section: 'Sidebar',
    sectionId: 'settings-section-sidebar',
    title: 'Recommended',
    description: 'Show the Recommended section in the sidebar. Turn this off to keep only your followed channels and favorites. Previously "Show recommended streams".'
  },
  {
    tab: 'Interface',
    section: 'Discover',
    sectionId: 'settings-section-discover',
    title: 'Personalized',
    description: 'Opt in to account-personalized Discover recommendations, or stay anonymous. Privacy, tracking, tailored suggestions. Previously "Personalized recommendations".'
  },
  {
    tab: 'Interface',
    section: 'Discover',
    sectionId: 'settings-section-discover',
    title: 'Languages',
    description: 'Filter the Discover tab and sidebar recommended streams by broadcast language: only show streams in english, french, german, spanish, or any other language you pick.'
  },
  {
    tab: 'Interface',
    section: 'Motion',
    sectionId: 'settings-section-motion',
    title: 'Amount',
    description: 'Full plays every animation, Reduced keeps quick fades only, Off makes everything instant. Previously "How much the interface animates".'
  },
  {
    tab: 'Interface',
    section: 'Window',
    sectionId: 'settings-section-window',
    title: 'Close button',
    description: 'Closing the window quits StreamNook, unless MultiChat popouts are still open, in which case it minimizes to the system tray so they keep working. Previously "What the close button does".'
  },
  {
    tab: 'Interface',
    section: 'Window',
    sectionId: 'settings-section-window',
    title: 'Full screen',
    description: 'The title bar and sidebar tuck away while you watch in full screen. Move your cursor to the top edge to bring the title bar back, or to the side edge for the sidebar. Previously "Show only the stream and chat".'
  },
  {
    tab: 'Interface',
    section: 'Compact View',
    sectionId: 'settings-section-compact',
    title: 'Keep on top',
    description: 'While Compact View is active, the small player floats above other apps so clicking your browser does not bury it. Previously "Keep on top in Compact View".'
  },
  {
    tab: 'Interface',
    section: 'Window',
    sectionId: 'settings-section-window',
    title: 'Centered settings',
    description: 'Settings open in a centered window; turn this off to open them as a full page that fills the app. Previously "Keep settings in a centered window".'
  },
  {
    tab: 'Interface',
    section: 'Compact View',
    sectionId: 'settings-section-compact',
    title: 'Compact View',
    description: 'Choose the window size when entering Compact View mode. Perfect for fitting the app on a second monitor.'
  },
  {
    tab: 'Interface',
    section: 'Window',
    sectionId: 'settings-section-window',
    title: 'Window',
    description: 'What the close button does (minimize to tray or quit), whether settings open in a centered window, and what full screen hides.'
  },
  // === Profile ===
  {
    tab: 'Profile',
    section: 'Accounts',
    title: 'Platform accounts',
    description: 'Connect or disconnect Kick and YouTube. Sign in, link platform, add account, multi-platform, Kick account, YouTube account, followed channels, subscriptions.'
  },
  // === Integrations ===
  // Platform accounts used to be indexed here; they moved to Profile → Accounts
  // with the Twitch ones, so searching "Kick" lands where the accounts are.
  {
    tab: 'Integrations',
    section: 'Discord Rich Presence',
    title: 'Discord Rich Presence',
    description: "Show what you're watching on your Discord profile. Discord RPC, activity, status."
  },
  {
    tab: 'Integrations',
    section: 'Ad Blocking',
    title: 'Ad Blocking',
    description: 'Block Twitch ads with the ad blocker plugin. Ad-free, TTV LOL, proxy, splice. Plugin integration panels appear here once installed.'
  },
  // === Notifications ===
  {
    tab: 'Notifications',
    section: 'Notifications',
    title: 'Notifications',
    description: 'Control notification system settings.'
  },
  {
    tab: 'Notifications',
    section: 'Notifications',
    title: 'All notifications',
    description: 'Turn this off to silence every notification at once; your choices below stay saved for when you turn it back on. Previously "Show notifications".'
  },
  {
    tab: 'Notifications',
    section: 'Display',
    title: 'Display',
    description: 'Choose how to display notifications: Dynamic Island, toasts, toast position, edge spacing. Previously "Notification Methods".'
  },
  {
    tab: 'Notifications',
    section: 'Display',
    title: 'Dynamic Island',
    description: 'Notifications appear in the notification center at the top of the window. Previously "Show in the Dynamic Island".'
  },
  {
    tab: 'Notifications',
    section: 'Display',
    title: 'Toasts',
    description: 'Each notification also pops up as a small card at the edge of the window you choose below. Previously "Show toast popups".'
  },
  {
    tab: 'Notifications',
    section: 'Display',
    title: 'Position',
    parent: 'Toasts',
    description: 'Click a spot on the mini screen to move toasts to that corner or edge. Previously "Where toasts appear".'
  },
  {
    tab: 'Notifications',
    section: 'Display',
    title: 'Edge distance',
    parent: 'Toasts',
    description: 'How far toasts sit from the top or bottom edge of the window; raise it to push them further in. Previously "Distance from the edge".'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'Notification Types',
    description: 'Enable or disable specific types: live streams, whispers, updates, drops, channel points, badges.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'Followed channels',
    description: 'You get a notification the moment someone you follow starts streaming, and clicking it opens the stream. Previously "When a followed channel goes live".'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'Favorite channels',
    description: 'Channels you have favorited notify you even if you do not follow them on Twitch. Previously "When a favorite channel goes live".'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'Whispers',
    description: 'A notification shows each new whisper, and clicking it opens the conversation. Previously "When you get a whisper".'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'App updates',
    description: 'You hear about new StreamNook versions as soon as they are available, and clicking takes you to the Updates page. Previously "When an app update is ready".'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'Update straight from the toast',
    description: 'Clicking the update toast starts installing right away instead of opening the Updates page first.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'Claimed drops',
    description: 'A notification confirms each drop StreamNook claims for you. Previously "When a drop is claimed".'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'New drops',
    parent: 'Claimed drops',
    description: 'At startup, StreamNook checks your favorite categories and tells you when they have new drops to earn. Previously "New drops in favorite categories".'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'Channel points',
    description: 'A notification confirms each channel points bonus claimed for you. Previously "When channel points are claimed".'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'New badges',
    description: 'You hear about new badges as soon as they become available to earn. Previously "When new badges appear".'
  },
  {
    tab: 'Notifications',
    section: 'Sound',
    title: 'Sound',
    description: 'Configure notification sounds.'
  },
  {
    tab: 'Notifications',
    section: 'Sound',
    title: 'Sound',
    description: 'A soft sound plays with each notification, in the style you pick below. Previously "Play a sound".'
  },
  {
    tab: 'Notifications',
    section: 'Sound',
    title: 'Tone',
    parent: 'Sound',
    description: 'Every option is soft and short, so none of them will startle you. Previously "Which sound to play".'
  },
  {
    tab: 'Notifications',
    section: 'Sound',
    title: 'Test notification',
    description: 'Fires a sample notification so you can check the position, sound, and style you picked. Previously "Send a test".'
  },
  {
    tab: 'Notifications',
    section: 'About',
    title: 'About',
    description: 'About notifications and how to use them.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'Gift subs',
    description: 'A notification when someone gifts you a sub, even for a channel you were not watching. When someone gifts you a sub.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'Earned rewards',
    description: 'Hear which badge you earned or which drop reward is waiting, by name. When Twitch names a reward for you.'
  },
  {
    tab: 'Notifications',
    section: 'Sound',
    title: 'Volume',
    parent: 'Sound',
    description: 'How loud the notification sound plays. Notification volume, quieter, louder.'
  },
  // === Cache ===
  {
    tab: 'Cache',
    section: 'Cache',
    title: 'Cache',
    description: 'Manage cached emotes and badges.'
  },
  {
    tab: 'Cache',
    section: 'Cache',
    title: 'Disk cache',
    description: 'Stores a copy on this PC after the first download so later channel loads are near instant. Off means every launch fetches them again. Previously "Load emotes and badges from disk".'
  },
  {
    tab: 'Cache',
    section: 'Cache',
    title: 'Refresh after',
    description: 'Anything older than this is fetched again the next time it is needed, so new emotes and badge art show up on their own. Previously "Refresh stored data after a number of days".'
  },
  {
    tab: 'Cache',
    section: 'Cache',
    title: 'Storage',
    description: 'View cache info shows a count of what is on disk, Open folder reveals the files, and Clear cache deletes every stored emote and badge so they download fresh. Previously "See what is stored, or clear it".'
  },
  {
    tab: 'Cache',
    section: 'Emote Prefetch',
    title: 'Emote Prefetch',
    description: 'Download every emote from all the channels you follow so the emote menu opens instantly. Dedupes shared emotes and skips anything already cached. Preload, warm cache, scan follows.'
  },
  {
    tab: 'Cache',
    section: 'Emote Prefetch',
    title: 'Followed channels',
    description: 'Scan your follows to see how many emotes are missing and how much space they need, then download them in the background while you do something else. Previously "Download emotes for every channel you follow".'
  },
  // === Command Palette ===
  {
    tab: 'Command Palette',
    section: 'Keyboard Shortcuts',
    sectionId: 'settings-section-keyboard',
    title: 'Keyboard Shortcuts',
    description: 'Keyboard controls for the command palette (Ctrl+K, arrows, Enter, Esc, Home, End).'
  },
  {
    tab: 'Command Palette',
    section: 'Palette Sections',
    title: 'Palette Sections',
    description: 'Overview of palette sections and available actions: quick actions, current stream, share, settings, categories, snippets. Previously "What lives in the palette".'
  },
  {
    tab: 'Command Palette',
    section: 'Palette Sections',
    title: 'Settings',
    description: "Every settings tab and section is searchable. Type 'ad block' to land on the ad blocking panel under Integrations."
  },
  {
    tab: 'Command Palette',
    section: 'Palette Sections',
    title: 'Streamers',
    description: 'Live and offline Twitch channels matching what you typed. Results appear once you have typed 2 or more characters.'
  },
  {
    tab: 'Command Palette',
    section: 'Snippet Manager',
    sectionId: 'settings-section-snippets',
    title: 'Snippet Manager',
    description: 'Star the snippets you use most, bind aliases for instant matching, and add your own copypastas.'
  },
  // === Keybindings ===
  {
    tab: 'Keybindings',
    section: 'Application',
    title: 'Application',
    description: 'App-wide keyboard shortcuts available everywhere. Hotkeys, binds, combos, rebind, customize.'
  },
  {
    tab: 'Keybindings',
    section: 'Navigation',
    title: 'Navigation',
    description: 'Keyboard shortcuts to jump between the main surfaces of StreamNook. Hotkeys, binds, combos.'
  },
  {
    tab: 'Keybindings',
    section: 'Player',
    title: 'Player Shortcuts',
    description: 'Keyboard shortcuts active while a stream or VOD is playing: play, pause, mute, fullscreen, volume. Hotkeys, binds, combos.'
  },
  {
    tab: 'Keybindings',
    section: 'Moderation',
    title: 'Moderation Shortcuts',
    description: 'Keyboard shortcuts for channels you moderate. Focus a message with J/K, then act on it. Hotkeys, binds, combos.'
  },
  {
    tab: 'Keybindings',
    section: 'Chat',
    title: 'Chat Shortcuts',
    description: 'Keyboard shortcuts for the chat compose field. Hotkeys, binds, combos.'
  },
  {
    tab: 'Keybindings',
    section: 'Multi-view',
    title: 'Multi-view Shortcuts',
    description: 'Keyboard shortcuts for MultiChat windows. Hotkeys, binds, combos.'
  },
  // === Support ===
  {
    tab: 'Support',
    section: 'Community Discord',
    title: 'Community Discord',
    description: 'Join the StreamNook community for help, feature requests, updates, and chat with other users.'
  },
  {
    tab: 'Support',
    section: 'Community Discord',
    title: 'Join the Discord',
    description: 'Open the StreamNook community Discord invite.'
  },
  {
    tab: 'Support',
    section: 'Diagnostics',
    title: 'Detailed log',
    description: 'Records connection, playback, and chat activity to streamnook.log on this PC so a problem can be traced after the fact. Previously "Keep a detailed log for bug reports".'
  },
  {
    tab: 'Support',
    section: 'Diagnostics',
    title: 'Log folder',
    description: 'Opens the folder that holds streamnook.log so you can attach it to a bug report. Previously "Find the log file".'
  },
  {
    tab: 'Support',
    section: 'Account Data',
    title: 'Channels and emotes',
    description: 'Counts which channels you watch and which emotes you use to fill in your profile stats and unlock accolades. Privacy: running totals only, nothing is counted while signed out.'
  },
  {
    tab: 'Support',
    section: 'Account Data',
    title: 'Version and platform',
    description: 'Records which build you are on, your operating system, and whether your updater is working, so a client that quietly stopped updating is visible.'
  },
  {
    tab: 'Support',
    section: 'Account Data',
    title: 'Linked accounts',
    description: 'Records which other platforms you have connected so they survive a reinstall.'
  },
  // === Backup ===
  {
    tab: 'Backup',
    section: 'Backup & Restore',
    title: 'Backup & Restore',
    description: 'Export your settings to a file, or import a saved backup to restore them after a reset, reinstall, or move to a new PC. Previously "Backup and restore".'
  },
  {
    tab: 'Backup',
    section: 'Backup & Restore',
    title: 'Backup',
    description: 'Writes a copy of your settings file wherever you like, such as a USB drive or a cloud-synced folder. Previously "Save a backup".'
  },
  {
    tab: 'Backup',
    section: 'Backup & Restore',
    title: 'Restore',
    description: 'Pick a backup file and StreamNook swaps in those preferences, then reloads itself so everything picks them up. Previously "Restore from a backup".'
  },
  {
    tab: 'Backup',
    section: 'Settings File',
    title: 'Location',
    description: 'The folder on this PC that holds your settings file. Previously "Where your settings file lives".'
  },
];
