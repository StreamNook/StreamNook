<div align="center">

<img src="src-tauri/images/logo.png" alt="StreamNook" width="200" />

# StreamNook

A native desktop client for Twitch, Kick, and YouTube.

<p>
  <a href="https://github.com/winters27/StreamNook/releases/latest"><img src="https://img.shields.io/github/v/release/winters27/StreamNook?color=ff6b6b&style=for-the-badge&logo=github&logoColor=white&labelColor=1a1a2e" alt="Latest release" /></a>
  <a href="https://github.com/winters27/StreamNook/stargazers"><img src="https://img.shields.io/github/stars/winters27/StreamNook?color=00d9ff&style=for-the-badge&logo=star&logoColor=white&labelColor=1a1a2e" alt="Stars" /></a>
  <a href="https://github.com/winters27/StreamNook/issues"><img src="https://img.shields.io/badge/Issues-ff6b6b?style=for-the-badge&logo=github&logoColor=white&labelColor=1a1a2e" alt="Issues" /></a>
  <a href="https://github.com/winters27/StreamNook/discussions"><img src="https://img.shields.io/badge/Discussions-4ecdc4?style=for-the-badge&logo=github&logoColor=white&labelColor=1a1a2e" alt="Discussions" /></a>
</p>

<p>
  <img src="https://img.shields.io/badge/Rust-orange?style=for-the-badge&logo=rust&logoColor=white&labelColor=1a1a2e" alt="Rust" />
  <img src="https://img.shields.io/badge/React-61DAFB?style=for-the-badge&logo=react&logoColor=white&labelColor=1a1a2e" alt="React" />
  <img src="https://img.shields.io/badge/Tauri-FFC131?style=for-the-badge&logo=tauri&logoColor=white&labelColor=1a1a2e" alt="Tauri" />
</p>

<h3><a href="https://github.com/winters27/StreamNook/releases/latest">Download the latest release</a></h3>

<sub>Windows. Extract and run, everything else is built in.</sub>

</div>

---

You're grinding through your 47th hour of that indie roguelike, talking to yourself about optimal build paths, when you realize *I need human voices*. But opening a stream in a browser? That's like inviting a resource-hungry elephant to sit on your CPU. Your fans spin up, your frame rate tanks, and suddenly you're choosing between watching streams and actually playing games.

StreamNook is the answer to this very specific but deeply relatable problem. Built from the ground up with Rust and React, it delivers a smooth viewing experience that sips resources instead of chugging them. The cozy corner of the internet where you can watch streams, chat with communities, and track your favorite streamers without turning your PC into a space heater. Twitch, Kick, and YouTube, all in the same window.

## Platforms

<div align="center">
  <img src="src-tauri/images/platforms.png" alt="Platform picker showing Twitch, Kick, and YouTube with live counts" width="1000" />
</div>

**One list, every platform.** A picker in the title bar switches between Twitch, Kick, and YouTube, or shows all of them at once, with a live count for each. Your follows from every connected account land in one list on Home and in the sidebar, so a single glance tells you what is live.

**Browse and watch the same way everywhere.** Directories, categories, and channel search per platform, all opening in the same player with the same quality control, theater mode, Compact View, and MultiNook.

<div align="center">
  <img src="src-tauri/images/home_platforms.png" alt="Followed channels from every platform in one list" width="1000" />
</div>

**Chat with the tools you already use.** Replies, mentions, tab completion, moderation, user cards, and room mode indicators on every platform, plus each one's own events rendered properly: Kick subscriptions, gifted subs, and pins, YouTube Super Chats, Super Stickers, memberships, and gift redemptions. Chat opens with recent history instead of an empty pane.

**7TV everywhere.** Channel emote sets, paints, and badges resolve on Kick and YouTube too, for you and for everyone else in chat. Kick's own channel emotes are in the picker, and YouTube's custom emoji are learned from chat as they arrive.

<div align="center">
  <img src="src-tauri/images/youtube_chat.png" alt="A YouTube stream with badges, emotes, replies, and moderation rendered like Twitch" width="1000" />
</div>

**Sign in once per platform.** Follows import automatically with their profile pictures, and sessions are rechecked in the background so a revoked login is flagged rather than silently returning nothing.

## Contents

[Platforms](#platforms) · [What you get](#what-you-get) · [MultiNook](#multinook) · [MultiChat](#multichat) · [Chat](#chat) · [Stream overlay](#stream-overlay) · [Drops and channel points](#drops-and-channel-points) · [Badge tracker](#badge-tracker) · [Power user](#power-user) · [Install](#install) · [License](#license)

## What you get

**Native playback up to 1440p.** A Rust-native player with picture-in-picture, theater mode, and jump-to-live-edge on every stream load. YouTube plays up to 1440p60 where the broadcaster offers it. Per-tile quality control keeps background streams light. When the channel you're watching goes offline, StreamNook can auto-switch you to another live one.

**Custom theming engine.** Built-in themes including Frosted Glass, Standard Issue, a color-pickable OLED, Dracula, Nord, Gruvbox, Tokyo Night, and Catppuccin, plus a full theme creator with color picker, live preview, and dynamic switching from the title bar. Build your own and tune every color in the UI, save it, swap back any time.

**VODs and offline-channel chat.** Watch past broadcasts the same way you watch live streams. Open chat for any offline channel and participate without waiting for them to go live, no need to load Twitch in a browser just to drop a message in someone's offline chat.

**Whispers.** Integrated send and receive with a dedicated chat window per conversation. Full history comes with you, including an import tool for prior Twitch whisper exports. Incoming whispers ping you live, never on a delay.

## MultiNook

Many streams at once, all their chats alongside. Grid layout for as many streams as you want running simultaneously, each with its own chat panel right next to it. Mix platforms freely: a Twitch stream and a YouTube stream can sit in the same grid.

<div align="center">
  <img src="src-tauri/images/multinook.png" alt="MultiNook grid running four streams across platforms" width="1000" />
</div>

**Dock, undock, drag.** Rearrange the grid however you want. Undock any tile into its own window to push it onto another monitor while the rest of the grid keeps running. Drop it back into the grid when you're done with it.

**Audio focus on click.** Whichever tile you click is the one you hear. Click another tile to switch.

**Per-tile quality control.** A background stream can sit at 480p while your focus tile runs at full quality. Watch a six-stream tournament without melting your bandwidth.

## MultiChat

Standalone chat client. Use StreamNook for chat only, no stream required.

<div align="center">
  <img src="src-tauri/images/multichat.png" alt="MultiChat with two channels split side by side" width="1000" />
</div>

**Tabs or splits.** Add 1 to N channels as tabs, or split a window into 2, 3, or 4 columns to read them all at once. Channels from any platform, side by side. Whatever layout fits how you watch chat.

**A window per monitor.** Run as many MultiChat windows as you have displays. Each window is independent, with its own channels and layout.

**Runs in the background.** MultiChat keeps going when the main window is hidden to the system tray, so you can use StreamNook as a pure chat surface and tuck the rest away.

**Pop chat out from anywhere.** Right-click any stream tile or use the chat widget's pop-out button. The popped-out chat keeps running in its own window even after you close the originating stream, so you can keep chatting long after you've moved on.

## Chat

The chat is the part this app gets right. Every piece of the live chat surface is rendered properly on every platform, and every customization you'd want is one click away.

<div align="center">
  <img src="src-tauri/images/chat.png" alt="StreamNook chat with emotes, badges, and events" width="900" />
</div>

**Every emote, everywhere.** Full 7TV, BetterTTV, and FrankerFaceZ support with animation and zero-width overlays. Apple-style emoji rendering across every surface. Native emoji picker built into the chat input.

**Twitch-native events done right.** Subscription and resubscription announcements with shareable banners, watch streak milestone banners, Hype Train overlay with progress and contributors and level-up animations, Predictions overlay with voting outcomes and channel points balance and countdown and win/loss resolution, Pinned messages, Bits cheers with animated icons and tier-colored displays. Everything Twitch surfaces in their own player, surfaced properly here too.

**Built-in copypastas.** Curated library of common copypastas you can fire with a click. Useful for when chat is moving fast and you don't have time to type the obvious thing.

**Custom command creator.** Build your own slash commands and text triggers from a UI inside the app. No config files, no restarts. Define an alias, set the expansion, save it, fire it the next time you type. Triggers can be slash-prefixed or plain text patterns.

<div align="center">
  <img src="src-tauri/images/command_creator.png" alt="Custom command creator UI" width="700" />
</div>

**Custom user profiles.** Set a nickname, custom color, and personal notes for any user. They're pulled up automatically every time that user chats. Persistent across sessions and synced across all your windows.

**Highlight phrases.** Match any text or username, assign a custom color, and pick an optional sound alert per phrase. The pings that matter actually catch your eye instead of getting lost in the scroll.

**Local-only commands.** `/clearmessages` to wipe a user from your view without involving Twitch. `/usercard <name>` to open a profile card on anyone. Plus the rest of the local chat commands, all client-side, none of them visible to the channel.

**Moderator tools.** Ban, timeout, clear, mod, VIP, all reflected in real time. Dedicated moderator log pane. Slash-command autocomplete and mention autocomplete (`@` as you type). Reply threads. Everything a mod needs without flipping back to a dashboard, on every platform you moderate.

**Spellcheck as you type.** Misspellings get a subtle underline, right click any of them for suggestions, and add names, slang, and channel in-jokes to a personal dictionary so they stop being flagged. Off in one click if you would rather not.

## Stream overlay

A chat overlay for your broadcast, designed in the app and hosted for you. Paste one browser source URL into OBS and it stays in sync as you change it.

**Design it visually.** Fonts, colors, bubbles, outlines, entrance animations, and per-source event filters, all with a live preview beside the controls.

<div align="center">
  <img src="src-tauri/images/overlay.png" alt="Stream overlay designer with a live preview" width="1000" />
</div>

**Write your own event text.** Replace each platform's system message with your own wording, built from tokens that resolve out of the real event: the name, tier, months, streak, gift count, recipient, bits, raid viewers, and more. A token with no value falls back rather than printing a blank.

<div align="center">
  <img src="src-tauri/images/overlay_events.png" alt="Custom event text with the available tokens" width="1000" />
</div>

**Control how chat reads.** Show replies in full, as a short mention, or not at all. Style links with your own color and underline, and choose whether your personal emotes appear for viewers.

**Profiles.** Keep separate looks for different scenes or games and switch between them without rebuilding anything.

## Drops and channel points

**Drops.** Campaign tracking and progress monitoring for the channels you watch, with an inventory viewer for everything you've earned. Pick a specific campaign to focus, and sign in for drops through a secure embedded browser. No separate app or window to manage.

<div align="center">
  <img src="src-tauri/images/drops.png" alt="Drops dashboard" width="900" />
</div>

**Channel points.** Claims point bonuses on channels you're watching, with a cross-streamer leaderboard so you can see where your points are concentrated. A quick toggle lives in the chat widget to turn it on or off mid-conversation. Raid auto-follow keeps your follow list current when streamers raid out.

## Badge tracker

For people who chase the collectibles. StreamNook tracks every cosmetic on the platform and lets you manage what you're wearing, when you're wearing it.

**Every Twitch badge, tracked and dated.** Browse the full collection with detail cards, live status (Available / Coming Soon / Expired), and quick actions for jumping straight to a badge's drop campaign. Newly-dropped badges show up the moment they ship.

<div align="center">
  <img src="src-tauri/images/twitch_badges.png" alt="Twitch badge collection" width="49%" />
  <img src="src-tauri/images/badge_details.png" alt="Badge detail card" width="49%" />
</div>

**Every 7TV cosmetic, kept current.** Browse every paint and badge in circulation, hover any item for set and author info, see what's new since you last checked.

<div align="center">
  <img src="src-tauri/images/7tv_paints.png" alt="7TV paints catalog" width="49%" />
  <img src="src-tauri/images/7tv_badges.png" alt="7TV badges catalog" width="49%" />
</div>

**Chatterino and Chatterino Homies badges.** Tracked and rendered next to Twitch and 7TV cosmetics, so you can spot the chat-client devs and their crews in any room.

**Manage everything from one place.** Pick which Twitch badges sit next to your name, equip and swap 7TV paints, choose which 7TV badges you wear. Everything renders live in chat the moment you change it.

**StreamNook ranks.** Every StreamNook user gets a permanent rank number based on signup order. A small StreamNook badge sits in front of your name in any Twitch chat, visible only to other StreamNook viewers. Hover it for a cypher-decode animation that resolves to your tier card and rank number. Some numbers in the registry are special. Whoever lands on them finds out why.

<div align="center">
  <img src="src-tauri/images/streamnook_badges.png" alt="StreamNook rank tiers" width="49%" />
  <img src="src-tauri/images/streamnook-profile.webp" alt="StreamNook profile and rank reveal" width="49%" />
</div>

## Power user

- **Command palette (Ctrl+K).** Jump to any channel, run any command, open any setting, all from one keyboard shortcut.
- **Cross-window settings sync.** Change a setting in any window, every other window picks it up immediately.
- **Optional auto-update.** Set it once and never think about updates again.
- **Compact view presets** for multi-monitor setups, with configurable window sizes per preset.
- **Mouse controls in the player.** Scroll to change volume, middle click to mute, shift and scroll to open the channel's About panel. Each one toggles independently.
- **Hover any channel** in the sidebar to see what it is streaming before you click.

<div align="center">
  <img src="src-tauri/images/command_palette.png" alt="Command palette (Ctrl+K)" width="700" />
</div>

## Lives where you work

- Dynamic Island notification center for drops progress, channel points, live alerts, and updates.
- Native desktop notifications with stream thumbnails, custom sounds, and one-click launch.
- Discord Rich Presence.
- System tray persistence so chat keeps flowing while the main window is hidden.

## Built on

Rust, TypeScript, React, and Tailwind. Packaged as a native desktop app with Tauri.

## Install

1. Grab the latest build from the [Releases page](https://github.com/winters27/StreamNook/releases/latest).
2. Extract and run.
3. Follow the setup wizard to sign in with Twitch, then connect Kick or YouTube from Settings whenever you want them.

Everything's built in. Nothing else to install.

## Credits

- [Tauri](https://tauri.app/), native desktop framework.
- [Plyr](https://plyr.io/), video player.
- [HLS.js](https://github.com/video-dev/hls.js), HLS streaming support.
- [7TV](https://7tv.app/), extended emotes and cosmetics.
- [Twitch](https://dev.twitch.tv/), [Kick](https://kick.com/), and [YouTube](https://www.youtube.com/), platforms and APIs.

## License

StreamNook is source-available under the
[PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0),
plus additional permissions granted in [LICENSE](LICENSE). Read that file for
the terms that actually apply.

The short version:

- **Using StreamNook is always free**, including if you make money streaming.
  Being a partnered or monetized streamer does not make your use commercial.
- **Forking, modifying, and republishing are allowed**, as long as your fork
  stays free to obtain and use, and you keep the LICENSE file with it.
- **Donations and sponsorship for a fork are allowed.**
- **Selling it is not.** No paid builds, paywalled features, bundling into a
  product you sell, or running it as a paid service. For a commercial license,
  open an issue or get in touch.

Third-party components keep their own licenses. See
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

> **Note on the earlier MIT notice.** Before 2026-08-21 this README stated
> StreamNook was MIT licensed, though no LICENSE file was ever published. That
> notice is withdrawn going forward. It is not revoked retroactively: if you
> obtained a copy while it stood, your MIT rights in that copy stand. Releases
> from 2026-08-21 onward are under the license above.

---

<div align="center">

<sub>StreamNook is not affiliated with Twitch Interactive, Inc., Kick, or Google LLC.</sub>

</div>
