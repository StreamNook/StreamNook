## [8.6.1] - 2026-09-07

> [!NOTE]
> **Two releases in one day, so nothing gets missed.** 8.6.0 shipped a few hours
> before this one. If you are updating from an older build you may never see its
> release page, so its notes are repeated here in full.
>
> **8.6.1 is not just a fix release.** It adds GIFs in chat and live 7TV emote
> updates. Those are listed at the very bottom of these notes, under
> "New in 8.6.1", below the 8.6.0 notes.

---

## From 8.6.0

## 🎉 New: Chat grows up
> The biggest chat release StreamNook has had. Search any chat, save filters, hide phrases, review AutoMod holds without leaving chat, split MultiChat however you like, float a see-through chat over a fullscreen game, paste a screenshot straight into your message, and browse every command with plain explanations and examples. The AutoMod queue needs two new Twitch permissions, so StreamNook asks you to sign in once more on first launch.

![Chat grows up](https://raw.githubusercontent.com/winters27/StreamNook/main/.github/assets/release-8.6.0-chat-grows-up.webp)

---

### ✨ Features
- **Paste an image to upload it.** Copy a screenshot, press Ctrl+V in the chat box, and the link lands in your message. Choose i.nuuls.com, catbox, Litterbox, uguu or your own host; links from all of them render as image cards in chat.
- **Search any chat with Ctrl+F.** Searches the recent history of that chat, with filters like from:, badge:, has:link and regex:, and a result list you can step through.
- **Saved filters and ignored phrases.** Build reusable chat filters from eight presets or short expressions of your own, and pick one from a chip above chat. Phrases you never want to see are dropped before they reach the screen.
- **Highlights, mentions and filters are decided in one place.** Every message is checked once, in Rust, so popouts and MultiChat panes always agree, and custom highlight sounds can be your own files.
- **AutoMod queue inside chat.** Held messages appear in a strip above the message box with Allow and Deny, and restricted users show up marked so moderators can act on them.
- **Streamer mode.** Turn it on, or let StreamNook notice OBS and friends, and chat hides link previews, viewer counts, sounds and low-trust users while you are live.
- **MultiChat splits, mentions feed and more windows.** Arrange panes in any split layout, open a mentions-only feed, spin up extra MultiChat windows, and move focus between panes with Ctrl+Alt+Arrows.
- **Chat over a fullscreen stream, and a floating see-through chat.** Fullscreen keeps chat readable over the video, and /overlay opens an always-on-top glass chat you can park over a game, fade to invisible, and make click-through (Ctrl+Alt+N, or the tray).
- **A command menu that explains itself.** The new button left of the emote picker lists every command, what it does, who can use it, and examples you can click into the box. Commands you cannot run in this channel show locked, and there is a larger reference view when you want more room.
- **New slash commands.** /r replies to your last whisper, /clip, /chatters, /ignore, /uptime, /overlay, /settitle and /setgame, and /popup for a new MultiChat window.
- **Pronouns and private notes on user cards.** Pronouns come from pronouns.alejo.io when you opt in; notes stay on your PC.
- **12-hour or 24-hour timestamps, animated emotes on hover only, dimmer backfill.** Three small chat design options people kept asking for.
- **See every StreamNook badge and Atmosphere in profile customization,** with the ones you have not earned yet shown locked.
- **Moltorino supporter badges**, and a Chat Clients gallery in the Badges window showing the badges every third-party client hands its developers and supporters.
- **Settings explain themselves.** Every row now says what you get and what it costs in plain language, long tabs have a section rail that follows you as you scroll, and image uploads have their own section.

### 🐛 Bug Fixes
- **Community badges that are taken back now disappear right away** instead of lingering until the next restart.
- **Release posts in Discord now carry the release picture.**

### 🔧 Maintenance
- **A resource line in the log every minute** breaks memory down by process (app, browser, GPU, each window, plugins), so a memory report can be read off the log.

---

## New in 8.6.1

### ✨ Features
- **GIFs in chat.** Chat GIFs appear as the animated image rather than a bracketed description, with a new GIF tab in the emote picker for searching and sending. The tab only appears in channels where your account can actually use it.
- **GIFs on the stream overlay.** The overlay draws chat GIFs at the gigantified size and follows your Giant emote placement, with its own toggle if you would rather show the description instead.
- **Chat GIF controls.** A Chat GIFs switch shows GIFs, or a chip that reveals them on click, and it follows your Animate emotes setting the same way animated emotes do.
- **7TV emote changes apply live.** When a channel adds, removes or renames a 7TV emote, chat and the emote picker pick it up as it happens instead of at the next restart.

### 🐛 Bug Fixes
- **Channel emote aliases are back.** Channels using one emote under two names kept only one of them, and a discovery list was being merged into channel sets, which could replace a channel's own name for an emote and render emotes nobody else in the room could see.
- **Large 7TV channels load again.** Very large emote sets timed out and tripped a cooldown that briefly took every other 7TV request down with it. Big sets now get the time they need, and only a genuine outage triggers the cooldown.
- **A steady live indicator.** The player flipped between LIVE and a timestamp, and the broadcast timeline's playhead slid backwards, several times a minute on a perfectly healthy stream.
- **Profile badges show up reliably.** A naming mismatch could group third-party badges under Other, or hide a member's chosen badge outright, depending on which lookup answered first.
- **Popovers stay readable at low glassiness.** At Glassiness 0 popovers kept their transparency but lost their frost, so chat scrolled visibly through them.
- **Emote picker recovers on its own.** When a channel's 7TV data failed to load, the picker held a fallback set until a manual refresh. It now re-checks quietly and fills in once 7TV answers.

### 🔧 Maintenance
- Refreshed dependencies, including the video playback libraries.
- Update packages are now rejected unless their signature verifies.

## [8.6.0] - 2026-09-07

## 🎉 New: Chat grows up
> The biggest chat release StreamNook has had. Search any chat, save filters, hide phrases, review AutoMod holds without leaving chat, split MultiChat however you like, float a see-through chat over a fullscreen game, paste a screenshot straight into your message, and browse every command with plain explanations and examples. The AutoMod queue needs two new Twitch permissions, so StreamNook asks you to sign in once more on first launch.

![Chat grows up](https://raw.githubusercontent.com/winters27/StreamNook/main/.github/assets/release-8.6.0-chat-grows-up.webp)

---

### ✨ Features
- **Paste an image to upload it.** Copy a screenshot, press Ctrl+V in the chat box, and the link lands in your message. Choose i.nuuls.com, catbox, Litterbox, uguu or your own host; links from all of them render as image cards in chat.
- **Search any chat with Ctrl+F.** Searches the recent history of that chat, with filters like from:, badge:, has:link and regex:, and a result list you can step through.
- **Saved filters and ignored phrases.** Build reusable chat filters from eight presets or short expressions of your own, and pick one from a chip above chat. Phrases you never want to see are dropped before they reach the screen.
- **Highlights, mentions and filters are decided in one place.** Every message is checked once, in Rust, so popouts and MultiChat panes always agree, and custom highlight sounds can be your own files.
- **AutoMod queue inside chat.** Held messages appear in a strip above the message box with Allow and Deny, and restricted users show up marked so moderators can act on them.
- **Streamer mode.** Turn it on, or let StreamNook notice OBS and friends, and chat hides link previews, viewer counts, sounds and low-trust users while you are live.
- **MultiChat splits, mentions feed and more windows.** Arrange panes in any split layout, open a mentions-only feed, spin up extra MultiChat windows, and move focus between panes with Ctrl+Alt+Arrows.
- **Chat over a fullscreen stream, and a floating see-through chat.** Fullscreen keeps chat readable over the video, and /overlay opens an always-on-top glass chat you can park over a game, fade to invisible, and make click-through (Ctrl+Alt+N, or the tray).
- **A command menu that explains itself.** The new button left of the emote picker lists every command, what it does, who can use it, and examples you can click into the box. Commands you cannot run in this channel show locked, and there is a larger reference view when you want more room.
- **New slash commands.** /r replies to your last whisper, /clip, /chatters, /ignore, /uptime, /overlay, /settitle and /setgame, and /popup for a new MultiChat window.
- **Pronouns and private notes on user cards.** Pronouns come from pronouns.alejo.io when you opt in; notes stay on your PC.
- **12-hour or 24-hour timestamps, animated emotes on hover only, dimmer backfill.** Three small chat design options people kept asking for.
- **See every StreamNook badge and Atmosphere in profile customization,** with the ones you have not earned yet shown locked.
- **Moltorino supporter badges**, and a Chat Clients gallery in the Badges window showing the badges every third-party client hands its developers and supporters.
- **Settings explain themselves.** Every row now says what you get and what it costs in plain language, long tabs have a section rail that follows you as you scroll, and image uploads have their own section.

### 🐛 Bug Fixes
- **Community badges that are taken back now disappear right away** instead of lingering until the next restart.
- **Release posts in Discord now carry the release picture.**

### 🔧 Maintenance
- **A resource line in the log every minute** breaks memory down by process (app, browser, GPU, each window, plugins), so a memory report can be read off the log.

## [8.5.5] - 2026-09-06

## 🎉 StreamNook Community Month is on
> Two new animated badges. Kindred, the cube heart, goes to anyone who brings a friend into the StreamNook Discord during Community Month, which runs through the end of September. Uplift, the rocket, is the standing thank-you for boosting the Discord, any time. Both appear in chat and on your profile the moment they are granted.

![Kindred and Uplift](https://raw.githubusercontent.com/winters27/StreamNook/main/.github/assets/release-8.5.5-community-month.webp)

---

### ✨ Features
- **Scrub back through any live stream.** The progress bar under a live stream now spans the whole broadcast, not just the part you have watched. Hover to see the time you would land at and how long ago that was, drop anywhere to go there in the stream's recording, and drag to the end to return to live. The scale is finer near live and coarser further back, so both are easy to hit. Channels that keep no VODs get a note instead of a dead scrub.
- **VODs remember where you left off.** Reopen a past broadcast and it resumes at your last position, with the chat replay following. Video cards show how far you got. A player setting turns resume off if you prefer starting from the top.
- **Badge details list the exact categories a badge can be earned in**, as real category cards you can jump to, instead of a line of prose.

### 🐛 Bug Fixes
- **VODs of a stream that is still live now play and seek properly.** They were being treated as a live stream, which snapped playback back to the end and broke seeking. (#216)
- **Badges and accolades no longer vanish from profiles when a lookup fails.** A Twitch change that removed the old badge query, or a brief network hiccup, wiped every badge until the next refresh; the last good set is kept instead.
- **The Drops channel picker only offers channels that are actually earning the drop right now**, per Twitch, instead of guessing from the allow list or category.
- **Your own chat message no longer shows twice for a moment** after sending an emote from the picker.
- **StreamNook stays in the tray under "Always minimize" when its last window closes**, instead of quitting and taking the tray icon with it.
- **7TV channel emotes are not marked as loaded before they actually parse.**

### ⚡ Performance
- **Chat rendering runs under the React Compiler**, with chat state written copy-on-write, so heavy chat costs less per message and the player keeps its frames.

## [8.5.4] - 2026-09-03

### 🐛 Bug Fixes
- **Chat now connects on networks that block the old chat port.** Some internet providers, routers and security suites drop the port Twitch chat has always used, which left chat showing older messages and never updating. Chat falls back to Twitch's secure connection on the standard web port, remembers that it had to, and goes straight there next time.
- **Age-restricted YouTube streams play when you are signed in.** A stream the anonymous request was refused for is retried with your YouTube session instead of stopping at a warning.
- **Accolades are called accolades on your profile.** Unlock text said "badge", which is the chat cosmetic, and Restless appeared under its internal codename.

## [8.5.3] - 2026-09-02

## 🎉 New: Prism, a theme made of light
> Prism is a dark, colorless theme where refracted light lands on the glass: a soft bloom in a corner, a streak of separated color across a dialog, a catch of light on a hovered button. It is free for everyone in Theme settings. Alongside it, the Prism set: Facet, a supporter badge; Lumen, a subscriber badge; and Dispersion, an atmosphere that puts dispersed light behind a subscriber's messages and profile. The set is available through October 31.

![Prism and the Prism set](https://raw.githubusercontent.com/winters27/StreamNook/main/.github/assets/release-8.5.3-prism.png)

---

### ✨ Features
- **Prism theme.** Ivory on near-black, with light instead of color: the interface stays neutral and the spectrum appears only where it would in a photograph of glass. It scales with your glassiness setting, so a solid setup gets a quieter version.
- **The Prism set, through October 31.** Supporters get Facet, subscribers get Lumen and the Dispersion atmosphere as well as Facet. Anyone who holds a tier before the end of October keeps their pieces for good.
- **A rebuilt Stream Overlay builder.** Eight short sections instead of one long scroll, a row of tiles to jump between them, a dot on any section you have changed, and one short line per setting with the detail a hover away.
- **Atmospheres can shape their chat wash.** An atmosphere can drop the left edge, defocus its image behind your messages, and draw a faint rim around the row, so a crisp piece of art reads as light on the row instead of a cropped picture.

### 🐛 Bug Fixes
- **Links like test.fr are recognized**, not only ones starting with https or www, and a full stop after a link stays out of it.
- **The shared-chat banner clears when a shared session ends**, instead of staying up until you change channels.

### ⚡ Performance
- **Chat memory no longer grows for as long as a channel is open.** The buffer trim leaked one row per flush, which on a busy channel added up to over a gigabyte in half an hour.
- **Far less CPU while watching.** Decorative animations in the title bar and update pill are held still during playback and run again while you browse; they were repainting the window on every frame.
- **Lighter badge and emote cache.** Lookups read one entry instead of cloning the whole manifest, the manifest is written compactly and atomically at a gentler interval, and the YouTube feed poll no longer re-encodes each response just to log its size.

## [8.5.2] - 2026-08-30

## 🎉 New: Favorites, a watchlist that spans every platform
> Star any channel on Twitch, Kick or YouTube, whether or not you follow it, and it gets its own section on Home and in the sidebar the moment it goes live. The section stays clear of anyone who is not streaming, and a favorite that is offline is still reachable further down with your other offline channels. Favorites also have their own go-live alert switch, so a long favorites list never forces you to silence alerts for the people you follow.

---

### ✨ Features
- **Favorites work everywhere, following optional.** The heart is on every stream card now, including category browsing, search results and offline channels, so you can save a channel the moment you find it. Favorited channels report who is live across all three platforms, and a channel you just starred appears right away instead of waiting for the next check. The Favorites section only ever lists channels that are actually live; a starred channel that is offline sits with your other offline channels, keeping its name and picture so you can still get back to it.
- **MultiNook runs Kick and YouTube.** One grid can hold everything you watch instead of Twitch only, including several YouTube streams at once. Add to MultiNook is offered on every platform, and stays disabled with the reason on hover for anything the grid cannot run yet.
- **The grid picker searches every platform at once.** Add Stream and the preset editor look across all supported platforms together, and results from the smaller platforms sit near the top instead of being buried under Twitch matches. You can also type a platform prefix such as `kick:name` to add a channel the search did not surface.
- **Presets remember which platform each channel is on**, so a mixed grid reloads exactly as you saved it. Two channels that share a name on different platforms stay separate everywhere: adding, removing, chat and presets.
- **Hide bots and specific people from chat.** Mute a chatter or a bot for one channel or everywhere, so the giveaway and gambling spam stops without you having to read past it.
- **A separate go-live switch for favorites**, so you can be alerted about a starred channel you do not follow, and keep follow alerts tuned independently.

### 🐛 Bug Fixes
- **Chat no longer misses messages on fast channels.** Repeat collapsing is opt-in and respects the threshold you set, and coming back from a pause keeps your scrollback instead of cutting it in one go.
- **YouTube chat stays connected.** It no longer goes silent partway through, and closing a tile actually stops its chat instead of leaving it running in the background.
- **Channel point redemptions show their emotes** instead of printing the emote name as plain text, and unrecognized emoji shortcodes no longer appear twice.
- **1440p and 2160p on YouTube keep working** when you come back to a stream later in the same session, instead of quietly dropping to 1080p. High resolution playback also stutters and stalls far less, and closing a stream releases what it was holding so the next one can still go above 1080p.
- **Opening the quality list on a YouTube stream no longer restarts it** on a different resolution.
- **YouTube channels are detected as live** whether or not you have opened their chat, so the live list and favorites stop missing them, and tiles show real channel details rather than a bare video id.
- **Favoriting a Kick or YouTube channel saves that exact channel.** A Kick channel could previously be stored as though it were a Twitch one, and a YouTube favorite saved a single broadcast rather than the channel, so neither ever came back as live. Existing YouTube favorites are repaired on launch.
- **A favorited Kick or YouTube channel appears once**, not twice under both Favorites and Followed, and go-live alerts no longer double up for a channel you both follow and favorite. A stream that briefly drops and returns no longer announces itself again.
- **Starring several channels quickly keeps all of them**, and un-starring then immediately re-starring no longer removes it a second later.
- **Kick follows are keyed correctly**, so following and unfollowing from any surface agrees with what the sidebar shows.
- **The player control bar renders correctly again**, in the main player and on MultiNook tiles, and the LIVE badge stays steady instead of flickering between LIVE and a timer.
- **Smoother catch-up when you fall behind live.** The player eases back to the edge against real buffer rather than overspeeding into a stall, which also removes the audio crackle that came with it.
- **The hype train wave animates cleanly** without a dark box behind it.
- **Channel search results past the first twenty** show the correct viewer counts, thumbnails and start times.
- **In MultiNook, the follow button and channel details match the tile you selected**, so two channels on different platforms that share a name can no longer be mixed up.
- **Adding two streams in quick succession** no longer risks the two tiles colliding on the same slot.
- **The reminder picker offers Twitch channels only**, so you cannot set a reminder that could never fire.

### ⚡ Performance
- **The app starts noticeably faster.** Boot no longer waits on two network round trips in a row, settings panels and the emoji set load when you first open them rather than up front, and the window paints the branded loader instead of flashing white.
- **Chat opens almost immediately** when you switch channels or open another chat tab. Emote lists load in the background instead of holding the connection open, so a slow emote provider no longer leaves chat blank for several seconds.
- **Paints, badges and personal emotes fill in right away** instead of trickling in one chatter at a time, including on a busy channel join with a large backlog.
- **Busy channels are much lighter.** Chat rows only re-render when they actually change, and several per message scans over the whole buffer are gone, so a fast chat costs a fraction of the CPU it used to.
- **YouTube streams start quicker**, because the same background lookup is no longer repeated up to three times before the first frame.
- **Long sessions stay bounded.** Caches that grew for the life of the process are now capped, and settings and log writes are batched instead of rewriting whole files on every change.
- **The download is smaller** and the app uses less memory for chat badges and emotes.

### 🔧 Maintenance
- **Update signing is now in place end to end.** Release packages can be signed and the app knows how to verify them against a pinned key, so a corrupted or tampered download can be refused rather than installed. Verification is not being enforced yet: this release lays the groundwork so it can be switched on without another update.
- **Content loaded into the app cannot run code of its own.** Chat, emotes and embedded pages are confined, and channel About panels are cleaned before display, so a channel cannot hide anything harmful in its own panel text.
- **Profile and cosmetic changes are saved over a signed-in connection**, so only you can change yours, and a failed profile save no longer stops your presence from updating.
- **Chat atmospheres only paint for members who genuinely unlocked them.**

## [8.5.1] - 2026-08-25

## 🎉 New: A smarter Discover feed
> The Discover tab and the sidebar's Recommended section now start from Twitch's real recommendation feed, the same one twitch.tv uses, instead of a plain most-viewers list. Recommendations are anonymous by default, nothing tied to your account is sent unless you choose otherwise, and you can now limit them to the languages you actually speak.

---

### ✨ Features
- **Recommendations, anonymous by default.** Out of the box, StreamNook asks Twitch for recommendations without attaching your account, so suggestions come from broad signals like your region and what's popular, not from who you are or what you watch. If you would rather have suggestions tailored to your account, turn on Personalized recommendations under Settings, Interface, Discover Feed. It stays off until you enable it, and switching it back off returns you to fully anonymous recommendations.
- **Pick your stream languages.** The new Languages filter in the same Discover Feed section limits the Discover tab and the sidebar's Recommended section to broadcast languages you choose, like English, French, or German. Select as many as you want, or leave it empty to see every language. Scrolling past the recommendations still continues into the global top streams, and the filter applies there too.
- **Mute everything in MultiNook at once.** A new speaker button in the MultiNook toolbar silences every tile in one click, handy when someone walks in or a call comes through. Unmuting brings back exactly the audio you had before, and unmuting a single tile by hand also lifts the global mute.
- **Easier bug reports.** A new Diagnostics toggle on the Support page controls verbose logging, so you can produce a useful log file without editing settings by hand.

### 🐛 Bug Fixes
- **Browsing YouTube works again.** The YouTube Categories and Discover views and the sidebar section reported "not supported" in 8.5.0. They now load as intended, and the underlying cause can no longer regress silently.
- **Clicking a sidebar channel while in MultiNook now takes you to that stream.** Previously the click appeared to do nothing and left you in the grid. Reminder: Ctrl+click does the opposite and adds the channel as a new MultiNook tile instead of leaving.

## [8.5.0] - 2026-08-25

## 🎉 New: Kick and YouTube join Twitch
> StreamNook is no longer a Twitch only app. Browse Kick and YouTube, watch them in the same player, and use the same chat you already know. Connect your accounts and follows from every platform land in one list, so a single glance tells you what is live.

![Kick and YouTube join Twitch](https://raw.githubusercontent.com/winters27/StreamNook/main/.github/assets/release-8.5.0-platforms.png)

---

### ✨ Features
- **Kick and YouTube, everywhere Twitch already was.** A picker in the title bar switches between platforms or shows all of them at once, with a live count for each. Everything below works the same on all three.
  - **Browse and watch.** Directories, categories and channel search per platform, in the same player with the same quality picker, theater mode, Compact View and MultiNook.
  - **Your account.** Sign in once and follows import automatically with their profile pictures, so offline channels look right immediately. Sessions are rechecked in the background, so a revoked login is flagged instead of silently returning nothing. Follow and unfollow from the heart, and open a Kick subscription or YouTube membership where the creator offers one.
  - **Chat, with the tools you actually use.** Send, reply, mention and tab complete. Ban, timeout and delete from the same drag targets, hover dock and slash commands as Twitch, with your moderator status checked properly rather than guessed. User cards carry recent messages, account age, follow and subscription state. Room modes show when a channel is in slow, followers only, subscriber only or emote only mode, and the composer says why a message was refused instead of quietly dropping it.
  - **Every event each platform sends.** Kick subscriptions, gifted subs, pins, deletions and bans. YouTube Super Chats and Super Stickers in their real tier colors, memberships and milestones, gift purchases and redemptions, mode changes and pinned banners.
  - **Chat opens with history** instead of an empty pane, so you arrive with context rather than waiting for someone to talk. Offline Kick channels show their chat too.
  - **Emotes and cosmetics.** 7TV channel sets and globals load for Kick and YouTube and appear in the picker, in chat and in tab completion. Kick's own channel emotes send correctly so they render for everyone else on kick.com, and YouTube's custom emoji are learned from chat as they arrive. 7TV paints and badges render for you and everyone else, resolved from whichever account they linked.
  - **YouTube specifics.** Streams play up to 1440p60 wherever the broadcaster offers it, with the rest of the quality ladder still there to step back down to. And you choose which chat to read: Live chat is everything, Top chat is YouTube's own filtered view that keeps a very fast chat readable.
- **Write your own event text on the overlay.** Every event category can carry your own wording in place of the platform's system message, built from tokens that resolve out of the real event: the name, tier, months, streak, gift count, recipient, bits, raid viewers, channel and more. Nothing is invented, so a token with no value falls back rather than printing a blank.
- **Finer control over how overlay chat reads.** Choose whether replies show the full quoted parent, a short mention, or nothing at all. Style links with your own color, with or without an underline, or leave them plain. And decide whether your personal emotes appear for viewers.
- **Spellcheck in the chat box.** Misspellings get a subtle underline as you type, right click any of them for suggestions, and add names, slang and channel in-jokes to a personal dictionary so they stop being flagged. Fully optional and off in one click.
- **Mouse controls in the player.** Scroll anywhere over the video to change volume and middle click to mute, with a readout that appears while you are adjusting and fades once you stop. Shift and scroll opens the channel's About panel. Each one can be turned off on its own, and the volume step is yours to set.
- **Know what a channel is streaming before you click.** Hovering a channel in the sidebar shows its title, category and viewers, and Compact View gained a live stats readout of its own.

### 🐛 Bug Fixes
- Fixed chat that could go quiet while still looking connected. A session that stopped receiving is now spotted and revived, instead of passing as healthy because sending still worked.
- Fixed a maximized window that could grow slightly larger every time it was dragged.
- Fixed shared chat headers being clipped, and follow and subscribe buttons showing on streams where they do not apply.
- The mod logs panel now says why it is empty. "You are not a moderator here", "your login expired" and "this user never spoke" used to look identical, all of them a blank panel.
- Sign-in popups now stay inside the app. Google's account picker in particular used to escape as a bare undecorated window floating over everything.

### 🔧 Maintenance
- Playback recovers on its own when a stream's access credentials expire mid session, instead of stalling or dropping to a lower quality.

## [8.4.2] - 2026-08-16

## 🔦 Feature Spotlight: The Command Palette
> Meet the Feature Spotlight, a new series in these release notes. Each spotlight shines a light on something StreamNook can already do that you might not have discovered yet. First up: press Ctrl+K anywhere and the command palette opens, one search box that finds everything in the app.

![The StreamNook command palette](https://raw.githubusercontent.com/winters27/StreamNook/main/.github/assets/spotlight-command-palette.png)

---

### ✨ Features
- If you have never tried the command palette, here is what Ctrl+K can do:
  - Jump straight to any streamer you follow, search for new ones, or browse categories.
  - Open any settings page instantly. Every setting is searchable by name, no digging through tabs.
  - Fire quick actions without touching the mouse: MultiChat, the Drops center, Badges and Paints, Whispers, your profile, Compact View, full screen, and more.
  - Act on the current stream: jump between your open chats, share the stream, or find recent chatters from the channel you are watching.
  - Save chat snippets with custom aliases and favorites, then send them straight from the palette. There is a built-in guide and snippet manager too, just search "guide".
- The command palette now introduces itself: look for hints in the loading messages, the settings sidebar, and the setup wizard.

### 🐛 Bug Fixes
- Fixed chat that could silently stop receiving new messages while sending kept working, even after a refresh. The chat connection now heals itself: dead or stalled connections are detected within about a minute, reconnection retries automatically with smart backoff, and sign-in credentials refresh on every attempt instead of going stale.
- Messages missed while chat was reconnecting are now fetched and stitched back into their correct place in the conversation, with an inline marker showing where the connection dropped.
- Reconnecting with many chats open now rejoins channels at a safe pace, avoiding Twitch's join limits.
- Fixed stream overlays that could collapse onto one shared link when switching profiles while edits were saving. Each profile now keeps its own overlay URL, and affected setups repair themselves the next time the profile is published.

### 🔧 Maintenance
- Chat connection events are now kept in a small diagnostic log, making connection reports much easier to track down.

## [8.4.1] - 2026-08-14

### 🐛 Bug Fixes
- Custom channel cheermotes now animate in chat with the right tier colors and art at every bits amount. Channels with their own cheermote (and even Twitch's Cheerwhal) used to show up as plain text.
- Overlay event cards now look identical for your viewers and in the builder preview. Cards on the hosted overlay rendered with smaller padding and rounding than the preview showed, and an event line could sit too high inside its own highlight at tighter line heights.

## [8.4.0] - 2026-08-10

## 🎉 New: Chat replay on clips
> Clips now play with the chat that was live when they happened, right beside the video. Scrub backwards and the chat rewinds with you. It works for clips opened from a profile, from a link in chat, or from browse, and it shows paints, badges and emotes exactly the way live chat does.

---

### ✨ Features
- Clips now replay the chat from the moment they were captured, in a panel beside the video. Turn it off any time in Chat settings.
- VODs opened from a streamer's Videos tab now show chat replay. Previously only the Watch VOD shortcut did.
- Your stream overlay gained text controls. Justify left, center or right, pick a text weight, and turn on italic or strikethrough.
- Overlay text shadow is now adjustable. Choose its color, size and strength instead of just on or off.
- Gigantified emotes on the overlay can sit left, centered, right, or inline right after the name.
- Bits messages on the overlay can render as an event card like subs and raids, or stay inline as a normal message.

### 🐛 Bug Fixes
- The profile popout no longer opens on a different monitor than the one you clicked on.
- Streamer profile pictures now appear while watching a VOD or a clip.
- The loading mark animates again for anyone with Windows animation effects turned off.
- Scrubbing backwards in a clip now takes away the chat that has not happened yet.

### 🔧 Maintenance
- A fresh set of loading messages and emotes.

## [8.3.12] - 2026-08-07

### ✨ Features
- Gigantified emotes now render like they do on Twitch: when someone uses the "Gigantify an Emote" power-up, the last emote in their message shows up big beneath the message, in both chat and your stream overlay. Turn it off any time in Chat > Render Style or in the overlay builder.
- Bits cheers show Twitch's animated bits gem, colored by the cheer amount, instead of a static icon.
- Run multiple stream overlays at once, each with its own style and its own OBS link. Create, rename, duplicate, and delete overlays right in the builder. Perfect for a clean look on face-cam scenes and a louder one for events.
- Watch streaks on the overlay now match the app: the fire icon, the streak count, any channel points earned, and the viewer's own message.

### 🐛 Bug Fixes
- Fixed the app not switching away when the stream you were watching ended; it kept trying to restart a dead stream instead.
- Fixed channel points and drops sometimes staying pointed at the previous channel after switching streams, and a case where an ended stream could keep counting watch time against its old broadcast.
- Drops now show as disconnected at startup if the stored connection has expired, instead of claiming to be connected while earning nothing.
- Event labels in the overlay builder now match each platform (Bits on Twitch, Super Chats on YouTube).

### 🔧 Maintenance
- Polished the overlay builder: tidier controls, sub-settings visually grouped under the setting they belong to, and the 7TV logo shown where 7TV features are named.

### ⚡ Performance
- Fixed a slow memory build-up over long sessions from chatter cosmetics caching, and stopped chat restarts from leaving stale background work and open connections behind.

## [8.3.11] - 2026-07-31

## 🎉 A big one for streamers and mods
> Polls and predictions can now be built, started and resolved from inside StreamNook. Chat learned to fold a copypasta wave into a single counted row. BetterTTV emote modifiers do what they do on BetterTTV. And a stack of window fixes ends the growing, drifting and off-screen behaviour on scaled displays.

---

### ✨ Features
- Run polls and predictions without leaving StreamNook. On your own channel a button beside the message box opens a builder with a live preview of exactly what viewers will see: write the question, add choices, pick how long it runs, and optionally let people buy extra votes with channel points. Recent ones are remembered, so running the same poll again is two clicks. /poll and /prediction open it too.
- End, cancel, lock and pay out from where you already are. /endpoll, /cancelpoll, /lockprediction, /cancelprediction and /completeprediction all work, and the moderator menu grows live controls whenever a poll or prediction is running.
- A poll and a prediction can both be on screen now. Twitch allows both at once, and the two cards used to land in the same spot and paint over each other. They stack instead, and Chat settings picks which one sits on top.
- Copypasta waves fold into one line. When several people post the same thing at once, chat keeps the first and counts the rest onto it as a small x12, with a tooltip naming who joined in. Emote-only messages count too, since that is the usual case. You can loosen or tighten the match, set how many copies it takes, pick a colour, keep mods, VIPs and the streamer on their own rows, and show everything in channels you moderate.
- BetterTTV emote modifiers work in chat. Prefix an emote with w!, h!, v!, l!, r!, c!, p!, s! or z! and it stretches, flips, rotates, tints, parties, shakes or closes the gap. BetterTTV's zero-width overlay emotes sit on the emote before them properly too. There is a toggle in Chat settings, and the emote picker previews each effect and says which way a modifier attaches.
- Pin messages and manage blocked terms from chat. /pin takes a message id or just a name and pins that person's latest message, with an optional duration; /unpin clears it. /blockterm and /unblockterm add and remove AutoMod phrases, and the moderator menu has a one-line field for blocking a phrase on the spot.
- See what a /nuke would hit before you run it. While you type the command, a line above the input counts the messages and people currently matching. /nuke stop ends a running window early, and you can write the reason recorded against nuke bans and timeouts.
- Save your ban and timeout reasons. Moderation settings takes a list and the first one is filled in for you. Shift-click any timeout or ban control on a message and the command drops into the box with the reason ready to edit instead of firing straight away.
- Broadcasters can mod and VIP straight from a user card, and the card now explains when a moderator cannot action someone rather than letting the click fail.
- User cards say more, and show only what you want. Suspended accounts are marked, you can see how many people are in that person's own chat, dates carry a plain "how long ago", and there is a link to their 7TV profile. Every row has its own toggle under Chat > User Cards.
- MultiChat tiles and the player overlay show who you are watching. Each tile carries the streamer's avatar, name, partner mark and current stream title, and the title refreshes itself while the grid is open. The main player picked up the same identity row.
- Keep the Compact View player on top. A pin appears in the title bar while Compact View is active, so the small player stays in front of your browser. Ctrl+Shift+A toggles it, and it releases as soon as you leave Compact View.
- Choose what the close button does. Interface settings can quit, always minimize to the tray, or keep today's behaviour of only minimizing while MultiChat popouts are open.
- Strip the message box down to just the box. Chat settings can hide the placeholder text, the emote button and the points balance. The points button comes back on its own whenever a bonus chest is waiting, so hiding it never costs you points.
- Click your points balance to refresh it, for when it still looks stale.

### 🐛 Bug Fixes
- The window no longer grows off the edge of the screen. On any display above 100% scaling the aspect-ratio lock was mixing two kinds of pixel and multiplying the window a little more on every resize. Sizes are worked out in the right units now and clamped to the screen, and a window already saved too big is pulled back at startup.
- Dragging a maximized window restores it first, the way every other app does, instead of sliding a full-screen window around. Double-click to restore still works.
- Leaving Compact View puts the window back exactly as it was, maximized included, instead of restoring it as a floating window with maximized dimensions.
- The aspect-ratio lock no longer fights OS fullscreen, and no longer reacts to its own resizes, which is what made it jitter back and forth by a pixel or two.
- Closing your last MultiChat popout no longer quits the app when you have asked it to live in the tray.

### 🔧 Maintenance
- One shared on/off switch and one shared overlay card behind the settings screens and the chat overlays, so a change to either lands everywhere at once.

## [8.3.10] - 2026-07-28

### 🐛 Bug Fixes
- Cosmetics that belong to another slot, like the Cologne Medallion, no longer take over your StreamNook badge in chat. If one had replaced your badge, it comes back on its own with nothing for you to do.
- Badge drop times now read in your own timezone instead of as a raw UTC timestamp, in the toast, the desktop notification and the notification list alike.
- A badge notification that arrived as "Coming soon" updates itself to "Now available" once the window opens, rather than sitting there stale.

## [8.3.9] - 2026-07-28

## 🎉 Big update rolling through
> A lot landed in this one. Badge drops now run on our own live feed, FrankerFaceZ modifier emotes finally do their thing in chat, any font you like can drive the whole interface, and mod rooms picked up presence, unread counts and a much better composer. Underneath all that is a long list of fixes, including the 7TV channel emotes that had gone missing.

---

### ✨ Features
- Badge drops arrive in seconds. StreamNook holds a live connection to our own badge feed instead of scanning on a timer, so a new Twitch badge appears in your gallery seconds after it is spotted, while the app is open, with no refresh. Anything that dropped while StreamNook was closed is caught up at launch, and each badge announces itself exactly once.
- Badge writeups are ours end to end. What you read on a badge page is the same writeup the Penrose bot posts in our Discord: written the moment a drop is spotted, then edited in place as more details surface. Those edits ride the same feed into the app, so a badge page fills itself in and corrects itself while you are looking at it, rather than depending on an outside source for the details.
- Get pinged the moment a badge drops, even with StreamNook closed. Join the [StreamNook Discord](https://discord.gg/2xvuF9TES7) and pick up the Badges role, and the Penrose bot will post every new drop as it is spotted, then edit that post as more details firm up.
- Badge pages read like a page about the badge instead of a table of fields. How to earn it, when the event runs in your local time, the exact Twitch category with its cover art, cards for the channels involved that take you straight to the stream, and the other badges in the same event. Ids and URLs moved into a Technical details drop-down.
- FrankerFaceZ modifier emotes now do what they do on FFZ. Stack ffzW, ffzX, ffzRainbow, ffzShake and the rest onto an emote and it stretches, flips, tints or animates in chat instead of sitting there as a plain overlay. There is a toggle in Chat settings if you prefer the old look, and the emote picker previews each effect and marks the ones that need an FFZ subscription.
- Use any font you like for the whole interface. Theme > Font has a new Custom option: type a name like Poppins or Bebas Neue and it downloads on the spot, or name a font already installed on your PC and it loads with no download at all. Eight popular picks are one click away.
- Mod Rooms stay with you. Rooms keep running in the background, so the Mods tab carries an unread count, red when someone pinged you, without your having to open it. Moderation settings shows which account rooms are connected as, with Connect and Disconnect.
- Writing in a mod room got a lot better. Messages send instantly and can be retried if they fail, @mentions autocomplete and highlight, images paste straight in and open full size, links are clickable, day and New dividers break up the history, and pressing Up edits your last message.
- Clicking someone in chat opens straight to their messages. That history stays live while the card is open, renders emotes properly, and stays pinned to the newest message until you scroll up. If you would rather land on the profile with their badges and stats, there is a toggle under Chat > User Cards.

### 🐛 Bug Fixes
- 7TV channel emotes came back. 7TV changed the shape of its channel response and stopped including the emote set inline, and channel emotes silently vanished. StreamNook now fetches the set properly on both Twitch and Kick, and the trending tab works again.
- Chat no longer looks dead just after joining a channel. New messages could stop appearing for the first stretch after a join.
- Drops you had already claimed could show as unclaimed when an old progress record still carried watch minutes.
- A badge queued before its earn window opened now reads as available instead of coming soon.
- The player's LIVE indicator stopped turning into a clock counting up from when you joined.
- The window comes back on the same monitor at the same size after an update restart.
- Mod rooms recover from a dropped connection instead of giving up. A brief network problem no longer asks an already connected account to reconnect, stalled connections are detected and replaced, and reconnecting no longer shows every message twice.
- Emotes typed with extra spaces between them no longer break modifier and zero-width stacking.
- Reply previews show emotes instead of raw emote codes.
- Usernames sit on the same line as the message again instead of floating slightly above it.
- Accent tinting works across the whole app again. Hundreds of surfaces meant to pick up your theme colour were rendering with no tint at all.
- Seasonal and cake-day badges are granted against the server's clock now, so they land on the right day regardless of your PC's date.

### ⚡ Performance
- Chat, the sidebar, the title bar, the player and the Multi Nook grid no longer re-render on unrelated activity. Every toast, mod-log entry and drops poll used to wake the whole app.
- Typing in the chat box no longer re-renders the visible messages.
- Hype train banners, tooltips and Multi Nook tiles stopped doing repaint work on every frame, which shows up most on busy channels and multi-stream layouts.
- 7TV's trending and global emote sets are fetched once and shared instead of once per channel join.

### 🔧 Maintenance
- The Drops page tucks fully collected games into their own collapsible section, so the grid shows what you can still earn.
- The loading widget is now the real StreamNook mark, retinted by your theme.
- Remaining native browser tooltips were replaced with the app's own.
- First-run setup installs one component instead of two.

## [8.3.8] - 2026-07-20

### ✨ Features
- Watch a VOD with its chat in sync. Historical chat now replays alongside the video and scrolls in time with the playhead, so you can follow the conversation even on streams that never kept an on-screen chat box. A header toggle switches between the synced replay and the channel's current live chat.
- See every drop, not just the ones you earn by watching. A new filter on the Drops page switches between "Watch to earn" and "All drops," so purchase and subscription drops are no longer hidden from the list.
- Channel point redemptions now read clearly in chat, showing the reward with its cost and the channel's points icon instead of a bare number.
- Community gift subs collapse into a single announcement that lists the recipients, so a big gift drop no longer floods chat with dozens of separate lines. This applies in both chat and the stream overlay.

## [8.3.7] - 2026-07-17

## 🎉 New: Subscriber milestone badges
> As our first subscribers approach three months, we are rolling out milestone badges for the people who stick around. Reach 3, 6, 9, or 12 months and you unlock a holographic Penrose badge to wear anywhere in StreamNook.

![Subscriber milestone badges](https://cdn.streamnook.app/badges/tenure-lineup.webp)

### ✨ Features
- Prediction outcomes now show emotes in their titles.
- Real-time badge drop alerts, so new Twitch badge drops surface the moment they go live.

### 🐛 Bug Fixes
- Mass gift-sub announcements collapse into a single card that lists the recipients, instead of flooding chat.
- Gift-sub recipients show in their own name color instead of the gifter's.
- Drops automation fixes so mining tracks the correct drop and channel and resumes cleanly.
- The what's-new popup renders release images and keeps descriptions in order.

## [8.3.6] - 2026-07-15

### 🐛 Bug Fixes
- Fixed YouTube live chat not showing up for some streams. When a creator was genuinely live but YouTube's live shortcut returned their channel page instead of the stream, the chat would silently never load. StreamNook now finds the live stream and connects to its chat directly, and it won't attach to an old stream's replay chat by mistake.

## [8.3.5] - 2026-07-12

### ✨ Features
- Overlay: Added a "Restore chat on reload" option. By default the overlay comes back cleared when you reopen OBS or start a stream; turn it on to bring the last messages back instead.

### 🐛 Bug Fixes
- Overlay: YouTube chat now uses YouTube's own badges and colors instead of Twitch ones. Moderators show the blue shield with a blue name, members show the channel's tier badge with a green name, verified users get the check, and the channel owner appears in a gold name tag, with badges in YouTube's order and at full resolution.
- Drops: Watch time now reports on two paths at once, so drop progress keeps crediting reliably through long sessions.
- Drops: The Drops panel's per-tier bars and "next reward" countdown update live now, instead of only after reopening the panel.
- Updates: The updater no longer gets stuck on the old version when a leftover StreamNook process is still running. It closes any stragglers and completes the swap.

### ⚡ Performance
- Cologne event: The animated backdrop stays smooth in the browser-based overlay view.

## [8.3.4] - 2026-07-12

## Overlay update

This release focuses on the stream chat overlay: new ways to highlight chatters and events, message bubbles, per-source event filters, and more control over what shows and how messages come in.

---

### ✨ Features
- **First-time chatter highlights.** Mark someone's first-ever message in your channel with a Twitch-style pink outline or StreamNook's purple highlight. Pick a custom accent color, add a subtle fill, and give the border an animated accent (Sheen, Pulse, or Chase) that plays as the message lands.
- **Outline event style.** A new look for subs, gifts, and raids: a thin ring in each platform's color, or one fixed color you choose, with an optional fill and the same border animations.
- **Message bubbles.** Each chat message can sit in its own bubble that hugs the text, with rounded, pill, or speech shapes plus adjustable corner radius, color, and opacity. Reads far better over busy gameplay than bare text.
- **Per-source event filters.** Choose which events each platform shows on its own, so you can keep Twitch raids while hiding TikTok gifts without one affecting the other.
- **More entrance animations, each with its own feel.** Drift, Rise, and Stamp join Fade, Slide, and Pop, and every entrance now moves distinctly instead of reading like a plain fade.
- **Chatter display controls.** Show or hide profile pictures (YouTube and TikTok send them), strip the leading @ from handles, and hide the "Replying to" context line. Settings that only affect certain platforms now show which ones at a glance.
- **Overlay housekeeping.** Hide messages containing chosen words or phrases, clamp long messages to a set number of lines so one copypasta can't fill the screen, and auto-remove messages after a set time so a quiet stream doesn't show stale chat.

### 🔧 Maintenance
- Every new overlay setting is searchable from Settings search and the command palette.

## [8.3.3] - 2026-07-10

### 🐛 Bug Fixes
- **The StreamNook member badge can now be hidden on your overlay.** It appears in the badge providers list alongside 7TV, FFZ, and the rest, and it follows the third-party badges toggle, so switching that off hides every extra badge at once.
- **Your overlay no longer starts from a blank screen after a reload.** It remembers the last messages it showed, so restarting OBS or refreshing the browser source brings recent chat right back.

## [8.3.2] - 2026-07-10

### ✨ Features
- **Consistent emoji across every platform, in the style you choose.** Emojis from Twitch, Kick, YouTube, and TikTok now render in one uniform style on your overlay instead of a mix that changed with your OS. Pick System, Apple, Google, Twitter, or Facebook in the overlay's Appearance settings, with live previews, and Twitter renders as crisp vector art.
- **Custom fonts for your overlay.** Type any Google Font name and it loads instantly, no download needed, with a short guide and a live preview as you type. Fonts installed on your PC work too.
- **Finer control over chat cosmetics.** New toggles to show or hide 7TV paints, StreamNook atmospheres, and third-party badges, and you can turn each badge provider (7TV, FFZ, Chatterino, and more) on or off individually.
- **Hide chat commands from your overlay.** Add a prefix like ! or # to hide every command, or a specific command like !title to hide just that one.
- **A cleaner overlay builder.** The settings are reorganized into clear tabs (Sources, Layout, Appearance, Filters, Events) with a roomier layout, so everything is easy to find.

### 🐛 Bug Fixes
- **Custom fonts now preview correctly inside the app,** matching how they already render on the live overlay.
- **Chat history goes deeper and fills in more reliably.** History cards recover sources that were silently coming up empty, keep working for people who use DNS-level ad blockers, and load deeper history in the background without the card freezing.
- **Drops auto-claim actually claims.** Watching a farmed channel now detects when a drop hits 100% and claims it, retries a failed claim instead of giving up for the session, and no longer repeats the "drop ready" notification.
- **Completed and reissued drops show correctly.** Fully earned campaigns no longer show a stray claim button or vanish from the panel, and a reissued reward is no longer mistaken for one you already own.
- **The drops channel picker only shows channels that can actually earn.** For allow-list events, it lists only channels that are live in the campaign's required category, so you don't pick someone who's live but can't credit the drop.
- **Kick reply emotes render properly** instead of showing raw markup.
- **Earned-points notifications use the channel's real name** instead of the lowercase login.
- **Shared links refresh their preview.** Re-sharing a channel link on a later day now shows the current stream in the unfurled preview instead of a stale one.

## [8.3.1] - 2026-07-10

### ✨ Features
- **Consistent emoji across every platform, in the style you choose.** Emojis from Twitch, Kick, YouTube, and TikTok now render in one uniform style on your overlay instead of a mix that changed with your OS. Pick System, Apple, Google, Twitter, or Facebook in the overlay's Appearance settings, with live previews, and Twitter renders as crisp vector art.
- **Custom fonts for your overlay.** Type any Google Font name and it loads instantly, no download needed, with a short guide and a live preview as you type. Fonts installed on your PC work too.
- **Finer control over chat cosmetics.** New toggles to show or hide 7TV paints, StreamNook atmospheres, and third-party badges, and you can turn each badge provider (7TV, FFZ, Chatterino, and more) on or off individually.
- **Hide chat commands from your overlay.** Add a prefix like ! or # to hide every command, or a specific command like !title to hide just that one.
- **A cleaner overlay builder.** The settings are reorganized into clear tabs (Sources, Layout, Appearance, Filters, Events) with a roomier layout, so everything is easy to find.

### 🐛 Bug Fixes
- **Chat history goes deeper and fills in more reliably.** History cards recover sources that were silently coming up empty, keep working for people who use DNS-level ad blockers, and load deeper history in the background without the card freezing.
- **Drops auto-claim actually claims.** Watching a farmed channel now detects when a drop hits 100% and claims it, retries a failed claim instead of giving up for the session, and no longer repeats the "drop ready" notification.
- **Completed and reissued drops show correctly.** Fully earned campaigns no longer show a stray claim button or vanish from the panel, and a reissued reward is no longer mistaken for one you already own.
- **The drops channel picker only shows channels that can actually earn.** For allow-list events, it lists only channels that are live in the campaign's required category, so you don't pick someone who's live but can't credit the drop.
- **Kick reply emotes render properly** instead of showing raw markup.
- **Earned-points notifications use the channel's real name** instead of the lowercase login.
- **Shared links refresh their preview.** Re-sharing a channel link on a later day now shows the current stream in the unfurled preview instead of a stale one.

## [8.3.0] - 2026-07-09

## 🎉 New: Design your stream's chat overlay, hosted for you
> Build a chat overlay right inside StreamNook and paste one link into OBS. It merges Twitch, Kick, YouTube, and TikTok into a single feed and renders 7TV paints, badges, and emotes the way your viewers see them. You design it in a live builder that shows exactly what goes on stream, and it keeps working with StreamNook closed.

Find it under:
```
Settings  >  Stream Overlay
```

---

### ✨ Features
- **Stream Overlay builder.** A new design studio under Settings with a live preview: choose your platforms, fonts, spacing, and background, toggle which events show per platform, convert Super Chat amounts to your currency, and block specific bot names per source. Publish once for a permanent OBS link that stays in sync as you tweak it.
- **Share a stream in one click.** New Share buttons on the player, the right-click menu, and profile cards copy a streamnook.app link, and opening a StreamNook link jumps straight to that channel in the app.
- **Channel point redemptions in chat.** Reward redemptions from across the channel now show inline in chat. New toggles let you hide redemptions, polls, or predictions to keep things clean.
- **Recent searches.** The search bar remembers what you looked up, kept separately for Following, Discover, and Categories, so you can jump back with one click.
- **Drops completion at a glance.** Fully earned games show a DONE badge, and the title bar shows a check when you have finished a game's watch-time drops.

### 🐛 Bug Fixes
- **Region-blocked high qualities play reliably.** Higher tiers your region restricts (like 1440p) that use newer video formats no longer fail to load. They play automatically, with nothing to set up.
- **Special-event drops are farmable again.** Large event campaigns like EWC that list hundreds of channels were being silently hidden or dropped. Every eligible live channel now shows and earns correctly, on any of them, regardless of category.
- **Finished and reissued drop campaigns stop disappearing** from the drops panel, and now render as complete instead of vanishing.
- **Farming recovers when a stream ends.** If a farmed stream goes offline but keeps serving stale video, StreamNook notices the stall and switches channels or tells you, instead of hanging.
- **Chat history fills in on more channels** by finding the right log source, and a slow source no longer stalls the whole history card.
- **The profile history spinner no longer hangs.** It clears itself if the lookup wedges.

### ⚡ Performance
- **Categories loads faster.** Viewer counts load in parallel and sort correctly by viewers, so the grid fills in quickly.

### 🔧 Maintenance
- Joining a channel now seeds more recent chat history, so chat looks alive right away.

## [8.2.2] - 2026-07-08

### 🐛 Bug Fixes
- **Improvements to region-based quality unlocking.** Refinements to how higher qualities are handled in regions where Twitch restricts them.

### 🤖 Also from us
Two free Discord bots for any server:
- **Cadence**, a full music bot you can run from your phone or the web, not just from chat commands. Everyone in the voice call can search, queue, and control playback from a single link, and you can connect Spotify to see your playlists and drop them straight into the queue.
- **Penrose**, an all-in-one moderation and community bot with deep, highly customizable controls: anti-nuke, moderation, automation, and per-server setups for nearly everything.

Both are completely free. Find them at [streamnook.app/bots](https://streamnook.app/bots).

## [8.2.1] - 2026-07-06

### ✨ Features
- **Watch in the qualities your region blocks.** Some streams offer higher resolutions like 1440p that Twitch restricts in certain countries. StreamNook now brings those options back automatically whenever they exist, with nothing for you to set up.
- **Full screen with F11.** A new borderless full-screen mode fills your entire display, over the taskbar, with the whole app: player, chat, and all. Toggle it from the title bar, the command palette, or the F11 key.

### 🐛 Bug Fixes
- **A cleaner first run.** New installs no longer open to a pile of past announcements, and the announcement banner now waits until setup is finished before it appears.

## [8.2.0] - 2026-07-03

## 🎉 New: 250 Years of the Fourth of July
> Open StreamNook on the Fourth of July and the sky lights up with a full fireworks show for America's 250th birthday. Log in any time from July 3 to 5 to earn the limited Semiquincentennial badge, which unlocks the new Old Glory atmosphere for your profile and chat. On the Fourth you can replay the show all day from the title bar, and those who stay to watch may find a little something extra.

---

### ✨ Features
- **Mod Rooms.** Private, end-to-end encrypted chat rooms for the moderator teams of channels you moderate, with message editing, image attachments, typing indicators, and live member presence. A one-time consent confirms your moderator status without ever touching your main login.
- **Filter the directory by tags.** Browse a category and narrow the live streams by tags. Filtering happens at the source, so results reach past just the streams already loaded, and the tags on each stream tile are clickable.
- **Nova Ember atmosphere.** A flowing red, white, and blue aurora for your profile and chat, free for everyone who earns the Semiquincentennial badge.
- **Atmospheres library.** The cosmetics gallery (now called Global Cosmetics) has a StreamNook section split into Badges and Atmospheres. The Atmospheres view shows every theme and how to get it, whether it comes with a subscription, is earned with a badge, or is a hidden challenge waiting to be discovered. Opening someone's profile now also shows which atmosphere they are wearing.
- **Hype Trains, everywhere and instant.** The same Hype Train banner now appears across single chat, MultiChat panes, and the blended feed, and level-ups celebrate the moment they happen instead of seconds late.

### 🐛 Bug Fixes
- Smoother video during a busy blended MultiChat feed. The merged feed now updates incrementally instead of rebuilding itself on every incoming message, so playback no longer stutters when chat is fast.

### 🔧 Maintenance
- Drops tooling now speaks in terms of "automation" and "collected" rather than "mining" and "farmed", a clearer name for what it actually does. Your existing settings and progress carry over automatically.

## [8.1.1] - 2026-06-25

### 🐛 Bug Fixes
- Fixed Kick account sign-in. Connecting Kick to send messages and moderate now works as expected.

## [8.1.0] - 2026-06-25

## 🎉 New: Multi-platform chat, all in one window
> MultiChat is now a full multi-platform command center. Watch Twitch, Kick, YouTube, and TikTok chat side by side or in one blended feed, reply and moderate on Twitch, Kick, and YouTube, and track every sub, raid, gift, and super chat (including TikTok gifts) in a unified activity feed. Connect each account once and work your whole audience without leaving the window.

---

### ✨ Features
- **Multi-platform chat.** Add Kick, YouTube, and TikTok channels next to Twitch. View them as separate panes or one blended, time-ordered feed where each message shows the platform it came from.
- **Connect your accounts per platform.** Sign in to Kick and YouTube to send and moderate, all managed from one Account Connections screen.
- **Cross-platform moderation.** Ban, time out, and delete messages on Kick and YouTube as well as Twitch, with a combined moderator log across every source.
- **Activity feed.** A dedicated panel for subs, resubs, gift bombs, raids, YouTube super chats and members, Kick and TikTok gifts, and Twitch Hype Trains, with per-platform filters and a Super Chat currency converter.
- **Kick emotes in the picker.** Kick channel and global emotes appear alongside Twitch, BetterTTV, FFZ, and 7TV.
- **Go Live mode that frees memory.** Going live can close the main app to reclaim around 350MB, and opening a profile, badge, clip, or VOD brings it right back.
- **Watch VODs from a popout.** With the main app closed, VOD links in a chat popout now play in place.
- **Bigger, more readable chat.** Chat text scales up to 48px with badges and emotes scaling to match, and the activity feed has its own separate text size.
- **Watch-to-earn rewards.** Event rewards are claimed automatically for whatever you are watching, with a notification when you earn one.
- **Clearer points notifications.** Channel-points notifications now name the top channels and amounts instead of just a count.
- **Redesigned MultiChat settings.** A full-window settings panel with a category rail, search, and a quick theme picker.

### 🐛 Bug Fixes
- Fixed a freeze that could show "Not Responding" for several seconds when the window was covered by another window.
- The channel About panel now scrolls smoothly instead of snapping back to the stream when you scroll up.
- Drops and farming settings take effect immediately, and saving settings no longer overwrites the Autopilot plugin's farming configuration.
- Playback pauses during ad breaks no longer interrupt points and drops earning.
- Whispering from a popout profile card works even when the main app is closed.

### ⚡ Performance
- Windows load only the code they need, so chat-only popouts start faster and lighter.
- Removed a startup image preload that held roughly 120MB; backgrounds now load as they appear.

### 🔧 Maintenance
- Reworked the chat backend so every platform shares one message pipeline, keeping normal Twitch chat unchanged.

## [8.0.11] - 2026-06-22

### 🐛 Bug Fixes
- Channel points now span every channel you follow, not just the ones you watched this session. The Drops tab summary, the points leaderboard, and your profile all show your full holdings, with a refresh button to pull the latest on demand.
- Profile pictures show again across the channel points leaderboard, even when you follow a large number of channels.
- Channel points update the instant they change instead of waiting for the next watch-time tick.
- Drops claimed now matches the total Twitch shows, counting every award instead of collapsing repeats.
- Achievement badges tied to a running total stay earned once you hit the milestone, even if the live number later dips.
- Subscription tenure and estimated spend are now based on your full lifetime months subscribed to each channel rather than your current streak.
- Atmospheres you unlock as a subscriber stay yours, available even if your subscription later lapses.
- The drops button now offers a sign-in prompt after you fully sign out.

## [8.0.10] - 2026-06-21
### ✨ Features
- **Chat on the left, plus reveal on hover**: Dock chat on the left of the player, not just right or bottom. Side-docked chat can also tuck against its edge and slide out when you move toward it, with the player shrinking to make room.
- **Cinema Mode**: Choose how the letterbox bars around the video look. Cinema Mode keeps them classic black; off matches them to your theme background so the video appears to float.
- **Profile atmospheres update live**: When someone changes the atmosphere they're wearing while you're already in chat with them, it now updates for you on their next message instead of staying stuck on what they had before.

### 🐛 Bug Fixes
- **Dropdowns stay put**: Open settings dropdowns no longer close when you scroll or resize the window. They follow their button instead.

### 🔧 Maintenance
- **Cleaner mention highlights**: The mention and reply flash now uses a consistent accent color and drops the extra glow.
- **Sharper profile customization preview**: The chat preview now matches a real chat row's size, wrapping, and readability.

## [8.0.9] - 2026-06-21
### ✨ Features
- **CS2 Major Cologne 2026**: Watch any Counter-Strike stream in StreamNook within 24 hours of the Cologne grand finals to unlock a limited Cologne look for your chat and profile. Supporters can add the gold coin, subscribers can add the gold border too, and everyone in chat sees what you have applied.
- **Song recognition**: Tap the new music button on the player or type /song in chat, and StreamNook names the track playing in the stream, with cover art, artist, and one-tap links to Spotify, Apple Music, YouTube Music, and more. No sign-in needed.
- **Reminders**: Have StreamNook auto-post a chat message on a repeating timer, after a delay, at a set time of day, once the stream hits a chosen uptime, or when a keyword appears in chat. Configure them in the new Reminders settings, or set one on the fly with a guided /remind.

### 🐛 Bug Fixes
- **Smoother playback during ads**: Fixed a live-playback glitch where the stream could replay or stutter the same moment during ad breaks, especially with the ad-bypass plugin.
- **No more freeze during login**: Fixed the app freezing or showing "Not Responding" when you alt-tab or minimize during Twitch login. Login, drops, and subscribe now open in a clean in-app window with a green padlock address bar.
- **Load more works on clips and videos**: Restored "Load more" on channel clip and video lists, and it no longer shows duplicate entries.
- **Accurate drop ownership**: Drop rewards are no longer wrongly marked as already earned when another reward in the same campaign shares its name.

### 🔧 Maintenance
- **Quieter, more findable**: Added a background watchdog that catches rare UI freezes so they can be diagnosed, and made the new settings searchable from settings search and the command palette.

## [8.0.8] - 2026-06-18
### 🔧 Maintenance
- **One-time sign-out after this update**: We did some behind-the-scenes maintenance on the login flow, cookie storage, and how your accounts are streamlined across all the activities you do. All you have to do is log back in, then sign in to Drops again.

## [8.0.7] - 2026-06-17
### ✨ Features
- **Your 7TV personal emotes work everywhere**: The personal emotes you set on 7TV now show up in every channel you watch, not just channels that happen to have added them. Like the 7TV browser extension, your personal set follows you from stream to stream.
- **Manage your personal emote set in the app**: The built-in 7TV emote manager now shows your personal emote set next to your channel sets, so you can add, remove, and rename your personal emotes without leaving StreamNook.
- **Three ways to open your emote manager**: Edit your emote sets from whichever spot is closest. A new gear in the emote picker opens it in one click, the command palette has "Open 7TV Emotes" (Ctrl+K), and the moderator menu opens it for any channel you moderate.

## [8.0.6] - 2026-06-16
### 🐛 Bug Fixes
- **Your chat color shows correctly right away**: Your own messages no longer flash a default color before snapping to your real one, and they no longer occasionally get stuck on the wrong color.
- **Low Latency is steadier under load**: Fixed the playback reload timeouts and refused connections that could cause stalls or buffering on the low-latency path.
- **Your live edge gap holds where you set it**: Low Latency now keeps your chosen delay more consistently instead of drifting too close to live, and moving the gap slider takes effect immediately while you watch instead of only after reloading the stream.
- **No more duplicate cards while browsing**: Loading more streams, clips, videos, or games in a category or the games grid no longer shows the same item twice.

## [8.0.5] - 2026-06-15
### 🐛 Bug Fixes
- **Experimental Low Latency plays cleanly on every channel**: On some streams, turning on experimental Low Latency could badly corrupt the picture and freeze playback. Video frames were being clipped short as the stream was repackaged for the low-latency path, so the decoder ran out of data partway down the frame. Frames now stay whole, and Low Latency holds up everywhere.
- **Following channels is more reliable**: Following and unfollowing now goes through your signed-in session directly, with no hidden background window needed to make it work.
- **Chat colors and badges fill in reliably**: When someone's name color or badges briefly failed to load, they could stay blank for the rest of the session. They now fill in on that person's next message instead.

### 🔧 Maintenance
- **One more sign-in after this update**: To finish moving following onto the more reliable path, this update signs you out once so everything reconnects cleanly. Just sign back in when prompted.

## [8.0.4] - 2026-06-15
### ✨ Features
- **First-run setup, expanded**: The setup wizard now walks you through picking a theme, choosing your sidebar style, and setting how you get notified, each with a live preview, so the app feels like yours before you start watching.
- **Remembers where you left it**: StreamNook now reopens at the same size and position on screen, so you no longer have to resize or move the window every launch.
- **Whisper history that stays in sync**: A new refresh button pulls your latest whisper messages straight from Twitch, so replies you sent from the website or your phone show up in StreamNook without losing anything you already had.
- **A live edge gap you control**: A new slider lets you dial exactly how close to live you want to ride, from a near-live feel to rock-solid playback, replacing the old all-or-nothing toggle. An opt-in Low Latency engine is there too for the tightest experience on channels that support it.
- **Drops and Channel Points sign-in, made obvious**: When drops and points need their own Twitch sign-in, the gift button in the title bar now clearly shows a Login prompt, and signing in reuses your existing Twitch session so you just approve instead of logging in all over again.
- **Natural live notifications**: Go-live alerts now read naturally for non-gaming streams, saying things like "Just chatting" or "Making music" instead of "Playing Just Chatting".
- **Plugin settings, all in one place**: Plugins can now add their own panels to the Integrations settings and link you straight to them, so connected apps and services live together in a consistent spot.

### 🐛 Bug Fixes
- **Account switching respects the account you picked**: Switching between linked accounts now correctly uses the one you selected, for your session, ad-free status (Turbo or a channel sub), badges, and following channels, so an alt no longer inherits another account's perks or leaks the previous session. Signing out fully resets the old account and stops it from earning drops or points.
- **Whispers stay private to each account**: Your whisper history is now tied to the account you are signed in with, so switching accounts or signing out no longer surfaces another account's messages.
- **Your look loads instantly**: Your paint, badges, atmosphere, and profile card now appear the moment chat or your profile opens after a restart, instead of popping in a beat later, and a brief hiccup talking to the cosmetics service no longer strips them away.
- **No more signed-out flash on launch**: Opening the app now shows a clean loading screen while it confirms your saved login, instead of briefly flashing a signed-out home.
- **Chat keeps flowing when the window is hidden**: Chat now keeps updating reliably even when the window is minimized, occluded, or moving in and out of fullscreen, instead of freezing until you refresh.
- **Signing in is more reliable**: The Twitch login code you approve is now always the one the app is waiting on, so logins no longer get stuck, and everything that needs your account works right after you sign in without a restart.
- **Drop progress you can trust**: The drops progress badge no longer sticks at 0% on streams where you already own the reward, and drop watch time keeps crediting reliably even after the connection sits idle.
- **An honest behind-live readout**: The "behind live" number in the stream stats overlay now lines up with what Twitch itself reports, instead of overstating how far behind you are.
- **Steadier standard playback**: Streams on the standard path now play on a single stable connection, removing a class of stalls and drift some channels saw over a long session.
- **More reliable auto-switch**: When a stream you are watching goes offline, StreamNook now finds another stream in the same category more accurately.

### 🔧 Maintenance
- **Default-on conveniences**: The bonus chest is now auto-claimed on the stream you are watching, and the video keeps its aspect ratio out of the box, so these work without you turning them on.
- **Interface polish**: A roomier title bar, a cleaner collapsed sidebar, tidier settings groupings, and a consistent live-red that stays the same across every theme.
- **Consistent badge order**: Your badges now appear in the same order everywhere, with your StreamNook badge leading, so chat and profile cards always match.

## [8.0.3] - 2026-06-14
### 🐛 Bug Fixes
- Fixed drops progress showing the wrong reward: watching a channel on a drop's allow-list while it streamed a different game would display and count that drop's progress even though you weren't earning it.
- Fixed the drops channel picker offering channels you can't earn on. It was listing offline channels, and for channel-restricted drops it listed participating channels that weren't currently streaming the campaign's game.

## [8.0.2] - 2026-06-14
### 🐛 Bug Fixes
- Drops campaigns that only run on specific channels now offer a channel picker, so you can choose a participating live channel straight from the campaign and start watching.
- Drop rewards now show a clear Badge, Emote, or Item label, so you can tell at a glance what each reward actually is.
- The stream stats panel can now be dragged anywhere over the player, and it remembers where you left it.
- Event and special-event drops now track progress correctly when you watch a participating channel, even when that channel's category doesn't match the campaign.
- Earned badges now count toward a campaign's "earned" tally consistently, instead of sometimes reading zero while the badge sits claimed in your collection.
- The post-update changelog popup now opens on the version you just installed, instead of occasionally sticking on an older release from a stale cache.
- The frosted glass on pop-up dialogs now follows your Glassiness setting, going fully solid at the lowest setting like the rest of the app.

## [8.0.1] - 2026-06-13
### 🔧 Maintenance
- **Update checks are downgrade-safe**: The built-in updater now compares versions properly and only offers a newer build, so a momentary hiccup in the update service can never prompt you to "update" to an older version.

## [8.0.0] - 2026-06-14
## 🎉 New: A built-in Marketplace, and StreamNook is now modular
> The features you know no longer all live in one binary. StreamNook 8.0 introduces a real plugin system with a Marketplace built right in: familiar add-ons that used to be part of the core app now ship as opt-in plugins you install in one click, so the app stays lean and you run only what you want. Best of all it is open. The community can build and submit their own plugins, so what is available keeps growing beyond the ones we make.

---

### ✨ What's new
- **The Marketplace**: Open the Store icon in the title bar to browse a curated catalog of plugins, see exactly what each one does, and install or update in one click. It is built in, so there is nothing to set up.
- **Community plugins**: The plugin system is open. Anyone can build a plugin and submit it for the Marketplace, so the catalog grows beyond what we ship, and each plugin's settings appear right where they belong in the app.
- **Update notifications**: When an installed plugin has a newer version, the Store icon shows a count and each plugin gets an Update button in the Installed tab.

### 🧩 Familiar features, now opt-in plugins
> These were already part of StreamNook. They work the same, they are just plugins now, so you install only the ones you use. Updating from an older version? Open the Store icon and add the ones you used to bring them right back.
- **Autopilot**: Background Drops and channel-points farming without keeping a stream open, plus a Mine button on every Drops card to farm a reward without watching it.
- **Ad-Free Playback**: Ad-free live viewing resolved through community playlist proxies, with a built-in proxy health checker.
- **Lists**: Your go-to text (usernames, commands, links) in a floating, pop-out panel that docks into mod logs, with a command-palette entry and a hotkey.

### ⚡ Low latency, tuned to match Twitch
> The streaming backend got a deep refactor. Playback now holds latency on par with Twitch's own player, and on channels that have low latency enabled you stay just a couple of seconds behind the broadcaster, in the solo player and in every Multi-Nook tile. New low-latency diagnostics and codec-aware resolution selection keep playback on the best stream and close to live, without the drift that creeps in over a long session.

### 🎨 Interface
> A wave of interface work across the app: the new Marketplace's clean app-store layout, clearer plugin badges (Official, Approved, and Installed), and polish throughout the UI.

### 🐛 Bug Fixes
- **Chat connections stay healthy**: Reworked how chat rooms are claimed so a closed tab or a reconnect can no longer pin a room and leave it stuck.
- **Drops progress is accurate and steady**: The title-bar and overlay progress now show the reward you are actually on, including the right tier in a multi-reward campaign, and hold steady instead of flickering.

## [7.8.7] - 2026-06-12
### 🔧 Maintenance
- **StreamNook is moving to its own GitHub organization**: The project is getting a proper home at github.com/StreamNook instead of living under a personal account. This update teaches the built-in updater to follow the project to its new address, so install it to keep receiving future releases automatically.

## [7.8.6] - 2026-06-10
## 🎉 New: Low latency playback, rebuilt to ride seconds behind live
> StreamNook now picks up video in small pieces as the broadcaster encodes them, instead of waiting for whole segments to finish. On channels with low latency enabled, that puts you roughly two to three seconds behind the broadcaster, in the solo player and in every Multi-Nook tile. Channels without low latency keep playing exactly as before.

---

### ✨ Features
- **Lists, a home for your go-to text**: Create as many lists as you like for usernames, commands, links, or anything else you keep reaching for. Click an entry to copy it, hover to send it into chat, and paste a whole batch at once (one per line, comma separated, or straight from a spreadsheet). The panel floats over the app without blocking chat, pops out into its own pinnable window, and can dock right into the mod logs view. Open it from the title bar, the command palette, or Ctrl+Shift+L.
- **A latency readout you can trust**: The stream stats overlay now shows your true delay to the broadcaster (the same number Twitch reports) plus your configured buffer target, so you can tell at a glance whether you are actually behind.

### 🐛 Bug Fixes
- **Low latency no longer freezes on refresh**: Upcoming segments promoted into the playlist could change identity between updates, which could crash playback mid-stream. Their numbering and addresses now stay stable from one update to the next.
- **Go Live lands where it should**: The button now jumps only to video that is actually ready to play, and if the stream has genuinely stalled it restarts playback instead of seeking somewhere unplayable.
- **Quieter quality switches**: Changing quality or restarting a stream no longer lets the outgoing player surface errors while the new one spins up.

### ⚡ Performance
- **Stays close to live on every channel**: On channels without low latency, playback now speeds up almost imperceptibly whenever you drift past your buffer target, so delay no longer creeps up over a long session.
- **Faster grid cold starts**: Multi-Nook tiles now start up in parallel when you load a full preset, instead of waiting on each other.

## [7.8.5] - 2026-06-07
### ✨ Features
- **Spotlight a stream in Multi-Nook**: Double-click any tile (or use the new spotlight button) to fill the whole grid with that one stream, then double-click again or press Esc to drop back to the grid. Spotlighting also moves audio and chat to that stream, so it behaves just like the solo player, and the other streams keep running in the background for an instant switch back.

### 🐛 Bug Fixes
- **Channel emotes no longer drop to plain text**: The emote prefetch added in 7.8.3 could leave a channel's emotes showing as plain words when 7TV was slow or down. Each channel's emote list is now saved locally and loaded the instant you open chat, a failed or partial 7TV fetch can no longer overwrite a good saved list, and your emotes survive restarts.
- **No more freezes in low-latency mode**: The player was riding too close to the live edge, so a normal gap in Twitch's segment delivery could drain the buffer and stall playback. It now keeps a bit more cushion, and Multi-Nook tiles use a steadier buffer, so streams play smoothly without stalling.

## [7.8.4] - 2026-06-06
## 🎉 New: Save and reload your multi-stream layouts with Grid Presets
> Build the perfect multi-stream grid once, then save it as a named preset and bring it back any time with a single click. Swap between your favorite lineups, drop a preset on top of what you are already watching, and give each one a custom icon from game art or a channel avatar. Searching for channels to add now works right inside the preset editor too.

---

### ✨ Features
- **Friendlier offline tiles**: When a channel is offline or cannot load, its tile now shows a clear status with one-tap Retry and Hide buttons instead of spinning forever.
- **Per-tile quality that sticks**: The stream quality you choose for a tile in multi-view is now remembered across restarts and saved with your backups.
- **Instant unlock celebrations**: The moment a Subscriber or Supporter badge purchase comes through, a gentle notification slides in to confirm it, with no restart needed.
- **Clickable member badges**: Tap someone's StreamNook number badge on their profile card to open their public profile, even from a popped-out window.
- **Favorites get their own section**: Favorited channels now sit in a dedicated section above Followed, and you can hide the Recommended section entirely from Interface settings.
- **One-tap reload**: The player refresh button, and a new /reload chat command, now reload the video and reconnect chat together in a single action.
- **Twitch-first emote search**: Typing a colon, like :PogU, now surfaces Twitch and sub emotes ahead of third-party sets.

### 🐛 Bug Fixes
- **Smoother multi-stream startup**: Tiles now wait for a small buffer cushion before playing, so cold-starting several streams at once no longer stalls right after they load.
- **One offline channel no longer drags down the grid**: A tile that cannot go live stops retrying in the background, keeping the rest of your streams responsive.
- **Login and popup windows restored**: 7TV login, automation, and chat-identity windows open reliably again.
- **Up-to-date 7TV sign-in**: Logging in to 7TV now uses the current flow that actually issues a token.
- **Cosmetics update right away**: Changing your badge or paint now repaints your chat row and profile card immediately instead of waiting up to a minute.
- **Missed unlocks catch up on their own**: Badges or perks granted while you were away now appear when you return to the app or reconnect, without a restart.
- **Fullscreen tiles come to the front**: Fullscreening a tile in the grid now lifts it cleanly above the other tiles.
- **Changelog dates show the right day**: Date-only entries no longer appear a day early in time zones behind UTC.
- **No more doubled badge**: Members without an equipped cosmetic now show a single default badge instead of two.

### 🔧 Maintenance
- **Refined title-bar pill**: The notification pill sits more cleanly in the title bar with a subtler unread indicator.
- **Consistent sidebar glass**: The floating sidebar now follows your Glassiness setting like every other glass surface.

## [7.8.3] - 2026-06-05
### ✨ Features
- **Audio Boost**: Make stream audio louder and more even. A built-in compressor evens out quiet talking and loud moments and lifts overall loudness past the source without the harsh distortion of cranking volume past max. Turn it on from the player control bar or Player settings, fine-tune it with on-screen sliders right over the video, and reset to defaults anytime. Off until you enable it.
- **Emote Prefetch**: Bulk-download the emotes from every channel you follow in one background pass, ideally while you are away. Once it finishes, chat and the emote menu fill in instantly with nothing left to fetch, and it takes load off 7TV. Start, watch progress, or stop it from Cache settings.
- **Place Notifications Anywhere**: Pick which corner or edge your popup notifications appear in with a simple visual picker, and use a spacing slider to lift them off the screen edge so they do not sit on top of chat.
- **Fully Flat Glass**: The Glassiness slider now goes all the way to a completely solid, blur-free interface for anyone who does not want any frosted-glass effect, not just slightly more opaque panels.
- **Jump Straight to a Setting**: Settings can now open directly to a specific section and scroll right to it, instead of dropping you on a tab to hunt for the option.

### 🐛 Bug Fixes
- **Chat no longer goes blank when you join**: A slow or down 7TV could leave chat empty for several seconds on join. Recent messages now backfill immediately while emotes load in the background, and a circuit breaker stops a 7TV outage from stalling the whole connection.
- **Emotes recover on their own during a 7TV outage**: When 7TV returns an empty set, the app no longer caches that gap for the rest of your session. It quietly re-fetches once 7TV is healthy again, so you do not have to refresh manually.
- **Animated FrankerFaceZ emotes actually animate**: Animated FFZ emotes were being shown as still images. They now play.
- **Emotes and badges render reliably**: Cached images are now saved in their true format and keyed per provider, so they no longer collide or show up broken. Existing miscached emotes are repaired automatically on launch.
- **Consistent drops progress everywhere**: The title-bar drops badge, the game cards, and the detailed drops panel now read from the same live source, so progress updates promptly and matches across all three instead of drifting apart.
- **Readable announcement banners**: In-app announcement banners are now solid and legible at any Glassiness setting, including fully transparent.
- **Steadier low-latency playback**: Hardened a low-latency code path that could freeze playback on certain streams.

### ⚡ Performance
- **Smoother, faster chat**: Busy channels scroll and animate far more smoothly, the emote picker opens faster and uses less memory over long sessions, and joining a channel does less work up front.

### 🔧 Maintenance
- The Cache settings panel now reports what is actually stored, clears everything in one click, and adds a button to open the cache folder.

## [7.8.2] - 2026-06-05
### ✨ Features
- **True Low-Latency Streaming**: Experience minimal delay with an actively managed HLS pipeline, featuring dynamic target duration adjustments and Twitch PREFETCH segment promotion, allowing your player to ride closer to the live edge than ever before.
- **Third-Party Chat Badges for Everyone**: Enjoy a richer chat experience as StreamNook now displays BTTV, FFZ, Chatterino, and other third-party badges for *all* chatters, not just StreamNook members. Badge resolution is now significantly more efficient.
- **Reimagined Profile & Live Preview**: Your public StreamNook profile has a stunning new design. Track how many times your profile is viewed, customize which sections are visible to others, and use the new 'Live Preview' in settings to see your edits in real-time.
- **Real-Time Stream Stats**: A new overlay provides vital diagnostics like latency to live, resolution, FPS, bitrate, and buffer status. Easily jump to the absolute live edge with the 'Go Live' button.
- **Global Motion Control**: Tailor StreamNook's animations to your preference with new settings: 'Full' for all fluid transitions, 'Reduced' for fades only (easier on the eyes and lighter on slower PCs), or 'Off' for an instant, snappy interface.
- **Full-Page Settings Window**: Configure StreamNook with more room! The settings dialog can now expand to a full-page layout, providing ample space for extensive customization.
- **Smoother UI Transitions**: Enjoy a more polished experience with new, fluid animations for opening and closing overlays, modals, and dropdown menus.

### 🐛 Bug Fixes
- **Accurate Drops Mining Completion**: Fixed issues where drops campaigns would sometimes continue mining indefinitely or display incorrect progress after completion. We now precisely identify when all drops are claimed and properly shut down background tasks.
- **Reliable Stream Information**: Resolved an issue where stream titles and game categories could appear blank after raid redirects, ensuring your UI and Discord Rich Presence always show the correct details.
- **YouTube Link Previews**: Corrected an issue preventing link previews from loading for some YouTube channel pages.
- **Chat Moderation Overlay**: The moderation drag layer no longer overlaps messages, ensuring clear visibility.

### 🔧 Maintenance
- Refined and updated the visual style of various settings and profile display components for a more cohesive 'glass-tile' aesthetic.

## [7.8.1] - 2026-06-04
## 🧰 New: Manage Your 7TV Emotes Inside StreamNook
> Connect your 7TV account and run your emotes entirely from the app. Add, rename, remove, and organize emotes across every channel you can edit, manage your emote sets, and add or remove editors, all without opening a browser. This release also reworks chat moderation with a drag-to-moderate gesture and real channel-wide message pinning, and lets you back up your entire setup to a single file and restore it on any machine.

---

### ✨ Features
- **7TV Emote Manager**: A full 7TV workspace built into StreamNook, covering your own channel and every channel you're an editor of.
  - Browse and search the entire 7TV directory with sort (Trending, Top, Newest) and filters (Animated, Zero-width, Exact match), or add an emote by pasting its 7tv.app link.
  - Add emotes under a custom name and as a zero-width overlay, then rename, re-toggle, or remove them later.
  - Open an emote's detail card for large previews, tags, flags, and how many channels use it.
  - Create, rename, resize, and switch your active emote set, with a live capacity meter.
  - Manage editors: invite by username, grant or revoke per-area permissions, and accept or decline editor invites from other channels.
  - Quick-add: left-click any 7TV emote in chat to drop it into a set you edit. Open the manager from the command palette or the moderator menu.
- **Reworked Chat Moderation**:
  - **Drag to moderate**: grab any message and drag it onto a floating dock of action buckets (Delete, Timeout, Ban, plus Unban and Untimeout for already-moderated users). Buckets snap magnetically, so no pixel-perfect aim is needed.
  - **Dial timeouts by feel**: drag further from the Timeout bucket to set a longer duration, from 5 seconds up to Twitch's 14-day max, with a live duration label.
  - **Undo**: ban and timeout confirmations include an Undo button.
  - **Pin messages**: mods can pin any message in one click. Pins use Twitch's real pin system, so they appear for every viewer on every client, not just locally. A collapsed pin bar keeps it one line tall until you expand it.
- **Settings Backup & Restore**: Export every preference (themes, chat design, keybindings, highlights, custom commands, moderation, integrations, and more) to a single file, then import it on any machine. Your Twitch login is never included, so backups are safe to move around.
- **Search Your Settings**: A search box in Settings matches across every tab and jumps you straight to the matching section.
- **Styled Chat Usernames**: New options for how names render in chat, including a separator glyph between name and message and several name styles (accent bar, frosted chip, brackets, and more), with a live preview in settings.
- **Expanded Link Previews**: Rich cards now cover Discord invites (server icon, live online and member counts, Join button), Steam store pages (art, description, and current price), Spotify tracks and albums, Instagram profiles, and Tenor GIFs. A new shield button lets you permanently trust a domain so its links always expand.
- **Customizable Player Buttons**: Choose which action buttons appear over the video (Follow, Subscribe, Create Clip, Clips & VODs, Add to MultiNook, Refresh, Close).
- **Channel About, on Scroll**: Scroll down over the player to reveal a Twitch-style About panel with the streamer's panels, social links, and quick actions. The stream keeps playing the whole time, and social links render as rich preview cards.
- **Color-Customizable OLED Theme**: The OLED theme now takes any accent color, with eight one-click presets. Existing OLED users keep their current look.
- **Image-Based Atmospheres**: Premium Atmospheres can now be animated image backdrops, with a portrait mode for the profile panel and a readability frost behind chat. Atmospheres can also be earned through accolades, not just subscriptions, and new ones can arrive without an app update.
- **Animated 7TV Profile Pictures**: Animated 7TV avatars now display on user cards.
- **7TV Paint Usage Stats**: Sort paints by Most Used or Least Used, and see how many people wear a given paint.
- **Redesigned Community & Integrations Tabs**: The Support and Integrations settings are now Discord-style cards with live server stats and a one-click join.
- **Drops: How to Unlock**: Locked campaign rewards now show a tooltip explaining what they require (sub, gift sub, Prime, bits, or follow).

### 🐛 Bug Fixes
- **Streams that wouldn't load now play**: Twitch rolled out a new stream playlist format that the old parser read as having zero qualities, so some channels simply wouldn't start. The parser now handles both the old and new formats.
- **Settings now stick between restarts**: A range of preferences (chat design, emote sizing, link-preview options, highlight phrases, custom commands, moderation preferences, custom themes, and the OLED accent color) were silently dropped on save and reset every launch. They now persist, including when loaded from older settings files.
- **Clips that wrongly said "not found" now open**: Clip lookups no longer break when Twitch rotates an internal query ID, and real errors are surfaced instead of masked.
- **Steadier ad-free playback**: When an ad-block proxy answers with an error page that still reports success, the app now treats it as a miss, races to another proxy, and falls back to the standard stream rather than failing outright.
- **Drops progress tracks the right drop**: Mining now follows the unclaimed drop closest to completion instead of pinning to an already-finished one, and claiming a drop no longer snaps the title-bar progress backward.
- **Mine any campaign you pick**: You can now preview and mine a manually chosen campaign even if it isn't in your priority list.
- **Badge sorting**: Badge entries missing usage stats are re-fetched so Most and Least Used sort correctly, and Available Now / Coming Soon now filter the list instead of just reordering it.
- **Reliable profiles for localized names**: Profile banners and extended profile data now resolve for users whose display name differs from their login, and profile actions (whisper, gift sub, mod commands) always target the correct account.
- **No more blank page after search**: Leaving a stream you opened from search returns you to where you were instead of an empty results page.
- **Overlays no longer cover the Dynamic Island or the drops hover preview.**

### ⚡ Performance
- **Instant profile cards**: Profiles open immediately from a cached snapshot and refresh in the background, so banners and backdrops no longer appear a beat late.
- **Smoother sidebar**: The compact sidebar's glass blur now settles after it finishes expanding, removing the choppy redraw during the animation.

### 🔧 Maintenance
- Cleaned decorative emoji out of internal logs and the proxy-health panel for clearer output.
- Renamed several built-in themes for clarity. Your current selection is preserved.

## [7.8.0] - 2026-06-03
## 🎬 New: Twitch Clip Creation & Advanced Streamer Profiles
> We've completely overhauled how you interact with Twitch clips and streamer profiles! Now you can create clips directly from any live stream or VOD, trim them with precision, and share them instantly. Plus, every streamer profile now includes dedicated tabs for browsing their clips and past broadcasts, complete with rich filters and reaction counts.

---

### ✨ Features
- **Twitch Clip Creation**: Create clips from live streams, or trim and title clips from VODs with an intuitive in-app editor. Share your creations to chat or copy the link to share anywhere.
- **Clip Playback & Management**: Play Twitch clips directly within StreamNook via a new modal player. Copy links, open in browser, or delete clips you own. New clips now show total reactions (likes, etc.) from other viewers.
- **Advanced Streamer Profiles**: The streamer profile panel is now a full-featured hub with dedicated tabs for 'About', 'Clips', and 'Videos'. Browse a streamer's entire catalog of clips and past broadcasts with filters for time, popularity, and video type.
- **New UI Customization**: Personalize StreamNook with new global interface font options and a slider to fine-tune the glass morphism transparency to your liking.
- **Enhanced Chat System**:
  - **Moderator Viewers List**: Moderators can now view the full list of chatters, grouped by role (Broadcaster, Mods, VIPs, Viewers).
  - **Message Recall**: Type a message, press `ArrowUp` to recall previously sent messages (Chatterino-style!), and `ArrowDown` to navigate forward or restore your current draft.
- **Richer Link Previews**: Twitch clips, VODs, and YouTube channel links now expand into beautiful, info-rich preview cards directly in chat.
- **Discord Rich Presence Overhaul**: Your Discord status is now more dynamic, showing proper category art (even for 'Just Chatting' or 'Music'), personalizing idle messages with your username, and resetting the 'watching for' timer more intelligently.
- **Easier Twitch Login**: A persistent 'Login with Twitch' button now appears on the Discover tab when you're logged out, making it simpler to sign in.
- **Quick Sign Out**: Access a convenient 'Sign out' button directly from the Profile tab in Settings, with a quick confirmation step.

### 🐛 Bug Fixes
- Fixed an issue where watch time and stream counts for analytics were inaccurate due to how stream URLs changed during ads or quality switches.
- Resolved a problem where re-authenticating after Twitch scope changes could leave your account in a broken state. The app now performs a full, clean re-authentication when necessary.
- Fixed an issue where clicking on links in chat could open multiple browser tabs by removing redundant `target="_blank"` attributes.
- Corrected an issue where stream lists would sometimes incorrectly filter historical clips, especially for 'last 30 days'.
- Addressed a rare startup issue where the universal cache flush task could fail before the app's core services were ready.

### ⚡ Performance
- **Smoother Chat & Sidebar**: Significant performance improvements to the chat widget and sidebar by optimizing component rendering, leading to faster updates and reduced UI lag.
- **Faster Link Previews**: Generic link previews now load much quicker by intelligently fetching only the essential parts of a webpage.
- **Optimized Emote Display**: Improved loading and caching for 7TV emotes ensures they display faster and more reliably, even with multiple resolution tiers.
- **Efficient Discord Integration**: The app now intelligently caches Discord's detectable game list, reducing network requests and speeding up rich presence updates.

## [7.7.2] - 2026-06-03
## 🎉 New: Inline Link Previews in Chat
> Sharing a link in chat just got a whole lot more engaging! StreamNook now automatically generates rich, interactive previews for YouTube videos, X/Twitter posts and profiles, Giphy GIFs, and most other webpages. See titles, descriptions, and thumbnails directly in your chat, making conversations smoother and more informative.

---

### ✨ Features
- **Inline Link Previews**: Supported links now display rich cards in chat with titles, descriptions, and thumbnails for a variety of services including YouTube, X (Twitter), Giphy, and general webpages.

### 🔧 Maintenance
- Various performance improvements, notably in high-speed-chat streams.

## [7.7.1] - 2026-06-02
### ✨ Features
- **Permanent Perks for Supporters & Subscribers**: Your commitment now grants permanent access to special profile backgrounds and StreamNook Atmospheres! Once unlocked (by being a Supporter or Subscriber), these cosmetics are yours forever, even if your active support ends. The 7TV Paint background is now a Supporter perk, and Atmospheres are a Subscriber perk.

### 🐛 Bug Fixes
- **Improved Fullscreen Experience**: Fullscreen mode now works seamlessly across both single-stream and MultiNook views, ensuring the entire display is covered (including the taskbar) and your layout is correctly restored upon exit.

### 🔧 Maintenance
- Updated internal Supabase tools.

## [7.7.0] - 2026-06-02
## 🚀 Native Streaming is Here
> StreamNook now directly plays Twitch streams without needing Streamlink or the TTVLOL plugin. Enjoy faster startup times, a smaller app footprint, and more reliable ad-free viewing, all handled seamlessly within the app. This is a foundational rewrite of how StreamNook connects to Twitch.

---

### ✨ Features
- **Native Stream Playback**: Faster startup, reduced memory usage, and a completely self-contained app. No more external installations or troubleshooting for Streamlink.
- **Proactive Ad-Block Pivoting**: If an ad-block proxy region starts leaking ads, StreamNook will automatically switch to a different, healthy region and reload the stream, ensuring continuous ad-free viewing.
- **Intelligent Drops Mining**: The automated drops miner now intelligently tracks your closest-to-completion drop in the current game, ensuring you always see the most relevant progress.
- **Robust Mining Recovery**: The drops miner can now detect if a streamer goes offline, switches games, or if your progress stalls. It will automatically switch to another eligible channel or temporarily blacklist problematic streamers, preventing wasted watch time.
- **View Your Subscriptions**: New section to see your active and past Twitch channel subscriptions.

### 🔧 Maintenance
- **Smaller App, Faster Updates**: The app bundle size has been significantly reduced, making downloads faster and updates more reliable. Existing users will also see disk space reclaimed from old bundled components.

## [7.6.1] - 2026-05-31
## ✨ Revamped: Account Switching & Identity Management
> Now you can effortlessly switch between your Twitch accounts directly within StreamNook! Manage which account you watch and stream as, and easily switch the identity you send chat messages from. Each account now has its own isolated web session, ensuring your Twitch logins and subscriptions always happen from the correct account.

---

### ✨ Features
- **Seamless Account Switching**: Promote any linked account to your 'main' account (the one you watch and stream as) directly from Settings. Your chat identity, follows, and other Twitch data will automatically update.- **Seamless Account Switching**: Promote any linked account to your 'main' account (the one you watch and stream as) directly from Settings. Your chat identity, follows, and other Twitch data will automatically update.

### 🐛 Bug Fixes
- **Reliable Chat Pause**: Chat now pauses immediately on user scroll intent (wheel or touch), preventing frustrating fights with auto-scroll and ensuring a more responsive experience.

### 🔧 Maintenance
- Minor backend and configuration updates for improved stability.- Minor backend and configuration updates for improved stability.

### ⚡ Performance
- **Smoother Chat & Emote Picker**: Significant optimizations to chat message rendering and the emote picker, reducing UI lag and improving responsiveness, especially in busy channels or with many emotes.- **Smoother Chat & Emote Picker**: Significant optimizations to chat message rendering and the emote picker, reducing UI lag and improving responsiveness, especially in busy channels or with many emotes.
## [7.6.0] - 2026-05-29
### ✨ Features
- **StreamNook Identity**: Curate your favorite badges (Twitch, 7TV, BTTV, FFZ, Chatterino, and more) to display in chat, configurable in the new Accounts settings.
- **Multi-Account Support**: Seamlessly link multiple Twitch accounts. Send messages in chat from any of your linked accounts with a new 'Send As' picker.
- **BetterTTV Badges**: Full integration for BetterTTV badges, including contributor badges and the unique BTTV Pro loyalty badges (now discoverable in a dedicated tab in the Badges gallery).
- **Persistent Mod Logs**: For moderators, a new Moderation Logs pane displays a persistent, real-time history of mod actions for the current channel. Configurable in new Moderation settings.
- **Real-time Moderation Events**: Live moderation events (bans, timeouts, deletes) are now streamed from Twitch via EventSub to enrich your moderation logs, even for channels you're not actively streaming.
- **Hype Train Confetti**: Celebrate every Hype Train level-up with a dazzling confetti burst animation.
- **Enhanced MultiChat**: Pop out all MultiNook chats into a single MultiChat window, which now tracks all opened popout channels.
- **Rich Chat Feedback**: Message sending now provides authoritative Twitch message IDs and detailed reasons if a message is dropped (e.g., by AutoMod).
- **Automated Installer Deployment**: Faster, more reliable downloads for new releases via Cloudflare R2 CDN.

### 🐛 Bug Fixes
- **MultiNook Ad-Blocking**: Fixed an issue where MultiNook streams could still display ads even when the ad-block proxy was enabled.
- **IRC Reconnects**: Improved IRC connection stability; all joined chat channels now correctly re-establish after an IRC disconnect.
- **7TV Cosmetics Display**: Resolved an issue where your own 7TV paint and badge might not consistently appear after connecting or re-authenticating.
- **EventSub Error Reporting**: Improved error reporting for EventSub subscription failures, providing better diagnostic information.
- **Chat Resizer**: Enhanced the responsiveness and usability of the chat and mod log resizer handles.
- **Hype Train UI**: Refined the visual styling and animation of the Hype Train progress bar and level-up celebration.
- **Moderation Actions**: Updated Twitch API endpoints for deleting messages and clearing chat, providing more robust actions and detailed error messages.

### 🔧 Maintenance
- **Local Crash Logs**: Replaced Discord webhook error reporting with a privacy-focused local crash log. Errors are now saved directly on your machine for optional review, instead of being sent externally.
- **Settings Navigation**: Added an ability to deep-link directly to specific settings tabs for improved navigation experience.

## [7.5.6] - 2026-05-27
### 🐛 Bug Fixes
- Improved the application update process on Windows, preventing a brief command prompt window from appearing and ensuring updates complete reliably in the background.

## [7.5.5] - 2026-05-27
## ✨ New: Live 7TV Emote & Cosmetic Updates
> Your chat just got smarter! Emote sets now update instantly when mods add, remove, or rename 7TV emotes. Plus, 7TV paints and badges for other chatters appear live, not on delay. It's a faster, more dynamic chat experience, keeping you connected to the pulse of your favorite communities.

---

### ✨ Features
- **7TV Live Events**: Get immediate updates for emote set changes and see chatters' 7TV cosmetics refresh in real-time.
- **Chat Client Badge Gallery**: Explore a new section in the Badges Overlay to discover and track badges from popular third-party chat clients like FrankerFaceZ, Chatterino, Chatsen, Chatty, DankChat, and Homies. See which ones you own, how many users have them, and easily access their project pages.
- **Stripe Payments Integration**: StreamNook now offers Stripe for donations and subscriptions via streamnook.app, providing a streamlined and secure experience for supporters. Existing Ko-fi subscribers can easily migrate their support.
- **Lifetime Channel Points**: Track the total channel points you've auto-claimed across all your StreamNook sessions, now persisted even after restarting the app.
- **Enhanced Drops & Rewards UI**: The Drops Center and Game Detail panels now feature a clearer rewards showcase with progress timelines, making it easier to track your journey towards coveted drop rewards.
- **7TV Emote Update Notices**: A new chat setting lets you toggle system messages for live 7TV emote set changes, giving you control over chat notifications.

### 🐛 Bug Fixes
- **Improved Drops Statistics**: Corrected calculations for 'Total Drops Claimed' and 'Drops in Progress' in the Drops Center, providing a more accurate overview of your earned and active drops.
- **Instant 7TV Cosmetic Updates**: Fixed an issue where changing your 7TV paint or badge in Profile Settings wouldn't immediately reflect in chat or your profile card until an app restart. Changes are now instant across the app.
- **Robust 7TV Cosmetic Fetching**: Enhanced reliability for fetching 7TV user cosmetics, reducing instances of blank paints/badges due to transient API errors or overly complex batch requests.
- **Accurate 7TV Paint Shadows**: 7TV paint shadows now render correctly in the Badges Overlay, matching their appearance in chat and on user profiles.

### ⚡ Performance
- **Optimized Chat Rendering**: Removed display of third-party chat client badges from individual chat messages, improving chat rendering performance and reducing visual clutter. These badges are now exclusively viewable in the new Chat Client Badge Gallery.

### 🔧 Maintenance
- **Updated Support Experience**: Streamlined the Support settings tab, removing the direct developer contact section to centralize support through the Discord community and streamnook.app. Your contact options are now clearer and more effective.
- **UI Polish**: Applied "liquid glass" visual refinements to overlays and panels, enhancing the overall aesthetic and consistency across the application.
- **Internal Payment System Upgrade**: Major internal upgrades to support new payment pathways via streamnook.app, including a refined system for awarding cosmetics based on payment tiers.

## [7.5.4] - 2026-05-25
## ✨ New: Emote Tab Completion in Chat!
> Typing in chat just got a whole lot smarter. Now you can type a few letters of an emote or a username, press `Tab`, and StreamNook will instantly suggest and cycle through matching emotes from Twitch, 7TV, BTTV, FFZ, and even current chatters! No more remembering full emote names, just fluid conversation.

---

### ✨ Features
- **Emote Tab Completion**: Type a partial emote name or username in chat, press `Tab` to cycle through suggestions in a slick carousel, and Shift+Tab to cycle backward. Customize matching modes (starts with/contains) and if chatters are included in Chat Settings. Thanks to <@FROGGICON> for the suggestion!
- **MultiNook Shortcut**: Quickly add any stream to MultiNook by Ctrl+clicking a stream card on the Home page or in the sidebar. Thanks to <@FROGGICON> for the suggestion!
- **Enhanced Emote Tooltips**: Hover over any emote in chat for a richer preview showing the full emote name, provider (Twitch, 7TV, BTTV, FFZ), a larger image, and a 'Zero-Width' indicator if applicable. Thanks to <@FROGGICON> for the suggestion!

### 🐛 Bug Fixes
- Resolved an issue where some chat identity badges in Profile Settings would appear duplicated. Spotted by <@grim_7tv>, much appreciated!
- Fixed a layering bug where the sidebar might be obscured by other fullscreen elements when in 'hidden' or 'overlay' mode.

## [7.5.3] - 2026-05-25
## ✨ New: Emote Tab Completion in Chat!
> Typing in chat just got a whole lot smarter. Now you can type a few letters of an emote or a username, press `Tab`, and StreamNook will instantly suggest and cycle through matching emotes from Twitch, 7TV, BTTV, FFZ, and even current chatters! No more remembering full emote names, just fluid conversation.

---

### ✨ Features
- **Emote Tab Completion**: Type a partial emote name or username in chat, press `Tab` to cycle through suggestions in a slick carousel, and Shift+Tab to cycle backward. Customize matching modes (starts with/contains) and if chatters are included in Chat Settings. Thanks to <@FROGGICON> for the suggestion!
- **MultiNook Shortcut**: Quickly add any stream to MultiNook by Ctrl+clicking a stream card on the Home page or in the sidebar. Thanks to <@FROGGICON> for the suggestion!
- **Enhanced Emote Tooltips**: Hover over any emote in chat for a richer preview showing the full emote name, provider (Twitch, 7TV, BTTV, FFZ), a larger image, and a 'Zero-Width' indicator if applicable. Thanks to <@FROGGICON> for the suggestion!

### 🐛 Bug Fixes
- Resolved an issue where some chat identity badges in Profile Settings would appear duplicated. Spotted by <@grim_7tv>, much appreciated!
- Fixed a layering bug where the sidebar might be obscured by other fullscreen elements when in 'hidden' or 'overlay' mode.

## [7.5.2] - 2026-05-25
## 📚 New: Deeper Chat Logs for Power Users
> Ever wonder what someone said five minutes ago, or what a specific user's chat history looks like in a channel? Our new chat log system pulls from multiple sources (Twitch, Justlog, Robotty) to give you a comprehensive, merged view of historical messages. Perfect for moderation or just satisfying curiosity.

---

### ✨ Features
- **Chat Log History**: View a user's full chat history in any channel, aggregated from multiple sources, by right-clicking their username.
- **Enhanced User Profiles**: Profile cards now display more details, including a user's last broadcast, total channels followed, and detailed subscription information (tier, type, gifter).
- **Improved Chat Popout Windows**: Popout chat windows now intelligently anchor to your cursor's position when opened and come with larger, more user-friendly default dimensions.

### 🐛 Bug Fixes
- **Ad-Free Assurance**: Resolved an issue where outdated TTVLOL Streamlink plugins could prevent ad-blocking from working correctly, ensuring a consistent ad-free experience.
- **Drops Mining Reliability**: Fixed drops mining getting stuck on offline channels and improved task management to prevent "ghost" mining sessions.
- **Channel Points Accuracy**: Eliminated a bug that caused duplicate channel points events to appear in chat.
- **Changelog Display**: The in-app changelog overlay now correctly renders all markdown content, including banners and structured sections, without accidental truncation.
- **Windows Update Experience**: Fixed the brief flash of a console window that previously occurred during auto-updates on Windows.

### ⚡ Performance
- **Faster App Startup & Responsiveness**: Major architectural improvements to our internal HTTP client and caching systems mean the app starts faster, feels snappier, and consumes fewer resources.
- **Optimized Chat Rendering**: Chat messages now render more smoothly with reduced flickering, and the app uses significantly less memory for chat history, especially in busy channels.
- **Smarter Background Tasks**: Non-critical background polling (like viewer counts and pinned messages) now intelligently pauses when the window is minimized or not visible, reducing CPU and network usage.

### 🔧 Maintenance
- Removed Magne RPC integration.

## [7.5.1] - 2026-05-25
## 🎉 New: StreamNook has its own badges

> A brand new community cosmetics system, sitting alongside Twitch, 7TV, and FFZ in chat. Every member gets the default StreamNook badge for free. The gold **Supporter** and animated **Subscriber** badges unlock when you back the project on Ko-fi (drop your `@twitchhandle` in the donation message and the badge lands on your account automatically). Browse the full set in the new "StreamNook" tab inside the Badges Overlay, then equip whichever one you want from your Profile.

---

### ✨ Features
- **Command Palette**: A brand new global Command Palette (Ctrl/Cmd+K) lets you quickly search for and execute any app command, setting, streamer, or snippet. It also integrates live Twitch searches and recently chatted users for lightning-fast navigation.
- **StreamNook Cosmetics**: Active selections render next to your name in chat and in your profile card, with animated designs for monthly subscribers.
- **Advanced Chat Customization**: Take control of your chat with new global emote scaling and spacing settings. Introduce new highlight types for specific users, badges, or built-in events like first-time chatters, returning users, self-messages, and raids, all with configurable colors. Important highlights can now flash your window title when unfocused.
- **Smart Chat Input**: Added 'Bypass Duplicate' to send identical messages without triggering Twitch's filter, and 'Quick Send' (Ctrl/Cmd+Enter) to send messages while keeping your input, perfect for rapid-fire responses.
- **Improved Streamlink Management**: StreamNook can now automatically detect existing Streamlink installations on your system (e.g., from Program Files, Scoop, Chocolatey) and validate custom paths, making setup much smoother. Plus, diagnostic logging is now more robust for easier troubleshooting.
- **Native Screen Capture**: Integrated native screen capture capabilities, enabling high-quality static PNGs and animated WebP exports of UI elements. This will power future sharing features, like exporting your StreamNook profile card.
- **Live Announcements**: Receive important messages, tips, or urgent recovery instructions directly within the app. Announcements are filtered by app version and can include actionable links.
- **Update Repair System**: Automatically detects and offers to repair issues where previous app updates might have been interrupted (e.g., by antivirus). This ensures your StreamNook app executable and components are always perfectly in sync, improving stability and performance.
- **Enhanced "What's New"**: Release notes in settings now feature richer markdown, including prominent headlines and styled callouts, making it easier to see what's new at a glance.

### 🐛 Bug Fixes
- **Update Reliability**: Significantly improved the internal update mechanism with retry logic and hardened file operations, preventing silent failures that could lead to desynced installations.
- **Streamlink Error Reporting**: Fixed an issue where Streamlink errors were sometimes silently swallowed, making it difficult to diagnose streaming problems. StreamNook now captures comprehensive diagnostic logs to provide clearer error messages.
- **Update Notifications**: Refined the update notification process to be less intrusive. Update availability is now passively displayed in the title bar, and toasts direct you to the 'What's New' section for details.
- **Chat Display Consistency**: Ensured consistent display for deleted messages and corrected the toast notification type for channel points.


### 🧹 Housekeeping
- The old 'Profile Modal' has been removed as its functionality is now integrated into the new StreamNook cosmetics system and improved search features.

## [7.5.0] - 2026-05-25
### ✨ Features
- **Command Palette**: A brand new global Command Palette (Ctrl/Cmd+K) lets you quickly search for and execute any app command, setting, streamer, or snippet. It also integrates live Twitch searches and recently chatted users for lightning-fast navigation.
- **StreamNook Cosmetics**: Unleash your unique style with the new StreamNook cosmetics system! Browse and equip special badges from the dedicated 'StreamNook' tab in the Badges Overlay, with support for animated designs and acquisition via Ko-fi.
- **Advanced Chat Customization**: Take control of your chat with new global emote scaling and spacing settings. Introduce new highlight types for specific users, badges, or built-in events like first-time chatters, returning users, self-messages, and raids, all with configurable colors. Important highlights can now flash your window title when unfocused.
- **Smart Chat Input**: Added 'Bypass Duplicate' to send identical messages without triggering Twitch's filter, and 'Quick Send' (Ctrl/Cmd+Enter) to send messages while keeping your input, perfect for rapid-fire responses.
- **Improved Streamlink Management**: StreamNook can now automatically detect existing Streamlink installations on your system (e.g., from Program Files, Scoop, Chocolatey) and validate custom paths, making setup much smoother. Plus, diagnostic logging is now more robust for easier troubleshooting.
- **Native Screen Capture**: Integrated native screen capture capabilities, enabling high-quality static PNGs and animated WebP exports of UI elements. This will power future sharing features, like exporting your StreamNook profile card.

### 🐛 Bug Fixes
- **Streamlink Error Reporting**: Fixed an issue where Streamlink errors were sometimes silently swallowed, making it difficult to diagnose streaming problems. StreamNook now captures comprehensive diagnostic logs to provide clearer error messages.
- **Update Notifications**: Refined the update notification process to be less intrusive. Update availability is now passively displayed in the title bar, and toasts directing you to the 'What's New' section for details.
- **Chat Display Consistency**: Ensured consistent display for deleted messages and corrected the toast notification type for channel points.

### 🧹 Housekeeping
- The old 'Profile Modal' has been removed as its functionality is now integrated into the new StreamNook cosmetics system and improved search features.

## [7.4.0] - 2026-05-22
### ✨ Features
- **StreamNook MultiChat (Experimental): A full standalone chat client built right into StreamNook. Open any channel's chat in its own window with no video stream required, and run as many at once as you want. Right-click a stream tile and pick "Pop out chat", or use the pop-out button at the top of any active stream's chat widget. Each MultiChat window keeps running even when the main app is hidden to the system tray.
- **StreamNook Identity Badge**: A new community badge for every StreamNook user, reflecting your rank (#N) based on when you joined. See it in chat, on user profiles, and check out the new StreamNook tab in the Badges Overlay!
- **User Chat Overrides**: Customize your chat experience by setting local nicknames and custom colors for other users directly from their profile card. Only you see these changes!
- **Custom Chat Highlight Phrases**: Define your own keywords or phrases to highlight in chat, complete with custom colors and sounds. Never miss an important message again!
- **User-Defined Custom Chat Commands**: Create your own slash commands or even plain-text triggers that expand into longer messages or templates. Perfect for quick responses or custom emotes.
- **New Moderator Commands**: `/clearmessages` allows for locally clearing recent chat messages visually (only on your screen). `/usercard <username>` quickly opens any user's profile card.
- **System Tray Integration**: StreamNook now lives in your system tray, ensuring MultiChat windows stay active even when the main app is hidden. Easily restore the main window or open new MultiChat windows from the tray menu.
- **Cross-Window Settings Sync**: All your settings now synchronize in real-time across all open StreamNook windows, so changes made in one window instantly apply everywhere else.
- **Enhanced User Profiles**: User profile cards now display the streamer's full channel banner image, not just their offline placeholder.
- **Stream Context Menu Action**: A new 'Pop out chat' option in the stream context menu allows you to quickly open any stream's chat in a dedicated MultiChat window without starting the video stream.

### 🐛 Bug Fixes
- **Channel Points Redemption**: Resolved issues with channel points redemption by updating our system to match recent Twitch API changes, ensuring all rewards can be redeemed as expected. All redemptions now include a confirmation step to prevent accidental spending.
- **Stale Badge Information**: Fixed an issue where some badge details (like event dates) were not updating correctly, causing outdated availability information. Badge metadata now aggressively refreshes to ensure accuracy.
- **Multi-Channel Emote Display**: Repaired a bug that prevented third-party emotes (7TV, FFZ, BTTV) from correctly displaying in newly joined channels, particularly in MultiChat popout windows.
- **Chat Scroll Stability**: Significantly improved chat scrolling smoothness, eliminating visual 'shimmering' or 'jumping' that occurred when new messages arrived and content (emotes, badges, timestamps) resolved.
- **Chat Message Dividers**: Adjusted chat message dividers to anchor to the top of each message, preventing layout shifts and improving overall chat fluidity.
- **Window Aspect Ratio**: Corrected an issue where the main window's aspect ratio would not adjust properly when its chat panel was dynamically hidden because a MultiChat popout was active for the same channel.
- **Minor UI Polish**: Squashed several small bugs related to UI element positioning and hover states in overlays (Badge Detail, Drops, Badges Overlay).

### 🔧 Maintenance
- Refactored internal notification sound system for better performance and consistency.

## [7.3.3] - 2026-05-18
### 🐛 Bug Fixes
- Fixed an issue where the Follow and Subscribe buttons could become unresponsive after restarting a stream or being automatically redirected to a new channel by a raid. StreamNook now ensures all necessary channel information is always correctly loaded to keep these features working seamlessly. Thanks to <@rainyyay> for the report!.

### 🔧 Maintenance
- Minor internal code improvements for the streamlink manager.

## [7.3.2] - 2026-05-15
### ✨ Features
- **Enhanced Streaming Options**: You can now enable support for h265 and AV1 codecs, unlocking higher quality tiers (like 1440p and 2160p) on compatible streams!
- **Improved Quality Selection**: The player now accurately reflects the actual quality being streamed, even if Streamlink had to automatically select a different one due to availability. You'll receive a notification if a fallback occurs.
- **Quality Fallback Notifications**: Get informed when the stream automatically switches to a different quality than your preference due to stream limitations.

### 🐛 Bug Fixes
- Corrected an issue where Streamlink might hang indefinitely when trying to resolve stream URLs under certain network conditions. Thanks to <@Rainy> on Discord for the feedback!
- Ensured that Streamlink uses anonymous authentication when starting multi-viewer sessions to prevent potential conflicts.
- Resolved a bug where certain Twitch badges or emotes might not render correctly in chat.
- Ensured that the correct VOD is selected when viewing offline chat for a streamer.

### 🔧 Maintenance
- Streamlined how Twitch authentication tokens are managed, improving reliability and security.
- Updated dependencies and improved internal logging for better diagnostics.

## [7.3.1] - 2026-05-15
### ✨ Features
- **Enhanced Streaming Options**: You can now enable support for h265 and AV1 codecs, unlocking higher quality tiers (like 1440p and 2160p) on compatible streams!
- **Improved Quality Selection**: The player now accurately reflects the actual quality being streamed, even if Streamlink had to automatically select a different one due to availability. You'll receive a notification if a fallback occurs.
- **Quality Fallback Notifications**: Get informed when the stream automatically switches to a different quality than your preference due to stream limitations.

### 🐛 Bug Fixes
- Corrected an issue where Streamlink might hang indefinitely when trying to resolve stream URLs under certain network conditions. Thanks to <@Rainy> on Discord for the feedback!
- Ensured that Streamlink uses anonymous authentication when starting multi-viewer sessions to prevent potential conflicts.
- Resolved a bug where certain Twitch badges or emotes might not render correctly in chat.
- Ensured that the correct VOD is selected when viewing offline chat for a streamer.

### 🔧 Maintenance
- Streamlined how Twitch authentication tokens are managed, improving reliability and security.
- Updated dependencies and improved internal logging for better diagnostics.

## [7.3.0] - 2026-05-15
### ✨ Features
- **Enhanced Streaming Options**: You can now enable support for h265 and AV1 codecs, unlocking higher quality tiers (like 1440p and 2160p) on compatible streams!
- **Improved Quality Selection**: The player now accurately reflects the actual quality being streamed, even if Streamlink had to automatically select a different one due to availability. You'll receive a notification if a fallback occurs.
- **Quality Fallback Notifications**: Get informed when the stream automatically switches to a different quality than your preference due to stream limitations.

### 🐛 Bug Fixes
- Corrected an issue where Streamlink might hang indefinitely when trying to resolve stream URLs under certain network conditions. Thanks to <@Rainy> on Discord for the feedback!
- Ensured that Streamlink uses anonymous authentication when starting multi-viewer sessions to prevent potential conflicts.
- Resolved a bug where certain Twitch badges or emotes might not render correctly in chat.
- Ensured that the correct VOD is selected when viewing offline chat for a streamer.

### 🔧 Maintenance
- Streamlined how Twitch authentication tokens are managed, improving reliability and security.
- Updated dependencies and improved internal logging for better diagnostics.

## [7.2.2] - 2026-05-07
### 🐛 Bug Fixes
- Fullscreen now fills the entire display when the app window isn't maximized
- Fixed blue border / chrome leak when re-entering fullscreen from a maximized window
- Window correctly returns to maximized after exiting fullscreen, no more drift off-screen
- Closes #125 (items 1 and 2) — special thanks to @swott for filing the very first issue on the repo

## [7.2.1] - 2026-05-07
### 🐛 Bug Fixes
- Fullscreen now fills the entire display when the app window isn't maximized
- Fixed blue border / chrome leak when re-entering fullscreen from a maximized window
- Window correctly returns to maximized after exiting fullscreen, no more drift off-screen
- Closes #125 (items 1 and 2) — special thanks to @swott for filing the very first issue on the repo
## [7.2.0] - 2026-03-31
> [!IMPORTANT]
> **Re-Authentication Required**
> When you launch this update, StreamNook will automatically log you out and prompt you to log back in. This is totally normal and expected! We've added powerful new moderation tools and features that require you to grant StreamNook the new necessary Twitch permissions.

### ✨ Features
- **Enhanced Category Browsing**: Dive into categories with new sub-tabs for Live Streams, Clips, and Videos, complete with sorting and filtering options.
- **Moderator Tools**: Manage your chat effectively with new commands like `/ban`, `/timeout`, `/clear`, `/mod`, `/vip`, and more, directly from the UI.
- **Command Autocomplete**: Type `/` in chat to see a helpful list of available commands and their usage.
- **Moderator Logs**: A new pane displays all moderation actions taken in chat, providing transparency and history.
- **Stream Nook Media Player**: Watch clips and VODs directly within the app with improved playback for MP4 files and a more robust HLS player.
- **User Profile Enhancements**: Quickly access moderator actions, copy messages, or pre-fill commands directly from user profiles.
- **Drops Integration**: Improved visibility and interaction with Twitch Drops campaigns directly from the Home screen.
- **Visual Polish**: Numerous UI refinements, including interactive command suggestions, refined glassmorphism effects, and a collapsible moderator menu.

### 🐛 Bug Fixes
- **Fixed Streamlink Issues**: Resolved problems with custom Streamlink paths and improved handling of VOD/clip playback.
- **Chat Stability**: Addressed various bugs in chat rendering, message parsing, and connection management for a more reliable experience.
- **Player Reliability**: Improved HLS player stability, especially during stream restarts and offline transitions.

### 🔧 Maintenance
- **Expanded Twitch API Integration**: Added support for a wide range of Twitch API endpoints to manage streams, chat settings, user data, and more.
- **Performance Optimizations**: Refined caching strategies and component rendering to improve overall application responsiveness.
- **Code Refactoring**: Various under-the-hood improvements to enhance code quality and maintainability.

## [7.1.2] - 2026-03-30
### 🔧 Maintenance
- **Update Checks**: Improved how StreamNook checks for updates by directly downloading necessary files from GitHub releases. This bypasses GitHub API rate limits, ensuring more reliable update checks, especially during active development.
- **Release Notes**: Release notes for updates are now fetched directly from the `CHANGELOG.md` file, providing a more accurate and up-to-date description of changes.
- **Download Size**: The download size for updates is now more accurately reported by using HTTP HEAD requests.

## [7.1.1] - 2026-03-30
### ✨ Features
- **Enhanced 7TV Emote Support**: 7TV emotes now display correctly with their intended resolutions thanks to improved `srcset` implementation, offering a crisper and more detailed viewing experience. Zero-width emotes are also better integrated.
- **Improved Whisper Functionality**: The Whisper feature now intelligently parses and displays URLs within messages, allowing direct clicking to open them. Additionally, clicking on a user's profile picture in the Whisper sidebar will now correctly open their profile modal.
- **Refined UI & UX**: Several quality-of-life improvements have been made across the application, including better input field focus management, smoother scrolling in chat and whisper windows, and an updated visual appearance for the Whisper widget with enhanced glassmorphism effects.

### 🐛 Bug Fixes
- Fixed an issue where certain input fields and chat areas would lose focus unexpectedly, improving overall usability.
- Resolved issues with scrolling behavior in chat and whisper components, ensuring a smoother experience.

### 🔧 Maintenance
- **Viewport Optimization**: Updated the viewport meta tag to prevent user scaling on mobile devices, ensuring a more consistent display.

### ⚡ Performance
- **Optimized Emote Grid Layout**: Emotes are now grouped by width categories, leading to a more organized and performant emote picker, especially for users with many emotes.

## [7.1.0] - 2026-03-29
### ✨ Features
- **MultiNook Enhancements**: Introducing the Grid Engine (MultiNook)! You can now dock, undock, arrange, and manage multiple streams in a highly customizable grid. The tutorial guides you through adding streams, focusing audio, reordering layouts, and more.
- **7TV Zero-Width Emotes**: Full support for 7TV's zero-width emotes has been added, allowing for more expressive chat interactions.
- **User Profile Modal**: A new modal allows you to view user profiles directly within the app, including their bio, social links, and panels. You can also follow or unfollow users directly from this modal.
- **Offline Channel View**: The Home screen now displays your followed channels that are currently offline, showing their last broadcast time for context.
- **Improved Search & Navigation**: Enhanced stream search with better handling of exact matches and offline users. Improved category browsing and navigation within the app.

### 🐛 Bug Fixes
- **Zero-Width Emote Rendering**: Corrected rendering of zero-width emotes, ensuring they display correctly within chat messages, especially when layered.
- **Chat Refresh Logic**: Fixed an issue where the chat wouldn't properly refresh after certain UI actions, ensuring a more stable chat experience.
- **Follow/Unfollow Status**: Resolved bugs related to accurately displaying follow status for users.
- **MultiNook Hot-Swapping**: Ensured that MultiNook correctly handles switching between streams, updating backend processes like EventSub and drops monitoring.

### 🔧 Maintenance
- **Dependency Updates**: Updated various dependencies, including Tauri core and plugins, for improved stability and performance.
- **Code Splitting**: Implemented code splitting in the Vite build process for faster initial load times.
- **UI Polish**: Numerous minor styling adjustments and usability improvements across various components for a smoother user experience.

## [7.0.0] - 2026-03-28
### ✨ Features
- **Multi-Stream Viewing**: Introducing MultiNook! Watch and manage multiple streams simultaneously.
- **Unified Chat**: View and send messages across multiple chat channels in a single widget.
- **Watch Streak Sharing**: Share your watch streak progress directly in chat.
- **Improved Player**: Enhanced HLS playback with better buffering, adaptive quality, and more robust error handling.
- **Proxy Optimization**: Automatically detects and applies the fastest proxy for ad-free viewing.
- **Customizable UI**: Expanded theme creator with more options and refined appearance.
- **Context Menus**: Added right-click menus for chat messages and streams.
- **Settings Improvements**: Redesigned settings pages with better organization and new options for MultiNook and proxy management.

### 🐛 Bug Fixes
- Fixed issues with stream playback freezing when switching quality.
- Resolved crashes related to WebSocket connections and chat participant updates.
- Improved stability of the automatic stream offline detection.
- Addressed various layout glitches and visual inconsistencies across the application.
- Fixed an issue where the custom Streamlink path setting was not always respected.

### 🔧 Maintenance
- Updated dependencies across frontend and backend for improved performance and security.
- Refactored various components for better code organization and maintainability.
- Added new scripts for diagnostic HLS analysis and SVG to PNG conversion.

### ⚡ Performance
- Optimized emote and badge rendering for significantly faster chat display.
- Improved caching strategies for emotes, badges, and profile data to enhance offline support and load times.
- Reduced application memory footprint for a lighter desktop experience.

## [6.11.0] - 2026-03-26
### ✨ Features
- **Streamer Info Panels**: You can now view detailed streamer information, including bio, social media links, and custom panels, directly within the app when switching to the 'About' view in the chat panel.
- **Pinned Chat Messages**: See important pinned messages at the top of the chat, providing crucial context or announcements from the streamer.
- **Improved Follow/Unfollow**: Follow and unfollow channels directly from the app using StreamNook's new GQL integration, offering a more seamless experience.
- **Enhanced Drops Mining**: Configure dedicated 'Priority Farm Channels' to ensure specific streams are always targeted for drops mining, giving you more control over your campaigns. The mining logic has also been refined for better recovery and stability.

### 🐛 Bug Fixes
- **Chat Layout Stability**: Addressed issues with chat widget layout, ensuring smoother transitions when switching between chat and the new streamer info panels.
- **Follow Action Reliability**: Fixed potential issues with follow/unfollow actions not completing due to changes in Twitch's backend automation.

### 🔧 Maintenance
- **GQL Integration for Follow/Unfollow**: Replaced outdated browser automation for follow/unfollow actions with more robust and efficient GQL mutations.
- **Stream Watching Logic Refinement**: Optimized the background channel points watching service for better performance and more reliable point earning.
- **UI Animation Polish**: Introduced new slide animations for smoother transitions between chat and streamer info views.

## [6.10.1] - 2026-03-08
### ✨ Features
- **Rich Presence Enhancements**: StreamNook now provides more detailed information for Discord and Magne presence, including the stream's title, category, and the game being played. This offers a richer experience for others viewing your activity.

### 🐛 Bug Fixes
- **Accurate Presence Updates**: Fixed an issue where the activity key for presence updates was not comprehensive, potentially leading to duplicate or inaccurate status displays. The presence will now correctly reflect the current stream details.

## [6.10.0] - 2026-03-06
### ✨ Features
- **Magne Rich Presence**: StreamNook now integrates with Magne, a Discord-like rich presence client. Your watching activity will be displayed automatically, including stream details, game, and a "Watch Stream" button that opens the stream directly in StreamNook. We have kept Discord RPC functionality, but will be shifting focus toward Magne, our new project, with ambitions of pursing a privacy-focused social platform that doesn't want to sell your identity.
- **Deep Link Support**: You can now open specific streams directly from external applications or the command line using `streamnook://watch/<channel>` URLs.
- **Automatic Proxy Optimization**: On startup, StreamNook will automatically check available proxy servers, select the fastest one, and configure your streamlink settings for optimal performance. This ensures you always have the best connection without manual intervention.
- **Enhanced Chat Moderation**: The chat now visually reflects moderation actions like timeouts and bans, clarifying which messages are affected. You'll also see timely notifications for chat room state changes (e.g., slow mode, sub-only).

### 🐛 Bug Fixes
- **Accurate Badge Cache Age**: The displayed age of your badge cache now correctly reflects the freshness of your local data, not just the last manifest sync.
- **Improved Date Parsing**: Badge availability dates are parsed more reliably, correctly identifying upcoming, active, and expired events.
- **Streamlink Proxy Stability**: Resolved issues with proxy configuration, ensuring more reliable streaming performance.
- **Chat Message Scoping**: Moderation actions in chat are now accurately scoped to the specific messages they affect.

### 🔧 Maintenance
- Updated dependencies and internal libraries for improved stability and performance.

## [6.9.0] - 2026-01-30
### ✨ Features
- **Proxy Health & Auto-Selection**: StreamNook now includes a robust proxy health checker! Test various proxy servers, see their real-time latency, and automatically select the fastest one for your region to improve stream stability.
- **Stream Restart**: Encounter a stuttering stream? You can now manually restart the current stream without closing and reopening the application.
- **Badge Drop Verification**: Improved logic for verifying ownership of Twitch badge drops by comparing against your earned badge collection.
- **Enhanced Drops Mining**: Subscription-only drops are now correctly handled, preventing them from interfering with active mining progress. Completed drops from expired campaigns are also more reliably displayed.

### 🐛 Bug Fixes
- **7TV Emote Display**: Fixed an issue where certain 7TV emotes and paints might not display correctly.
- **Date Parsing for Badges**: Improved accuracy in parsing badge metadata dates for better display.
- **Chat Timestamp Accuracy**: Ensured chat timestamps are displayed correctly, leveraging backend data where available.
- **Proxy Settings**: Addressed issues with manual proxy argument input and improved the handling of applying optimal proxy settings.
- **UI State Updates**: Optimized various UI components to use `queueMicrotask` for state updates, preventing rendering glitches and ensuring smoother transitions.

### 🔧 Maintenance
- **Dependency Updates**: Updated `lucide-react` for newer icons.
- **Code Refactoring**: Cleaned up various internal components for better maintainability and performance, including improvements to chat message rendering and sidebar logic.

## [6.8.0] - 2026-01-29
### ✨ Features
- **7TV Cosmetics Deep Linking**: You can now open the Badges Overlay directly to a specific 7TV paint or badge using deep links.
- **7TV Cosmetic Sorting & Filtering**: Enhanced the Badges Overlay with sorting options (newest, oldest, name) and filtering for animated/static 7TV paints.
- **Cheermote Segment**: Added support for rendering Twitch Cheermotes (animated bits) in chat messages.
- **Diagnostic Logging Control**: Introduced a new setting to toggle diagnostic log verbosity (Debug/Info vs. Warn/Error), controllable via `Settings > Support > Enable Diagnostic Logging`.
- **ULID Timestamp Utilities**: Added helper functions to extract creation dates from ULID identifiers, used for sorting 7TV cosmetics.

### 🐛 Bug Fixes
- **Chat Badge Display**: Fixed an issue where 7TV badges were not correctly rendered with their associated metadata.
- **Profile Overlay**: Resolved a bug where the profile overlay would fail to open if the user ID was not numeric.
- **Streamlink Missing Dialog**: Improved error handling and fallback behavior when Streamlink is not found.
- **Subscription Status Display**: Corrected the logic for showing subscription status in the video player (e.g., correctly indicating lapsed subscribers).

### 🔧 Maintenance
- **Centralized Logger**: Replaced all `console.log`, `console.warn`, `console.error` calls throughout the application with a new centralized `Logger` utility. This allows for runtime control over log verbosity based on user diagnostic settings, reducing noise in production builds.
- **Dependency Updates**: Updated various project dependencies to their latest stable versions.
- **Code Refactoring**: Applied various refactoring efforts across the codebase to improve code quality and maintainability.

## [6.7.0] - 2026-01-24
### ✨ Features
- **7TV Cosmetics Integration**: Access and view 7TV badges and paints directly within the application! Browse them in the new 'Attainables' section of your profile.
- **Twitch Resub Notifications**: Get notified when you can share your subscription anniversary in chat, with options to include your sub streak.
- **@Mention Autocomplete**: Type '@' in the chat input to bring up a list of recent chat participants, making it easier to tag users.
- **Network Speed Test**: A new 'Network' tab in Settings allows you to run a quick speed test via Cloudflare to understand your connection quality and get recommendations for optimal streaming settings.
- **Improved Emote Handling**: Emotes now load more efficiently with background caching and lazy loading in the emote picker, reducing UI stutter.

### 🐛 Bug Fixes
- **HLS Jump-to-Live Buffer**: Fixed an issue where jumping to live could cause playback stalls due to insufficient buffer.
- **Streamlink Timeouts**: Streamlink processes now have a timeout to prevent the application from hanging indefinitely.
- **Emoji Caching**: Improved emoji caching to prevent retrying failed fetches, ensuring a smoother experience.

### 🔧 Maintenance
- **Code Refinements**: Various code improvements and dependency updates for better performance and stability.

## [6.6.1] - 2026-01-09
### ✨ Features
- **Hype Train Enhancements**: Experience more engaging Hype Trains with animated level-up celebrations, confetti, and special "HYPE" messages! See Hype Train status directly on stream cards in the Home feed and Sidebar. Golden Kappa trains now have a distinct visual flair.
- **Improved Chat Scrolling**: Chat now scrolls smoothly to specific messages, respecting the container boundaries to prevent unexpected document scrolling. Auto-scrolling to new messages is also more robust.
- **7TV Badge Fallbacks**: 7TV badges now utilize a fallback system, ensuring they load reliably even if the primary resolution fails.
- **Stream Card Badges**: A new Hype Train indicator badge has been added to stream cards in the Home feed and Sidebar, showing the current level and if it's a Golden Kappa train.

### 🐛 Bug Fixes
- **Badge Loading**: Addressed issues with loading badges in chat messages and user profiles by pre-initializing caches and implementing a fallback image component.
- **Chat History Loading**: Ensured chat history (IVR messages) loads with fully populated badges by initializing caches before fetching.
- **Hype Train Timer**: Fixed an issue where the Hype Train timer could expire unexpectedly.

### 🔧 Maintenance
- **Badge Caching**: Improved the logic for caching and displaying Twitch badges, preventing cross-channel pollution.
- **Chat Message Limit**: Increased the default chat message history limit to 100 for a more complete view.

## [6.6.0] - 2026-01-08
### ✨ Features
- **Channel Points Rewards - BETA**: You can now view and redeem channel points rewards directly within the chat interface! This includes features like "Highlight My Message", "Unlock Random Emote", "Modify Emote", and "Choose Emote to Unlock".
- **Hype Train Integration**: StreamNook now visually displays active Hype Trains directly in the chat header, showing progress, level, and remaining time. It also subscribes to EventSub notifications for Hype Train events.
- **7TV Emote Improvements**: Emotes from 7TV are now fetched at higher resolution (4x) for better clarity, and animated emotes are handled correctly.
- **Emoji Rendering**: Improved emoji display using the Apple CDN via a local proxy for better performance and consistency, with support for animated emojis.
- **Badge Caching**: Implemented a reactive caching system for Twitch badges, ensuring faster loading and offline availability.

### 🐛 Bug Fixes
- **Badge Rendering**: Fixed issues with badge display, ensuring correct URLs and fallbacks.
- **Message Parsing**: Improved parsing of chat messages to correctly handle emojis and various message types.

### 🔧 Maintenance
- **Satoshi Variable Fonts**: Updated font loading to use variable Satoshi fonts, leading to better text rendering performance and consistency across all weights and styles.
- **Cache Optimizations**: Enhanced the universal file cache to check for existing files before downloading, reducing redundant operations.
- **EventSub Subscriptions**: Added Hype Train EventSub subscriptions and improved handling for events requiring moderator access.

## [6.5.0] - 2026-01-08
### ✨ Features

- **Custom Channel Points Display**: StreamNook now supports displaying custom channel points names (like "Kisses" instead of "Points") and their unique icons directly in chat, predictions, and channel point widgets. This brings a more personalized experience to your favorite streamers' communities.

- **Native Chat Virtualization**: The chat display has been rebuilt to leverage the browser's built-in `content-visibility` CSS property. This significantly improves performance by allowing the browser to skip rendering off-screen messages entirely, leading to smoother scrolling and reduced CPU usage, especially in busy chats.


- **Improved Scroll-to-Message**: The functionality to jump to a specific message has been rewritten for greater reliability. It now uses native DOM scrolling and ensures chat remains paused during navigation, preventing interference and ensuring you land precisely where you intend.

### 🐛 Bug Fixes

- **Chat Height Calculation**: Resolved inconsistencies in chat message height calculations, particularly for historical messages and during fast scrolling, by adopting a DOM-first measurement approach.

### 🔧 Maintenance

- **Backend Chat Parsing Simplification**: Streamlined the backend parsing of historical chat messages, offloading layout calculations to the frontend for a more efficient architecture.

- **Layout Command Cleanup**: Removed unused backend commands related to chat layout configuration, simplifying the backend API.

### ⚡ Performance

- **Chat Performance Boost**: Significant performance improvements in chat rendering due to native CSS virtualization and optimized message handling, making the chat experience much smoother.

## [6.4.0] - 2026-01-05
### 🔧 Maintenance
- **Release Process Update**: Streamlined the release workflow to ensure `main` accurately reflects the `dev` branch before builds.

### ✨ Features
- **Enhanced Drops Mining Resilience**: The Drops mining feature is now more robust. If all streams for a campaign go offline, StreamNook will intelligently attempt to switch to the next campaign in your "Mine All" queue or notify you if no streams are available.

### 🐛 Bug Fixes
- Fixed an issue where the stream would freeze when switching channels.
- Correctly handle reserved streams going offline, returning the watch token to the general rotation pool and notifying the user.
## [6.3.1] - 2026-01-04
### 🐛 Bug Fixes
- **Date Parsing Accuracy**: Resolved an issue where certain badge acquisition dates were parsed incorrectly, ensuring historical data is displayed accurately.

## [6.3.0] - 2026-01-04
ATTENTION: ALL USERS LOG OUT OF TWITCH THEN LOG BACK IN FOR SOME CHANGES OT TAKE EFFECT <3

### ✨ Features
- **Custom Themes**: Unleash your creativity with a new theme editor! Design and save your own unique color schemes to personalize StreamNook.
- **Compact View Presets**: Easily resize your window to predefined sizes optimized for multitasking or second monitors. Choose from various aspect ratios, including 16:9, and create your own custom presets.
- **Favorite Drops Notifications**: Get notified on startup if new drops become available in your favorited game categories.
- **Enhanced Twitch Emotes**: View a richer set of emotes, including your subscription, bits, and follower emotes, directly in chat when authenticated.
- **Bits Cheer Styling**: Messages containing Twitch bits cheers now have distinct styling with dedicated layouts and icons.
- **Universal Cache Auto-Sync**: The app now automatically downloads the latest badge and emote data in the background, ensuring you always have the most up-to-date assets without manual intervention.

### 🐛 Bug Fixes
- Fixed an issue where the stream player could freeze when switching channels.
- Resolved an issue where older badge metadata dates might not parse correctly.
- Improved the accuracy of chat message height calculations to prevent clipping or extra spacing.

### 🔧 Maintenance
- Updated dependencies and Rust backend components for improved stability and performance.
- Refined logging to provide clearer debugging information, especially for HLS streaming errors.
- Optimized emote loading and caching mechanisms for faster chat initialization.

## [6.2.0] - 2025-12-18
### ✨ Features
- **7TV Cosmetics Integration**: Connect your 7TV account to customize your profile with unique paints and badges, directly from StreamNook!
- **Global Badge Management**: Easily view and select your Twitch global badges within the application. Access your selected badge and make changes directly.
- **Enhanced Profile View**: A new dedicated profile modal provides a comprehensive view of your badges and cosmetic items, with improved controls for managing them.

### 🐛 Bug Fixes
- **Chat Reconnection Stability**: Chat reconnection logic has been refined. It now performs a silent check of the stream status before attempting a full reconnect or stream offline action, preventing unnecessary disruptions when only chat has temporarily stalled.
- **HLS Player Offline Detection**: The HLS player's offline detection has been made more robust by using a ref for `isAutoSwitching`, ensuring accurate error handling and stream switching even when settings change.

### 🔧 Maintenance
- **7TV Authentication Flow**: StreamNook now handles 7TV account connections and authentications more smoothly, including improved token management and error handling.
- **Profile Data Caching**: User profile data, including badges and cosmetics, is now cached more effectively for faster loading and better offline support.
- **Chat Identity Badge Caching**: Implemented a robust caching system for chat identity badges, ensuring faster UI updates and reducing unnecessary background fetches.

## [6.1.0] - 2025-12-17
### ✨ Features

- **Chat Heartbeat & Stability**: The chat system now includes background heartbeats to ensure a stable connection, automatically reconnecting if issues arise. You'll see clearer connection status and fewer stale warnings.
- **Drops Favorites**: You can now mark your favorite games in the Drops Center! These will appear at the top, and you'll receive notifications when new drops become available for them.
- **Improved Drops UI**: Games in the Drops Center are now better sorted, with favorites first. We've also refined the display of drop types (time-based, subscription, etc.) for easier understanding.
- **Optimized User Profile Loading**: User profile cards now load significantly faster by instantly displaying cached data while fetching fresh information in the background. Badge and cosmetic rendering is also more performant.
- **Sidebar Auto-Refresh**: Your followed and recommended streams in the sidebar will now automatically refresh when you interact with or expand the sidebar, ensuring you always see the latest status.

### 🐛 Bug Fixes

- Fixed an issue where manually selected drops in the mining service were not bypassing game filters.
- Resolved a bug preventing the correct fetching of channel points balance in the Chat widget and Prediction overlay due to an incorrect GraphQL query path.
- Corrected the Twitch Client ID used for certain channel points queries in the backend.
- Improved the detection of the currently mining game in the Drops detail panel.
- Removed unused Picture-in-Picture (PIP) logic from the main App component.

### 🔧 Maintenance

- Enhanced the internal health check for the chat WebSocket, making it more resilient to temporary network fluctuations and correctly triggering reconnection logic.
- Refined the styling of the Home screen background for better contrast when overlays are present.
- Consolidated and optimized badge rendering logic within the user profile card.

### ⚡ Performance

- Initial loading of user profile cards is now much faster due to improved caching and background refresh strategies.

## [6.0.1] - 2025-12-14
### 🐛 Bug Fixes
- **Chat Performance**: Resolved a performance issue in the chat widget that could lead to incorrect item sizing calculations, improving overall chat rendering stability.

## [6.0.0] - 2025-12-14
### ✨ Features
- **Unified User Profile Service**: StreamNook now fetches and caches comprehensive user profiles (including 7TV cosmetics, all earned Twitch badges, and third-party badges from FFZ/Chatterino/Homies) directly from the Rust backend. This significantly improves performance and reliability.
- **7TV Cosmetics Support**: Display user 7TV paints and selected badges in profile views.
- **Enhanced Chat Rendering**: Chat messages now utilize pre-parsed segments and metadata computed in Rust, ensuring pixel-perfect layout prediction and smoother rendering.
- **Twitch EventSub Integration**: Real-time notifications for channel updates (title, category), stream online/offline status, and raids are now handled directly by the Rust backend.
- **Improved Emoji Handling**: Emoji shortcode conversion is now offloaded to the Rust backend, reducing frontend JavaScript load.
- **Activity Tracking & Logging**: Comprehensive logging system with error buffering, automatic batching, and Discord webhook integration (via Rust) for improved error reporting.
- **Badge Polling & Notifications**: Background polling for new and available Twitch badges is now managed by the Rust backend, triggering desktop notifications.

### 🐛 Bug Fixes
- **UI Layout Stability**: Addressed various layout calculation issues, particularly with messages containing replies, first-time messages, and complex formatting, ensuring accurate heights and preventing overlaps.
- **Reconnect Logic**: Improved reliability of WebSocket connections for chat and EventSub.
- **Emote Rendering**: Correctly prioritizes 7TV emotes over Twitch emotes when both are available.
- **Historical Message Parsing**: Ensures messages fetched from the IVR API are parsed and laid out correctly, matching live message appearance.
- **Third-Party Badge Display**: Resolved issues with loading and displaying badges from FFZ, Chatterino, and Homies.

### 🔧 Maintenance
- **Rust Backend Enhancements**: Significant portion of data fetching, parsing, and caching logic has been moved to the Rust backend for performance and reliability gains.
- **Dependency Updates**: Added `phf` crate for efficient static maps (e.g., emoji conversion).
- **Code Cleanup**: Removed deprecated services and internal data structures (e.g., `emojiMap.ts`, `badgePollingService.ts`, `twitchBadges.ts`, `cosmeticsCache.ts` helpers) in favor of the unified Rust backend calls.

## [5.1.1] - 2025-12-12
### ✨ Features
- **Channel Points Display**: You can now see your current channel points balance in the chat widget.
- **Prediction Overlay**: Participate in live predictions directly from StreamNook!  View active predictions, place bets, and see the results in real-time. This includes automatic bet input validation and enhanced resolution states (win/loss/refund).

### 🐛 Bug Fixes
- Resolved edge cases where prediction events would not display winning outcome correctly.
- Fixed an issue where the stream would freeze when switching channels.

## [5.1.0] - 2025-12-12
### ✨ Features
- **Predictions**: You can now participate in Twitch channel predictions directly in StreamNook! See active predictions in a floating overlay, make your bets with channel points, and track the results.
- **Automated Drop Mining Improvements**: StreamNook now utilizes inventory polling and a new stream selection algorithm to ensure drops mining is highly optimized. Experience improved stability and speed for drops mining.
- **Completed Drops Display**: Completed drops are now displayed in the inventory tab.  

### 🐛 Bug Fixes
- Auto-selecting in home is fixed.

### ⚡ Performance
- The app switches more quickly and reliably to the best stream for drops mining.

## [5.0.4] - 2025-12-11
### ✨ Features
- **Badge Collection**: Introducing a new Twitch Global Badge collection system! Track your collected badges, earn ranks, and show off your achievements.
- **Drops Mining**: Enhanced the "Mine All" feature for Twitch Drops, prioritizing campaigns with existing progress and skipping already completed ones. Mining now starts with the drop closest to completion!
- **Profile**: 7TV users can now create accounts directly from the profile overlay. 
- **Video Player**: Added automatic closing of the subscription window after a successful subscription.

### 🐛 Bug Fixes
- Fixed an issue where the app window wouldn't focus after exiting Picture-in-Picture mode.
- Resolved an issue where the home screen tabs would reset after navigating from a category or search result.
- Fixed a bug where the Drops login window was not closed after successful authentication.
- Fixed DropsCenter logging and drop ID parsing

### ⚡ Performance
- Improved the Drops mining queue for faster and more efficient reward collection.

## [5.0.3] - 2025-12-11
### ✨ Features
- **Automatic Stream Recovery**: StreamNook can now automatically recover from common mining issues like offline streamers, stalled progress, and game category changes! Configure the new recovery settings to customize the behavior.
- **Improved Sidebar**: The sidebar's expand-on-hover mode is now smoother and prevents layout jumps.
- **Game Completion Status**: The Drops Center now shows when you've claimed all available drops for a game.

### 🐛 Bug Fixes
- Improved whisper error messages to be more specific about why a message failed to send.

## [5.0.2] - 2025-12-09
### ✨ Features
- **Smart Auto-Switch**: Improved auto-switching logic to prevent it from interrupting raid redirects. Now, StreamNook will wait a short period after a raid before switching channels.

### 🐛 Bug Fixes
- Resolved an issue where clicking a stream in the sidebar would unnecessarily toggle the home view.

## [5.0.1] - 2025-12-09
### ✨ Features
- **Drop Images**: Drops are now displayed with their images throughout the Drops Center, making it easier to identify the rewards you're earning.

### 🐛 Bug Fixes
- Fixed an issue where stopping drops mining would leave stale data in the UI.
- Improved game session tracking to ensure drops progress is correctly displayed when switching between games.

## [5.0.0] - 2025-12-09
### ✨ Features
- **Revamped Drops Center**: Experience a completely redesigned Drops Center with a sleek, user-friendly interface. Enjoy intuitive navigation, clear progress tracking, and comprehensive game information, making drops mining easier than ever.
- **Enhanced Drops Mining**: Improved drops mining system with more accurate progress tracking, real-time updates, and better channel selection for optimal rewards.
- **Improved Drop Claiming**: Resolved issues with automated drop claiming. The app now correctly identifies and claims earned drops, guaranteeing that you receive all your rewards.

### 🐛 Bug Fixes
- Resolved an issue where the stream would freeze when switching channels.
- The app will now automatically re-establish the stream connection if it is interupted by a momentary internet blip.
- Fixed problems with incorrect channel selection while mining.

### ⚡ Performance
- Improved the performance of loading emotes in chat, especially during peak hours.

## [4.9.6] - 2025-12-08
### ✨ Features
- **Notification Management**: You can now mark individual notifications as read, or mark all notifications as read within the notification center.
- **Improved Stream Controls**: The stream controls overlay (follow/subscribe) now appears on mouse movement and automatically hides after a short delay, even in fullscreen mode. This provides a cleaner viewing experience.

### 🐛 Bug Fixes
- **Whisper Import**: Improved whisper history import to correctly handle user ID resolution, ensuring imported conversations are properly linked and display correctly.

## [4.9.5] - 2025-12-08
### ✨ Features
- **Discord Rich Presence**: Improved the stream watching animation in Discord rich presence for a more engaging experience.

## [4.9.4] - 2025-12-08
### 🔧 Maintenance
- Updated the README with new screenshots and descriptions to better showcase StreamNook's features.

## [4.9.2] - 2025-12-08
### ✨ Features
- **Authentication**: Improved authentication process with a one-time data cleanup to ensure a fresh login experience.

### 🐛 Bug Fixes
- **Chat**: Improved chat auto-scrolling behavior to keep up with the latest messages even during resets.
- **Follow/Unfollow**: Added helpful error messages when following or unfollowing a channel fails. These messages suggest logging out and back in to re-authenticate.

## [4.9.1] - 2025-12-08
### ✨ Features
- **Whisper Import**: You can now import your entire Twitch whisper history directly into StreamNook! This feature runs automatically in the background after logging in and will allow you to access your private messages.
- **Custom Streamlink Path**: Added the ability to specify a custom path to your Streamlink installation. This resolves issues where the bundled Streamlink version fails to extract correctly and you can select from your drive a portable or manually installed version of the software.

### 🐛 Bug Fixes
- Fixed a conflict with Plyr's local storage interfering with configured app settings.

### ⚡ Performance
- Loading may take slightly longer with proxy enabled. A notification is now shown that provides an explanation and guidance on removing it.

### 🔧 Maintenance
- Login via the Twitch website is only required once to authorize the app. StreamNook will then attempt to re-authenticate silently in the background to avoid needing re-authorization.

## [4.9.0] - 2025-12-08
### ✨ Features
- **Automated Follow/Unfollow**: You can now follow and unfollow channels directly from StreamNook using a new automation feature. This uses your existing Twitch login and a hidden browser window to perform the action, so you don't have to leave the app!
- **In-App Drops Login**: The Drops login process now happens within the StreamNook app using a secure in-app browser window. This simplifies the login process and improves security.

### ⚡ Performance
- Optimized the process for checking if you're following a streamer to use a more efficient Twitch API endpoint.

## [4.8.3] - 2025-12-08
### 🐛 Bug Fixes
- **Sidebar**: Fixed a bug where the sidebar would sometimes incorrectly display the default profile image instead of the streamer's custom profile image. Also optimized sidebar rendering for improved performance.

### 🔧 Maintenance
- Removed some unused code and temporary files to improve build times and reduce the application's overall size.

## [4.8.2] - 2025-12-08
### ✨ Features
- **Raid Auto-Follow:** Automatically follow raids to the raided channel, ensuring you never miss the action (requires Twitch login).
- **Emoji Rendering**: Enjoy consistent emoji display across all platforms with new Apple-style emoji rendering in chat and stream titles.
- **In-App Notifications**: Notifications are now delivered directly within the StreamNook app, keeping you informed without relying on system notifications.
- **Jump to Live:** Added new video player setting to jump to the live edge of the stream on load.

### 🐛 Bug Fixes
- Moderators can now delete messages and apply timed-out or bans in chat, and these actions now reflected in the chat
- Toast (Pop-up) notifications now load emojis and are consistent with what's displayed in the in-app chat

## [4.8.1] - 2025-12-07
### ✨ Features
- **Browse Categories**: Improved browsing experience with infinite scroll for top games.
- **Chat Timestamps**: Added an option to display timestamps in chat, including optional seconds, for better message context.

### 🐛 Bug Fixes
- Resolved overflow issues with 7TV paint displays on user profiles.

### 🔧 Maintenance
- Improved user interface consistency by standardizing toggle switches in settings panels.
- Improved Changelog rendering and support for markdown

## [4.8.0] - 2025-12-07
### ✨ Features
- **Auto-Updating**: StreamNook now automatically checks for updates and installs them for you! You can enable or disable auto-updates in the settings.
- **Admin Dashboard**: Added a local analytics dashboard that admins can use to view usage statistics. Admins are detected via Twitch authentication.
- **Drops Mining in Chat**: You can now quickly start/stop mining drops directly from the chat window if the stream is playing a drops-enabled game.
- **Badge Quick Actions**: Badge details now include quick actions for drops campaigns, allowing you to easily view linked drops campaigns directly from a badge.
- **Persistent Video Dimensions**: The video player now preserves pixel dimensions as you adjust the chat size, ensuring that the displayed content is consistent.
- **Drops Search**: Added basic search to the Drops Overlay.

### 🐛 Bug Fixes
- Fixed stream auto-restart issue

## [4.7.9] - 2025-12-06
### ✨ Features
- Added support for animated 7TV emotes in chat, bringing more expressive communication to your viewing experience.
- Integrated the analytics dashboard directly into the StreamNook application. This eliminates the need for a separate resource folder and simplifies deployment.

## [4.7.8] - 2025-12-06
### ✨ Features
- Added an analytics dashboard to visualize usage data, providing insights into how StreamNook is used and helping guide future development. This dashboard is built with React/TypeScript and uses Supabase for data storage and access.  It is bundled during the release process.

## [4.7.7] - 2025-12-06
### ✨ Features
- Added Tauri Dialog plugin for native dialogs.

### 🔧 Maintenance
- Updated ACL manifests.

## [4.7.6] - 2025-12-05
### ✨ Features
- Updated README with screenshots and comprehensive feature list.

### 🐛 Bug Fixes
- Disabled "Auto Claim Channel Points" by default.

## [4.7.5] - 2025-12-05
### 🐛 Bug Fixes
- Fixed an issue where chat replies and optimistic message updates would fail when using JSON message format in addition to IRC string format.

## [4.7.4] - 2025-12-05
### ✨ Features
- Added analytics dashboard for admins (requires configuration of Supabase environment variables during build)
- Implemented basic Supabase analytics to track usage.
- Tracked number of messages sent in the chat.

### 🔧 Maintenance
- Updated build process to include Supabase environment variables.
- Added Supabase JavaScript library as a dependency.

## [4.7.3] - 2025-12-05
### ✨ Features
- Added dynamic layout configuration for chat messages, allowing for responsive text sizing.
- Implemented batch retrieval for cached items, improving performance when loading multiple resources.

### 🐛 Bug Fixes
- Fixed HTML entity decoding in badge metadata extraction, ensuring proper display of special characters.

## [4.7.2] - 2025-12-04
### ✨ Features
- Improved Changelog and Updates UI with formatted markdown support.
- Improved emote caching and preloading for faster loading.

## [4.7.1] - 2025-12-04
### ✨ Features
- Release notes are now displayed when updating.
- Improved emote caching and logging for better performance and debugging.
- Enhanced the 'Live' indicator in the video player with a pulsing effect.

### 🐛 Bug Fixes
- Improved Streamlink process killing and directory removal during updates to avoid errors.

### 🔧 Maintenance
- Clarified app data directory names for development and production environments.

## [4.7.0] - 2025-12-04
### ✨ Features
- Added file caching functionality for emotes and badges, improving performance and offline support.
- Chat messages now queue emotes for caching, enhancing loading times.
- Chat messages now use local URLs for badges, decreasing dependency on external requests.

### 🐛 Bug Fixes
- Fixed drops authentication issues, handling failures more gracefully and preventing campaign restarts.



## [4.6.3] - 2025-12-04
### ✨ Features
- Improved Updates Settings with component-specific changelogs.

### 🐛 Bug Fixes
- Fixed Streamlink update issues by forcing process termination.
- Removed redundant condition for showing session restore toast.


## [4.6.2] - 2025-12-04
### ✨ Features
- Added Picture-in-Picture (PIP) mode that auto-triggers when navigating to the home screen while a stream is playing.
- Improved error reporting, including stack traces and recent logs, to assist with debugging.

### 🐛 Bug Fixes
- Added error boundary around chat rendering to prevent the entire app from crashing due to chat parsing errors.

### 🔧 Maintenance
- Centralized handling of live stream notifications for better consistency and control.
- Removed unused CSS rules.

## [4.6.1] - 2025-12-04
### ✨ Features
- Added option to automatically download and install updates on startup.  This can be enabled in Settings > Updates.
- Added "Quick Update on Toast Click" option in Notification Settings. When enabled, clicking the update toast will immediately start the update process.

### 🐛 Bug Fixes
- Fixed missing icons within toasts.

## [4.6.0] - 2025-12-04
### ✨ Features
- Added a new home screen with support for browsing categories, searching channels, and picture-in-picture (PIP) mode.
- PIP mode allows you to continue watching the stream in a small window while browsing other content on the home screen.
### 🔧 Maintenance
- Removed the LiveOverlay and LiveStreamsOverlay components.

## [4.5.0] - 2025-12-04
### ✨ Features
- Implemented a new home screen with sections for followed and recommended streams.
- Added the ability to browse Twitch categories and view streams within those categories.
- Implemented a search functionality for finding Twitch channels.
- Added a picture-in-picture mode for watching streams while browsing the home screen.

## [4.4.0] - 2025-12-04
### ✨ Features
- Implemented a new home screen with stream browsing functionality, including followed streams, recommended streams, game categories, and search.
- The current stream can now be displayed in a mini-player on the home screen.
- Added a "Return to Stream" button to easily switch back to the full stream view.
- Improved Dynamic Island notifications for live streams.

## [4.3.4] - 2025-12-04
### ✨ Features
- Improved build process by using authenticated requests to the GitHub API to avoid rate limits during release.

### 🔧 Maintenance
- Removed the 'all' target from the Tauri build configuration, leaving the targets array empty.

## [4.3.3] - 2025-12-04
### Changes

- chore(release): bump version to 4.3.2
- chore(deps): update dependencies
- Merge branch 'dev'
- chore(deps)(deps): bump the rust-dependencies group across 1 directory with 3 updates (#20)
- chore(deps)(deps): bump cookie_store from 0.21.1 to 0.22.0 in /src-tauri (#10)
- chore(deps)(deps): bump warp from 0.3.7 to 0.4.2 in /src-tauri (#8)


## [4.3.2] - 2025-12-04
### 🐛 Bug Fixes
- Fixed an issue where the drops websocket would remain connected after deactivating manual mining.

### 🔧 Maintenance
- Updated dependencies.
- Standardized imports across services for better consistency.
- Fixed lifetime issues with warp 0.4.

## [4.3.1] - 2025-12-04
### ✨ Features
- Added React error boundaries for improved app stability.
- Improved badge date parsing to handle more formats.

### 🐛 Bug Fixes
- Prevented badge polling errors when Tauri invoke is not available.

### 🔧 Maintenance
- Added backdrop blur to sidebar.
- Added logging improvements for React error boundary errors.

## [4.3.0] - 2025-12-04
### ✨ Features
- Added new sidebar for stream navigation and discovery.
- Added interface settings tab with options for sidebar behavior.

### 🐛 Bug Fixes
- Improved reliability of ttvlol plugin detection for streaming.

## [4.2.1] - 2025-12-02
### ✨ Features
- Added a GitHub Actions workflow to automatically merge Dependabot pull requests for patch and minor version updates. Major version updates will require manual review.

## [4.2.0] - 2025-12-02
### ✨ Features
- Added support for desktop notifications.
- Added window focus and minimize checks to capabilities.

## [4.1.14] - 2025-12-02
### ✨ Features
- Improved Streamlink integration by checking for the ttvlol plugin's installation before using it.
- Added drops authentication check to the setup wizard to streamline initial configuration.

## [4.1.13] - 2025-12-02
### 🐛 Bug Fixes
- Prevent duplicate proxy arguments being passed to Streamlink when using the ttvlol plugin, resolving a potential error.

## [4.1.12] - 2025-12-02
### 🐛 Bug Fixes
- Fixed Streamlink executable path to use `streamlinkw.exe` for better GUI application compatibility (avoids unnecessary terminal window).

## [4.1.11] - 2025-12-02
### ✨ Features
- Added streamlink diagnostics to help debug stream starting issues.
- Added the ability to show points balance in the dynamic island.
- Enhanced streamlink error reporting to Discord for better troubleshooting.

### 🐛 Bug Fixes
- Fixed: Streamlink now uses `streamlink.exe` instead of `streamlinkw.exe` for better CLI compatibility.

## [4.1.10] - 2025-12-01
### ✨ Features
- Improved badge metadata parsing to handle various date formats.
- Enhanced logging for new badge metadata.
- Associated channel logins with channel IDs in channel points events for better identification.
- Improved badge availability date display in the UI, including support for abbreviated date ranges.

### 🐛 Bug Fixes
- Included the path, URL, and quality in Streamlink error messages for easier debugging.

## [4.1.9] - 2025-11-30
### 🐛 Bug Fixes
- Improved streamlink path resolution during development, searching in CWD and parent directory.

## [4.1.8] - 2025-11-30
### 🔧 Maintenance
- Standardized application bundle naming across build workflows and update checks to `StreamNook.7z`.
- Improved reliability of update checks by ensuring consistent bundle name matching.

## [4.1.7] - 2025-11-30
### 🔧 Maintenance
- Use the bundled Streamlink path within the application, ensuring consistent functionality regardless of installation location.

## [4.1.6] - 2025-11-30
### ✨ Features
- Replaced external 7z dependency with native `sevenz-rust` library for component bundle extraction.
- Improved update script to wait for app closure and ensure proper restart.

## [4.1.5] - 2025-11-30
### ✨ Features
- Added support for portable mode, bundling Streamlink within the application directory.
- Enhanced Dynamic Island with new notification settings and improved functionality, including update notifications, drops, and channel points.

## [4.1.4] - 2025-11-30
### ✨ Features
- Implemented device code flow for Twitch Drops login during setup.

## [4.1.3] - 2025-11-30
### 🐛 Bug Fixes
- Fixed an issue where Streamlink would not validate the user path correctly.

## [4.1.2] - 2025-11-30
### ✨ Features
- Improved bundle update check and component management.
- Moved 'Updates' tab in settings for better user experience.

## [4.1.1] - 2025-11-30
### 🔧 Maintenance
- Updated comment regarding clippy warnings suppression.

## [4.1.0] - 2025-11-30
### ✨ Features
- Implemented bundled distribution of Streamlink and TTV LOL plugin for simplified installation and updates.

## [4.0.2] - 2025-11-30
### ✨ Features
- Added first-time setup wizard to guide new users through the initial configuration.
- Improved WebSocket event handling for mining service, enhancing drop progress updates.

### 🐛 Bug Fixes
- Improved chat scroll and pause behavior, addressing issues with automatic pausing and scrolling to the bottom.



## [4.0.1] - 2025-11-29
### ✨ Features
- Redesigned the AboutWidget with tech stack icons and shoutouts to contributors and libraries.
- Enhanced the Dynamic Island with an updated notification badge and sound indicators.
- Implemented maximize/restore functionality in the TitleBar and updated relevant icons.

## [4.0.0] - 2025-11-29
### ✨ Features
- Added whisper functionality with message sending and receiving.
- Implemented whisper history retrieval using Twitch's undocumented GraphQL API.
- Added a whisper history scraper script for exporting data from the Twitch website.
- Added real-time whisper notifications using EventSub Websockets.

## [3.4.17] - 2025-11-29
### ✨ Features
- Added joke messages when clicking test notifications. Test notifications now have an `is_test` flag.

## [3.4.16] - 2025-11-29
### ✨ Features
- Added a "Test Notification" button in settings to trigger a sample notification.
- Implemented proper stream rotation for channel points farming, cycling through all followed live streams.
- Implemented notification sounds with customizable options.

### 🐛 Bug Fixes
- Stream watching loop now properly handles cases where no streams are live.

### 🔧 Maintenance
- Settings UI now has updates settings at the right spot.

## [3.4.15] - 2025-11-29
### ✨ Features
- Implemented drops websocket error reporting to the frontend.
- Improved offline detection logic in the video player, considering non-fatal errors and fragment stalling.
- Added activity tracking and user context (Twitch username, current stream) to error reports for better debugging.

### 🐛 Bug Fixes
- Fixed a bug where offline stream detection was too aggressive causing early auto-switch.

### 🔧 Maintenance
- Added user activity tracking to improve error context.
- Saved user and stream context to local storage for error reporting purposes.

## [3.4.14] - 2025-11-29
### ✨ Features
- Added a toggle to enable or disable anonymous error reporting to help improve StreamNook. The toggle is located in the settings under the 'Support' section.

## [3.4.13] - 2025-11-29
### ✨ Features
- Added a "Support" settings tab with a log viewer and bug report export functionality.
- Implemented automatic error reporting for easier debugging.

### 🐛 Bug Fixes
- Improved stream offline detection logic in the video player.

## [3.4.12] - 2025-11-28
### ✨ Features
- Added support for displaying Bits cheers in chat with animated icons and tier colors.
- Improved shared chat functionality, including fetching channel names and profile images.
- Integrated IVR user data (moderator, VIP, subscriber status) into the User Profile Card.

### 🐛 Bug Fixes
- Fixed issues with chat scrolling when historical messages are loaded, ensuring proper scrolling to the bottom.

### 🔧 Maintenance
- Updated badge display logic in User Profile Card to prioritize display badges and filter broadcaster badges when necessary.

## [3.4.12] - 2025-11-28
### ✨ Features
- Added support for displaying Bits cheers in chat with animated icons and tier colors.
- Improved shared chat functionality, including fetching channel names and profile images.
- Integrated IVR user data (moderator, VIP, subscriber status) into the User Profile Card.

### 🐛 Bug Fixes
- Fixed issues with chat scrolling when historical messages are loaded, ensuring proper scrolling to the bottom.

### 🔧 Maintenance
- Updated badge display logic in User Profile Card to prioritize display badges and filter broadcaster badges when necessary.

## [3.4.11] - 2025-11-28
### ✨ Features
- Added support for native emojis in chat messages.
- Implemented an emoji picker for easy emoji selection in the chat input.
- Added window `isMaximized` permission to prevent potential UI issues with glass morphism.

## [3.4.10] - 2025-11-28
### ✨ Features
- Implemented aspect ratio lock for the video player, with adjustments during window resize.
- Enhanced subscription and donation messages in chat with clickable usernames and cosmetics.

### 🔧 Maintenance
- Removed the unused `dark_mode` setting from chat design settings.

## [3.4.9] - 2025-11-28
### ✨ Features
- Penrose triangle colors now dynamically adjust based on the selected theme.
- Added new themes: Antidepressant's Tactical, prince0fdubai's OLED v2, GitHub Dark, Solarized Sand, Material Theme, Ayu Dark, Night Owl, Synthwave '84.

## [3.4.8] - 2025-11-28
### ✨ Features
- Implemented a new theming system, allowing users to customize the application's appearance. Users can now select from a variety of themes in the settings menu.

### 🔧 Maintenance
- Refactored the UI to support dynamic theme switching and persistent theme settings.

## [3.4.7] - 2025-11-28
### 🔧 Maintenance
- Added Dependabot configuration for automated dependency updates.
- Added `update-deps` script for interactive and automatic dependency updates using npm and cargo.

## [3.4.6] - 2025-11-28
### 🔧 Maintenance
- Updated the build date comment in the README.

## [3.4.5] - 2025-11-28
### ✨ Features
- Added React DevTools connection script for development.
- Added `react-devtools` and `concurrently` dependencies for debugging.
- Added `dev:debug` script to run React DevTools and Tauri concurrently.
- Set initial window size to 1600x1000, minimum size to 800x600, and centered the window.
- Skip aspect ratio adjustment when window is maximized.

## [3.4.4] - 2025-11-28
### Fixed
- Fixed stream caching issues by adding a timestamp to the stream URL.
- Added `client_secret` when refreshing Twitch token to keep the service running.

## [3.4.3] - 2025-11-27
### Fixed
- Fixes a rendering issue with chat messages overlapping one another.
- Resolved stability issues due to frequent re-renders. Improves ChatWidget performance.

### Changed
- Increased thumbnail resolution in LiveOverlay and LiveStreamsOverlay for better quality.
- Improved video player buffering for smoother playback.

### Added
- Added scroll anchoring to maintain stable scroll position when messages are removed from the chat.
- Badge click will now open a badge details overlay.

## [3.4.2] - 2025-11-27
### Added
- Implemented an auto-switch feature that automatically switches to another stream when the current stream goes offline.
  - Settings allow enabling/disabling auto-switch.
  - Option to switch to a stream in the same category or a followed stream.
  - Toggle notification for auto-switch events.
- Added commands `check_stream_online` and `get_streams_by_game_name` to check stream status and retrieve streams by game name.

### Improved
- Improved video player error handling for fatal network errors, now triggering auto-switch when appropriate.

## [3.4.1] - 2025-11-27
### Added
- Implemented stream switching with API refresh: If the current stream fails repeatedly, StreamNook will attempt to find a new one by refreshing the eligible channels from the API. This significantly improves reliability.
- Added toast notifications for stream switches and for when mining is stopped due to no available streams.

### Improved
- Overall channel switching logic and error handling for more reliable mining.

## [3.4.0] - 2025-11-27
### Added
- Manual channel selection for drops mining. Users can now choose a specific Twitch channel to watch for drops, providing more control over the mining process.

## [3.3.3] - 2025-11-27
### Added
- Implemented token refresh mechanism to automatically renew access tokens when they expire or are about to expire, ensuring uninterrupted access to Twitch services.
- Introduced a token health check API (`verify_token_health`) that validates the current token and proactively refreshes it on app startup, enhancing reliability.
- Added new Tauri commands (`verify_token_health`, `force_refresh_token`) for manual token management.
- Enhanced cookie and file storage to include refresh tokens and expiration times for persistence across sessions.
- Add token validation for drops authentication

### Changed
- Refactored token loading logic to prioritize file storage, fallback to cookies, and then keyring for redundancy.



## [3.3.2] - 2025-11-27
## v3.3.1

### Fixed
- **Discord Presence Reliability**: Implemented reconnection logic to handle Discord IPC socket failures, ensuring presence updates remain functional even if the initial connection is lost.
- **HLS.js Memory Leaks**: Fixed memory leaks by completely destroying the existing HLS instance when switching streams, ensuring a fresh instance is created each time.

### Improved
- **Discord Error Handling**: Downgraded Discord presence errors to warnings, preventing UI errors when Discord is not running. Stream Nook will not block if it cannot connect.

## [3.3.1] - 2025-11-27
## [3.3.0]

### Added
- Cache and forward user badges from IRC USERSTATE messages for accurate badge representation.
- Replace optimistic chat messages with server messages to ensure badge accuracy.
- Health checks to detect and recover from stale connections.

### Fixed
- Improved reconnection logic to handle abnormal closures and channel changes gracefully.
- Prevented duplicate message display.

## [3.3.0] - 2025-11-27
### Added
- Enhanced `BadgeDetailOverlay` with highlighted dates in the More Info section.
- Improved badge loading and metadata fetching process in `BadgesOverlay`.
  - Pre-load metadata from cache for instant sorting.
  - Background fetching of missing metadata.
  - Updated UI to reflect fetching status.

### Changed
- Improved logging clarity in `update-universal-cache.js`.

## [3.2.3] - 2025-11-27
### Added
- Ability to dynamically switch stream quality within the video player.
- Exposed additional Streamlink parameters within settings.

### Fixed
- More resilient drops URL parsing.
- Correctly chain opener function calls.
- More resilient date parsing.

## [3.2.2] - 2025-11-27
### Fixed
- Suppressed unused variable warning in HLS manifest parsed event in VideoPlayer component.

## [3.2.1] - 2025-11-27
## v3.2.0

### Changed
- Updated version to 3.2.0.

## [3.2.0] - 2025-11-27
### Added
- Implemented advanced Streamlink settings to allow configuring options such as low-latency mode, HLS live edge, and proxy settings.
- Added quality selection to the Plyr video player using HLS.js.

### Fixed
- Corrected Discord rich presence logo path.
- Optimized HLS.js settings for more stable playback and buffer management.

## [3.1.2] - 2025-11-26
### Fixed
- Improved live stream latency with dynamic playback adjustment based on buffer growth rate.

## [3.1.1] - 2025-11-26
## v3.1.0

### Changed
- Updated application version to 3.1.0.

## [3.1.0] - 2025-11-26
### Fixed
- Fixed unexpected video pausing during initial load and throughout playback.
- Fixed ChatWidget pausing when scrolling and added chat buffer.

### Changed
- Optimized chat history and set a fixed limit of 200 messages for improved performance and stability.

### Added
- Added Twitch user cosmetics and clickable usernames for system messages in chat.

## [3.0.2] - 2025-11-26
### Fixed
- Prevent `fetchBTTVEmotes` from crashing on undefined `channelName` by removing the paramater. This function is now called without any `channelName` argument.

## [3.0.1] - 2025-11-26
### Changes

- chore: update Cargo.lock after build


## [3.0.0] - 2025-11-26
### Added
- Replaced video.js with Plyr for a better streaming experience. This includes HLS support through HLS.js.
- Implemented theater mode, which automatically resizes the window to a 16:9 aspect ratio.
- Added a check for new versions on startup, displaying the changelog if the app has been updated.

### Changed
- Updated dependencies to their latest versions for improved stability and security.

### Removed
- Removed video.js and related dependencies as they are no longer needed.

## [2.9.2] - 2025-11-26
### Chore
- Added `openid` scope to Twitch authentication. This is needed for user identification.

## [2.9.1] - 2025-11-26
### Changed
- Refactor release notes fetching to parse `CHANGELOG.md` instead of using the GitHub API.
- Bumped version to 2.9.0

## [2.9.0] - 2025-11-26
### Fixed
- Improved robustness of app update installation by adding retries to file deletion in the batch script.
- Added cleanup logic to remove leftover update files (StreamNook_new.exe, update_streamnook.bat) on application startup, addressing potential issues from interrupted update processes.

## [2.8.0] - 2025-11-26
### Added
- Implemented automatic changelog display after app updates.  The changelog will now show on first launch after an update to inform users of new features and changes.

### Changed
- The app now checks for version changes on startup and displays the changelog if a new version is detected. This is powered by calls to GitHub releases API.



## [2.7.0] - 2025-11-26
### Changed
- Upgraded StreamNook version to 2.6.0

## [2.6.0] - 2025-11-26
## v2.5.0

### Changed
- Bumped application version to 2.5.0.

## [2.5.0] - 2025-11-26
## [2.4.0]

- Bump version to 2.4.0.

## [2.4.0] - 2025-11-26
### Changed
- Bumped version to 2.3.0

## [2.3.0] - 2025-11-26
## v2.2.0

### Changed
- Bumped version to 2.2.0 to include a new desktop release build.

### Added
- Desktop release build `StreamNook-Desktop-Setup.7z` and associated checksums.

# Changelog

## [2.2.0] - 2025-11-26
### Added
- Automated workflow to periodically update the universal badge cache.
- Ability to fetch and cache Twitch badge metadata from BadgeBase.
- Commands to force refresh global badges, get cache age, and check for new badges.

### Fixed
- Issue in calculating window aspect ratio when resizing with different chat placements.







