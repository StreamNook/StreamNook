export interface SettingsIndexEntry {
  tab: string;
  section: string;
  sectionId?: string;
  title: string;
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
    const tab = entry.tab.toLowerCase();
    const haystack = `${title} ${description} ${section} ${tab}`;

    let allMatch = true;
    let score = 0;
    for (const token of tokens) {
      if (!haystack.includes(token)) {
        allMatch = false;
        break;
      }
      if (title.startsWith(token)) score += 100;
      else if (title.includes(token)) score += 50;
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
    section: 'Player Overlay Buttons',
    title: 'Player Overlay Buttons',
    description: 'Choose which action buttons (follow, subscribe, create clip, identify song, clips & vods, add to multinook, refresh, close) appear in the top-right of the video player.'
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Scroll to change volume',
    description: 'Scroll the mouse wheel over the video to change volume up or down, louder quieter, one handed mouse only control.'
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Scroll to open Channel About',
    description: "Scroll down over the player (with Shift when the wheel is set to volume) to slide the channel's About panel up over the stream."
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Middle-click to mute',
    description: 'Click the scroll wheel or middle mouse button over the player to mute and unmute the stream without the keyboard.'
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Volume Step',
    description: 'How much one mouse wheel notch changes the volume, from 1 to 25 percent. Also used by the volume up and volume down keyboard shortcuts.'
  },
  {
    tab: 'Player',
    section: 'Mouse Controls',
    sectionId: 'settings-section-mouse-controls',
    title: 'Resume VODs where you left off',
    description: 'Reopening a past broadcast picks up at your last position.'
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
    title: 'Move to another stream when this one ends',
    description: 'When the channel you are watching goes offline, StreamNook picks a new live stream and starts it for you.'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Where to go next',
    description: 'The most-watched live stream in the same category, or one of your followed channels that is live right now.'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Tell me when it switches',
    description: 'A toast names the new channel each time StreamNook switches for you.'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Follow raids automatically',
    description: 'When the streamer raids another channel, StreamNook jumps there with them (you need to be signed in).'
  },
  {
    tab: 'Player',
    section: 'Auto-Switch',
    sectionId: 'settings-section-auto-switch',
    title: 'Stay in chat after the stream ends',
    description: "Keeps you in the channel's chat when the stream goes offline instead of switching you away."
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
    title: 'Allow AV1 and h265 streams',
    description: 'Asks Twitch for AV1 and h265 (HEVC) versions of the stream alongside h264, which some channels offer at better quality for the same bandwidth.'
  },
  {
    tab: 'Player',
    section: 'Streaming',
    sectionId: 'settings-section-streaming',
    title: 'Keep trying for a set time',
    description: 'How long StreamNook keeps trying to open a stream before giving up, which helps when a channel has only just gone live.'
  },
  {
    tab: 'Player',
    section: 'Streaming',
    sectionId: 'settings-section-streaming',
    title: 'Pause between attempts',
    description: 'How long to wait between attempts while a stream is not available yet (0 means a single attempt).'
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'Video Player',
    description: 'Playback behavior: autoplay, live edge, low latency, buffer, quality, volume, aspect ratio, mute.'
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'Play as soon as a stream opens',
    description: 'The stream starts playing the moment it loads, with no need to press play.'
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'How close to live to stay',
    description: 'How far behind the live edge the player rides; lower is closer to live (reopen the stream to apply).'
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'Low Latency',
    description: 'Uses the low-latency engine to hold a tight live edge gap smoothly on channels that support it.'
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'Buffer up to a set length',
    description: 'How much video the player keeps loaded ahead of playback; more is steadier on a shaky connection but adds delay.'
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'Quality to start streams at',
    description: 'Every stream opens at this quality, and you can change it anytime from the player controls.'
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'Keep the window at 16:9',
    description: "Resizing the window snaps to the video's shape, so you never see black bars around the picture."
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'Cinema Mode',
    description: 'Letterbox bar color. Cinema Mode uses classic black bars; off matches the bars to your theme background so the video floats. Black bars, color-matched, immersive, pillarbox.'
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'Start streams muted',
    description: 'Every stream opens silent until you unmute it.'
  },
  {
    tab: 'Player',
    section: 'Video Player',
    sectionId: 'settings-section-video-player',
    title: 'Starting volume',
    description: 'The volume every stream opens at before you adjust it.'
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
    title: 'Turn on Audio Boost',
    description: "Evens out the stream's loudness and lifts it, on top of the normal volume slider."
  },
  {
    tab: 'Player',
    section: 'Audio Boost',
    sectionId: 'settings-section-audio-boost',
    title: 'Boost',
    description: 'How much louder to make the stream after compression (volume boost / gain).'
  },
  {
    tab: 'Player',
    section: 'Audio Boost',
    sectionId: 'settings-section-audio-boost',
    title: 'Advanced Compressor Controls',
    description: 'Threshold, ratio, knee, attack and release controls for the audio compressor.'
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
    title: 'Retries when nothing matches',
    description: 'If the first listen finds nothing, StreamNook listens again this many times.'
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
    section: 'Chat Placement',
    title: 'Chat Placement',
    description: 'Choose where to display the chat window (right, bottom) or hide it completely.'
  },
  {
    tab: 'Chat',
    section: 'Chat Placement',
    title: 'Where chat sits',
    description: 'Dock chat to the left, right, or bottom of the player, or hide it to give the video the whole window.'
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
    title: 'Auto-claim bonus chests',
    description: 'Collects the bonus chest on the stream you are watching the moment it appears.'
  },
  {
    tab: 'Chat',
    section: 'YouTube Chat',
    sectionId: 'settings-section-youtube-chat',
    title: 'Which chat to read',
    description: "Live chat shows everything, while Top chat is YouTube's own filtered view that keeps a very fast chat readable."
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
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Polls',
    description: 'Show a live poll card at the top of chat when the streamer runs one, with the running vote tally.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Predictions',
    description: 'Show a live prediction card at the top of chat, with the outcomes and how points are stacking up.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Starting a poll or prediction',
    description: 'On your own channel, the chart button beside the message box opens a builder for polls and predictions, with outcomes, a duration, channel-point voting and a live preview. Also reachable with /poll and /prediction.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'When both are running',
    description: 'Pick which card sits on top when a poll and a prediction run at the same time.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Channel point redemptions',
    description: 'Shows a chat row when someone redeems a reward that does not post its own message, like a no-input reward.'
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Collapse gift-sub floods',
    description: "Shows one 'gifting N subs' row with the recipients attached when someone gifts a batch, instead of a row per gift."
  },
  {
    tab: 'Chat',
    section: 'Chat Events',
    sectionId: 'settings-section-chat-events',
    title: 'Chat replay on clips',
    description: 'Shows the chat that was live while a clip was recorded, beside the clip.'
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
    title: 'Save chat logs',
    description: 'Writes chat to plain text files as you watch: one folder per channel, one file per day.'
  },
  {
    tab: 'Chat',
    section: 'Chat Logging',
    title: 'Log folder',
    description: 'Where the files are written. Browse to pick your own folder, Reset to go back to the default.'
  },
  {
    tab: 'Chat',
    section: 'Chat Logging',
    title: 'Only log these channels',
    description: 'Restrict logging to specific channels. Leave empty to log every channel you open.'
  },
  {
    tab: 'Chat',
    section: 'Chat Logging',
    title: 'Events and moderation',
    description: 'Also log subscriptions, raids, announcements, timeouts, and deleted messages.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Chat Design',
    description: 'Customize the appearance of chat messages: dividers, backgrounds, spacing, font, timestamps, mentions, name prefix.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Lines between messages',
    description: 'Draws a thin line between messages so a fast chat is easier to scan.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Striped message rows',
    description: "Gives every other message a slightly different background, in your theme's colors, so rows are easier to follow."
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Message spacing',
    description: 'Blank space between one message and the next; more room means fewer messages on screen.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Text size',
    description: 'Size of message text, with room to go large when MultiChat fills a whole monitor.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Activity feed size',
    description: 'Text size for the MultiChat activity feed, where subs, raids, and gifts land.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Text weight',
    description: 'How heavy the message text is, from light to bold.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Flash when you are mentioned',
    description: 'Briefly flashes any message that mentions or replies to you, so you spot it in a fast chat.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Show timestamps',
    description: 'Shows the time each message was sent, next to the name.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Include seconds',
    description: 'Shows seconds too, so 7:42 PM reads 7:42:30 PM.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Pins start collapsed',
    description: 'Shows the pinned message as a compact one-line bar when you enter a channel.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Collapsed pin style',
    description: 'Shrinks a collapsed pin to a thin one-line bar you can click to expand, or hides it completely.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Name separator',
    description: 'The mark between a name and its message, like a colon or an arrow.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Name style',
    description: 'How names stand out from the message: plain, or with a bar, chip, brackets, or dot.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Prefix color',
    description: "Colors the separator, bar, dot, brackets, or chip with the chatter's own color or your theme accent."
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Mention color',
    description: 'The highlight color on messages that mention you.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Reply thread color',
    description: 'The color that marks replies in a thread.'
  },
  {
    tab: 'Chat',
    section: 'Link Previews',
    title: 'Link Previews',
    description: 'Show rich preview cards when links are posted in chat. Unfurl, embed, trusted sources, shorten links.'
  },
  {
    tab: 'Chat',
    section: 'Link Previews',
    title: 'How links show',
    description: 'Off keeps links as plain text, Card + Link adds a preview card under the link, and Clean shows only the card.'
  },
  {
    tab: 'Chat',
    section: 'Link Previews',
    title: 'Shorten links',
    description: 'Shows each link as a compact label, the site plus a short path, instead of the full raw URL.'
  },
  {
    tab: 'Chat',
    section: 'Link Previews',
    title: 'Trusted sites',
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
    title: 'Emote size',
    description: 'Scales emotes in chat relative to the text, with 1.00x being the default size.'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Emote hover size',
    description: 'How large an emote grows when you hover it, in chat and in the emote menu.'
  },
  {
    tab: 'Chat',
    section: 'Emotes',
    title: 'Emote spacing',
    description: 'Space on each side of an emote; go negative to let neighboring emotes overlap.'
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
    title: 'Send the same message twice',
    description: 'Adds an invisible character when you repeat a message, so Twitch does not reject the second send.'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Ctrl+Enter sends and keeps the text',
    description: 'Sends the message and leaves it in the box, so you can send it again straight away.'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Check spelling as you type',
    description: 'Underline misspelled words in the message box and offer corrections when you right-click one. Emotes, chatters, commands and links are left alone.'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Spell check dictionary',
    description: 'Words you have taught the spell checker so it stops flagging them. Add or remove entries here.'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Hide the placeholder text',
    description: 'Leave the message box empty instead of prompting you to send a message. Notices you can act on, like read-only or subscriber-only mode, still show.'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Hide the command button',
    description: 'Removes the slash button from inside the message box. It opens a browsable menu of every command you can run here, with examples; typing / still opens the quick list.'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Hide the emote button',
    description: 'Remove the smiley from inside the message box. The emote picker is still reachable from its keyboard shortcut and from tab completion.'
  },
  {
    tab: 'Chat',
    section: 'Chat Input',
    title: 'Hide the points balance',
    description: 'Remove the channel points button next to the message box. It comes back on its own whenever a bonus chest is waiting.'
  },
  {
    tab: 'Chat',
    section: 'Emote Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'Emote Tab Completion',
    description: 'Tab cycles forward through matching emotes in the chat input, Shift+Tab cycles back. Autocomplete.'
  },
  {
    tab: 'Chat',
    section: 'Emote Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'Complete emote names with Tab',
    description: 'Press Tab while typing to insert the best-matching emote, and Tab again to cycle to the next match.'
  },
  {
    tab: 'Chat',
    section: 'Emote Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'How names match',
    description: 'Starts With needs the emote to begin with what you typed; Contains matches it anywhere in the name.'
  },
  {
    tab: 'Chat',
    section: 'Emote Tab Completion',
    sectionId: 'settings-section-emote-tab-completion',
    title: 'Complete chatter names too',
    description: 'Also cycles through the names of people currently in chat.'
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'Render Style',
    description: 'How specific message classes look in chat: deleted messages, shared chat, mention paint, emote tooltips, scroll, message buffer.'
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'Deleted messages',
    description: 'What happens to a message once it is deleted or its sender is timed out or banned: crossed out, dimmed, left as is, or removed.'
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'Hide shared chat messages',
    description: "Hides messages that came from the other channel in a Twitch Shared Chat, so you only see this channel's own chatters."
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'Paint @mentions inline',
    description: "Draws a mentioned name in that person's 7TV paint instead of a flat color."
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'Compact emote tooltips',
    description: 'Show just the emote name on hover instead of the full hint.'
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'FFZ emote effects',
    description: 'Applies FrankerFaceZ modifiers (wide, flips, rainbow, shake) to the emote before them, the way FFZ does.'
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'BetterTTV emote modifiers',
    description: 'Applies BetterTTV modifiers (w! wide, h! and v! flips, c! cursed, p! party, s! shake) to the emote after them, the way BetterTTV does.'
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'Giant emotes',
    description: 'Draws the last emote of a "Gigantify an Emote" power-up message at 4x below the message, like Twitch does.'
  },
  {
    tab: 'Chat',
    section: 'Hidden Users & Bots',
    sectionId: 'settings-section-chat-filters',
    title: 'Hide known bots',
    description: 'Hide chat messages from StreamElements, Nightbot, Moobot and other well-known bots in every channel. Local only; nothing is sent to the platform.'
  },
  {
    tab: 'Chat',
    section: 'Hidden Users & Bots',
    sectionId: 'settings-section-chat-filters',
    title: 'Hidden everywhere',
    description: 'Users whose messages never appear in your chat, on any platform. Add names here or from a user card in chat.'
  },
  {
    tab: 'Chat',
    section: 'Hidden Users & Bots',
    sectionId: 'settings-section-chat-filters',
    title: 'Hidden in one channel',
    description: 'Users hidden only in a single channel, added from their user card while watching. Remove a name to see their messages again.'
  },
  {
    tab: 'Chat',
    section: 'Chat Placement',
    title: 'Chat over fullscreen video',
    description: 'Float the chat panel over the video while the player is fullscreen, as a translucent column. Opacity, width, side, and hide-with-controls options.'
  },
  {
    tab: 'Chat',
    section: 'Hidden Users & Bots',
    sectionId: 'settings-section-chat-filters',
    title: 'Ignored phrases',
    description: 'Hide any message containing a word, phrase or regular expression, in every channel. Evaluated before the message reaches chat.'
  },
  {
    tab: 'Chat',
    section: 'Message Filters',
    sectionId: 'settings-section-chat-query',
    title: 'Saved filters',
    description: 'Saved message filters (mods only, subs only, mentions, redemptions, links, custom expressions). Apply one to a chat pane from its header.'
  },
  {
    tab: 'Chat',
    section: 'Message Filters',
    sectionId: 'settings-section-chat-query',
    title: 'Search history size',
    description: 'How many recent messages per channel Ctrl+F can search. Kept in the Rust backend, not in the chat view.'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'When a message repeats',
    description: 'Fold a run of the same message into one row with a count like x12, just number them in place, or leave repeats alone. Helps when a copypasta wave or one emote floods chat.'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'How closely they must match',
    description: 'Whether nearly-identical messages count as repeats, ignoring capitals, extra spaces and trailing punctuation, or only exactly identical ones.'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'Repeat counter threshold, window and colour',
    description: 'How many copies before the counter shows, how long copies keep joining the same run, and what colour the counter is.'
  },
  {
    tab: 'Chat',
    section: 'Repeated Messages',
    sectionId: 'settings-section-repeated-messages',
    title: 'Never fold mods, VIPs or the streamer',
    description: 'Keep messages from the broadcaster, moderators and VIPs on their own rows, and optionally show everything in channels you moderate so nothing you might need to action is hidden.'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Open on their messages',
    description: 'Whether clicking someone in chat opens their recent messages first or their profile first. The card switches between the two either way.'
  },
  {
    tab: 'Chat',
    section: 'User Cards',
    sectionId: 'settings-section-user-cards',
    title: 'Which details show on the card',
    description: 'Pick the rows the user card displays: joined Twitch, following since, channels they follow, chatters, past subscriber, last live, how long ago, and the 7TV profile link. Hide fields you never read.'
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: '7TV emote update notices',
    description: 'Shows a chat notice when a mod adds, removes, or renames a 7TV emote in the channel.'
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'Smooth scroll on Resume',
    description: 'Animates the scroll back to the bottom when you click Resume; auto-scroll for new messages stays instant.'
  },
  {
    tab: 'Chat',
    section: 'Render Style',
    title: 'Message buffer',
    description: 'How many messages each chat keeps on screen to scroll back through; more history uses more memory.'
  },
  {
    tab: 'Chat',
    section: '7TV Cosmetics',
    title: '7TV Cosmetics',
    description: 'Visual controls for 7TV-rendered usernames (paints), including drop shadows.'
  },
  {
    tab: 'Chat',
    section: '7TV Cosmetics',
    title: 'Paint drop shadows',
    description: 'Some paints stack several drop shadows for readability; keep them all, just one, or none if names look too noisy.'
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
    title: 'Display style',
    description: 'How a highlighted message is emphasized (standard tint and other styles).'
  },
  {
    tab: 'Chat',
    section: 'Highlight Appearance',
    title: 'Tint opacity',
    description: 'Strength of the highlight tint behind a matched message.'
  },
  {
    tab: 'Chat',
    section: 'Highlight Appearance',
    title: 'Flash window title when unfocused',
    description: 'Flash the window title bar when a highlight fires while the app is in the background.'
  },
  {
    tab: 'Chat',
    section: 'Highlight Phrases',
    title: 'Highlight Phrases',
    description: 'Flash chat messages that match specific words, names, or patterns. Mentions of your own name and replies to you are always highlighted; these are extra.'
  },
  {
    tab: 'Chat',
    section: 'Built-in Event Highlights',
    title: 'Built-in Event Highlights',
    description: 'Auto-highlight messages from event types: first-time chatters, returning chatters, your own messages, and raid announcements.'
  },
  {
    tab: 'Chat',
    section: 'Built-in Event Highlights',
    title: 'First-time chatters',
    description: "Highlight a chatter's very first message in the channel."
  },
  {
    tab: 'Chat',
    section: 'Built-in Event Highlights',
    title: 'Returning chatters',
    description: 'Highlight the first message from a returning chatter.'
  },
  {
    tab: 'Chat',
    section: 'Built-in Event Highlights',
    title: 'Your own messages',
    description: 'Highlight messages you send.'
  },
  {
    tab: 'Chat',
    section: 'Built-in Event Highlights',
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
    section: 'User Overrides',
    title: 'User Overrides',
    description: "Nicknames you've set for individual chatters. Only visible to you. Set or clear a nickname from the user's profile card in chat."
  },

  // === Moderation ===
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Timestamp clock',
    description: '12-hour or 24-hour timestamps next to chat messages.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
    title: 'Animate emotes',
    description: 'Play animated emotes always, only on hover, or never (first frame). Never is lightest on the GPU.'
  },
  {
    tab: 'Chat',
    section: 'Chat Design',
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
    title: 'Paste images to upload',
    description: 'Paste a screenshot into the chat box and StreamNook uploads it to a host you pick (nuuls, catbox, Litterbox, uguu or your own) and inserts the link.'
  },
  {
    tab: 'Chat',
    section: 'Image Uploads',
    sectionId: 'settings-section-image-uploads',
    title: 'Image host',
    description: 'Choose where pasted images are uploaded: i.nuuls.com, catbox.moe, Litterbox (72 h), uguu.se (3 h), or a custom multipart uploader. Includes a one-click test upload.'
  },
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
    title: 'Streamer mode',
    description: 'Hide viewer counts, link previews, restricted users and highlight sounds while live. Auto-detects OBS, Streamlabs, XSplit, Twitch Studio and vMix.'
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
    title: 'Reason for /nuke',
    description: 'The reason written against every ban and timeout that /nuke issues, shown in the channel mod view. Defaults to "/nuke".'
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
    title: 'How you act on a message',
    description: 'Buttons show delete, timeout, and ban when you hover a message; Drag lets you pick a message up and drop it on a color-coded action bucket; Both gives you both.'
  },
  {
    tab: 'Moderation',
    section: 'Moderation Actions',
    title: 'Where the drop buckets appear',
    description: 'Beside chat puts a column of bigger tiles to the left of chat, clear of the player controls; Above chat puts a compact cluster right above the message for when space is tight.'
  },
  {
    tab: 'Moderation',
    section: 'Moderation Actions',
    title: 'Pin from the drag gesture too',
    description: 'Moderators always get a Pin button beside Copy on a message; this adds a Pin tile to the drag buckets as well.'
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
    title: 'Show the mod log beside chat',
    description: 'Adds a panel inside chat that lists recent timeouts, bans, and deleted messages as they happen.'
  },
  {
    tab: 'Moderation',
    section: 'Message Visibility',
    title: 'Message Visibility',
    description: 'Control how removed messages are shown in chat.'
  },
  {
    tab: 'Moderation',
    section: 'Message Visibility',
    title: 'Announce mod actions inline',
    description: 'Add an extra system row to chat when a mod times someone out, bans, or deletes a message (on top of the strikethrough you already see).'
  },
  {
    tab: 'Moderation',
    section: 'Message Visibility',
    title: 'Hide strikethrough on removed messages',
    description: 'Banned, timed-out, and deleted messages stay exactly as they were, with no line through them.'
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
    title: 'Highlight style',
    description: 'How each mod-log entry is emphasized by severity.'
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
    sectionId: 'settings-section-sources',
    title: 'Sources',
    description: 'Choose which platforms feed the overlay (Twitch, Kick, YouTube, TikTok) and whether to tag each message with its source platform.'
  },
  {
    tab: 'Overlay',
    section: 'Sources',
    sectionId: 'settings-section-sources',
    title: 'Source tag',
    description: 'Shows which platform each message came from, as a dot, an icon, or the platform name.'
  },
  {
    tab: 'Overlay',
    section: 'Layout',
    sectionId: 'settings-section-layout',
    title: 'Presets',
    description: 'Common sizes to start from, then fine-tune below.'
  },
  {
    tab: 'Overlay',
    section: 'Layout',
    sectionId: 'settings-section-layout',
    title: 'Width',
    description: 'How wide the overlay is; long messages wrap sooner in a narrow one.'
  },
  {
    tab: 'Overlay',
    section: 'Typography',
    title: 'Font and Size',
    description: 'Overlay font family, font size, line height, and spacing between messages.'
  },
  {
    tab: 'Overlay',
    section: 'Typography',
    title: 'Text alignment',
    description: 'Left, center, or right; event cards line up the same way.'
  },
  {
    tab: 'Overlay',
    section: 'Typography',
    title: 'Text style',
    description: 'Make overlay message text bold, light, italic, or strikethrough. Font weight, slant, crossed out, line through.'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    sectionId: 'settings-section-emotes-and-badges',
    title: 'Emotes and Badges',
    description: 'Emote size on the overlay and whether chatter badges are shown: platform badges, third-party badges (7TV, FFZ, Chatterino), the StreamNook member badge, 7TV paints, and atmospheres.'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    sectionId: 'settings-section-emotes-and-badges',
    title: 'Giant emotes',
    description: 'Render the last emote of a Gigantify power-up message at 4x below the message on the overlay.'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & Badges',
    sectionId: 'settings-section-emotes-and-badges',
    title: 'Giant emote placement',
    description: 'Where a Gigantify power-up emote lands on the overlay: left, centered, or right below the message, or inline next to the username.'
  },
  {
    tab: 'Overlay',
    section: 'Appearance',
    title: 'Appearance',
    description: 'Message text color, text shadow for legibility over any scene, timestamps, and a transparent or solid background.'
  },
  {
    tab: 'Overlay',
    section: 'Appearance',
    title: 'Text shadow',
    description: 'Shadow behind overlay text for legibility over any scene: color, size, blur, spread, opacity, strength. Outline, stroke, drop shadow, contrast, readable.'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    sectionId: 'settings-section-chatters',
    title: 'Profile pictures',
    description: 'Show or hide chatter avatars (profile pictures) on the overlay. YouTube and TikTok send them. Pfp, user photo, author image.'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    sectionId: 'settings-section-chatters',
    title: '@ before usernames',
    description: 'Show or strip the leading @ on usernames on the overlay. YouTube handles arrive as @name; turn off to remove the at sign.'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    sectionId: 'settings-section-messages',
    title: 'Replies',
    description: 'How a reply renders on the overlay: the "Replying to" context line, just the @username in front of the message the way old Twitch chat did, or nothing. Reply thread, reply preview, reply context.'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    sectionId: 'settings-section-messages',
    title: 'Links',
    description: 'Give links on the overlay their own accent color or leave them in the body text color, and turn the underline on or off. Blue links, url color, hyperlink styling.'
  },
  {
    tab: 'Overlay',
    section: 'Emotes & badges',
    sectionId: 'settings-section-emotes-and-badges',
    title: '7TV personal emotes',
    description: "Show or hide 7TV personal emotes on the overlay. A subscriber's own set works in every channel, so chatters show emotes your channel never added. Unknown emotes, random emotes, emotes not in my channel."
  },
  {
    tab: 'Overlay',
    section: 'Events',
    sectionId: 'settings-section-events',
    title: 'Custom event text',
    description: 'Write your own wording for subs, gifts, raids, bits, milestones, follows and announcements on the overlay, using {username}, {months}, {streak}, {tier}, {recipient}, {count}, {bits} and {viewers} tokens. Custom message, event template, resub message, welcome message.'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    sectionId: 'settings-section-messages',
    title: 'Restore chat on reload',
    description: 'Bring back the last on-screen messages after an OBS browser source reload instead of clearing. Off by default: clear on reload, OBS refresh, restart, stream start, keep buffer, persistence, blank overlay.'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    sectionId: 'settings-section-chatters',
    title: 'First-time chatters',
    description: 'Mark the first message someone ever sends in the channel on the overlay: Twitch style (pink outline like Twitch chat) or StreamNook style (purple highlight like the app chat). First message highlight, new chatter, first time chat border.'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    sectionId: 'settings-section-chatters',
    title: 'Fill the highlight',
    description: 'Nearly transparent color-matched tint inside the first-time chatter outline on the overlay. Fill, background tint, highlight.'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    sectionId: 'settings-section-chatters',
    title: 'First-time highlight animation',
    description: 'Border accent when a first-time chatter\'s message lands on the overlay: Sheen (glint sweep), Pulse (border breathes), or Chase (spark orbits the ring). Plays once, or repeats every 5 seconds with the repeat toggle. Animation, sweep, shimmer, border flash, loop.'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    sectionId: 'settings-section-events',
    title: 'Bits messages',
    description: 'Show a Twitch cheer on the overlay inline like a normal message, or promote it to an event card like subs and raids. Bits, cheer, gem, tier.'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    sectionId: 'settings-section-events',
    title: 'Event style',
    description: 'How subs, gifts, raids, and other events look on the overlay: Plain per-platform tint, Outline thin ring in the platform color, or the StreamNook signature gradient wash.'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    sectionId: 'settings-section-events',
    title: 'Show events',
    description: 'Per-source event filter: choose which event types each platform shows on the overlay, separately for Twitch, YouTube, TikTok, and Kick. Hide subs, gifts, raids, bits, follows, milestones, or announcements per platform.'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    sectionId: 'settings-section-events',
    title: 'Fill the outline',
    description: 'Nearly transparent color-matched tint inside the Outline event ring on the overlay. Fill, background tint.'
  },
  {
    tab: 'Overlay',
    section: 'Chatters',
    sectionId: 'settings-section-chatters',
    title: 'Highlight color',
    description: 'Custom accent color for the first-time chatter highlight on the overlay (outline, fill, bar, and label together). Default is Twitch pink or StreamNook purple.'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    sectionId: 'settings-section-messages',
    title: 'Message bubbles',
    description: 'Each overlay chat message sits in its own bubble with adjustable shape (rounded, pill, speech), corner radius, color, and opacity. Chat bubble, pill, messenger style, message background.'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    sectionId: 'settings-section-messages',
    title: 'Max lines per message',
    description: 'Clamp long overlay messages to a number of lines with an ellipsis so walls of text and copypasta can\'t fill the canvas. Truncate, line limit.'
  },
  {
    tab: 'Overlay',
    section: 'Messages',
    sectionId: 'settings-section-messages',
    title: 'Remove messages after',
    description: 'Takes a message off the overlay once it has been up this long, so a quiet stream never shows stale chat.'
  },
  {
    tab: 'Overlay',
    section: 'Filters',
    sectionId: 'settings-section-filters',
    title: 'Hide messages containing',
    description: 'Hide overlay messages containing chosen words or phrases, case-insensitive. Profanity filter, banned words, phrase blocklist, spoiler shield.'
  },
  {
    tab: 'Overlay',
    section: 'Filters',
    sectionId: 'settings-section-filters',
    title: 'Hide command messages',
    description: 'Keeps chat commands like !title off the overlay; choose which ones below.'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    sectionId: 'settings-section-events',
    title: 'Outline color',
    description: 'One fixed ring color for Outline-style events on the overlay, or the default where each event uses its platform\'s color.'
  },
  {
    tab: 'Overlay',
    section: 'Events',
    sectionId: 'settings-section-events',
    title: 'Event outline animation',
    description: 'Border accent when an Outline-style event lands on the overlay: Sheen (glint sweep), Pulse (border breathes), or Chase (spark orbits the ring). Plays once, or repeats every 5 seconds with the repeat toggle. Animation, sweep, shimmer, border flash, loop.'
  },
  {
    tab: 'Overlay',
    section: 'Behavior',
    title: 'Behavior',
    description: 'Whether new messages appear at the bottom or top, message entrance animation (fade, slide, drift, rise, pop, stamp), and the maximum messages kept on screen.'
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
    title: 'How the sidebar appears',
    description: 'Choose Expanded, Compact (profile pictures only, hover for details), Hidden (slides in from the left edge), or Disabled.'
  },
  {
    tab: 'Interface',
    section: 'Sidebar',
    sectionId: 'settings-section-sidebar',
    title: 'Expand when you hover',
    description: 'Move your cursor over the compact sidebar to open it fully, and it folds back when you leave.'
  },
  {
    tab: 'Interface',
    section: 'Sidebar',
    sectionId: 'settings-section-sidebar',
    title: 'Show recommended streams',
    description: 'Show the Recommended section in the sidebar. Turn this off to keep only your followed channels and favorites.'
  },
  {
    tab: 'Interface',
    section: 'Discover Feed',
    sectionId: 'settings-section-discover',
    title: 'Personalized recommendations',
    description: 'Opt in to account-personalized Discover recommendations, or stay anonymous. Privacy, tracking, tailored suggestions.'
  },
  {
    tab: 'Interface',
    section: 'Discover Feed',
    sectionId: 'settings-section-discover',
    title: 'Languages',
    description: 'Filter the Discover tab and sidebar recommended streams by broadcast language: only show streams in english, french, german, spanish, or any other language you pick.'
  },
  {
    tab: 'Interface',
    section: 'Motion',
    sectionId: 'settings-section-motion',
    title: 'How much the interface animates',
    description: 'Full plays every animation, Reduced keeps quick fades only, Off makes everything instant.'
  },
  {
    tab: 'Interface',
    section: 'Closing the Window',
    sectionId: 'settings-section-window-close',
    title: 'What the close button does',
    description: 'Closing the window quits StreamNook, unless MultiChat popouts are still open, in which case it minimizes to the system tray so they keep working.'
  },
  {
    tab: 'Interface',
    section: 'Keep on Top',
    sectionId: 'settings-section-window-on-top',
    title: 'Keep on top in Compact View',
    description: 'While Compact View is active, the small player floats above other apps so clicking your browser does not bury it.'
  },
  {
    tab: 'Interface',
    section: 'Settings Window',
    sectionId: 'settings-section-settings-window',
    title: 'Keep settings in a centered window',
    description: 'Settings open in a centered window; turn this off to open them as a full page that fills the app.'
  },
  {
    tab: 'Interface',
    section: 'Compact View',
    sectionId: 'settings-section-compact',
    title: 'Compact View',
    description: 'Choose the window size when entering Compact View mode. Perfect for fitting the app on a second monitor.'
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
    title: 'Show notifications',
    description: 'Turn this off to silence every notification at once; your choices below stay saved for when you turn it back on.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Methods',
    title: 'Notification Methods',
    description: 'Choose how to display notifications: Dynamic Island, toasts, toast position, edge spacing.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Methods',
    title: 'Show in the Dynamic Island',
    description: 'Notifications appear in the notification center at the top of the window.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Methods',
    title: 'Show toast popups',
    description: 'Each notification also pops up as a small card at the edge of the window you choose below.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Methods',
    title: 'Where toasts appear',
    description: 'Click a spot on the mini screen to move toasts to that corner or edge.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Methods',
    title: 'Distance from the edge',
    description: 'How far toasts sit from the top or bottom edge of the window; raise it to push them further in.'
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
    title: 'When a followed channel goes live',
    description: 'You get a notification the moment someone you follow starts streaming, and clicking it opens the stream.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'When a favorite channel goes live',
    description: 'Channels you have favorited notify you even if you do not follow them on Twitch.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'When you get a whisper',
    description: 'A notification shows each new whisper, and clicking it opens the conversation.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'When an app update is ready',
    description: 'You hear about new StreamNook versions as soon as they are available, and clicking takes you to the Updates page.'
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
    title: 'When a drop is claimed',
    description: 'A notification confirms each drop StreamNook claims for you.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'New drops in favorite categories',
    description: 'At startup, StreamNook checks your favorite categories and tells you when they have new drops to earn.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'When channel points are claimed',
    description: 'A notification confirms each channel points bonus claimed for you.'
  },
  {
    tab: 'Notifications',
    section: 'Notification Types',
    title: 'When new badges appear',
    description: 'You hear about new badges as soon as they become available to earn.'
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
    title: 'Play a sound',
    description: 'A soft sound plays with each notification, in the style you pick below.'
  },
  {
    tab: 'Notifications',
    section: 'Sound',
    title: 'Which sound to play',
    description: 'Every option is soft and short, so none of them will startle you.'
  },
  {
    tab: 'Notifications',
    section: 'Sound',
    title: 'Send a test',
    description: 'Fires a sample notification so you can check the position, sound, and style you picked.'
  },
  {
    tab: 'Notifications',
    section: 'About',
    title: 'About',
    description: 'About notifications and how to use them.'
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
    title: 'Load emotes and badges from disk',
    description: 'Stores a copy on this PC after the first download so later channel loads are near instant. Off means every launch fetches them again.'
  },
  {
    tab: 'Cache',
    section: 'Cache',
    title: 'Refresh stored data after a number of days',
    description: 'Anything older than this is fetched again the next time it is needed, so new emotes and badge art show up on their own.'
  },
  {
    tab: 'Cache',
    section: 'Cache',
    title: 'See what is stored, or clear it',
    description: 'View cache info shows a count of what is on disk, Open folder reveals the files, and Clear cache deletes every stored emote and badge so they download fresh.'
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
    title: 'Download emotes for every channel you follow',
    description: 'Scan your follows to see how many emotes are missing and how much space they need, then download them in the background while you do something else.'
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
    section: 'What lives in the palette',
    title: 'What lives in the palette',
    description: 'Overview of palette sections and available actions: quick actions, current stream, share, settings, categories, snippets.'
  },
  {
    tab: 'Command Palette',
    section: 'What lives in the palette',
    title: 'Settings',
    description: "Every settings tab and section is searchable. Type 'ad block' to land on the ad blocking panel under Integrations."
  },
  {
    tab: 'Command Palette',
    section: 'What lives in the palette',
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
    title: 'Keep a detailed log for bug reports',
    description: 'Records connection, playback, and chat activity to streamnook.log on this PC so a problem can be traced after the fact.'
  },
  {
    tab: 'Support',
    section: 'Diagnostics',
    title: 'Find the log file',
    description: 'Opens the folder that holds streamnook.log so you can attach it to a bug report.'
  },

  // === Backup ===
  {
    tab: 'Backup',
    section: 'Backup and restore',
    title: 'Backup and restore',
    description: 'Export your settings to a file, or import a saved backup to restore them after a reset, reinstall, or move to a new PC.'
  },
  {
    tab: 'Backup',
    section: 'Backup and restore',
    title: 'Save a backup',
    description: 'Writes a copy of your settings file wherever you like, such as a USB drive or a cloud-synced folder.'
  },
  {
    tab: 'Backup',
    section: 'Backup and restore',
    title: 'Restore from a backup',
    description: 'Pick a backup file and StreamNook swaps in those preferences, then reloads itself so everything picks them up.'
  },
  {
    tab: 'Backup',
    section: 'Settings file',
    title: 'Where your settings file lives',
    description: 'The folder on this PC that holds your settings file.'
  },
];
