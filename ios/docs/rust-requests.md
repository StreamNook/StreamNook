# Rust requests from the iPhone screens

What the Swift screens need from the core that it does not expose yet. Swift only presents: until each item lands, the screen shows what the core gives (or a stand-in named below) and never ports the TypeScript logic. Each entry names the TypeScript it replaces and the matching item in [`rust-moves.md`](rust-moves.md) when there is one.

Path shorthand follows the parity contract: `m:` is `src/mobile/`, `s:` is `src/` outside it, `d:` is desktop code under `src/`, `r:` is `src-tauri/src/`.

## Shell and design (DESIGN lane)

### 1. Resolved theme palette and theme list

- **Need:** one call answering the resolved palette for the current settings (the theme's colours with custom themes and the OLED accent applied, plus `light_content` for the status bar), a theme list for the wizard and the Theme panel (id, name, category, the five palette dots), and an event when either changes.
- **Replaces:** d:themes/index.ts:L1515-L1535 (theme id to palette, OLED accent), d:themes/index.ts:L1510-L1513 and m:systemBars.ts:L34-L59 (luminance rule), s:boot/useThemeBoot.ts:L22-L46.
- **rust-moves:** Top 15 item 15, "Theme resolution" (P1).
- **Until then:** Swift draws the default Frosted Glass palette (`Design/Palette.swift`) whatever `theme` says; the wizard's "Pick your look" page and the Theme panel's cards are left out.

### 2. Glassiness numbers

- **Need:** the effective glass strength (`glass_transparency` / 100, capped at 0.45 when `glass_blur` is off) and the blended `surface`, `surface-hover` and `surface-active` colours, as fields on the resolved palette.
- **Replaces:** d:themes/index.ts:L1572-L1605, d:themes/index.ts:L1703-L1738.
- **rust-moves:** the glassiness line under m:systemBars.ts and s:themes/index.ts (P1).
- **Until then:** Swift reads `glass_transparency` as 0 to 100 and lays the theme's opaque tertiary floor under its own panels at (1 - glassiness), which is the CSS composite, not the TypeScript blend. The 0.45 cap with blur off is not applied; blur off only drops the frost.

### 3. Interface font table

- **Need:** the font ids, the default (`satoshi`) and the chat body weights (300, 200 for Inter) from Rust.
- **Replaces:** d:themes/index.ts:L1740-L1822.
- **Until then:** the app bundles Satoshi (converted from `src/assets/fonts/Satoshi-Variable.woff2`) and maps `system` to SF Pro; every other id draws in Satoshi.

### 4. Boot answer and finishing setup

- **Need:** `needs_setup` in one boot answer (with the localStorage backstop `streamnook-setup-complete` folded into a backend-owned flag and the revert root-caused), and a `complete_setup` command that writes `setup_complete` and baselines announcements.
- **Replaces:** m:MobileApp.tsx:L85-L92, d:App.tsx:L1199-L1226, d:components/SetupWizard.tsx:L485-L523.
- **rust-moves:** Top 15 item 1 and the first-run gate line (P1).
- **Until then:** the wizard shows when `load_settings` answers `setup_complete: false`, and "Start watching" sends `patch_settings {setup_complete: true}`.

### 5. Sign-in reason for the signed-out screens

- **Need:** why StreamNook signed the person out (for "Twitch needs you to sign in again for new features."), on the account roster or the boot answer.
- **Replaces:** s:utils/twitchSignIn.ts:L12-L17, m:MobileOnboarding.tsx:L28-L32.
- **rust-moves:** Top 15 item 2.
- **Until then:** the landing always says "Watch Twitch with your emotes, badges, and cosmetics everywhere."

## Following and Browse (frames built by the DESIGN lane)

### 6. Browsing signed out

- **Need:** anonymous answers for the category directory (`get_top_games_paginated`), a category's streams (`get_streams_by_game`), and channel and category search (`search_channels`, `search_categories`), the way the recommended section already reads Twitch's web client anonymously. Today all four go to Helix with no token when signed out and answer empty lists (r:services/twitch_service.rs:L1932, L2148 with its `None => Ok(Vec::new())` at L2347, L3164).
- **Replaces:** nothing in TypeScript: Android never shows these screens signed out (the whole shell sits behind the sign-in screen, m:MobileApp.tsx:L136-L141).
- **Until then:** signed out, Browse shows the recommended list, and Categories and search show "Sign in with Twitch" notes instead of empty results. Only needed if Brandon keeps the tabs browsable while signed out (see the open question in the design report).

### 7. One "refresh following" entry with the freshness stamp

- **Need:** one call that refreshes the followed-live list, hype trains and Shared Viewership, applies the shared 60 s freshness stamp (first load, pull and resume), merges the live Kick follows (the `following` section), and returns finished preview URLs with the `sn=` generation.
- **Replaces:** m:followRefresh.ts:L21-L73, m:followingMerge.ts:L1-L31, m:ui/MobileStreamCard.tsx:L24-L31.
- **rust-moves:** Top 15 item 3; the m:followRefresh.ts lines; the thumbnail line under m:ui/MobileStreamCard.tsx.
- **Until then:** Following calls `refresh_home_section` for `followed_live` then `hype_trains` and reads `get_home_snapshot` (Twitch rows only, no Kick merge); `SNKit.TwitchImage` fills the `{width}x{height}` template and appends a stamp that changes on a pull. Delete `TwitchImage` when Rust returns the final URLs.

### 8. Per-row card flags

- **Need:** the drops-earnable flag (Twitch only, `campaignEarnableOn`) and the Shared Viewership or Shared Chat group with its accessibility label, stamped on each stream row Rust returns.
- **Replaces:** m:ui/MobileStreamCard.tsx:L74-L93, m:dropsEligibility.ts, m:ui/MobileStreamCard.tsx:L83-L85, s:utils/sharedViewers.ts:L14-L32.
- **rust-moves:** the drops-earnable and group lookup lines under m:ui/MobileStreamCard.tsx.
- **Until then:** cards draw no DROPS chip and no Together tag. Hype trains and watch streaks are read from the snapshot by Twitch user id, which needs nothing new.

## Rewards (frame only)

### 9. Badge wall model

- **Need:** the finished badge wall (global catalogue merged by title with owned badges marked and earn status stamped).
- **Replaces:** m:rewards/badgeGalleryStore.ts:L105-L152, s:services/badgeStanding.ts:L113-L135.
- **rust-moves:** "the badge gallery store" in the list after the top 15.
- **Until then:** the Badges segment is a frame with a placeholder line; the Drops segment shows the connect card from `is_drops_authenticated` with its button not yet wired (the drops sign-in sheet belongs to the accounts lane).

## Answers from the core lane (branch `ios/core2`)

What the core now offers for the items above and for the plan's phase 2, and the contract Swift codes against. Command arguments and answers are JSON, as `invoke` sends and returns them.

### #4 and #5: boot answer, finishing setup, sign-in reason

- `get_boot_state` (no arguments) answers `{needs_setup, signed_in, sign_in_reason, sign_in_text}`. `needs_setup` is `!setup_complete`; `signed_in` means a Twitch sign-in is stored; `sign_in_reason` is `"new_permissions"` or `null`; `sign_in_text` is `{sentence, short}` or `null` (sentence for the signed-out landing, short for a row that already says Twitch). The roster still carries the reason too (`account-roster-changed`).
- `complete_setup` (no arguments) writes `setup_complete: true` to disk at once and answers `{baseline_announcements}`: `true` the first time setup finishes on this install, so the announcements banner, once Swift has one, marks the current backlog seen instead of showing it. Replaces `patch_settings {setup_complete: true}`.
- The mobile `setup_complete` revert is already handled in `run()` (settings re-read once the data dir resolves); there is no localStorage backstop on iOS.

### #6: browsing signed out

- `get_top_games_paginated`, `get_streams_by_game` (and the by-id and by-name variants), `search_channels`, `search_categories` and the Discover tail after the recommendations page answer from Twitch's public GQL when no Twitch token is stored, or when Helix refuses one. Row shapes match Helix (box art and previews keep the `{width}x{height}` template).
- Cursors stay opaque. A signed-out cursor starts `gql:` and keeps paging on the public side even if a sign-in lands mid-scroll; just pass back what came.
- The reads use the Android client id with no account, not the web client id: Twitch fails the integrity check on anonymous web-client reads from the second page on (checked for categories and a category's streams), and the Android id is exempt. No Drops token is involved.

### Host requests (`HostSink.host_request(kind, payload)`)

| Kind | Payload | What Swift does |
| --- | --- | --- |
| `open_url` | `{url}` (http or https only) | Open the link outside the app (`UIApplication.open`). |
| `copy_text` | `{text}` | Put the text on `UIPasteboard.general`. |
| `share` | `{path, mime, name}` | Present a share sheet for the file at `path` (Save to Files is one of its choices), offered as `name`. |
| `open_sheet` | `{url, title, stop_at: [prefix], cookie_origins: [origin]}` | Show (or re-navigate) the in-app web sheet at `url`, persistent default store. A navigation whose URL starts with a `stop_at` prefix is cancelled and sent to `sheet_redirected`; the sheet stays up. While it is up, push the cookies of each `cookie_origins` site through `set_web_cookies` as they change. |
| `close_sheet` | `{}` | Take the sheet down (do not call `sheet_closed` for this). |
| `clear_cookies` | `{origins: [origin]}` | Delete every cookie of those sites from the sheet's store, then push the new snapshots. |
| `exit`, `restart` | as before | Unchanged. |

New commands for the sheet (iOS only, allowlisted in `mobile-commands.toml`):

- `sheet_redirected {url}`: the sheet stopped at a `stop_at` URL.
- `sheet_closed` (no arguments): the person dismissed the sheet.

### Kick, Subscribe, chat identity badges, log export

- `kick_account_sync {interactive: true}` runs the Android flow on iOS: `open_sheet` at `https://kick.com/login`, then the consent page in the same sheet; the core polls the `https://kick.com` cookies (`session_token`) and takes the consent redirect (`http://localhost:3000/callback?...`) from `sheet_redirected`, then sends `close_sheet`. `interactive: false` only reads the pushed `kick.com` cookies, so `WebSessionSite` should gain Kick (origin `https://kick.com`, session cookie `session_token`) to keep a silent import working after launch.
- `kick_disconnect` sends `clear_cookies {origins: ["https://kick.com", "https://id.kick.com"]}`.
- `open_subscribe_window {channelLogin, title?, provider?}` sends `open_sheet` at the platform's checkout and answers `subscribe-<login>`; the person closes the sheet.
- `fetch_chat_identity_badges_gql {username}` and `update_chat_identity_gql {badgeId, badgeVersion}` are registered on iOS (GQL with the Drops token; `NEEDS_DROPS_AUTH` when it is missing).
- `export_logs {to: "share" | "downloads"}` zips the logs into the cache dir and sends `share`; answers `"shared"`, or `"empty"` when there is nothing to send.

### Start inputs, lifecycle and events

- `start(..., version, vaultKey, codecs)`: `version` is now the app's version everywhere (package info, `get_client_identity`, `get_app_version`, version reports, log headers, user agents). `vaultKey` (32 bytes) is the token vault's key; files an older build sealed under the sandbox key file move to it on the first vault pass and the key file is then deleted. `codecs` names the hardware decoders (`h264`, `hevc`/`h265`/`hvc1`, `av1`/`av01`): AV1 and HEVC tiers the phone cannot decode are dropped from every master, so they never reach the quality menu or `best`; with Enhanced Broadcasting on, H.264 still leads and HEVC then AV1 serve only the tiers Twitch offers in nothing else.
- `lifecycle("memory_warning")` drops the in-memory caches that refill on their own (emote sets, badges, profiles, 7TV cosmetics and pictures, BTTV Pro, emoji).
- `lifecycle("foreground")` re-binds the relay listener on its own port if a loopback connect finds it dead (the player URL keeps working), brings back the chat bridge if its task ended, and emits `app-foregrounded {relay_rebound, chat_port, chat_alive}`. Reconnect the chat socket to `chat_port`. A bridge whose task still runs on a dead socket (`chat_alive: false`) cannot be forced back without changing `irc_service.rs` (chat lane).
- Audio only: `start_stream {quality: "audio_only"}` answers a URL ending `audio.m3u8?...`, a master that declares `CODECS="mp4a.40.2"` over `plain.m3u8`; play it as returned.
- Ad-free playback now runs on iOS (`video_player.ad_bypass_enabled`, default on). A mid-stream pivot emits `ad-pivot {url, channel}`: reload the player on `url`.
- Caches live under the cache dir Swift passes (`Library/Caches/StreamNook`); settings, tokens and logs stay in the data dir. The startup log names both.
- Every event name the core emits is listed in `src-tauri/tests/event_names.txt` (kept exact by `tests/event_registry.rs`).

## Chat view and composer (lane `ios/chat`)

Done in this lane, for reference: `r:services/chat_row_view.rs` stamps `metadata.view` (readable name colour, Twitch and 7TV badge art, the worn 7TV paint, the grouped body with mentions, link labels, overlays and modifier effects, emote sizes, the quoted parent) on every row while a native shell asks for it, posts `ROW_COSMETICS` when a chatter's cosmetics arrive late, and patches views in `HISTORY_SEGMENTS`. `r:services/command_catalog.rs` serves `command_catalog`. 7TV emotes carry their height.

### Rows

- [ ] **Third-party and StreamNook badges.** FFZ, Chatterino, Homies and the other providers' badges and the StreamNook member badge are resolved on the page (s:services/badgeService.ts:L216, s:stores/chatUserStore.ts:L816-L930, d:components/ChatMessage.tsx:L937-L938, d:components/ChatMessage.tsx:L1138-L1163). The view carries Twitch and 7TV badges only. Rust should add the rest to `view.badges` (late ones through `ROW_COSMETICS`), filtered by `show_third_party_badges` and `hidden_badge_providers`.
- [ ] **Viewer overrides.** The per-user nickname and colour overrides are applied on the page (d:components/ChatMessage.tsx:L1056-L1060, d:components/ChatMessage.tsx:L1086-L1092). The view should apply them to `shown_name` and `name_color`.
- [ ] **Mention colours and paints for chatters not seen yet.** The page looks unknown names up with `get_user_by_login` and fetches their paint (d:components/ChatMessage.tsx:L423-L534). The view's mention span has a colour only for chatters seen in a room this app shows, and no paint.
- [ ] **Emote sizes for BetterTTV and FFZ.** Both APIs send `width`/`height` for many emotes, which `emote_service` drops (r:services/emote_service.rs parse sites; the page measures wide ones in d:components/ChatMessage.tsx:L210-L238). Swift lays an unsized emote out square until its image loads and remembers the shape for later rows. Store height for both, and width without changing the desktop picker's width groups.
- [ ] **Twitch channel badges on join.** The page calls `fetch_channel_badges` for the room (s:services/twitchBadges.ts:L52-L110); the view's badge index fills from that call. Rust should fetch a joined room's channel badges and the global set itself; meanwhile the iPhone calls `get_cached_global_badges` and `fetch_channel_badges` when a room opens, and rows that arrive before the answer keep no Twitch badge art.
- [ ] **Sub notice name spans.** Gifter and recipient names inside `system-msg` are tappable and painted on the page (d:components/ChatMessage.tsx:L2834-L2931). The iPhone draws the sentence plain.
- [ ] **Event wording and hidden events.** `chat_events.event_templates` and `hidden_provider_events` run on the page (d:components/ChatMessage.tsx:L2211-L2231, s:utils/chatEvents.ts:L1-L60). Not applied on the iPhone.
- [ ] **Shared-chat source avatar.** Fetched on the page (d:components/ChatMessage.tsx:L2332-L2374). Not drawn on the iPhone.
- [ ] **Personal emote repaints.** `PERSONAL_EMOTES` names message ids and the page calls `apply_personal_emotes` per row (s:stores/chatConnectionStore.ts:L1283-L1313). The frame should carry the re-stamped views, as `HISTORY_SEGMENTS` now does.
- [ ] **Link previews.** Extraction, trust and fetching live in s:services/linkPreviewService.ts and d:components/chat/LinkPreviewCard.tsx. Not built on the iPhone.

### The room's buffer

- [ ] **Trim with event rescue and paused overflow.** s:stores/chatBufferTrim.ts:L1-L148 keeps the newest rows plus rescued event rows and decays the paused overflow. `SNKit/ChatRoomBuffer` keeps a plain cap with the 150-row paused allowance and the 5-row decay. Rust should own the room buffer and send changes.
- [ ] **Moderation marks.** CLEARCHAT and CLEARMSG are applied to rows on the page (s:stores/chatConnectionStore.ts:L2366-L2476), including the optional mod system rows (`moderation.show_mod_messages`). `ChatRoomBuffer` marks rows by user id and message id; no system rows.
- [ ] **Join snapshot.** The page holds the first live rows until history lands, at most 1500 ms (s:stores/chatConnectionStore.ts:L1107-L1149). The iPhone shows live rows at once and prepends `load_channel_history` when it answers.
- [ ] **Repeat folding and gift bombs.** s:stores/chatConnectionStore.ts:L2836-L2953. Not on the iPhone.

### Composer

- [ ] **One send command.** rust-moves.md lists `composer_send { room_key, text, reply_to }` [P1]: slash dispatch (s:utils/commandHandler.ts:L77-L1200, s:utils/extendedCommands.ts), the own row (s:stores/chatConnectionStore.ts:L3619-L3672) and the outcome. Meanwhile the iPhone sends with `send_chat_message` (slash text goes to Twitch as typed, so app-run commands like `/timer` do nothing), builds the own row with `get_user_info` plus `build_own_chat_message`, and appends the invisible duplicate tag itself when `chat_input.bypass_duplicate` is on (d:components/ChatWidget.tsx:L3504-L3521).
- [ ] **Chatters in the suggestion answer.** Recent chatters are matched on the page (s:utils/chatInputWord.ts:L95-L130, s:stores/chatUserStore.ts:L945-L973). The iPhone matches the room buffer's recent authors. `match_emote_tokens` should return them.
- [ ] **Chat gating.** m:chat/chatGating.ts:L1-L87 and m:chat/useFollowStatus.ts decide who may type. The iPhone composer never blocks; Twitch's refusal arrives as the send's drop reason.
- [ ] **Picker rows.** Twitch groups by owner and third-party groups by width are built on the page (s:components/chat/EmotePickerPanel.tsx:L575-L647). The iPhone groups by the `group` field `match_emote_tokens` sends when browsing; rows carry no zero-width or modifier flag for the cell rings, and no favorite flag.
- [ ] **Desktop and Android on the command catalog.** `command_catalog` mirrors `COMMAND_DEFINITIONS` (s:utils/chatCommands.ts:L22-L198) with a drift test; the desktop CommandMenu and the phone command sheet still read the TypeScript table and should switch.
