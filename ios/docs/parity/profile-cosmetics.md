# Parity contract: Profiles, user cards and cosmetics

Read from StreamNook dev 577f3c02 + working tree of 2026-10-12 (the Android phone shell in `src/mobile`, the shared stores and services in `src`, and the Rust core). Every line is a behavior the iPhone app must match.

Path shorthand used in the citations:

- `m:` is `src/mobile/` (the Android phone shell)
- `s:` is `src/` outside `src/mobile/` (shared stores, services, utils, styles)
- `d:` is desktop code under `src/` consulted for RULE ZERO (usually `src/components/`)
- `r:` is `src-tauri/src/` (the Rust core)
- `k:` is `src-tauri/gen/android/app/src/main/java/app/streamnook/` (the Android Kotlin layer)
- `t:` is `src-tauri/` outside `src/` (config, Android resources)
- `p:` is the repository root
- `pref:` is a file in the StreamNook memory folder (`C:\Users\Brandon\.claude\projects\C--StreamNook\memory\`): Brandon's standing preferences and recorded traps

Units: CSS px map 1:1 to iOS points. Tailwind classes are converted to px (Tailwind spacing unit = 4 px; `text-sm` 14 px, `text-xs` 12 px etc.). Colours as written in the source (hex, rgba, or the CSS variable name).

Legend: `[ ]` not built on iPhone yet; `[x]` seen working on the iPhone; `[~]` built but needs work, OR impossible/replaced on iOS (always with the reason).

## 1. Opening a user's profile sheet

- [~] Tapping a chatter's name on a chat row opens the profile sheet for that chatter, carrying the row's user id, login, display name, name colour and the row's badge list (`{key, info}` pairs) (m:chat/MobileChatPane.tsx:L465-L480) (iPhone: name tap in the UIKit chat list calls a Swift closure that presents the profile `.sheet`) (iPhone now: built (name tap in the UIKit list calls onProfile); drives cannot tap; the drive opens the sheet through the same ProfileTarget)
- [~] The sheet opens ONLY in Twitch rooms: in a Kick or YouTube room a name tap does nothing, because the sheet is Helix-backed and a Kick id would open an unrelated Twitch account (m:chat/MobileChatPane.tsx:L472-L476) (iPhone: same gate on `provider == .twitch`) (iPhone now: built (provider == twitch gate); not seen in a Kick room)
- [ ] The sheet component itself is also handed `null` in non-Twitch rooms, so a sheet left open while switching to a Kick tab disappears (m:chat/MobileChatPane.tsx:L714-L721) (iPhone: dismiss the sheet when the active chat tab's provider is not Twitch)
- [~] Tapping an `@mention` in a Twitch room opens the same sheet: if the mentioned login is in the chatter store its id, login, display name and colour are used; otherwise the login is resolved through the shared mention lookup (Helix by login, cached including not-found) and the sheet opens with that id, login, display name and the mention's shown colour; mentions pass an EMPTY badge list (d:components/ChatMessage.tsx:L536-L566) (iPhone: mention span tap resolves through the same Rust lookup, then presents the sheet) (iPhone now: built (mention tap opens the sheet by login, Rust resolves the id); drives cannot tap; the drive opens the sheet through the same ProfileTarget)
- [ ] A mention whose login cannot be resolved opens nothing and logs a warning (no toast) (d:components/ChatMessage.tsx:L552-L566) (iPhone: no sheet, no alert)
- [~] The long-press fan-out's "Profile" action opens the sheet with the login as BOTH login and display name, colour `#9147FF`, and no badges (m:chat/MobileChatPane.tsx:L556-L563) (iPhone: context-menu "Profile" item, SF Symbol `person.crop.circle`, same payload) (iPhone now: built; long-press not drivable)
- [~] The fan-out hides "Profile" in non-Twitch rooms (`OTHER_PLATFORM_HIDES = ['profile']`, read-only adds `reply`) (m:chat/MobileChatPane.tsx:L51-L52) (iPhone: omit the menu item); the fan-out itself is owned by chat-view.md (iPhone now: built; not seen in a Kick room)
- [~] The sheet receives the room context: `channelId` = the room's broadcaster id, `channelName` = the active channel login, so channel-scoped Twitch badges and the sub tile resolve for this room (m:chat/MobileChatPane.tsx:L714-L721) (iPhone: pass the room's broadcaster id and login into the sheet model) (iPhone now: built (roomID and roomLogin ride ProfileTarget); real-account read not seen yet: the phone was signed out by the new user:edit scope)
- [x] The sheet is keyed on the user id: opening a different person mounts a fresh sheet seeded from that person's cached data, never painting the previous person's badges under the new name for a frame (m:profile/UserProfileSheet.tsx:L103-L124) (iPhone: `.sheet(item:)` with an `Identifiable` keyed on user id, new view model per user) (iPhone now: seen: `.sheet(item:)` keyed on ProfileTarget, a fresh model per person)
- [x] The phone has no "Messages first" user-card setting; `chat_design.user_card_opens_messages` is desktop-only and hidden on the phone (d:components/settings/ChatSettings.tsx:L2878-L2884) (iPhone: no such setting; the sheet always shows profile then messages)

## 2. Sheet container

- [x] Bottom sheet, content-sized, capped at 80% of the viewport height (`maxHeightFraction={0.8}`), and never taller than viewport minus keyboard minus top safe area minus 12 px (m:profile/UserProfileSheet.tsx:L254) (m:ui/MobileSheet.tsx:L106-L111) (iPhone: `.sheet` with a fitted detent capped at `.fraction(0.8)` plus `.large` disallowed, `presentationDragIndicator(.visible)`)
- [x] Width cap, grab handle, drag-to-dismiss, backdrop, body padding and back handling are the shared sheet's and follow shell-design.md § 10 (m:ui/MobileSheet.tsx:L76-L150) (iPhone: system `.sheet` with detents)
- [~] Closing the profile sheet unmounts it immediately with NO slide-out, because `UserProfileSheet` returns null when the user is cleared, which removes the sheet's AnimatePresence along with it (the shared sheet only animates out when `open` flips while mounted) (m:profile/UserProfileSheet.tsx:L110) (m:ui/MobileSheet.tsx:L152-L159) (iPhone: Android quirk, not wanted: use the normal sheet dismiss animation)
- [x] No title row on the profile sheet (m:profile/UserProfileSheet.tsx:L254) (iPhone: no navigation title)

## 3. Profile header

- [x] Row: avatar then the identity column, 12 px gap, vertically centred (m:profile/UserProfileSheet.tsx:L256) (iPhone: `HStack(spacing: 12)`)
- [~] Avatar 56 x 56 px circle, `object-cover`, 1 px ring `white/10`, drawn through the 7TV avatar (section 4) with `motion="animate"` and the Twitch picture as fallback (m:profile/UserProfileSheet.tsx:L257-L266) (iPhone: 56 pt circular image with a 1 pt white 10% stroke) (iPhone now: built; FakeCore has no picture, real-account read not seen yet: the phone was signed out by the new user:edit scope)
- [~] Until the Helix user lookup answers, the avatar slot is a plain 56 px `bg-surface` circle; the 7TV picture is not drawn before then either (m:profile/UserProfileSheet.tsx:L267-L269) (iPhone: placeholder circle, no spinner) (iPhone now: built; the placeholder circle draws a person glyph (design lane))
- [~] Avatar comes from `invoke get_user_by_id { userId }` -> `profile_image_url`; a failure leaves the placeholder silently (m:profile/UserProfileSheet.tsx:L227-L234) (r:commands/twitch.rs:L1549) (iPhone: Rust call through UniFFI) (iPhone now: Rust answers `picture` in the sheet; real-account read not seen yet: the phone was signed out by the new user:edit scope)
- [x] Display name 18 px bold, single line truncated (m:profile/UserProfileSheet.tsx:L272-L274) (iPhone: `.font(.system(size: 18, weight: .bold))`, `.lineLimit(1)`)
- [x] Name drawn in the 7TV paint when there is one, with every paint shadow (`'all'`, NOT the viewer's paint-shadow setting); otherwise in the chatter's colour after the readability adjustment (`chat_design.name_color_adjustment`, default `hsl_loop`), defaulting to `#9147FF` when the row carried no colour (m:profile/UserProfileSheet.tsx:L241-L244) (s:hooks/useNameColor.ts:L12-L23) (iPhone: paint renderer of section 15; colour through the same Rust adjust) (iPhone now: seen with a gradient paint, all shadows)
- [x] The paint comes from the live chatter store first (anyone who has spoken), else the profile inventory's `selected` paint (m:profile/UserProfileSheet.tsx:L135) (m:profile/UserProfileSheet.tsx:L241) (iPhone: the Rust-resolved worn paint) (iPhone now: Rust `worn_paint`)
- [~] `@login` line, 13 px `textMuted`, truncated, shown only when the display name differs from the login ignoring case (m:profile/UserProfileSheet.tsx:L276-L278) (iPhone: secondary `Text("@\(login)")`) (iPhone now: built; not seen with a differing login)
- [x] "Paint: <name>" line, 11.5 px `textSecondary`, 2 px top margin, truncated, shown only when a paint is worn (m:profile/UserProfileSheet.tsx:L279-L283) (iPhone: caption text)
- [x] No follow age, account age, follower count, pronouns, bio, mod/VIP status line or StreamNook member number appear on the phone sheet (m:profile/UserProfileSheet.tsx:L253-L362) (iPhone: match; see Beyond Android)

## 4. 7TV avatar (shared by the sheet and the You tab)

- [ ] One image whose box never changes size; only its source switches between the 7TV profile picture and the platform picture (s:components/SevenTvAvatar.tsx:L81-L86) (iPhone: one `AsyncImage`-like view with a fixed frame)
- [ ] The 7TV picture is used only when the viewer's `cosmetics.profile_picture_source` is `seventv` (default; anything but `twitch` reads as `seventv`) and the id is one 7TV can know (bare numeric Twitch id up to 20 digits, `kick:<digits>`, `youtube:UC` + 22 chars) (s:services/sevenTvPictures.ts:L101-L130) (s:hooks/useProfilePicture.ts:L22-L29) (iPhone: Rust decides; Swift asks with the cosmetics id)
- [ ] Animation follows `cosmetics.profile_picture_animation` (default `always`): `never` draws the still; `always` moves; `hover` lets focal surfaces (`animate`, used by the sheet and the You header) move; neither the app Motion setting nor system reduce-motion changes it (s:services/sevenTvPictures.ts:L132-L158) (iPhone: same rule; do NOT tie to `accessibilityReduceMotion`)
- [ ] A moving picture pauses on its still while the window is hidden or the picture is out of view (s:services/sevenTvPictures.ts:L168-L175) (s:components/SevenTvAvatar.tsx:L126-L137) (iPhone: pause animated image when scene is not active or the view is offscreen)
- [ ] Rendition choice: smallest scale covering `px * devicePixelRatio / 32`, else the largest; 56 px at 3x picks scale 6 or the largest available (s:services/sevenTvPictures.ts:L47-L56) (iPhone: same rule with `UIScreen.scale`)
- [ ] Switching between still and animated renditions waits until the new one is decoded, so the picture never blanks (s:components/SevenTvAvatar.tsx:L146-L164) (iPhone: swap only after the new image has decoded)
- [ ] If the 7TV image fails to load it falls back to the platform picture for that URL (s:components/SevenTvAvatar.tsx:L197-L203) (iPhone: on failure show the Twitch avatar)
- [ ] Lookups batch every id requested in one 16 ms frame into `invoke seventv_profile_pictures { ids }` (at most 200 per call), answers held 5 min, failed ids retried after 30 s keeping the last drawn picture, memo capped at 2000 (s:services/sevenTvPictures.ts:L179-L247) (r:commands/seventv.rs:L98-L105) (iPhone: Rust batch call; Swift memo or Rust-held cache)
- [ ] `listen seventv-profile-pictures` delivers pictures whose images arrived after the cosmetics answer named them (s:services/sevenTvPictures.ts:L250-L271) (iPhone: UniFFI callback / event stream)

## 5. Sub standing tile (this room)

- [~] When the sheet has a channel, it asks `invoke get_sub_standing { login, channel, wornBadges }` with the opening row's badge keys (e.g. `subscriber/3012`); errors are logged and the tile is simply absent (m:profile/UserProfileSheet.tsx:L195-L211) (r:commands/user_profile.rs:L913-L943) (iPhone: Rust call) (iPhone now: built; needs a room and a signed-in account, real-account read not seen yet: the phone was signed out by the new user:edit scope)
- [~] Rust writes every word: title ("Subscribed to <Channel>", "Past subscriber", "Not subscribed to <Channel>"), tier label ("Tier 1"), facts ("Founder", "Prime", "<n>-month streak", "Gifted by <name>" or "Gifted"), gift label ("Gift a sub", "Gift <recipient> a sub", "Gift <recipient> a sub to <Channel>") and next-badge text ("<n months> to the <n-month|n-year> badge") (r:services/sub_standing.rs:L170-L196) (r:services/sub_standing.rs:L336-L405) (iPhone: render Rust strings verbatim) (iPhone now: Rust from dev (sub_standing.rs); real-account read not seen yet: the phone was signed out by the new user:edit scope)
- [ ] Placed 12 px under the header, rendered at the phone's `sheet` size (m:profile/UserProfileSheet.tsx:L287-L291) (iPhone: same position)
- [ ] Current sub: glass tile, padding 14 px sides, 12 px bottom, 10 px top; tier tint on the edge only (radial wash from the top-left at 20% and a 1 px lit top rim at 50%, ring at 18%); tint `#00a8e1` Prime, `#f2c14e` tier 3, `#bf94ff` tier 2, `#a970ff` tier 1 (s:components/profile/SubStandingRow.tsx:L30-L37) (s:components/profile/SubStandingRow.tsx:L59-L83) (iPhone: glass card with a tinted gradient overlay and top highlight stroke)
- [ ] Title row: 11 px semibold uppercase wide-tracked `textSecondary`, truncated; a 36 px round gift button (gift icon 18 px, accent, `aria-label` = the gift label) at its trailing edge when giftable (s:components/profile/SubStandingRow.tsx:L85-L99) (iPhone: `Button` with SF Symbol `gift`, accessibility label = gift label)
- [ ] Badge art 44 x 44 px with a soft drop shadow (Rust's art, else the worn sub/founder badge art from the row, else a 10 px tint dot); months 30 px bold tabular, unit 13 px, tier label 13 px semibold in the tint (s:components/profile/SubStandingRow.tsx:L111-L137) (s:utils/wornSubBadge.ts:L11-L18) (iPhone: same sizes)
- [ ] Facts row 12.5 px `textSecondary`, wrapping with 12 px column and 4 px row gaps; each fact has a 17 px mark (Twitch's badge art, else the watch-streak flame or a gift icon); Prime and Founder facts are semibold `textPrimary` (s:components/profile/SubStandingRow.tsx:L39-L49) (s:components/profile/SubStandingRow.tsx:L138-L149) (iPhone: SF Symbols `flame.fill` / `gift` when no art)
- [ ] Next badge: 3 px progress bar on `white/8%`, fill scaled to `max(0.04, progress)` with a tint gradient, one-shot fill animation stilled by the Motion setting; caption 11.5 px with the next badge art at 16 px (s:components/profile/SubStandingRow.tsx:L153-L173) (iPhone: `ProgressView` styled thin, animate once unless reduce motion)
- [ ] Not a current sub: the whole row is a glass button with a 19 px gift icon, the gift label (14 px semibold) and "title · detail" under it (12.5 px), and a trailing "→"; hidden entirely when there is no gift label or URL (the channel's own broadcaster) (s:components/profile/SubStandingRow.tsx:L178-L197) (iPhone: full-width glass `Button`, SF Symbol `chevron.right`)
- [ ] Gift taps open `https://www.twitch.tv/subs/<channel>?giftrecipient=<login>` externally (s:components/profile/SubStandingRow.tsx:L55-L57) (r:services/sub_standing.rs:L400-L404) (iPhone: `UIApplication.open` / `openURL`)
- [ ] From the fan-out "Profile" path and mentions there are no worn badges, so the art falls back to Rust's ladder art or the dot (m:chat/MobileChatPane.tsx:L556-L563) (iPhone: same)

## 6. Badges on the sheet

- [x] Three provider groups in canonical order: "Twitch", "7TV", "Chat clients", then "StreamNook" last (m:profile/UserProfileSheet.tsx:L307-L325) (s:utils/badgeOrder.ts:L1-L26) (iPhone: four `Section`-like blocks in that order) (iPhone now: seen Twitch, 7TV (when its images load), StreamNook; Chat clients only with a real chatter)
- [x] Group title 11 px semibold uppercase wide tracking `textMuted`, 6 px below; group 12 px below the previous block (m:profile/UserProfileSheet.tsx:L77-L80) (iPhone: caption2 uppercase)
- [x] Badges laid out as a wrapping row, 8 px gaps; each a 32 x 32 px button holding a 28 x 28 px image, scale to 90% while pressed (m:profile/UserProfileSheet.tsx:L81-L96) (iPhone: `LazyVGrid(.adaptive(minimum: 32))` or flow layout; `.scaleEffect` press style)
- [ ] Badge accessibility label = title, else name, else "Badge" (m:profile/UserProfileSheet.tsx:L87) (iPhone: `.accessibilityLabel`)
- [x] Entries without a usable image are dropped; a group with nothing left is not rendered (m:profile/UserProfileSheet.tsx:L72-L75) (iPhone: same filter, in Rust) (iPhone now: seen: a group whose images all fail is not drawn)
- [ ] Images try 4x then the fallback list (7TV: 3x, 2x, 1x CDN) and hide themselves if every URL fails (s:components/FallbackImage.tsx:L14-L43) (s:utils/profileBadges.ts:L95-L106) (iPhone: image loader with fallback URL list, hide on total failure)
- [ ] Twitch group = the person's displayed badges merged with every EARNED badge (display first, deduped by id), resolved in this room's context, so it is an inventory, not what they wear (s:utils/profileBadges.ts:L64-L86) (s:services/cosmeticsCache.ts:L446-L459) (iPhone: Rust-provided grouped list)
- [ ] 7TV group = every 7TV badge the person OWNS (inventory), not just the worn one (s:services/cosmeticsCache.ts:L436-L449) (iPhone: same)
- [ ] "Chat clients" group = FFZ, Chatterino, Homies and the other chat-client badges from Rust; BTTV Pro is NOT included on the phone (the desktop card adds it via `get_bttv_pro_badge`) (s:utils/profileBadges.ts:L108-L132) (d:components/UserProfileCard.tsx:L1157) (iPhone: match the phone: no BTTV Pro)
- [ ] Tapping a badge shows an inline panel 12 px above the groups: rounded 8 px, `surface/60`, padding 10 x 6 px, 12 px bold title (title or name), " · <provider>" muted when known, and the description in 11.5 px muted beneath; tapping another badge replaces it; there is no close control (tooltips do nothing under a thumb) (m:profile/UserProfileSheet.tsx:L293-L305) (iPhone: inline callout card; or `.popover` with `presentationCompactAdaptation(.popover)`, keep inline to match)
- [x] StreamNook group shows only when the chatter is a StreamNook member (member number known through the member resolver); it renders the member's equipped badge at 24 px (default StreamNook logo when none), and never shows their number (m:profile/UserProfileSheet.tsx:L246-L251) (m:profile/UserProfileSheet.tsx:L318-L325) (s:components/StreamNookBadge.tsx:L471-L500) (pref: no-total-user-count-public.md) (iPhone: same gate; 24 pt image)
- [ ] On the phone the StreamNook badge has no tooltip and no tap action (the shared Tooltip renders bare children on mobile); its alt text is "StreamNook", "StreamNook <badge name>", or the badge name when it already starts with StreamNook (s:components/ui/Tooltip.tsx:L63) (s:components/StreamNookBadge.tsx:L480-L487) (iPhone: accessibility label with the same rule; no popover)
- [ ] Empty state when no badges in any group AND not a member: "No badges to show for this user." centred, 12.5 px muted, 16 px top margin (m:profile/UserProfileSheet.tsx:L327-L331) (iPhone: same copy)
- [ ] No loading indicator for badges: groups appear when the profile resolves (cached profiles paint on the first frame) (m:profile/UserProfileSheet.tsx:L137) (m:profile/UserProfileSheet.tsx:L213-L226) (iPhone: no spinner; seed from cache synchronously)

## 7. Recent messages

- [x] Section titled "Recent messages" (same caption style), followed by the count (medium weight, normal case) when there is at least one (m:profile/UserProfileSheet.tsx:L336-L342) (iPhone: header text plus count)
- [x] Source: `invoke get_user_message_history_limited { userId: <bare Twitch id>, limit: 50 }`, the same Rust per-user store the desktop card reads (m:profile/UserProfileSheet.tsx:L51-L52) (m:profile/UserProfileSheet.tsx:L153-L165) (r:commands/layout.rs:L13-L20) (iPhone: Rust call)
- [ ] The store keeps up to 200 messages per user for up to 1000 users (LRU evicts 10% when full) and is session-only; the sheet shows the latest 50 (r:services/user_message_history_service.rs:L9-L10) (r:services/user_message_history_service.rs:L153-L164) (iPhone: same Rust store)
- [ ] The store is keyed by the bare user id across EVERY Twitch room this session, so the list can include messages from other channels even though the copy says "this chat" (r:services/user_message_history_service.rs:L46-L53) (m:profile/UserProfileSheet.tsx:L148-L155) (iPhone: match; ambiguity flagged for Brandon)
- [ ] While the first answer is pending nothing renders under the title (no "empty" flash) (m:profile/UserProfileSheet.tsx:L140-L142) (m:profile/UserProfileSheet.tsx:L343) (iPhone: no placeholder text until answered)
- [ ] Empty, or the call failed: "Nothing from them in this chat yet." 12.5 px muted (m:profile/UserProfileSheet.tsx:L343-L346) (m:profile/UserProfileSheet.tsx:L166-L168) (iPhone: same copy)
- [ ] Live while open: `invoke watch_user_history { userKey }` on open, `listen user-history-message`, rows for this key appended unless the id is already present, list trimmed to the newest 50; `invoke unwatch_user_history` on close (m:profile/UserProfileSheet.tsx:L169-L190) (r:commands/channel_state.rs:L39-L50) (iPhone: Rust event stream subscribed for the sheet's lifetime)
- [x] Order is chat order: oldest at the top, newest at the bottom; the list does not auto-scroll (m:profile/UserProfileSheet.tsx:L333-L357) (iPhone: plain stack, no scroll-to-bottom)
- [x] Each row: time column 11.5 px muted tabular, fixed 4.5 ch wide, 1 px top nudge; message 13 px `textPrimary`, wrapping, snug leading; 4 px between rows (m:profile/UserProfileSheet.tsx:L348-L357) (iPhone: `HStack(alignment: .firstTextBaseline)`)
- [x] Time is the locale short time (`hour: 'numeric', minute: '2-digit'`, e.g. "3:07 PM"); timestamps may be unix ms or seconds as a string or RFC 3339; an unreadable stamp shows an empty time cell (m:profile/UserProfileSheet.tsx:L54-L65) (iPhone: `Date.FormatStyle.dateTime.hour().minute()`) (iPhone now: seen "11:07 PM")
- [x] Message text is plain: emotes appear as their codes, no images, no links (m:profile/UserProfileSheet.tsx:L333-L335) (iPhone: plain `Text`)
- [ ] Moderator history (Twitch's mod logs) and justlog archive lookups are desktop-card only; the phone sheet has only the session store (d:components/UserProfileCard.tsx:L1202-L1230) (iPhone: match the phone)

## 8. What the phone sheet does not have

- [x] No action buttons at all: no mention, reply, whisper, follow, block, share, copy name, open channel, nickname, colour override, private note or hide-in-chat; reply, copy and moderation of a message live in the long-press fan-out (chat-view.md) (m:profile/UserProfileSheet.tsx:L253-L362) (d:components/UserProfileCard.tsx:L1874-L1910) (iPhone: match; desktop actions listed in Beyond Android)
- [x] No moderator actions in the sheet (timeout, ban, unban, purge are the fan-out's, see chat-view.md) (d:components/UserProfileCard.tsx:L2605-L2860) (iPhone: none in the sheet)
- [x] No Kick or YouTube profile (the desktop card asks `kick_user_profile` / `youtube_user_profile`) (d:components/UserProfileCard.tsx:L1065-L1085) (iPhone: match; backlog)
- [ ] No haptic on open, on badge tap or on close (m:profile/UserProfileSheet.tsx:L1-L363) (iPhone: no feedback generator)

## 9. Data, caching and refresh for the sheet

- [x] Profile data comes from the shared cosmetics cache `getFullProfileWithFallback(userId, login, channelId, channelName)`: Twitch display + earned badges and chat-client badges from `invoke get_user_badges_with_earned_unified` in parallel with the 7TV inventory `invoke seventv_user_inventory { id }` (falling back to the worn-only `seventv_user_cosmetics`) (m:profile/UserProfileSheet.tsx:L213-L226) (s:services/cosmeticsCache.ts:L416-L459) (s:services/badgeService.ts:L153-L165) (r:commands/seventv.rs:L83-L96) (iPhone: one Rust call returning the grouped profile) (iPhone now: one Rust call, `user_profile_sheet`)
- [x] The profile is cached per USER id only (LRU 256, no TTL, session-long, in-flight requests deduped), so the first room a person's profile was opened in decides the channel-scoped Twitch badges for the rest of the session (s:services/cosmeticsCache.ts:L269-L272) (s:services/cosmeticsCache.ts:L416-L431) (iPhone: Rust cache; flag: key should include the channel, see moves) (iPhone now: fixed in Rust: the cache is keyed by user AND room (LRU 256))
- [x] A cached profile is returned as-is with no background refresh (the doc comment says otherwise) (s:services/cosmeticsCache.ts:L412-L425) (iPhone: match, or fix in Rust) (iPhone now: Rust paints the cached answer first, then the fresh one)
- [ ] Without a room, the channel context falls back to the person's own channel (`channelId || userId`, `channelName || login`) (s:services/cosmeticsCache.ts:L432-L434) (iPhone: same default in Rust)
- [ ] Rust caches inventory reads 60 s and worn cosmetics 5 min (30 s after a hard failure) (r:services/seventv_cosmetics_resolver.rs:L54-L72) (iPhone: same Rust cache)
- [ ] Cosmetics failures and avatar failures are silent (debug log only); the sheet still renders name and messages (m:profile/UserProfileSheet.tsx:L224-L234) (iPhone: no alert)
- [ ] All lookups for a closed sheet are cancelled-in-effect (late answers are dropped) (m:profile/UserProfileSheet.tsx:L213-L239) (iPhone: cancel the view model's tasks on dismiss)

## 10. Identity keys on these surfaces

- [x] The sheet's `userId` is a CHAT key; in a Twitch room it is the bare numeric Twitch id, used for the chatter store row, the cosmetics cache, Helix `get_user_by_id`, the history key (`historyKey(id, 'twitch')` = bare) and the 7TV avatar cosmetics id (m:profile/UserProfileSheet.tsx:L135-L155) (s:utils/chatterIdentity.ts:L42-L44) (iPhone: Twitch id; never a login or display name)
- [x] StreamNook membership is filed under a Twitch user id and found through the member resolver: a bare id passes through, `kick:` / `youtube:` keys resolve only through a claimed alias, never by numeric coincidence (m:profile/UserProfileSheet.tsx:L247-L251) (s:utils/memberIdentity.ts:L84-L89) (iPhone: Rust resolver) (iPhone now: Rust `cosmetics_registry::member_number`)
- [x] The sheet passes the raw chat key (not the resolved member id) to the StreamNook badge; harmless today because the sheet is Twitch-only (m:profile/UserProfileSheet.tsx:L323) (iPhone: pass the resolved member id) (iPhone now: Rust resolves the member before the badge)
- [x] The sub tile is keyed by login + channel login, both lowercased in Rust (r:commands/user_profile.rs:L919-L942) (iPhone: logins, never display names)
- [x] A chatter's name is display-only: no store or target is ever keyed by `display_name` or the shown name (Brain StreamNook_Identity_Keying.md § Chatter names) (iPhone: same rule)

## 11. Own identity header (top of the You tab)

- [x] The header row around this block (56 px avatar, top alignment, the "Signed in" fallbacks) is the You tab's and follows accounts-signin.md § 1.2; this area owns the identity block `OwnIdentityHeader` below (m:screens/YouScreen.tsx:L40-L69)
- [ ] Name size steps with length so even a 25-character name fits in two lines: over 20 chars 14.5 px, over 16 chars 16 px, over 12 chars 17 px, else 18.5 px; bold, tight leading, wraps (breaks long tokens), never truncated or ellipsised (m:profile/OwnIdentityHeader.tsx:L37-L52) (m:profile/OwnIdentityHeader.tsx:L186-L198) (iPhone: same step table, `.lineLimit(2)`, `.fixedSize(horizontal: false, vertical: true)`)
- [x] Name drawn in the worn 7TV paint (all shadows) over `--color-text-primary` as the fallback colour; with no paint it is plain `textPrimary` (a heading, not a chat colour) (m:profile/OwnIdentityHeader.tsx:L110-L116) (iPhone: paint renderer of section 15, else `.primary`)
- [x] Paint chip on the name's line when a paint is worn: glass pill, 8 px horizontal padding, 1 px vertical, the paint's name 11 px bold drawn IN the paint (fallback colour `#9146FF`); the chip truncates first, the name never does (m:profile/OwnIdentityHeader.tsx:L199-L211) (iPhone: glass capsule `glassEffect`, text with paint fill, `.layoutPriority` lower than the name)
- [ ] `@login` line 13 px muted, truncated, only when login differs from the display name ignoring case (m:profile/OwnIdentityHeader.tsx:L214-L216) (iPhone: same)
- [~] Badge row 6 px under the name, wrapping, 6 px gaps, each badge 18 x 18 px; order Twitch worn, 7TV worn, other active, then StreamNook last (m:profile/OwnIdentityHeader.tsx:L218-L242) (pref: chat-header-scale.md, 18 px faces) (iPhone: flow layout, 18 pt images) (iPhone now: seen; a badge whose image fails still leaves a gap (design lane))
- [ ] Badges shown are WORN, not owned: Twitch = badges the account displays (Twitch `displayBadges`) minus channel-scoped sets (broadcaster, moderator, lead_moderator, vip, subscriber, founder, bits, bits-leader, sub-gifter, sub-gift-leader, artist-badge, predictions, hype-train, clip-champ) (m:profile/OwnIdentityHeader.tsx:L120-L130) (s:utils/badgeOrder.ts:L42-L69) (iPhone: Rust-provided worn list)
- [x] 7TV = only the selected badge (chat store's worn badge first, else the inventory's `selected`) (m:profile/OwnIdentityHeader.tsx:L132-L148) (iPhone: same rule in Rust)
- [ ] Other active = the StreamNook Identity loadout badges (`invoke get_streamnook_identity_resolved { userId }`) for a member; a non-member falls back to their raw chat-client badges (m:profile/OwnIdentityHeader.tsx:L150-L169) (s:services/identityService.ts:L226-L251) (r:commands/identity.rs:L210-L223) (iPhone: Rust call)
- [x] Deduped by id, images-only, and hard-capped at 8 badges (backstop: a 127-badge wall once buried the tab) (m:profile/OwnIdentityHeader.tsx:L171-L181) (iPhone: same cap) (iPhone now: Rust caps at 8)
- [x] StreamNook badge (24 px default size) after the others when the account has a member number; the row also renders for a member with zero other badges (m:profile/OwnIdentityHeader.tsx:L182) (m:profile/OwnIdentityHeader.tsx:L218-L240) (iPhone: same gate)
- [x] Badge `title` hover text does nothing on touch; no tap action on header badges (m:profile/OwnIdentityHeader.tsx:L223-L231) (iPhone: accessibility labels only, no tap)
- [~] Paints on frame one: boot registers every own account (primary + linked), hydrates their persisted cosmetics (`localStorage streamnook_own_cosmetics_v1`) and identity (`streamnook_own_identity_v1`), then revalidates 7TV without blanking and pre-fetches the full profile with the exact arguments the header uses (`selfId, selfLogin, selfId, selfLogin`) (m:boot/useMobileBoot.ts:L114-L163) (m:profile/OwnIdentityHeader.tsx:L89-L108) (s:services/cosmeticsCache.ts:L148-L157) (iPhone: Rust persists own cosmetics and identity on disk and seeds them at launch) (iPhone now: Rust keeps the header on disk (`own_identity_header_cached`); a cold launch not checked)
- [ ] The header refreshes itself on mount (profile and resolved identity) and follows identity changes made elsewhere through the resolved-identity subscription, without a remount (m:profile/OwnIdentityHeader.tsx:L66-L87) (iPhone: observe a Rust identity-changed event)
- [ ] Resolved identity cache: 5 min TTL, 1000 entries, in-flight deduped; an API failure caches the empty default (s:services/identityService.ts:L23-L24) (s:services/identityService.ts:L226-L251) (iPhone: Rust cache)
- [ ] 7TV session keep-alive: 8 s after boot, a hidden 7TV re-mint runs if the session is near expiry (owned by accounts-signin.md) (m:boot/useMobileBoot.ts:L127-L138) (iPhone: cross-reference)
- [x] No "Contributor" or member-number line in the header; a Contributor badge, once granted, is an ordinary StreamNook cosmetic (section 13) (s:services/supabaseService.ts:L1678-L1682) (iPhone: match)

## 12. Cosmetics entry row (You tab)

- [x] The You tab's "Cosmetics" row that opens this screen follows accounts-signin.md § 1 (m:screens/YouScreen.tsx:L101-L115) (m:navStore.ts:L80)

## 13. Cosmetics screen: container and navigation

- [x] Full-screen layer over the tabs (absolute, z-50, opaque `background`), top padded by the safe area; slides in from the trailing edge and back out, 220 ms, ease `[0.2, 0.8, 0.2, 1]` (m:screens/CosmeticsScreen.tsx:L449-L454) (m:ui/DrillInScreen.tsx:L15-L53) (iPhone: NavigationStack push; hide the tab bar with `.toolbar(.hidden, for: .tabBar)` to match) (iPhone now: NavigationStack push, tab bar hidden)
- [x] Header: back button (44 px min target, arrow 22 px, `aria-label` "Back") and the title "Cosmetics" 18 px bold; 1 px bottom border `borderSubtle` (m:screens/CosmeticsScreen.tsx:L455-L464) (iPhone: navigation title "Cosmetics", system back)
- [ ] Android back closes it (after any open sheet and the settings view) (m:navStore.ts:L95-L98) (iPhone: swipe back)
- [x] Body scrolls, 16 px sides, 12 px top, bottom padding = safe area + 16 px (m:screens/CosmeticsScreen.tsx:L465-L468) (iPhone: `ScrollView` respecting safe area)
- [x] The screen stays mounted for the session, so which collapsible sections are open persists across close and reopen (m:screens/CosmeticsScreen.tsx:L133) (m:MobileApp.tsx:L168) (iPhone: keep section expansion in a session-lived model) (iPhone now: open sections live in CosmeticsStore for the session)
- [x] Every open re-reads everything (section 19); there is no pull-to-refresh (m:screens/CosmeticsScreen.tsx:L218-L224) (iPhone: load on appear; `.refreshable` only as backlog)
- [x] Section order: "Badge", "Atmosphere" (+ chat toggle), "7TV" header, "7TV paints", "7TV badges", "Twitch badges" (m:screens/CosmeticsScreen.tsx:L469-L734) (iPhone: same order) (iPhone now: plus Name color and Bio after Twitch badges (Beyond Android))
- [x] Section captions 12 px semibold uppercase wide tracking `textMuted`, 6 px bottom margin (m:screens/CosmeticsScreen.tsx:L469-L471) (iPhone: list section headers)
- [ ] No haptics anywhere on this screen (m:screens/CosmeticsScreen.tsx:L1-L738) (iPhone: none)

## 14. StreamNook badge picker ("Badge")

- [~] Lists the catalog cosmetics of kind `badge` (missing kind counts as badge) that the account owns: explicit grants plus every `is_default` cosmetic, in catalog `sort_order`; owner-only hidden cosmetics still list when owned (m:screens/CosmeticsScreen.tsx:L230-L232) (s:services/supabaseService.ts:L1846-L1875) (iPhone: Rust-provided owned list) (iPhone now: Rust `streamnook_picker`; seen on FakeCore, real-account read not seen yet: the phone was signed out by the new user:edit scope)
- [x] Grid 4 columns, 8 px gaps, 16 px under the grid; tile glass panel, 8 px padding, image 36 x 36 px (placeholder rounded `surface` square when no asset), name 10.5 px `textSecondary` centred, two lines max (m:screens/CosmeticsScreen.tsx:L477-L496) (iPhone: `LazyVGrid` 4 columns)
- [x] Worn tile: 2 px accent ring and a 13 px bold check at the top-right corner (4 px inset) (m:screens/CosmeticsScreen.tsx:L485-L502) (iPhone: accent stroke + SF Symbol `checkmark`)
- [x] Badge art resolution: bundled art for the three house slugs (`streamnook-default`, `streamnook-supporter`, `streamnook-subscriber`), else the asset manifest, else the catalog's `https` `asset_path` (cdn.streamnook.app); full-size art on the 36 px tile (s:components/cosmeticAssets.ts:L14-L63) (pref: badge-masters-full-size.md) (iPhone: ship the house art in the asset catalog or load the CDN URL; never downscale it ourselves) (iPhone now: bundled house art and CDN art seen)
- [~] Tap equips optimistically; tapping the worn badge UNEQUIPS it (writes `null`, back to the plain logo) (m:screens/CosmeticsScreen.tsx:L254-L263) (iPhone: match the phone; ambiguity: desktop never unequips, d:components/settings/ProfileSettings.tsx:L1453-L1461) (iPhone now: equip seen (FakeCore); taking the worn badge off not driven)
- [x] The write goes to the StreamNook API `/api/cosmetics/equip` when enabled, else Supabase `user_cosmetic_active`; a non-badge slug is refused; a failure rolls back and toasts the server's error or "Could not equip that badge." (m:screens/CosmeticsScreen.tsx:L258-L262) (s:services/supabaseService.ts:L2306-L2350) (iPhone: Rust command, same rollback and copy) (iPhone now: seen: a refused equip rolls back and toasts the server's error)
- [ ] Empty: "No badges owned yet. Earned and event badges land here." in a glass panel, 16 px padding, 13 px muted (m:screens/CosmeticsScreen.tsx:L472-L475) (iPhone: same copy)
- [~] The list is read synchronously from the cosmetics registry when the screen renders and is not subscribed to registry updates, so a registry still loading reads as the empty state until the next re-render (m:screens/CosmeticsScreen.tsx:L139-L140) (m:screens/CosmeticsScreen.tsx:L230-L232) (iPhone: Android quirk: show the list when the registry loads)
- [ ] A Contributor badge tier (granted for shipped contributions) appears here like any other owned badge once granted (s:services/supabaseService.ts:L1678-L1682) (iPhone: same)

## 15. Atmosphere picker

- [x] Grid 2 columns, 8 px gaps; tile glass panel, 10 px padding, a 48 px tall rounded swatch then a name row (12.5 px `textPrimary`, truncated) (m:screens/CosmeticsScreen.tsx:L508-L547) (iPhone: `LazyVGrid` 2 columns)
- [x] First tile "Default" (theme id `tier`) with a plain `surface` swatch, always selectable (m:screens/CosmeticsScreen.tsx:L512-L523) (iPhone: same copy; desktop calls it "Tier aura" and also offers a 7TV paint background, the phone offers neither, d:components/settings/ProfileSettings.tsx:L980-L1000)
- [x] One tile per atmosphere in the server-driven catalog; the swatch is the atmosphere's own `swatch` CSS background (a static preview) (m:screens/CosmeticsScreen.tsx:L524-L536) (s:services/atmospheres.ts:L17-L23) (pref: visual-settings-get-previews.md) (iPhone: render the swatch gradient natively; backlog for a live animated preview) (iPhone now: swatches drawn natively from Rust `look` layers)
- [x] Accolade-gated atmospheres are hidden until the accolade is earned; event atmospheres (Cologne) are listed (m:screens/CosmeticsScreen.tsx:L273-L278) (iPhone: same filter in Rust) (iPhone now: Rust filter)
- [x] Allowed: accolade atmospheres when earned; others when the slug is owned OR the account is a subscriber (Stripe status `active`/`past_due`, cached 5 min) (m:screens/CosmeticsScreen.tsx:L265-L271) (s:services/subscriberService.ts:L19-L41) (iPhone: Rust decides) (iPhone now: Rust decides; a locked tile seen)
- [x] Not allowed: tile disabled at 45% opacity with a 12 px lock icon; tapping does nothing (no upsell link on the phone; desktop opens the support page) (m:screens/CosmeticsScreen.tsx:L528-L543) (d:components/settings/ProfileSettings.tsx:L462-L476) (iPhone: disabled tile, SF Symbol `lock.fill`)
- [x] Worn: 2 px accent ring and a 13 px check; a Cologne theme with add-on suffixes (`<id>+coin`, `+border`) still marks its base tile (m:screens/CosmeticsScreen.tsx:L526) (m:screens/CosmeticsScreen.tsx:L539-L541) (iPhone: same match rule from Rust)
- [x] Tap writes the theme optimistically through the API `/api/cosmetics/theme`, else Supabase `user_profile_prefs`; the failure toast "Could not apply that atmosphere." exists but can never fire, because the write swallows its own errors (m:screens/CosmeticsScreen.tsx:L280-L290) (s:services/supabaseService.ts:L1338-L1356) (iPhone: Rust returns a real result; show the same copy on failure) (iPhone now: Rust returns a real result)
- [~] Unlike desktop, the phone does not push the new atmosphere into its own chat rows or patch the cached profile snapshot right away; own rows update on the realtime echo (m:screens/CosmeticsScreen.tsx:L280-L290) (d:components/settings/ProfileSettings.tsx:L444-L460) (iPhone: Rust emits the own-identity change immediately; flag) (iPhone now: Rust emits member-theme-changed and a ROW_MEMBER frame at once; own rows not checked after a change)
- [x] "Show in my chat messages" row appears only while an atmosphere (or the Cologne look) is worn: glass panel 8 px under the grid, padding 12 x 10 px, title 13.5 px, subtitle "Off keeps it on your profile only." 12 px muted (m:screens/CosmeticsScreen.tsx:L549-L555) (iPhone: `Toggle` row in a glass card)
- [~] Switch 44 x 24 px, accent on / `surface` off, 20 px white knob at left 22 / 2 px sliding 200 ms ease-out; `role=switch`, `aria-label` "Hide your atmosphere in chat" when on, "Show your atmosphere in chat" when off (m:screens/CosmeticsScreen.tsx:L556-L569) (iPhone: system `Toggle` with that accessibility label) (iPhone now: system Toggle seen; accessibility label not checked)
- [~] Toggling repaints own chat rows immediately (`setOwnThemeHiddenInChat`), writes `/api/v1/profile/prefs { chat_atmosphere }` (Supabase fallback), and on failure reverts both and toasts "Could not save that. Try again in a moment." (m:screens/CosmeticsScreen.tsx:L292-L302) (s:services/supabaseService.ts:L1358-L1378) (iPhone: Rust command with rollback) (iPhone now: built with rollback; not driven)
- [ ] Default when prefs cannot be read: theme `tier`, show in chat on (m:screens/CosmeticsScreen.tsx:L141-L149) (s:services/supabaseService.ts:L1233-L1238) (iPhone: same defaults)

## 16. 7TV section

- [x] The "7TV" header row appears only when the account owns at least one 7TV paint or badge: "7TV" caption, status "Signed in" / "Not signed in" (12 px muted), and a trailing text button "Sign in" / "Sign out" / "Signing in…" (12.5 px accent, 50% while busy); 20 px top margin (m:screens/CosmeticsScreen.tsx:L573-L589) (iPhone: section header with a trailing button)
- [x] Owned paints and badges list whether or not 7TV is signed in; the 7TV token only authorises changing what is worn (m:screens/CosmeticsScreen.tsx:L6-L9) (m:screens/CosmeticsScreen.tsx:L184-L201) (iPhone: same)
- [~] Sign in runs the 7TV login sheet (title "Sign in to 7TV"); the flow and its overlay are accounts-signin.md (m:screens/CosmeticsScreen.tsx:L375) (m:cosmetics/sevenTvConnect.ts:L83-L109) (iPhone: WKWebView sign-in sheet per accounts-signin.md) (iPhone now: built (SevenTvSignIn); not run, it would sign in a real 7TV account)
- [~] Sign out calls `invoke logout_seventv`; failure toasts "Could not sign out of 7TV." (m:screens/CosmeticsScreen.tsx:L377-L385) (r:commands/seventv_cosmetics.rs:L296-L302) (iPhone: Rust call) (iPhone now: built; never run against the real 7TV session)
- [~] Tapping a paint or 7TV badge while signed out starts sign-in first, then applies the tap; a second tap while signing in is ignored (m:screens/CosmeticsScreen.tsx:L304-L344) (iPhone: same sequencing) (iPhone now: built; not driven)
- [~] Sign-in outcomes: cancelled or no token, nothing happens; token stored but status not authenticated, "7TV did not finish signing you in. Try again."; an exception, "Could not sign in to 7TV."; no 7TV user id known, "7TV did not identify your account." (m:screens/CosmeticsScreen.tsx:L313-L342) (iPhone: same copy) (iPhone now: built with the same copy; not driven)
- [x] "7TV paints" collapsible with the owned count; grid 2 columns, 8 px gaps; tile glass panel, 10 px padding, the paint's name 14 px bold drawn IN the paint (fallback `#9146FF`), truncated; worn tile ring + 13 px check at the trailing edge (m:screens/CosmeticsScreen.tsx:L591-L625) (pref: visual-settings-get-previews.md, the paint is its own preview) (iPhone: paint-rendered text tiles) (iPhone now: paints drawn in themselves)
- [x] "7TV badges" collapsible with the owned count; grid 4 columns; tile 8 px padding, 36 px image with fallbacks, name 10.5 px two lines max, worn ring + check (m:screens/CosmeticsScreen.tsx:L627-L665) (iPhone: same)
- [~] Tapping the worn paint or badge unequips it (writes `null`); tapping another wears it (m:screens/CosmeticsScreen.tsx:L408-L446) (iPhone: same toggle) (iPhone now: wearing another seen (FakeCore); unequip not driven)
- [x] Writes: `invoke set_seventv_paint { userId: <7TV user id>, paintId }` / `invoke set_seventv_badge { userId, badgeId }`; `success: false` counts as a failure (m:screens/CosmeticsScreen.tsx:L413-L418) (m:screens/CosmeticsScreen.tsx:L433-L438) (r:commands/seventv_cosmetics.rs:L357-L392) (iPhone: Rust call) (iPhone now: FakeCore write seen)
- [x] On success the selection flips locally without a re-read (7TV's read API lags its own write by seconds), the whole 7TV cosmetics cache is cleared (`seventv_clear_cosmetics`), and the own cached cosmetics are flipped and republished so own chat rows and profile repaint at once (m:screens/CosmeticsScreen.tsx:L403-L421) (s:services/cosmeticsCache.ts:L172-L221) (s:services/seventvService.ts:L245-L251) (iPhone: Rust flips its own cache and emits the change) (iPhone now: Rust holds own picks 120 s over 7TV's lagging reads)
- [~] On failure the selection reverts; an error containing `SESSION_EXPIRED` marks 7TV signed out and toasts "Your 7TV sign-in expired. Tap again to sign back in."; anything else toasts "Could not equip that paint." / "Could not equip that badge." (m:screens/CosmeticsScreen.tsx:L387-L401) (r:services/seventv_auth_service.rs:L436) (iPhone: typed Rust error, same copy) (iPhone now: built with the same copy; not driven)
- [x] The 7TV write id is the inventory's `seventvUserId` when present, otherwise the signed-in token's 7TV id (m:screens/CosmeticsScreen.tsx:L176-L191) (iPhone: Rust picks; flag: these can differ, see moves) (iPhone now: Rust picks)

## 17. Twitch global badge picker

- [~] With the Drops sign-in (`invoke is_drops_authenticated` true), the list is `invoke fetch_chat_identity_badges_gql { username: <login> }`: global badges only, each with set id, version, title, image (4x, else 2x, 1x, else the static CDN URL) and whether it is worn; Rust sorts worn first, then by title (m:screens/CosmeticsScreen.tsx:L202-L215) (r:commands/chat_identity.rs:L202-L273) (r:commands/chat_identity.rs:L376-L387) (iPhone: Rust call; the command is `#[cfg(target_os = "android")]` today and must be enabled for iOS) (iPhone now: command enabled for iOS; seen on FakeCore, real-account read not seen yet: the phone was signed out by the new user:edit scope)
- [x] Selectable list: "Twitch badges" collapsible with the count; grid 4 columns; tile 36 px image, title 10.5 px two lines, worn ring + check (m:screens/CosmeticsScreen.tsx:L667-L700) (iPhone: same)
- [~] Tap wears it optimistically via `invoke update_chat_identity_gql { badgeId, badgeVersion }`; tapping the worn one does nothing (Twitch has no unequip) (m:screens/CosmeticsScreen.tsx:L346-L360) (r:commands/chat_identity.rs:L389-L400) (iPhone: Rust call, also `#[cfg(target_os = "android")]` today) (iPhone now: built; not driven)
- [~] Failure reverts and toasts "Sign in to Drops to change your Twitch badge." when the error contains `NEEDS_DROPS_AUTH`, else "Could not equip that badge." (m:screens/CosmeticsScreen.tsx:L361-L371) (iPhone: typed error, same copy) (iPhone now: built with the same copy; not driven)
- [~] Without the selectable list (no Drops sign-in, or its fetch failed), "Twitch badges" lists every displayed + earned Twitch badge read-only in one glass panel (12 px padding), 32 px images wrapping with 10 px gaps (m:screens/CosmeticsScreen.tsx:L701-L725) (iPhone: read-only flow of images) (iPhone now: built; not seen)
- [~] Read-only list note, only when Drops is not signed in: "Sign in to Drops to choose which badge you wear." 11.5 px muted, 10 px above (m:screens/CosmeticsScreen.tsx:L726-L730) (iPhone: same copy; Drops sign-in is accounts-signin.md) (iPhone now: built; not seen)
- [x] The phone has no webview fallback when GQL declines (desktop scrapes or drives Twitch's editor in a hidden window) (r:commands/chat_identity.rs:L365-L375) (iPhone: match the phone, GQL only)
- [x] Desktop dedupes the badge list by `(id, version)`; the phone does not (d:components/settings/ProfileSettings.tsx:L660-L670) (iPhone: dedupe in Rust) (iPhone now: Rust dedupes by (id, version))

## 18. Collapsible sections

- [x] Header button: caption (12 px semibold uppercase) + count (12 px muted) + trailing caret 14 px that rotates to 90 degrees in 220 ms, ease `[0.16, 1, 0.3, 1]`; 16 px top and 6 px bottom margin; pressed opacity 70%; `aria-expanded` set (m:screens/CosmeticsScreen.tsx:L51-L75) (iPhone: `DisclosureGroup` with SF Symbol `chevron.right` rotation)
- [ ] Body expands height 0 to auto with a fade over 260 ms, same ease; collapses the same way (m:screens/CosmeticsScreen.tsx:L76-L88) (iPhone: `withAnimation(.easeOut(duration: 0.26))`)
- [x] All sections start collapsed on first open (m:screens/CosmeticsScreen.tsx:L133) (iPhone: same)
- [ ] A section with nothing in it is not rendered at all (no count of 0) (m:screens/CosmeticsScreen.tsx:L591) (m:screens/CosmeticsScreen.tsx:L627) (m:screens/CosmeticsScreen.tsx:L701-L702) (iPhone: same)

## 19. Cosmetics screen loading

- [x] On every open, in parallel: (a) StreamNook registry reads (owned slugs, worn slug), profile prefs, subscriber status and accolades; (b) 7TV status (`invoke get_seventv_auth_status`), the own full profile (cached), the Drops check and the selectable Twitch list (m:screens/CosmeticsScreen.tsx:L135-L224) (iPhone: one Rust "cosmetics picker state" call) (iPhone now: two Rust calls: `cosmetics_picker_state`, `external_cosmetics_state`)
- [ ] Each read fails soft: prefs failure keeps defaults with a warning, subscriber failure reads as not subscribed, accolades failure leaves accolade atmospheres hidden, 7TV and Twitch failures leave their sections as they were (m:screens/CosmeticsScreen.tsx:L143-L164) (m:screens/CosmeticsScreen.tsx:L188-L215) (iPhone: same, no error UI)
- [x] No loading indicators; sections fill in as reads land (m:screens/CosmeticsScreen.tsx:L448-L736) (iPhone: no spinners)
- [ ] Accolades are re-read only when the screen opens (desktop also re-reads on window focus) (m:screens/CosmeticsScreen.tsx:L156-L164) (d:components/settings/ProfileSettings.tsx:L433-L441) (iPhone: re-read on open and on scene becoming active)

## 20. 7TV paint rendering (shared by the sheet, the header and the picker)

- [x] A paint draws text with its layers: linear gradients (angle, stops, repeating), radial gradients (circle or ellipse), image layers, a single colour; layers stack, the first single colour is the base colour, else the caller's fallback colour (s:services/paintStyle.ts:L79-L157) (s:services/paintStyle.ts:L183-L230) (iPhone: render the text as a mask over a native gradient / image fill) (iPhone now: linear and radial seen; image layers built)
- [ ] Image layers draw the scale-1 image, and the animated file whenever the layer animates (7TV lists `_static` stills first) (s:services/paintStyle.ts:L111-L125) (pref: never-drop-fps.md) (iPhone: animated paint image at full frame rate; the pick comes from Rust)
- [x] Shadows are drop shadows in order (offset, blur, colour); these three surfaces always draw all of them (s:services/paintStyle.ts:L159-L175) (iPhone: stacked `.shadow` modifiers)
- [ ] The lowest layer opacity under 1 becomes the whole name's opacity; the paint spans the full text width (s:services/paintStyle.ts:L214-L232) (iPhone: same)
- [ ] Bake each paint once and reuse it across surfaces (bounded cache), never rebuilt per row (research: OSS paint rasterising, 256 entries) (s:services/paintStyle.ts:L174-L182) (iPhone: paint image cache keyed by paint id + fallback colour + shadow mode)

## Decisions for Brandon

Not parity lines: open questions found while reading, each to be settled before its screen is built.

- Tapping the StreamNook badge you already wear takes it off on the phone; desktop says a member always wears one, so the tap does nothing there. Pick one.
- "Recent messages" reads a store keyed by user only, so it lists messages from every Twitch room this session while the copy says "Nothing from them in this chat yet." Scope it to the room on iPhone (and Android)?
- The "Could not apply that atmosphere." toast can never fire because the write swallows its own errors. Surface the error on iPhone?

## Beyond Android (backlog, Brandon picks)

- [x] (beyond Android) A live preview row at the top of Cosmetics: your badges, your name in your paint or Twitch color, and your Atmosphere's wash and frost, updating as you pick (seen on FakeCore)
- [x] (beyond Android) Twitch name color: the named colors from Rust, the current one ringed, applied through `update_user_chat_color` (seen on FakeCore)
- [x] (beyond Android) Twitch bio: read and edited in Cosmetics (`get_twitch_bio` / `update_twitch_bio`, `user:edit` joins the required scopes, 300 characters checked in Rust) (seen on FakeCore)
- [ ] (backlog, not parity) Profile sheet header stats the desktop card already has: account created, following since (or "Not following" / "Hidden"), follows count, chatters, last live, from IVR (d:components/UserProfileCard.tsx:L2455-L2500)
- [ ] (backlog, not parity) Profile sheet actions row: Mention (inserts `@login` into the composer), Whisper, Follow / Unfollow, Block, Copy login, Open channel (push the channel page), Share profile link (`ShareLink`)
- [ ] (backlog, not parity) Nickname, chat colour override, private note and hide-in-chat on the sheet, as the desktop card has (d:components/UserProfileCard.tsx:L530-L640)
- [ ] (backlog, not parity) Kick and YouTube chatter profiles in their rooms (desktop has `kick_user_profile` / `youtube_user_profile`), with moderation authority home-only
- [ ] (backlog, not parity) Moderator history in the sheet for channels the viewer moderates (Twitch mod logs), and a deep archive lookup
- [ ] (backlog, not parity) Message rows in the sheet rendered with emotes and badges, tap a row to jump to it in chat
- [ ] (backlog, not parity) The StreamNook member reveal (number decoding out of the cipher, tier label, atmosphere backdrop) on a long-press of a member's badge, as desktop shows on hover (s:components/StreamNookBadge.tsx:L401-L460)
- [ ] (backlog, not parity) Live atmosphere previews in the picker (the animated backdrop, and a sample chat row with the wash) instead of a static swatch (pref: visual-settings-get-previews.md)
- [ ] (backlog, not parity) Locked atmosphere and badge tiles open the support page with the tier and handle prefilled, as desktop does
- [ ] (backlog, not parity) StreamNook Identity loadout editor on the phone (choose which chat-client badges you display), mirroring desktop AccountIdentityEditor
- [ ] (backlog, not parity) Pull to refresh on the Cosmetics screen (`.refreshable`)
- [ ] (backlog, not parity) Haptics: `UISelectionFeedbackGenerator` when equipping a cosmetic, `UINotificationFeedbackGenerator(.error)` on a rolled-back equip
- [ ] (backlog, not parity) Context menu on a chatter name with a profile preview (`.contextMenu(menuItems:preview:)`) for a peek without opening the sheet
- [ ] (backlog, not parity) A Home Screen / Lock Screen widget showing your avatar, painted name and worn badges
- [ ] (backlog, not parity) App Intent "Change my 7TV paint" for Shortcuts
