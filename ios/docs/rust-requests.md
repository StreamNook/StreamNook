# Rust requests from the iPhone lanes

Things an iPhone screen needs that the core does not answer yet. Each entry names the TypeScript the desktop and Android apps run today, what the iPhone does meanwhile, and what Rust should ship so Swift only presents. Path shorthand as in `parity/README.md` (`s:` is `src/`, `d:` desktop components, `r:` `src-tauri/src/`).

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
