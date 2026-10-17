# Parity contract: Chat view

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

Scope note: the phone renders chat rows with the SHARED desktop row components (`d:components/ChatMessageList.tsx`, `d:components/ChatMessage.tsx`), driven by the phone pane `m:chat/MobileChatPane.tsx`. Those two desktop files carry no `IS_MOBILE` branch of their own. The phone path differs from desktop only through the props the phone passes (no `isModerator`, no `broadcasterId`, no `onMessageCopy`, no-op badge and emote callbacks) and through shared platform switches (`IS_MOBILE` in `s:components/ui/Tooltip.tsx`, `s:features.ts`, `s:services/emoteService.ts`, `s:stores/chatConnectionStore.ts`, `s:stores/chatUserStore.ts`). Every row line below cites the desktop renderer with `d:` because that IS the phone behavior.

## 1. Pane structure and placement

- [x] The chat pane is one vertical column: the chat tab strip (only with 2 or more rooms, see § 10), then the message list filling all remaining height, then the composer (absent in the landscape overlay) (m:chat/MobileChatPane.tsx:L638-L713) (iPhone: VStack of tab strip, a UIKit chat list island via `UIViewControllerRepresentable`, composer) (iPhone now: seen with one room: list then composer; the tab strip is not built)
- [x] The phone pane is deliberately not the desktop ChatWidget: it renders the shared message list plus a touch composer over N concurrent rooms, without mod rooms or MultiNook (m:chat/MobileChatPane.tsx:L1-L9) (iPhone: native chat screen, no MultiNook surface)
- [x] The pane renders whichever tab is active; switching tabs re-renders an already connected room and never reconnects (m:chat/MobileChatPane.tsx:L91-L105, m:chat/chatTabsStore.ts:L1-L12) (iPhone: one Rust subscription per open room; the list swaps its data source) (iPhone now: switching rooms re-renders the kept rows; the log shows no reconnect)
- [~] The stream-following tab is re-pointed whenever the playing stream's login, id, display name, avatar or platform changes (m:chat/MobileChatPane.tsx:L70-L89) (iPhone: observe the current stream and call the Rust tab sync) (iPhone now: seen pointed at the opened stream; a stream switch was not driven)
- [x] A Kick stream's room is `kick:slug`, never a same-named Twitch room (m:chat/MobileChatPane.tsx:L70-L73) (iPhone: provider-keyed room) (iPhone A9: seen 2026-10-16 (A9): tabs `kick:absi*`)
- [ ] Chat stays mounted (hidden, not torn down) in the mini player and in PiP, so folding, rotating or entering PiP never drops the chat connection; the column layout itself is watch-player's (m:screens/WatchScreen.tsx:L924-L942) (iPhone: keep the chat view model alive across mini player and PiP; see watch-player.md)
- [x] The landscape overlay mounts this same pane in read-only mode (see § 16) (m:screens/WatchScreen.tsx:L1230) (iPhone: same chat list with a read-only flag)
- [ ] The list reserves 40 px of empty space above the oldest scrollback row (the list's default `headerInset`; the phone passes neither `headerSpaceReserved` nor a measured inset) (d:components/ChatMessageList.tsx:L264, d:components/ChatMessageList.tsx:L648-L653) (iPhone: 40 pt top content inset on the collection view)
- [x] The list area cannot start text selection or the system long-press callout (`select-none`, `-webkit-touch-callout: none`); chat text is never selectable, the fan's Copy bucket is the way to take a message (m:chat/MobileChatPane.tsx:L641-L657) (iPhone: no text selection on chat rows)
- [x] The synthesized context menu on the list is suppressed so nothing opens behind the fan (m:chat/MobileChatPane.tsx:L507-L511) (iPhone: no `.contextMenu` on chat rows; the long-press fan owns the gesture)

## 2. Chat rows

### 2.1 Row box and type

- [x] Plain row horizontal padding is 12 px each side (`px-3`) (d:components/ChatMessage.tsx:L3255-L3260) (iPhone: cell content insets 12 pt leading and trailing)
- [x] Plain row vertical padding is `max(4, message_spacing / 2)` px top and bottom; the phone first-run `message_spacing` 8 gives 4 px each side (d:components/ChatMessage.tsx:L3165-L3172, r:models/settings.rs:L630-L648) (iPhone: same formula from the Rust settings value)
- [x] Row text size is `chat_design.font_size` px: phone first run 14 (desktop 16), an absent settings block also 14 (r:models/settings.rs:L630-L648, d:components/ChatMessage.tsx:L3430-L3433) (iPhone: `UIFont` at that point size)
- [x] The badge, name and text line has line height 1.625 (d:components/ChatMessage.tsx:L3430-L3433) (iPhone: paragraph `lineHeightMultiple` 1.625)
- [ ] The line container carries `chat_design.font_weight` (default 400) (d:components/ChatMessage.tsx:L3516-L3522) (iPhone: base weight from settings)
- [x] The message body itself is weight `var(--chat-body-weight)`, 300 by default and lowered for denser theme fonts (d:components/ChatMessage.tsx:L3617, s:styles/globals.css:L2410-L2414) (iPhone: body run light (300) unless the theme font says otherwise)
- [x] Body text colour is `--color-text-primary` and wraps anywhere (`break-words`) (d:components/ChatMessage.tsx:L3617) (iPhone: theme label colour, character wrap for long tokens)
- [x] Badges, name and body flow inline, so a wrapped line starts at the left edge under the badges rather than indented under the name (d:components/ChatMessage.tsx:L3421-L3433) (iPhone: one attributed string per row with badges and emotes as text attachments, or TextKit 2 runs)
- [x] A tall emote grows the line without moving the name away from the badges; the badge group's centre sits 0.38 em above the text baseline at every badge scale (d:components/ChatMessage.tsx:L3426-L3429, s:styles/globals.css:L3285-L3296) (iPhone: attachment `bounds.origin.y` set so badge centres sit 0.38 em above baseline)
- [ ] Dividers between rows are off by default (`chat_design.show_dividers` false); when on, a 1 px `--color-border-subtle` line sits at the TOP of each row so the newest row never has a bottom line (d:components/ChatMessage.tsx:L3206-L3212, r:models/settings.rs:L555) (iPhone: optional 1 pt top hairline per cell)
- [ ] Alternating backgrounds are off by default (`chat_design.alternating_backgrounds` false); when on, every even row in the list gets `--color-surface` (s:styles/globals.css:L2679-L2685, d:components/ChatMessageList.tsx:L650) (iPhone: even index cells get the surface colour)
- [x] A tap never leaves a highlight on a row (the hover tint only applies to pointer hover) (d:components/ChatMessage.tsx:L3257) (iPhone: no cell selection highlight)
- [ ] History rows from the join backfill draw at `chat_design.backfill_opacity` percent (default 100, clamped 30 to 100); live rows stay fully opaque (d:components/ChatMessageList.tsx:L139-L145, d:components/ChatMessageList.tsx:L635, s:styles/globals.css:L7091-L7095) (iPhone: cell alpha from the Rust `from_backfill` flag)

### 2.2 Arrival animation

- [x] New live rows animate in per `chat_design.message_entrance` (phone first run `slide`; absent means `none`) (s:styles/globals.css:L3298-L3328, r:models/settings.rs:L643) (iPhone: animate inserted cells)
- [ ] `fade`: opacity 0 to 1 over 220 ms ease-out (s:styles/globals.css:L3307-L3322) (iPhone: alpha animation 0.22 s)
- [x] `slide`: from x -8 px and opacity 0 over 260 ms `cubic-bezier(0.16,1,0.3,1)` (s:styles/globals.css:L3310-L3325) (iPhone: `UIViewPropertyAnimator` with that curve)
- [ ] `rise`: from y +6 px and opacity 0 over 240 ms `cubic-bezier(0.22,1,0.36,1)` (s:styles/globals.css:L3314-L3328) (iPhone: same)
- [x] Only the last three rows can play the animation, so a batch arriving at once (a tab switch, a held backlog) stays still; history rows never animate (s:styles/globals.css:L3298-L3306) (iPhone: animate only the newest 3 inserted live cells)
- [ ] The OS reduced-motion preference turns arrival animation off (s:styles/globals.css:L3329-L3333) (iPhone: skip when `UIAccessibility.isReduceMotionEnabled`)

### 2.3 Names and name colour

- [x] The author name is bold (700), in the chatter's colour or 7TV paint, and may break inside itself so a very long handle never pushes the body onto its own line (d:components/chat/StyledChatName.tsx:L60-L88) (iPhone: bold name run with character wrapping)
- [ ] The shown name is Rust's stamped `shown_name`, with the viewer's per-user nickname override applied on top (keyed the way the profile card saves it) (d:components/ChatMessage.tsx:L1047-L1059) (iPhone: Rust row field plus the Rust override map)
- [ ] With `chat_design.show_at_sign` on (default off) the name reads `@name` (d:components/ChatMessage.tsx:L1060-L1067) (iPhone: "@" prefix)
- [ ] Separator after the name per `chat_design.username_separator` (default `none`; else `:` `·` `›` `|` `(en dash glyph)`), bold in the accent colour; the colon hugs the name, the others sit 0.35 em after it (d:components/chat/StyledChatName.tsx:L7-L16, d:components/chat/StyledChatName.tsx:L128-L135) (iPhone: separator run)
- [x] `chat_design.username_style` `plain` (default) draws only the name (d:components/chat/StyledChatName.tsx:L50-L57) (iPhone: plain)
- [ ] `bar` draws a 2 px wide, 0.95 em tall accent bar with 1 px radius 0.4 em before the name (d:components/chat/StyledChatName.tsx:L102-L108) (iPhone: attachment)
- [ ] `dot` draws a 0.45 em accent dot 0.4 em before the name (d:components/chat/StyledChatName.tsx:L109-L115) (iPhone: attachment)
- [ ] `chip` draws the name in a capsule: accent 16 percent fill, 1 px inset ring at 34 percent, radius 0.5 em, padding 0.05 em by 0.45 em (d:components/chat/StyledChatName.tsx:L60-L73) (iPhone: rounded background behind the name run)
- [ ] `brackets` wraps the name in accent-coloured `[` and `]` (d:components/chat/StyledChatName.tsx:L93-L96) (iPhone: bracket runs)
- [ ] The accent colour is the chatter's colour, or the theme accent when `chat_design.username_accent_source` is `theme` (default `user`) (d:components/ChatMessage.tsx:L1319-L1323) (iPhone: same)
- [ ] Base name colour priority: the viewer's per-user colour override, else the IRC colour, else `#9147FF` (d:components/ChatMessage.tsx:L1086-L1092, s:services/twitchChat.ts:L283) (iPhone: Rust resolves, see moves)
- [x] Readable name colours (`chat_design.name_color_adjustment`, default `hsl_loop`): when a hex colour's YIQ brightness `(r*299 + g*587 + b*114)/1000` is on the wrong side of 128 for the theme, its HSL lightness moves toward white (dark theme) or black (light theme) by a tenth of the remaining distance per step, at most 20 steps, keeping hue and saturation (s:utils/nameColor.ts:L1-L114) (iPhone: Rust pure function, see moves) (iPhone now: computed in Rust (`chat_row_view::readable_name_color`))
- [ ] `name_color_adjustment` `off` shows the colour as sent; a paint, CSS variable or non-hex value is never adjusted (s:utils/nameColor.ts:L82-L94, s:hooks/useNameColor.ts:L1-L28) (iPhone: same)
- [ ] An override colour is adjusted too (a navy override is as hard to read as a navy Twitch colour) (d:components/ChatMessage.tsx:L1086-L1092) (iPhone: same)

### 2.4 Badges

- [x] Badge boxes are `1.4em * chat_design.badge_scale` square (about 20 px at 14 px text; scale default 1, clamped 0.5 to 2.5) (s:styles/globals.css:L3270-L3275, d:components/ChatMessageList.tsx:L636) (iPhone: attachment size from font size times scale)
- [x] Badges sit 4 px apart with 6 px between the group and the name (d:components/ChatMessage.tsx:L3472-L3477, s:styles/globals.css:L3291-L3296) (iPhone: attachment spacing)
- [~] Badge order: Twitch badges in Rust's chat order (role, subscription, earned, global), then the 7TV badge, then other third-party badges, then the StreamNook badge rightmost by the name (d:components/chat/ChatBadgeIcons.tsx:L76-L166) (iPhone: same order from the Rust row) (iPhone now: Twitch then 7TV seen; third-party and StreamNook badges are not in the Rust view yet (rust-requests.md))
- [ ] The StreamNook badge is inset 0.09 em in its box so it reads the same weight as art with a transparent margin (s:styles/globals.css:L3277-L3283) (iPhone: inner padding on that attachment)
- [x] Channel-scoped Twitch badges (subscriber, bits and the like) resolve against the message's source room (`source-room-id` for shared chat, else `room-id`), never a shared global cache (d:components/ChatMessage.tsx:L778-L799, d:components/chat/ChatBadgeIcons.tsx:L36-L65, s:services/twitchBadges.ts:L190-L217) (iPhone: Rust resolves badge URLs per row, see moves) (iPhone now: resolved in Rust; seen on a live room)
- [ ] Global Twitch badges are served from the on-disk badge cache, queued on first sight (d:components/chat/ChatBadgeIcons.tsx:L54-L66) (iPhone: Rust asset cache path)
- [ ] Third-party badges use their 2x image, never the 4x animated file, in a 20 px box (d:components/chat/ChatBadgeIcons.tsx:L146-L160) (iPhone: request the 2x asset)
- [ ] A badge whose image fails to load is hidden, never a broken icon (d:components/chat/ChatBadgeIcons.tsx:L104-L112) (iPhone: hide attachment on failure)
- [ ] `chat_design.show_badges` off (default on) hides Twitch badges (s:utils/chatterBadges.ts:L49-L74) (iPhone: Rust filter)
- [ ] `chat_design.show_third_party_badges` off (default on) hides 7TV, third-party and StreamNook badges (s:utils/chatterBadges.ts:L49-L74) (iPhone: Rust filter)
- [ ] `chat_design.hidden_badge_providers` (default empty) hides named sources: `7tv`, `streamnook`, a provider name, or `homies-custom` apart from the Homies role badges (s:utils/chatterBadges.ts:L22-L44, d:components/ChatMessage.tsx:L1138-L1163) (iPhone: Rust filter)
- [x] Kick rows carry baked badge art, Kick role badges without art fall back to bundled icons; YouTube rows show only badges that ship an image (s:services/twitchChat.ts:L200-L223) (iPhone: Rust supplies URLs; bundled Kick icons as assets) (iPhone A9: seen 2026-10-16 (A9): Rust stamps Kick rows; role badges as PNG data URIs (k-live-2.png))
- [x] Tapping a Twitch badge does nothing on the phone (no-op handler) (m:chat/MobileChatPane.tsx:L668) (iPhone: no tap action)
- [x] Tapping a 7TV badge sets the desktop badge overlay state, which the phone never renders, so nothing visible happens (d:components/chat/ChatBadgeIcons.tsx:L115-L145, s:utils/openBadgesInMain.ts:L43-L51) (iPhone: no tap action; see Decisions for Brandon)
- [ ] A shared-chat row leads its badges with the source channel's avatar at `round(font_size * 1.3)` px; tapping it switches the player to that channel's stream (d:components/ChatMessage.tsx:L3479-L3503) (iPhone: avatar attachment; tap starts that stream)

### 2.5 Emotes and emoji

- [x] Inline emote height `2em * chat_design.emote_scale` (28 px at 14 px text, scale default 1), max width `9em * scale` (d:components/ChatMessage.tsx:L1530-L1544) (iPhone: attachment sized from font size; Rust should ship emote width and height so rows lay out once) (iPhone now: Rust ships 7TV and Twitch sizes; BTTV and FFZ lay out square until loaded (rust-requests.md))
- [x] Horizontal margin around each emote is `chat_design.emote_margin` rem per side (default 0.125 rem, 2 px) (d:components/ChatMessageList.tsx:L586-L587, d:components/ChatMessage.tsx:L1544) (iPhone: kern around attachments)
- [x] Emotes are vertically centred on the line (d:components/ChatMessage.tsx:L1529) (iPhone: attachment centred)
- [x] The phone fetches the smallest 7TV tier covering `28 * emote_scale * devicePixelRatio` px (1x 32, 2x 64, 3x 96, 4x 128) (s:services/emoteService.ts:L131-L179) (iPhone: same rule with `UIScreen.scale`; scale 3 picks 3x) (iPhone now: WebP rather than AVIF for 7TV)
- [~] Emotes draw from the on-disk cache first and queue a miss for caching so the next view is local (d:components/ChatMessage.tsx:L1494-L1506) (iPhone: Rust asset cache gives local files) (iPhone now: the Swift image pipeline keeps its own disk cache in Library/Caches instead of the Rust asset cache)
- [ ] A stale disk file falls back once to the CDN (7TV at the rendered tier with a srcset, others at the base URL) (d:components/ChatMessage.tsx:L1572-L1583) (iPhone: one CDN retry)
- [ ] A final failure replaces the image with the emote's text code (d:components/ChatMessage.tsx:L1584-L1586) (iPhone: text run fallback)
- [x] Emote images load lazily and decode off the main thread (d:components/ChatMessage.tsx:L1521-L1529) (iPhone: background decode, downsample to display size)
- [x] `chat_design.animate_emotes` `always` (default) animates emotes (d:components/ChatMessage.tsx:L1180-L1183) (iPhone: animate)
- [ ] `never` shows the CDN first-frame file (d:components/ChatMessage.tsx:L1498-L1519) (iPhone: show first frame)
- [ ] `hover` behaves like `never` on a phone, because a touch never sets the hover flag (d:components/ChatMessage.tsx:L3294-L3298) (iPhone: treat `hover` as `never`)
- [x] Every copy of one animated emote on screen shows the same frame, at full frame rate (pref: never-drop-fps.md) (d:components/ChatMessage.tsx:L1518) (iPhone: one shared `CADisplayLink` clock for all emote layers) (iPhone now: one CADisplayLink clock; 120 Hz measured)
- [ ] 7TV personal emotes render as images unless `chat_design.show_personal_emotes` is off (default on), then they read as their text (d:components/ChatMessage.tsx:L1448-L1451) (iPhone: Rust `is_personal` flag)
- [x] Zero-width emotes (Rust `is_zero_width`, from 7TV `data.flags & 256`) stack over the previous emote or emoji in one cell, across any run of spaces (d:components/ChatMessage.tsx:L1902-L1936, r:services/irc_service.rs:L897) (iPhone: overlay layers on the base; Rust should ship grouped segments, see moves) (iPhone now: grouped in Rust)
- [x] A zero-width emote at the very start of a message overlays at zero width, shifted left by its own width (d:components/ChatMessage.tsx:L1699-L1709) (iPhone: same)
- [~] Overlay emotes get a 2 px black drop shadow at 50 percent (d:components/ChatMessage.tsx:L1529) (iPhone: layer shadow) (iPhone now: not drawn: an alpha-shaped shadow on animated layers needs an offscreen pass every frame (never-drop-fps))
- [x] BetterTTV prefix modifiers (`MOD_PREFIX`) attach forward across exactly one single space to the next emote or emoji; a doubled space breaks the attachment and the modifier shows as its own art (d:components/ChatMessage.tsx:L1847-L1900) (iPhone: Rust groups) (iPhone now: grouped in Rust)
- [ ] `chat_design.bttv_emote_modifiers` off (default on) renders BetterTTV modifiers as plain emotes (d:components/ChatMessage.tsx:L1850, d:components/ChatMessage.tsx:L1876-L1884) (iPhone: same switch)
- [x] FFZ modifiers attach backward and apply only while `chat_design.ffz_emote_effects` is on (default on) (d:components/ChatMessage.tsx:L1942-L1953) (iPhone: same switch)
- [~] Modifier effects: flips and rotations as transforms, colour effects as filters, animated effects as classes, slide as a clipped marquee (d:components/ChatMessage.tsx:L123-L168) (iPhone: `CATransform3D`, Core Image filters, layer animations) (iPhone now: flips, turns, spin, shake, bounce and jam built; colour filters (greyscale, sepia, cursed, hyper red, rainbow) have no Core Animation filter on iOS and draw unfiltered)
- [ ] BetterTTV z! pulls the group 4 px left, except as the first element of a message (d:components/ChatMessage.tsx:L169-L178) (iPhone: negative kern)
- [x] FFZ GrowX stretches the base emote to double its natural width, capped at `256px * scale` (d:components/ChatMessage.tsx:L210-L238) (iPhone: measured width times 2)
- [ ] BetterTTV wide stretches to a fixed 4:1 box, capped at `512px * scale`; rotation wins over widening (d:components/ChatMessage.tsx:L240-L259, d:components/ChatMessage.tsx:L1969-L1973) (iPhone: same)
- [ ] A rotated emote is squared to `1.75rem * scale` before turning so it never spills vertically (d:components/ChatMessage.tsx:L1545-L1547) (iPhone: same)
- [x] Hidden-bit modifiers draw no image and only lend their effect to the group (d:components/ChatMessage.tsx:L1984-L1989) (iPhone: same)
- [x] Tapping a 7TV emote sets the desktop emote spotlight state, which the phone never renders, so nothing visible happens; other emotes ignore taps (d:components/ChatMessage.tsx:L1550-L1557) (iPhone: no tap action; see Decisions for Brandon)
- [x] No hover cards anywhere on the phone (emotes, badges, names, links): every shared tooltip renders its child untouched under `IS_MOBILE` (s:components/ui/Tooltip.tsx:L63-L67) (iPhone: no tooltips)
- [~] Emoji follow `chat_design.emoji_style` (default `apple`): Apple images from the Rust-baked URL via the emoji cache; `google`, `twitter`, `facebook` from their CDNs; `system` as plain text (d:components/ChatMessage.tsx:L1714-L1724) (iPhone: native Apple Color Emoji glyphs serve `apple` and `system`; other sets are images) (iPhone now: Apple glyphs natively for `apple` and `system`; the other emoji sets are not drawn yet)
- [x] Emoji images are 20 px square (d:components/ChatMessage.tsx:L1727-L1731) (iPhone: 20 pt)
- [x] A missing Apple emoji image retries once with `-fe0f` appended, then shows the native glyph (d:components/ChatMessage.tsx:L1732-L1750) (iPhone: native glyph)
- [ ] Kick, YouTube and TikTok text is split into emoji image segments on the page so those rows match Twitch rows (d:components/ChatMessage.tsx:L264-L322) (iPhone: Rust tokenises emoji for every provider, see moves)
- [ ] A local echo row without Rust segments resolves emote names against the loaded sets, Twitch first, then 7TV, BTTV, FFZ, Kick, YouTube (d:components/ChatMessage.tsx:L324-L370, d:components/ChatMessage.tsx:L870-L884) (iPhone: Rust stamps own messages, see moves) (iPhone now: Rust stamps own rows (`build_own_chat_message`); needs sign-in to verify)
- [ ] A row rendered before the room's emote set loaded repaints once the set lands (d:components/ChatMessage.tsx:L613-L618) (iPhone: Rust re-stamps segments; the cell reconfigures) (iPhone now: `HISTORY_SEGMENTS` patches carry views; needs sign-in to verify)

### 2.6 Gigantified emotes, cheermotes and chat GIFs

- [x] A "Gigantify an Emote" message (`gigantified-emote-message`) lifts its LAST plain emote (with its overlays and modifiers) out of the text into a block below the body (d:components/ChatMessage.tsx:L2017-L2050, d:components/ChatMessage.tsx:L3653-L3660) (iPhone: block image under the text)
- [x] The giant emote is `8em * emote_scale` tall (112 px at 14 px), at most `24em * scale` wide, using 4x art (Twitch 4.0, 7TV 4x, BTTV 3x, FFZ /4) (d:components/ChatMessage.tsx:L181-L208) (iPhone: same sizes)
- [ ] A failed 4x giant retries once with the inline URL (d:components/ChatMessage.tsx:L1564-L1571) (iPhone: one retry)
- [x] Giant alignment follows `chat_design.giant_emote_align` (default `center`; `left`, `right`, or `inline` to keep it in the text at normal size) (d:components/ChatMessage.tsx:L1080-L1085, d:components/ChatMessage.tsx:L2027-L2030) (iPhone: same) (iPhone now: center seen)
- [ ] `chat_design.giant_emotes` off (default on) disables the lift (d:components/ChatMessage.tsx:L3174-L3180) (iPhone: same)
- [x] Cheermotes draw the animated cheer image 28 px tall followed by the bit count in bold 14 px in the tier colour (default `#979797`) (d:components/ChatMessage.tsx:L1756-L1777) (iPhone: attachment plus coloured run)
- [ ] Twitch chat GIFs draw inline 80 px tall times `chat_design.gif_scale` (default 1, Rust clamps 0.5 to 2), width following the asset, radius 6 px, top-aligned so the name stays on the first line (d:components/ChatMessage.tsx:L1807-L1830, d:components/ChatMessageList.tsx:L631-L633, r:models/settings.rs:L314-L321) (iPhone: attachment 80 pt times scale)
- [ ] A GIF URL is used exactly as sent and never enters the emote cache (d:components/ChatMessage.tsx:L1779-L1786) (iPhone: load the URL directly)
- [ ] With `chat_design.show_chat_gifs` off (default on), or `animate_emotes` `never` / `hover`, a GIF shows as a chip: "GIF" in semibold then the description, 12 px, max width 256 px, white/10 border, white/5 fill, radius 6 px (d:components/ChatMessage.tsx:L1787-L1806) (iPhone: chip button)
- [ ] Tapping the GIF chip reveals that one GIF in that row (d:components/ChatMessage.tsx:L1798) (iPhone: per-row reveal)
- [ ] A GIF whose asset fails shows Twitch's bracketed description instead (d:components/ChatMessage.tsx:L1822-L1826) (iPhone: text fallback)

### 2.7 Links and link previews

- [x] URLs (`https://`, `http://`, `www.`) in text become links in `text-info` (or `chat_design.link_color` when set), underlined unless `chat_design.link_underline` is off (default on) (d:components/ChatMessage.tsx:L2053-L2097) (iPhone: link attribute)
- [x] With `chat_design.shorten_links` on (default) the label drops the scheme and `www.` and truncates the path past 40 characters with an ellipsis, always keeping the full host (d:components/ChatMessage.tsx:L2071-L2072, s:services/linkPreviewService.ts:L175-L189) (iPhone: Rust supplies the label, see moves) (iPhone now: label made in Rust (`chat_row_view::link_label`))
- [ ] Tapping a link opens it in the system browser (d:components/ChatMessage.tsx:L2084-L2093) (iPhone: `UIApplication.shared.open`)
- [ ] Link previews (`chat_design.link_previews`, default on) take at most 2 trusted and 2 untrusted links per message (d:components/ChatMessage.tsx:L1019-L1039, s:services/linkPreviewService.ts:L130-L170) (iPhone: Rust extracts)
- [ ] Trusted hosts expand automatically: YouTube, Twitch, X/Twitter, imgur, GIPHY, Tenor, Reddit, GitHub, Streamable, Kick, Spotify, SoundCloud, TikTok, Discord, Steam, Instagram, nuuls, catbox, uguu, Bluesky, Vimeo, Bandcamp, plus `chat_design.link_preview_trusted_domains` (s:services/linkPreviewService.ts:L46-L100) (iPhone: Rust allowlist)
- [ ] Untrusted links are never fetched passively (s:services/linkPreviewService.ts:L46-L49) (iPhone: no fetch until asked)
- [ ] Clean mode (`chat_design.link_preview_keep_link` off, default) hides the inline URL of a trusted link because its card is the link; keep-link mode shows both (d:components/ChatMessage.tsx:L1027-L1029, d:components/ChatMessage.tsx:L2064-L2069) (iPhone: same)
- [ ] An untrusted link shows a chip "Load preview · <host>" (12 px, 13 px eye icon) that loads on tap (d:components/chat/LinkPreviewCard.tsx:L290-L313) (iPhone: glass chip, `eye`)
- [ ] Beside it, a shield button (aria-label "Always trust <host>") adds the host to the trusted list and loads the preview (d:components/chat/LinkPreviewCard.tsx:L314-L325) (iPhone: `checkmark.shield` button writing the Rust setting)
- [ ] A loading card is a 56 px skeleton at 50 percent opacity, fetched only within 200 px of the viewport via `invoke fetch_link_preview` (d:components/chat/LinkPreviewCard.tsx:L180-L200, d:components/chat/LinkPreviewCard.tsx:L328-L346, s:services/linkPreviewService.ts:L205-L212) (iPhone: fetch on cell display)
- [ ] An untrusted link that resolved to nothing shows "No preview available · <host>" (d:components/chat/LinkPreviewCard.tsx:L348-L361) (iPhone: same text)
- [ ] A trusted link with no preview falls back to a link chip in clean mode, nothing in keep-link mode (d:components/chat/LinkPreviewCard.tsx:L271-L287) (iPhone: same)
- [ ] Card kinds: YouTube video, YouTube channel, Instagram, Twitch clip, Twitch VOD, Discord invite, Steam, Spotify, image, tweet, generic media; max width 448 px, glass panel (d:components/chat/LinkPreviewCard.tsx:L368-L845) (iPhone: native card views)
- [ ] Tapping a clip card opens the phone clip sheet (d:components/chat/LinkPreviewCard.tsx:L26-L47, m:MobileApp.tsx:L170-L175) (iPhone: clip `.sheet`)
- [ ] Tapping a VOD card plays it in the in-app player (d:components/chat/LinkPreviewCard.tsx:L49-L69) (iPhone: push the VOD player)
- [ ] Tapping any other card opens the link externally (d:components/chat/LinkPreviewCard.tsx:L244-L262) (iPhone: `UIApplication.open`)
- [ ] Previews are hidden while streamer mode is active (d:components/ChatMessage.tsx:L3665) (iPhone: same)

### 2.8 Mentions and highlights (Rust stamps)

- [x] Mention-of-me, reply-to-me, highlight-rule match and built-in tint are decided once per message by the Rust rule engine and stamped on `metadata`; rows only read the stamps (d:components/ChatMessage.tsx:L1189-L1234, r:services/chat_rules.rs:L1060-L1102) (iPhone: read stamps; never match on device)
- [x] An `@name` token (1 to 25 word characters) renders as a mention span with 2 px by 6 px padding and 4 px radius (d:components/ChatMessage.tsx:L2109-L2122, d:components/chat/mentionStyle.ts:L42-L48) (iPhone: Rust should emit mention segments, see moves) (iPhone now: Rust emits mention spans)
- [x] Mention weight `chat_design.mention_weight` (default `medium`), slant `mention_italic` (default `inherit`, italic only inside `/me`) (d:components/chat/mentionStyle.ts:L19-L40) (iPhone: same)
- [ ] Mention shape `mention_style` `plain` (default) or `pill` (box tinted with the mention colour at 16 percent) (d:components/chat/mentionStyle.ts:L50-L57) (iPhone: rounded background)
- [~] Mention colour: `mention_text_color` when set, else the mentioned user's adjusted name colour, or their 7TV paint when `paint_mentions_in_body` is on (default on) (d:components/ChatMessage.tsx:L510-L534, d:components/chat/mentionStyle.ts:L59-L62) (iPhone: same) (iPhone now: a seen chatter's readable colour from Rust; paints in mentions not yet (rust-requests.md))
- [ ] A mentioned user not yet seen in chat is looked up with `invoke get_user_by_login`, cached 10 minutes (misses 60 s, at most 500 names, concurrent lookups shared), then their paint fetched (d:components/ChatMessage.tsx:L423-L508) (iPhone: Rust resolves, see moves)
- [ ] Tapping a mention opens that user's profile sheet (d:components/ChatMessage.tsx:L536-L569) (iPhone: tap on the mention run)
- [x] A row mentioning me gets a 4 px left border in `chat_design.mention_color` (default `#ff4444`) (d:components/ChatMessage.tsx:L3186-L3192, d:components/ChatMessage.tsx:L3265-L3275) (iPhone: 4 pt leading bar)
- [x] With `mention_animation` on (default), that row flashes from 50 percent of the colour to transparent over 2 s ease-out (s:styles/globals.css:L3185-L3212) (iPhone: 2 s background fade)
- [ ] A reply to me gets the same bar in `chat_design.reply_color` (default `#ff6b6b`) and a 30 percent to transparent 2 s flash (d:components/ChatMessage.tsx:L3193-L3195, s:styles/globals.css:L3214-L3233) (iPhone: same)
- [ ] A highlight-rule match flashes in the rule's colour from 35 percent over 2 s and keeps a 4 px left border; mentions and replies take precedence over it (d:components/ChatMessage.tsx:L3196-L3198, s:styles/globals.css:L3235-L3254) (iPhone: same precedence)
- [ ] `chat_highlights.appearance.display_style` `standard` (default) paints a left-to-right tint from `opacity` percent (default 20) through half that to transparent, plus the border; `border` shows only the border; `none` neither (d:components/ChatMessage.tsx:L3222-L3253) (iPhone: gradient layer)
- [ ] A highlight rule with a sound plays it once per match, at most once per `cooldown_ms` (default 3000) per rule, never for rows older than 5 s, never in streamer mode (d:components/ChatMessage.tsx:L1236-L1252, s:utils/notificationSound.ts:L180-L216) (iPhone: AVAudioPlayer in an ambient, mixing category; Rust decides, see moves)
- [~] Window-title flash on highlight is desktop-only (d:components/ChatMessage.tsx:L1254-L1270) (iPhone: iOS has no window title; nothing replaces it)
- [ ] A NEW live mention of me buzzes once (20 ms) when the phone pref `mentionHaptic` is on (default on) (m:chat/MobileChatPane.tsx:L111-L147, m:ui/haptics.ts:L35-L38) (iPhone: `UIImpactFeedbackGenerator(.medium)`)
- [ ] The buzz skips my own messages, rows older than 8000 ms, the first pass over a room and every reload backfill, and anything while the app is hidden (m:chat/MobileChatPane.tsx:L111-L147) (iPhone: driven by a Rust mention event, see moves)
- [x] The desktop mention SOUND (Rust `chat-mention-ping`) is not wired on the phone (r:services/mention_ping.rs:L1-L33, s:hooks/useMentionPing.ts:L19-L23) (iPhone: haptic only for parity; sound is backlog)

### 2.9 Built-in event tints and labels

- [x] Built-in tints come from Rust's `metadata.built_in` stamp with precedence raider, returning chatter, first-time chatter, self (d:components/ChatMessage.tsx:L2253-L2269) (iPhone: Rust stamp)
- [x] Defaults: first-time chatter on in `#a855f7`; raider `#ef4444`, returning `#22d3ee` and self `#facc15` off (d:components/ChatMessage.tsx:L2270-L2286) (iPhone: Rust defaults)
- [x] The label sits right-aligned on its own line above the row, 12 px at 60 percent opacity in the tint colour: "First message in chat", "Returning chatter", "Raid", "You" (d:components/ChatMessage.tsx:L3341-L3351) (iPhone: trailing caption line)
- [x] `/announce` rows tint by `msg-param-color`: BLUE `#3b82f6`, GREEN `#22c55e`, ORANGE `#f97316`, PURPLE `#a855f7`, PRIMARY or other `chat_design.mention_color` (fallback `#9147ff`); label "Announcement" (d:components/ChatMessage.tsx:L2287-L2298) (iPhone: Rust should stamp it, see moves)
- [ ] Low-trust chatters (shown to moderators): `#ef4444` "Restricted user" or `#f59e0b` "Monitored user", when no other tint applies (d:components/ChatMessage.tsx:L2299-L2304) (iPhone: same)
- [ ] First-time rows can wear a ring instead of the wash (`chat_highlights.built_in.first_time_chatter.style` `ring`, optional `fill`) (d:components/ChatMessage.tsx:L3239-L3245) (iPhone: border layer)
- [ ] First-time rows can play a glint (`animation`), once or looping (`animate_repeat`), off under reduced motion (d:components/ChatMessage.tsx:L3242-L3245, s:styles/globals.css:L3440-L3443) (iPhone: shimmer layer)

### 2.10 Replies

- [x] With `chat_design.reply_style` `full` (default), a reply starts with a context line: 2 px left border at text-secondary 40 percent, 8 px left padding, 6 px gap below (d:components/ChatMessage.tsx:L3362-L3375) (iPhone: quote line above the message)
- [x] The context line holds a reply arrow (1.1 em), the parent author's shown name in semibold, and the parent text truncated to one line, all at `round(font_size * 0.85)` px (12 px at 14) in text-secondary (d:components/ChatMessage.tsx:L3376-L3404) (iPhone: one-line quote)
- [x] The quoted text shows emotes and emoji 1.35 em tall from Rust's `parent_segments`; rows without them fall back to matching names against the room's sets, only for rows from the room's own platform (d:components/ChatMessage.tsx:L886-L901, d:components/ChatMessage.tsx:L3387-L3401) (iPhone: Rust segments) (iPhone now: Rust `parent_spans`)
- [ ] `reply_style` `mention` drops the line and starts the body with `@<parent>` (semibold, 85 percent opacity) (d:components/ChatMessage.tsx:L1071-L1079) (iPhone: prefix run)
- [ ] `reply_style` `off` shows neither (d:components/ChatMessage.tsx:L1071-L1079) (iPhone: none)
- [x] The leading `@parentLogin` Twitch puts at the start of a reply is stripped from the body (s:services/twitchChat.ts:L263-L268) (iPhone: Rust strips, see moves)
- [ ] Tapping the context line pauses chat (forced past the settle window) and glides the list until the parent sits centred (m:chat/MobileChatPane.tsx:L437-L456, s:utils/scrollChatToMessage.ts:L58-L97) (iPhone: `scrollToItem(at:at:.centeredVertically,animated:true)`)
- [ ] The parent then flashes orange (`rgba(255,140,0,0.5)` fading to 0.2 over 2 s) and the mark clears after 2000 ms (m:chat/MobileChatPane.tsx:L457-L461, s:styles/globals.css:L3165-L3183) (iPhone: 2 s highlight on the target cell)
- [ ] Tapping the context line never also starts a reply to the child row (d:components/ChatMessage.tsx:L3366-L3374) (iPhone: the quote's tap consumes the gesture)
- [ ] The jump scrolls only the list and re-measures once after 260 ms, correcting only if off by more than 8 px (s:utils/scrollChatToMessage.ts:L58-L97) (iPhone: collection view scroll; correction harmless)
- [ ] When the parent has left the buffer, an info toast reads "Original message is no longer in chat history" and chat stays paused (m:chat/MobileChatPane.tsx:L452-L456) (iPhone: info toast)

### 2.11 Timestamps

- [x] Timestamps are off by default (`chat_design.show_timestamps` false) (r:models/settings.rs:L566) (iPhone: off)
- [ ] When on, the time sits on its own line above the badges and name: 10 px, tight leading, 2 px below, text-secondary at 50 percent (d:components/ChatMessage.tsx:L3409-L3420) (iPhone: caption line above the row)
- [ ] The strings come pre-formatted from Rust: 12 h `h:mm AM/PM` by default, 24 h `HH:mm` when `chat_design.timestamp_format` is `24h`, with seconds when `show_timestamp_seconds` is on (r:services/irc_service.rs:L4877-L4910, d:components/ChatMessage.tsx:L828-L861) (iPhone: read `formatted_timestamp` / `formatted_timestamp_with_seconds`)
- [ ] Over an Atmosphere that wants frost, the time gets its own small frosted chip (radius 2 px, 4 px padding, 80 percent opacity) (d:components/ChatMessage.tsx:L3410-L3417) (iPhone: tinted capsule)

### 2.12 Cosmetics on rows (detail in profile-cosmetics.md)

- [~] A chatter's 7TV paint draws on their name and on mentions of them; paint shadows follow `cosmetics.paint_shadows` (default `all`); paint rendering is owned by profile-cosmetics.md (d:components/ChatMessage.tsx:L1302-L1313) (iPhone: baked paint per paint id reused across rows, see profile-cosmetics.md) (iPhone now: paints on names seen (gradients drawn once per paint); mentions not yet; only the first paint shadow)
- [~] Paint layer images are cached per layer id and must be the animated file, never 7TV's `_static` still (d:components/ChatMessage.tsx:L1286-L1300) (iPhone: see profile-cosmetics.md) (iPhone now: image paints are not drawn yet; those names keep their colour)
- [x] A StreamNook member's Atmosphere draws its animated wash behind their rows; a member can hide their theme from chat (d:components/ChatMessage.tsx:L1105-L1133, d:components/ChatMessage.tsx:L3300-L3312) (iPhone: see profile-cosmetics.md) (iPhone now: seen on FakeCore rows (aurora, midnight); real members real-account read not seen yet: the phone was signed out by the new user:edit scope)
- [ ] The Major Cologne theme draws its chrome instead of the wash; its frame adds 18 px side padding, 7 px vertical padding and a 36 px minimum row height (d:components/ChatMessage.tsx:L3286-L3309) (iPhone: see profile-cosmetics.md)
- [x] On the phone an Atmosphere that wants frost behind the text gets the tint without a per-row blur (`atm-frost--flat`) (d:components/ChatMessage.tsx:L1164-L1171, s:features.ts:L27-L28) (iPhone: tinted rounded rect, no per-row `UIVisualEffectView`) (iPhone now: seen: a tinted rounded rect behind the text)
- [~] Atmosphere and Cologne rows always paint fully to dodge stale paint ghosts (d:components/ChatMessageList.tsx:L114-L137) (iPhone: WebView compositing workaround, not needed natively: cell reuse redraws correctly)
- [x] The member decode card on name hover is desktop-only (d:components/ChatMessage.tsx:L3610-L3615) (iPhone: none in rows)
- [x] The phone registers every chatter it sees, plus the signed-in user on connect with colour `#9147FF` (Twitch never echoes my own messages), so cosmetics resolve once per chatter and every row reads them (m:chat/MobileChatPane.tsx:L246-L337) (iPhone: Rust resolves on ingest, see moves) (iPhone now: Rust resolves cosmetics on ingest (`chat_row_view`); 7TV badges seen on a live room)
- [~] Cosmetics keys: bare Twitch user id for Twitch, `provider:id` for Kick, YouTube and TikTok, so a Kick id never borrows a Twitch account's paint (m:chat/MobileChatPane.tsx:L300-L314, d:components/ChatMessage.tsx:L918-L930) (iPhone: Rust keys; see moves) (iPhone A9: built: Rust keys; member looks keyed `kick:<id>`, SNKit test `aMemberNeverDressesAKickChatterWithTheSameNumber` (A9))
- [x] A row from a different platform than the room's carries a provider logo at `round(font_size * 0.85)` px first on the line; single-platform rooms (every phone room) show none (pref: platform-marks-only-where-mixed.md) (d:components/ChatMessage.tsx:L3339-L3344, d:components/ChatMessage.tsx:L3434-L3446, m:chat/MobileChatPane.tsx:L673-L675) (iPhone: none in phone rooms)
- [ ] YouTube and TikTok rows lead with the chatter's round avatar at `round(font_size * 1.4)` px unless `chat_design.show_avatars` is off (default on) (d:components/ChatMessage.tsx:L3447-L3470) (iPhone: latent on the phone, no YouTube rooms)

### 2.13 Repeat counter and gift bombs

- [ ] With repeat folding on (`message_repeat.mode`, default `off`), a folded run shows `x<count>` after the body in 12 px semibold tabular figures, coloured `message_repeat.color` or text-secondary, once the count reaches `threshold` (default 2) (d:components/ChatMessage.tsx:L812-L826, d:components/ChatMessage.tsx:L3637-L3646, s:stores/chatConnectionStore.ts:L2871-L2900) (iPhone: trailing count run; Rust folds)
- [ ] A community gift bomb (collapsed by default, `collapse_gift_subs`) lists recipients under the announcement: "<n> of <expected> gifted to" or "Gifted to" (d:components/ChatMessage.tsx:L3014-L3020) (iPhone: same text)
- [ ] Up to 8 recipient names show, each with their own cosmetics and colour, then "+<n> more" in accent that expands the list (d:components/ChatMessage.tsx:L3021-L3036) (iPhone: tappable "+n more")

## 3. Event and system rows

- [x] Event rows (cheer, donation, Super Chat, watch streak, subscription, raid) use 12 px side padding, `max(4, message_spacing / 2)` vertical padding, a 1 px top border `--color-border-subtle`, a 20 px leading icon and a 10 px gap (d:components/ChatMessage.tsx:L2451-L2470) (iPhone: event cell layout)
- [~] Event row dress per `chat_events.event_style`: `cards` (default) the kind's tinted gradient; `outline` a 1 px inset ring in `event_outline_color` or the accent at 62 percent; `plain` no decoration (d:components/chat/eventCard.ts:L1-L20, s:styles/globals.css:L3350-L3360) (iPhone: gradient or border layer) (iPhone now: `cards` seen; `outline` and `plain` not built)
- [ ] Optional event glint `chat_events.event_animation` (`none` default, `sheen`, `pulse`, `chase`), looping with `event_animate_repeat` (d:components/chat/eventCard.ts:L8-L16) (iPhone: shimmer, off under reduced motion)
- [x] Event gradients are static by design: subscription pink, purple, blue, cyan, yellow at 8 to 15 percent; bits purple, blue, cyan, pink at 12 to 18 percent; raid red and orange at 12 to 18 percent; redemption cyan and blue at 10 to 18 percent (s:styles/globals.css:L3445-L3660) (iPhone: `CAGradientLayer` with the same stops)

### 3.1 Cheers

- [x] A cheer is its own card unless `chat_events.cheer_display` is `message` (d:components/ChatMessage.tsx:L2202-L2206) (iPhone: same switch)
- [x] The cheer card's icon is Twitch's animated tier gem `https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/animated/<tier>/2.gif` with tiers 1, 100, 1000, 5000, 10000 (d:components/ChatMessage.tsx:L2394-L2398) (iPhone: Rust stamps the URL, see moves)
- [x] The header reads badges, the tappable name, then " cheered <n> bits" in bold in the tier colour: red `#ff1f1f` at 10000+, blue `#0099fe` 5000+, teal `#1db2a6` 1000+, purple `#9c3ee8` 100+, grey `#979797` below (d:components/ChatMessage.tsx:L2383-L2392, d:components/ChatMessage.tsx:L2472-L2483) (iPhone: same)
- [x] The cheer's message follows in text-secondary, with a gigantified emote lifted below if present (d:components/ChatMessage.tsx:L2484-L2491) (iPhone: same)

### 3.2 Donations and Super Chats

- [ ] Charity donation card: heart icon in the success colour, name, " donated <amount>" in bold success, " to support <charity>" in text-secondary; the amount comes from minor units and `msg-param-exponent` (default 2), currency-formatted (d:components/ChatMessage.tsx:L2498-L2516, d:components/ChatMessage.tsx:L2638-L2672) (iPhone: Rust formats)
- [ ] YouTube Super Chat / Super Sticker card: coin icon, " sent a <amount> Super Chat" or Super Sticker, YouTube's own formatted amount, converted to `chat_events.superchat_currency` when set and a rate is known; the comment shows raw (latent on the phone) (d:components/ChatMessage.tsx:L2517-L2546, d:components/ChatMessage.tsx:L2660-L2683) (iPhone: Rust formats and converts)
- [ ] Shared-chat event rows from another channel show "From Shared Chat - <login>" in 12 px accent; tapping the login switches to that stream, failing with the toast "Failed to switch to <login>'s stream" (d:components/ChatMessage.tsx:L2608-L2636, d:components/ChatMessage.tsx:L2963-L2993) (iPhone: header line; Rust supplies the source login)

### 3.3 Watch streak notices

- [~] Watch streak milestone card (`viewermilestone` category `watch-streak`): a 20 px flame in highlight-orange, badges, the name, then a points icon and "+<points>" in highlight-orange when the milestone paid points (d:components/ChatMessage.tsx:L2690-L2776) (iPhone: `flame.fill` or the bundled flame) (iPhone now: flame, name and "+points" seen; the points glyph is not drawn)
- [x] Its second line reads "Watched <n> consecutive streams and sparked a watch streak!" in text-secondary, then the chatter's message in text-primary (d:components/ChatMessage.tsx:L2777-L2791) (iPhone: same text)

### 3.4 Subscriptions

- [x] Subscription cards cover sub, resub, subgift, submysterygift, anonymous gifts, shared-chat notices and YouTube memberships (d:components/ChatMessage.tsx:L2128-L2149) (iPhone: Rust `msg_type`)
- [x] The icon is a Prime crown for Prime subs, otherwise Twitch's sub-gifter badge art (a gift icon until the badge cache loads) (d:components/ChatMessage.tsx:L183-L193, d:components/ChatMessage.tsx:L2996-L3005) (iPhone: same; SF Symbol `gift.fill` placeholder) (iPhone now: SF Symbols `crown.fill` and `gift.fill`)
- [~] The text is Twitch's `system-msg` with the gifter's and recipient's names tappable and painted with their own cosmetics (matching localized display names too) (d:components/ChatMessage.tsx:L2834-L2931) (iPhone: Rust should stamp name spans, see moves) (iPhone now: the sentence is drawn plain (rust-requests.md))
- [ ] Fallback copy when `system-msg` is missing: "<user> subscribed!", "<user> subscribed for <n> months!", "<user> gifted a subscription to <recipient>!", "<user> is gifting <n> subscriptions to the community!", "An anonymous user is gifting <n> subscriptions to the community!", "An anonymous user gifted a subscription to <recipient>!" (d:components/ChatMessage.tsx:L2933-L2953) (iPhone: same strings)

### 3.5 Raids

- [x] Raid card: people icon in highlight-red, badges, the name, then " is raiding with <n> viewer(s)!" (or " is raiding!" without a count) in bold highlight-red (d:components/ChatMessage.tsx:L3049-L3128) (iPhone: `person.3.fill`)

### 3.6 Wording, hiding and redemptions

- [ ] Viewer wording templates (`chat_events.event_templates` for subscription, gift, cheer, milestone, raid) replace the platform sentence when every token they name is present (d:components/ChatMessage.tsx:L2211-L2231) (iPhone: Rust renders templates, see moves)
- [ ] Categories in `chat_events.hidden_provider_events` (for example `twitch:gift`) are not rendered (d:components/ChatMessageList.tsx:L656-L657, s:utils/chatEvents.ts:L1-L60) (iPhone: Rust filters, see moves)
- [x] Channel points redemption rows use the cyan redemption gradient and a right-aligned 12 px label at 60 percent with a points icon: "Redeemed Highlight My Message", "Sent a Gigantified Emote", "Sent an Animated Message", "Sent a Message in Sub-Only Mode", "Redeemed a channel points reward" (d:components/ChatMessage.tsx:L2170-L2200, d:components/ChatMessage.tsx:L3352-L3361) (iPhone: same labels)
- [ ] An injected no-input redemption row appends its cost: the channel's own points icon (14 px round) or the default glyph, then the cost with separators in highlight-cyan at 90 percent (d:components/ChatMessage.tsx:L3621-L3635) (iPhone: trailing attachment and number)
- [ ] Shared-chat plain rows get a 2 px accent/50 left border and an accent/5 background (d:components/ChatMessage.tsx:L3259) (iPhone: same)
- [ ] `chat_design.hide_shared_chat` (default off) hides rows whose source room differs from the room (d:components/ChatMessageList.tsx:L708-L715) (iPhone: Rust filter)

### 3.7 System rows

- [ ] System rows (author "System") show no name or badges: 12 px side padding, warning/20 hairlines top and bottom, warning/10 background, a 16 px info icon at warning/80, 12 px medium text in the warning colour (d:components/ChatMessage.tsx:L3130-L3162) (iPhone: notice cell, `info.circle`)
- [ ] A `/song` result shows its song card under the system text, indented 24 px (d:components/ChatMessage.tsx:L3155-L3159) (iPhone: card view)
- [ ] 7TV emote changes post system rows while `chat_design.seventv_emote_notices` is on (default): "<actor> switched the 7TV emote set to <name>", "<actor> added the emote <name>", "<actor> removed the emote <name>", "<actor> renamed the emote <old> to <new>"; actor "Someone" when unknown (s:services/seventvEventApi.ts:L87-L103) (iPhone: Rust emits notice rows)
- [ ] Those come from `listen 7tv://emote-set-update`; live paint and badge changes come from `listen 7tv://cosmetic-update` (m:boot/useMobileBoot.ts:L168-L172) (iPhone: Rust events via UniFFI callbacks)
- [x] `/me` rows draw the whole body italic in the author's colour or paint, body weight 300, name weight 700 (d:components/ChatMessage.tsx:L3523-L3567) (iPhone: italic run)

## 4. Moderation display (deleted, timed out, banned)

- [x] A deleted message (CLEARMSG) is marked, never removed, unless `moderation.ignore_clear_chat` is on (default off) (s:stores/chatConnectionStore.ts:L2366-L2373) (iPhone: Rust marks rows) (iPhone now: applied by `SNKit/ChatRoomBuffer`; Rust should own it (rust-requests.md))
- [x] A timeout or ban (CLEARCHAT with a target) marks every buffered message of that Twitch user id with the type and duration (s:stores/chatConnectionStore.ts:L2426-L2460) (iPhone: Rust marks rows)
- [ ] A full chat clear marks nothing (s:stores/chatConnectionStore.ts:L2435) (iPhone: same)
- [x] Moderated look follows `chat_design.deleted_message_style`: phone first run `italic`, absent means `strikethrough` (r:models/settings.rs:L644, d:components/chat/deletedMessage.ts:L1-L6) (iPhone: Rust setting) (iPhone now: italic seen)
- [ ] `strikethrough` strikes the body and dims the row to 50 percent (d:components/chat/deletedMessage.ts:L8-L18, d:components/ChatMessage.tsx:L3260) (iPhone: strikethrough attribute, alpha 0.5)
- [ ] `dimmed` dims the row to 50 percent only (d:components/chat/deletedMessage.ts:L8-L11) (iPhone: alpha 0.5)
- [x] `italic` sets the body italic in text-secondary (d:components/chat/deletedMessage.ts:L13-L18) (iPhone: italic secondary)
- [ ] `keep` shows the row as if nothing happened; `hidden` removes it (d:components/ChatMessageList.tsx:L696-L720) (iPhone: same)
- [ ] Strikethrough rows append "[deleted by mod]", "[timed out for <duration>]" (or "[timed out]"), or "[banned]" in 12 px error at 70 percent (d:components/chat/ModerationTag.tsx:L17-L31) (iPhone: trailing run)
- [x] Italic rows append a pill: "Deleted" (neutral), "Timed out" plus "· <duration>" (warning), or "Banned" (error), 0.78 em, full radius, tone at 12 percent fill with a 24 percent ring, icon 1.1 em (d:components/chat/ModerationTag.tsx:L32-L50, s:styles/globals.css:L2687-L2727) (iPhone: capsule; `trash`, `timer`, `nosign`)
- [x] Durations read like "45s", "10m", "1h 30m", "2d 3h" (s:utils/timeoutRamp.ts:L44-L54) (iPhone: shared formatter from Rust)
- [ ] With `moderation.show_mod_messages` on (default off), system rows say "<user> was timed out for <n>s|m|h|d.", "<user> was banned.", "Chat was cleared by a moderator." (s:stores/chatConnectionStore.ts:L2462-L2479) (iPhone: Rust injects)
- [ ] With it on, deletions say "<user>'s message was deleted by a moderator." or "<user>'s message was deleted (<reason>)." ("a user" when unknown) (s:stores/chatConnectionStore.ts:L2393-L2405) (iPhone: Rust injects)
- [x] The desktop per-row mod toolbar, hover Copy/Pin cluster and drag-to-moderate never appear on the phone (the phone passes no `isModerator`, `broadcasterId` or `onMessageCopy`); moderation is the long-press fan (d:components/ChatMessage.tsx:L1325-L1374, d:components/ChatMessage.tsx:L3677-L3807, m:chat/MobileChatPane.tsx:L658-L677) (iPhone: no per-row buttons)
- [ ] Streamer mode hides rows from restricted (low-trust) users (d:components/ChatMessageList.tsx:L589-L591, d:components/ChatMessageList.tsx:L701-L707) (iPhone: same filter)

## 5. Row interactions on the phone

- [ ] Tap a name: opens the profile sheet with the row's id, login, display name, colour and badges (a worn sub badge answers "subscribed here now"), scoped to the room's channel id and login (m:chat/MobileChatPane.tsx:L465-L481, m:chat/MobileChatPane.tsx:L714-L721) (iPhone: `.sheet` per profile-cosmetics.md)
- [~] Name taps do nothing in non-Twitch rooms (the sheet is Helix-backed and a Kick id would name an unrelated Twitch account) (m:chat/MobileChatPane.tsx:L473-L475) (iPhone: same gate) (iPhone A9: built: ChatPanel gates on `provider == twitch` (A9))
- [ ] Tap a row while PAUSED (not in the landscape overlay, not while the fan is armed, not on a link, button or name): starts a reply to that message (m:chat/MobileChatPane.tsx:L612-L635) (iPhone: tap enabled only while paused)
- [ ] A tap on a live (unpaused) row does nothing, because the row is moving under the finger (m:chat/MobileChatPane.tsx:L612-L618) (iPhone: same)
- [ ] The reply draft is keyed by room: switching tabs hides it, switching back shows it again if still set (m:chat/MobileChatPane.tsx:L212-L217) (iPhone: draft per room key)
- [x] Paused rows carry a `data-tap-reply` marker that no style reads; the hint pill is the only affordance (m:chat/MobileChatPane.tsx:L654-L656) (iPhone: hint pill only)
- [ ] Long press anywhere on a row arms the fan (§ 8) (m:chat/useLongPressDrag.ts:L58-L78) (iPhone: long-press recognizer)

## 6. Scroll behaviour: follow, pause, resume

- [x] Live (unpaused): every new row pins the list to the bottom before it paints (d:components/ChatMessageList.tsx:L399-L429) (iPhone: scroll to bottom inside the same batch update, unanimated)
- [x] A second pin runs 50 ms later to catch late layout (d:components/ChatMessageList.tsx:L431-L445) (iPhone: not needed with self-sizing cells; keep the bottom anchored)
- [x] A room's first load (0 to N rows) always pins, a third time at 150 ms (d:components/ChatMessageList.tsx:L408-L458) (iPhone: initial scroll to bottom)
- [ ] Late height growth re-pins only when the reader was already at the bottom and shows no scroll-up intent (d:components/ChatMessageList.tsx:L361-L393) (iPhone: keep bottom anchoring on content size change)
- [ ] Pause is triggered by intent, not distance: the first touch move with the finger travelling DOWN (revealing older rows) pauses at once; a finger travelling up clears the intent (d:components/ChatMessageList.tsx:L335-L359, m:chat/MobileChatPane.tsx:L363-L367) (iPhone: pan translation sign in `scrollViewDidScroll` while dragging)
- [ ] Pause and resume are idempotent and guarded by a 120 ms settle window so a drag and the scroll it causes cannot fight (m:chat/MobileChatPane.tsx:L42-L45, m:chat/MobileChatPane.tsx:L345-L390) (iPhone: same 120 ms guard)
- [ ] An explicit Resume tap and a reply jump bypass the settle window (m:chat/MobileChatPane.tsx:L426-L430, m:chat/MobileChatPane.tsx:L449-L451) (iPhone: forced transitions)
- [ ] Scrolling back to the bottom resumes: once paused the reader must have been more than 80 px from the bottom, then coming within 30 px resumes live (m:chat/MobileChatPane.tsx:L392-L422) (iPhone: same thresholds from `contentOffset`)
- [ ] A read-back that never leaves the 80 px band stays paused (m:chat/MobileChatPane.tsx:L398-L404) (iPhone: same)
- [ ] Automatic scrolls are never reported as user scrolls (d:components/ChatMessageList.tsx:L461-L494) (iPhone: ignore code-driven offset changes)
- [x] While paused a stack floats centred 8 px above the list bottom, 4 px between items, passing touches through except on the button (m:chat/MobileChatPane.tsx:L678-L693) (iPhone: overlay VStack)
- [x] The top item is a hint pill "Tap a message to reply": 10.5 px text-muted, glass badge, 8 px by 2 px padding, full radius; hidden in the landscape overlay (m:chat/MobileChatPane.tsx:L680-L684) (iPhone: caption capsule)
- [x] Below it, a "Resume live" button: 12 px medium text-primary, an 11 px bold caret-down, 4 px gap, 12 px by 4 px padding, full radius, glass (m:chat/MobileChatPane.tsx:L685-L691) (iPhone: Liquid Glass capsule button with `chevron.down`)
- [x] The phone shows no count of new messages while paused (desktop's pill reads "Chat Paused (<n> new)") (d:components/ChatWidget.tsx:L5610-L5618) (iPhone: no count, matching the phone)
- [ ] Resume glides to the bottom with ease-out cubic over `clamp(distance * 0.4, 200, 420)` ms, re-targeting every frame so rows arriving mid-glide are included (d:components/ChatMessageList.tsx:L538-L572) (iPhone: `CADisplayLink` glide with the same timing)
- [ ] Gaps under 40 px snap instantly; `chat_render.smooth_scroll_on_resume` off (default on) always snaps (d:components/ChatMessageList.tsx:L527-L536) (iPhone: same)
- [ ] A new scroll-up gesture during the glide aborts it (d:components/ChatMessageList.tsx:L545-L553) (iPhone: cancel on drag begin)
- [~] Paused state is keyed by room: switching tabs shows the new room live, and returning never restores a stale pause (m:chat/MobileChatPane.tsx:L197-L205) (iPhone: pause per room key) (iPhone now: built: each room's list is rebuilt live on a switch; a paused switch was not driven)
- [x] Pausing tells the store, which lets that room's buffer grow 150 rows over the cap so the scrollback being read is not trimmed (m:chat/MobileChatPane.tsx:L387, s:stores/chatBufferTrim.ts:L26-L27, s:stores/chatConnectionStore.ts:L3781-L3794) (iPhone: Rust `set_paused(room, bool)`) (iPhone now: 250 rows held at a cap of 100 while paused)
- [ ] On resume, the overflow decays 5 rows per flush from the top instead of being cut at once (s:stores/chatBufferTrim.ts:L28-L29, s:stores/chatBufferTrim.ts:L111-L148) (iPhone: Rust decay)
- [x] No scrollbar marks on the phone (d:components/ChatMessageList.tsx:L606-L609) (iPhone: none)

## 7. Buffer, batching and rendering cost

- [x] Each room keeps `chat_render.message_buffer_cap` rows (default 100), clamped 50 to 300 on the phone (1000 on desktop); a bad value falls back to 100 (s:stores/chatConnectionStore.ts:L1031-L1046) (iPhone: Rust buffer cap)
- [~] Trimming keeps the newest `cap` rows plus up to 30 recent event rows rescued from just above the window (s:stores/chatBufferTrim.ts:L1-L84) (iPhone: Rust trim) (iPhone now: a plain cap without the event rescue (rust-requests.md))
- [ ] A rescued event row is held for at most 600 further live messages (s:stores/chatBufferTrim.ts:L30-L36, s:stores/chatBufferTrim.ts:L63-L84) (iPhone: same)
- [ ] Event rows for this purpose: subs, gifts, raids, unraids, milestones, announcements, bits badge tiers, charity donations, redemptions, any system message (s:stores/chatBufferTrim.ts:L38-L54) (iPhone: Rust classification)
- [x] Rows are batched: one flush per display frame while visible, with a 250 ms timer as the fallback when frames are throttled (s:stores/chatConnectionStore.ts:L700-L740) (iPhone: Rust batches; Swift applies one batch update per frame) (iPhone now: laid out off the main thread, one batch per display frame; 0 dropped frames at 300 rows a second)
- [ ] A freshly joined room holds its first live rows until history lands (or 1500 ms), so history and the live tail paint in one update (s:stores/chatConnectionStore.ts:L1107-L1149) (iPhone: Rust delivers the joined snapshot in one batch)
- [x] Twitch history comes from `invoke load_channel_history`; Kick history from `invoke kick_chat_history`, deduped and placed above live rows (s:stores/chatConnectionStore.ts:L1121-L1135, s:stores/chatConnectionStore.ts:L908-L949) (iPhone: same Rust commands) (iPhone A9: Kick seen 2026-10-16 (A9): 25 history rows laid out)
- [x] A row id renders at most once, even if the buffer holds a duplicate (d:components/ChatMessageList.tsx:L594-L666) (iPhone: diffable data source keyed by row id)
- [x] A row repaints only when its id, its `segments` identity, highlight flag, moderation type or duration, emote set, or source mark changes; a patch that does not replace `segments` does not repaint (d:components/ChatMessage.tsx:L587-L623, d:components/ChatMessageList.tsx:L62-L91) (iPhone: reconfigure a cell only on a new Rust row revision)
- [~] The newest 40 rows always paint fully; older rows are virtualized with an estimated height `round(font_size * 1.5) + (timestamps ? 15 : 0) + 2 * max(4, spacing / 2) + 1` px (d:components/ChatMessageList.tsx:L15-L19, d:components/ChatMessageList.tsx:L287-L309) (iPhone: WebView virtualization, not needed natively: UICollectionView cell reuse with self-sizing cells; keep the rule that a row never visibly changes height after it appears)
- [x] Background tabs keep receiving and buffering every message (phone tabs are never parked), so a switch shows recent history instantly (m:chat/chatTabsStore.ts:L1-L12, s:stores/chatConnectionStore.ts:L3232-L3241) (iPhone: every open room subscribed in Rust) (iPhone now: a hidden room went from 0 to 100 rows while another was on screen)
- [ ] The phone tracks at most 1500 chatters for cosmetics (desktop 8000), evicting the least recently seen beyond 2500 (s:stores/chatUserStore.ts:L740-L753) (iPhone: Rust cosmetic cache cap)

## 8. Long-press fan-out

### 8.1 Arming

- [ ] Hold a row for 420 ms to arm the fan (m:chat/useLongPressDrag.ts:L22, m:chat/useLongPressDrag.ts:L69-L75) (iPhone: `UILongPressGestureRecognizer.minimumPressDuration = 0.42`)
- [ ] Moving more than 10 px before the hold fires cancels it (it was a scroll) (m:chat/useLongPressDrag.ts:L23-L24, m:chat/useLongPressDrag.ts:L80-L91) (iPhone: `allowableMovement = 10`)
- [ ] Only the primary pointer counts; a second finger never re-arms (m:chat/useLongPressDrag.ts:L60-L61) (iPhone: single-touch recognizer)
- [ ] A system cancel before arming abandons the hold (m:chat/useLongPressDrag.ts:L102-L109) (iPhone: `.cancelled` state)
- [ ] Arming buzzes 15 ms (m:ui/haptics.ts:L19-L23) (iPhone: `UIImpactFeedbackGenerator(.medium)`)
- [ ] Once armed, the list stops scrolling for the rest of the gesture (m:chat/useLongPressDrag.ts:L111-L136) (iPhone: the recognizer cancels the list pan; disable scrolling while armed)
- [~] The Android scroll lock is a lifetime non-passive touchmove listener (m:chat/useLongPressDrag.ts:L8-L18, m:chat/useLongPressDrag.ts:L111-L136) (iPhone: WebView workaround, not needed natively: gesture recognizers own the touch)

### 8.2 Look and layout

- [x] The fan dims the whole screen with black at 45 percent, fading in over 100 ms, on a layer above everything (z 9500) (m:chat/ChatFanOut.tsx:L269-L278) (iPhone: full-screen overlay, `Color.black.opacity(0.45)`)
- [x] Buckets for everyone: Reply and Profile on the near arc, Copy on the far arc (m:chat/ChatFanOut.tsx:L52-L59) (iPhone: same)
- [x] Extra buckets for armed moderators: Delete on the near arc; Timeout, Ban and Pin on the far arc (m:chat/ChatFanOut.tsx:L61-L66, m:chat/ChatFanOut.tsx:L114-L117) (iPhone: same)
- [x] Pin appears only with mod tools armed AND a Twitch channel id (m:chat/MobileChatPane.tsx:L725-L728) (iPhone: same)
- [ ] The landscape overlay drops Reply; a non-Twitch room drops Profile (both in a read-only non-Twitch room) (m:chat/MobileChatPane.tsx:L47-L52, m:chat/MobileChatPane.tsx:L729-L737) (iPhone: same)
- [x] Distance is severity: everyday actions on the near arc, heavier ones on the far arc, so Ban takes a deliberate longer reach than Delete (m:chat/ChatFanOut.tsx:L16-L19) (iPhone: same geometry)
- [x] The fan centre is the window's horizontal centre, not the press point, so an edge press never piles tiles up (m:chat/ChatFanOut.tsx:L119-L138) (iPhone: centre on the window)
- [~] On an unfolded foldable with a vertical hinge, the centre moves to the right-hand segment (m:chat/ChatFanOut.tsx:L130-L138) (iPhone: no foldable iPhones; centre on the window)
- [x] Vertically the centre sits at the press point, pushed down to at least 302 px from the top (far radius 150 + half tile 32 + chip clearance 60) and kept at least 8 px above the bottom (m:chat/ChatFanOut.tsx:L139-L142) (iPhone: same clamp)
- [x] Near arc radius 92 px spanning 120 degrees; far arc radius 150 px spanning 160 degrees; both centred straight up; tiles spaced evenly, a lone tile sits straight up (m:chat/ChatFanOut.tsx:L68-L76, m:chat/ChatFanOut.tsx:L144-L165) (iPhone: same)
- [x] Tiles are 64 px squares, radius 16 px, 1 px border, shadow `0 8px 22px rgba(0,0,0,0.5)`, icon 20 px over an 11 px semibold label (d:components/chat/ModBucketTile.tsx:L64-L86) (iPhone: SF Symbols `arrowshape.turn.up.left`, `person`, `doc.on.doc`, `trash`, `clock`, `nosign`, `pin`)
- [x] Inactive tiles are solid zinc-900 with a white/15 border and white/80 content (d:components/chat/ModBucketTile.tsx:L52-L56) (iPhone: solid dark tiles, no translucency over busy chat)
- [x] The active tile fills solid per action: Delete orange-600, Timeout amber-600, Ban red-600, Pin sky-600, Reply violet-600, Copy zinc-600, Profile indigo-600 (d:components/chat/ModBucketTile.tsx:L22-L34) (iPhone: same colours)
- [x] The active tile scales to 1.18 on a spring (stiffness 500, damping 17), scale only so its centre never drifts (d:components/chat/ModBucketTile.tsx:L60-L66) (iPhone: spring scale)
- [ ] A danger tile wiggles its icon -10, 6, 0 degrees over 0.3 s when it becomes active (d:components/chat/ModBucketTile.tsx:L67-L72) (iPhone: keyframe rotation)
- [ ] Tiles fly from the press point to their place on a spring (stiffness 520, damping 34), each 12 ms after the previous (m:chat/ChatFanOut.tsx:L307-L321) (iPhone: staggered springs)
- [~] A name chip floats centred above the far arc (at least 28 px from the top): the chatter's name in 14 px bold with their 7TV paint or adjusted colour (accent when none), 12 px by 8 px padding, radius 12 px, white/15 border, blur, shadow `0 12px 32px rgba(0,0,0,0.6)` (m:chat/ChatFanOut.tsx:L166-L173, m:chat/ChatFanOut.tsx:L280-L305) (iPhone: Liquid Glass capsule with the baked paint) (iPhone now: Liquid Glass capsule with the name in its colour; the paint is not on the chip yet)
- [~] The chip background is tertiary at 90 percent, fully opaque at 0 percent glassiness (pref: glassiness-zero-contract.md) (m:chat/ChatFanOut.tsx:L297-L299) (iPhone: opaque fallback when glass is off) (iPhone now: built: solid tertiary when blur is off; not seen at 0 percent)
- [ ] The chip pops in from scale 0.6, sits tilted -4 degrees while nothing is selected, and straightens to scale 0.86 when a bucket is active (spring 500 / 26) (m:chat/ChatFanOut.tsx:L292-L296) (iPhone: spring)

### 8.3 Selection, dialling and commit

- [ ] Magnetic selection: the nearest tile within 60 px of the finger is active; beyond 60 px of every tile nothing is (m:chat/ChatFanOut.tsx:L78-L82, m:chat/ChatFanOut.tsx:L197-L208) (iPhone: nearest within 60 pt)
- [ ] Timeout is sticky: once engaged it stays engaged as the finger moves further away (m:chat/ChatFanOut.tsx:L210-L221) (iPhone: same)
- [x] The timeout duration dials from the distance beyond 30 px from the Timeout tile on a 10th-power ramp over 260 px up to 14 days (s:utils/timeoutRamp.ts:L9-L42, m:chat/ChatFanOut.tsx:L219) (iPhone: same ramp)
- [x] Durations snap to 5 s steps under a minute, whole minutes under an hour, 30 minutes under a day, whole days above; minimum 1 s (s:utils/timeoutRamp.ts:L15-L25) (iPhone: same)
- [x] The active Timeout tile's label shows the live duration ("45s", "10m", "1h 30m", "2d") (d:components/chat/ModBucketTile.tsx:L79-L83) (iPhone: same)
- [ ] A tick buzz (8 ms) fires whenever the active bucket changes to a bucket (m:chat/ChatFanOut.tsx:L223-L228, m:ui/haptics.ts:L25-L28) (iPhone: `UISelectionFeedbackGenerator`)
- [ ] A step buzz (5 ms) fires when the dialled duration crosses seconds, minutes, hours or days, never on every snap (m:chat/ChatFanOut.tsx:L229-L237, s:utils/timeoutRamp.ts:L56-L57) (iPhone: `UIImpactFeedbackGenerator(.soft)`)
- [ ] Release on a bucket commits; release away from all buckets cancels; a system cancel cancels (m:chat/ChatFanOut.tsx:L240-L260) (iPhone: `.ended` vs `.cancelled`)
- [ ] Committing a danger bucket (Delete, Timeout, Ban) buzzes 15 / 40 / 15 ms; others buzz 20 ms (m:chat/ChatFanOut.tsx:L247-L249, m:ui/haptics.ts:L35-L43) (iPhone: `UINotificationFeedbackGenerator(.warning)` for danger, `UIImpactFeedbackGenerator(.rigid)` otherwise)
- [ ] A Timeout released without a dialled duration uses 600 s (m:chat/ChatFanOut.tsx:L250) (iPhone: 10 min default)
- [x] There is no confirmation dialog; the far-arc reach is the protection (m:chat/ChatFanOut.tsx:L16-L19, m:chat/MobileChatPane.tsx:L174-L178) (iPhone: no alert)
- [ ] Reply sets the room's reply draft for that message (m:chat/MobileChatPane.tsx:L546-L552) (iPhone: same)
- [ ] Copy puts the message text on the clipboard, failures ignored, with no toast (m:chat/MobileChatPane.tsx:L553-L555) (iPhone: `UIPasteboard.general.string`)
- [ ] Profile opens the profile sheet with the login as display name, colour `#9147FF` and no badges (m:chat/MobileChatPane.tsx:L556-L563) (iPhone: same)
- [ ] The fan's target author id is the Twitch `user-id` tag (empty on Kick rows) (m:chat/MobileChatPane.tsx:L483-L505) (iPhone: Rust row gives author id with its key space)

## 9. Moderator actions

- [x] Moderator status in a room comes from my own badges there: `moderator/` or `broadcaster/`; Twitch rooms only (m:chat/modActions.ts:L9-L13, m:chat/MobileChatPane.tsx:L155) (iPhone: Rust room-state flag, see moves) (iPhone now: seen: `mod true` in the viewer's own room (broadcaster))
- [x] Moderator tools are ON by default wherever I have the badge (m:chat/MobileChatPane.tsx:L174-L185) (iPhone: same default) (iPhone now: the chat menu shows Moderator tools On)
- [~] Turning tools off is remembered per channel for the session only, and losing the badge disarms regardless; the toggle row and its hint "Long-press any message for delete, timeout and ban." live in the composer menu (chat-input-emotes.md) (m:chat/MobileChatPane.tsx:L180-L195, m:chat/ComposerMenuSheet.tsx:L183-L211) (iPhone: per-room off set in memory) (iPhone now: the row is seen in the chat menu; toggling it was not driven)
- [ ] All mod actions use the room's numeric Twitch channel id; Kick tabs have none, so no mod buckets appear there (m:chat/MobileChatPane.tsx:L155-L157, m:chat/chatTabsStore.ts:L29-L32) (iPhone: same)
- [~] Delete: `invoke delete_chat_message {broadcasterId, messageId}`; failure toast "Could not delete that message." (error) (m:chat/modActions.ts:L15-L23, m:chat/MobileChatPane.tsx:L564-L570) (iPhone: Rust command, error toast) (iPhone now: built; never run from a drive (it writes to Twitch))
- [~] Timeout: `invoke ban_user {broadcasterId, targetUserId, duration, reason: null}` (m:chat/modActions.ts:L25-L43, m:chat/MobileChatPane.tsx:L571-L586) (iPhone: Rust command) (iPhone now: built; never run from a drive)
- [~] Timeout success toast "Timed out <username> for <duration>" (success) with an "Undo" action that calls `invoke unban_user {broadcasterId, targetUserId}` (m:chat/MobileChatPane.tsx:L575-L583, m:chat/modActions.ts:L45-L54) (iPhone: toast with Undo, chrome per shell-design.md) (iPhone now: built (app toast with Undo); never run from a drive)
- [~] Timeout failure toast "Could not time out that user." (m:chat/MobileChatPane.tsx:L574) (iPhone: same) (iPhone now: built; never run from a drive)
- [~] Ban: `invoke ban_user` with `duration: null`; success "Banned <username>" with "Undo"; failure "Could not ban that user." (m:chat/MobileChatPane.tsx:L587-L598) (iPhone: same) (iPhone now: built; never run from a drive)
- [~] Pin: `invoke pin_chat_message {broadcasterId, messageId, durationSeconds: null}` (pinned until removed) (m:chat/modActions.ts:L56-L64) (iPhone: Rust command) (iPhone now: built; never run from a drive)
- [~] A successful pin refreshes the pinned banner at once; failure toasts "Could not pin that message." (m:chat/MobileChatPane.tsx:L599-L606) (iPhone: re-read pins after a pin) (iPhone now: built: the core re-reads pins after a pin (`settle_pin`); never run from a drive)
- [x] There is no unpin on the phone (desktop has one in its hover cluster) (d:components/ChatMessage.tsx:L3685-L3714) (iPhone: none)
- [~] Ban and timeout reasons are always empty on the phone (m:chat/modActions.ts:L32-L37) (iPhone: no reason entry) (iPhone now: built (reason null); never run from a drive)
- [ ] `TIMEOUT_OPTIONS` (60s, 10m, 1h, 24h) is defined but unused; the fan dial replaces fixed chips (m:chat/modActions.ts:L66-L71) (iPhone: no chips)

## 10. Chat tabs

### 10.1 Strip

- [x] The strip is hidden while only one room is open, so single-room chat keeps every pixel of height; it shows at 2 or more tabs (m:chat/ChatTabStrip.tsx:L1-L17, m:chat/ChatTabStrip.tsx:L61) (iPhone: shown when tabs.count >= 2)
- [x] Strip height is fixed at 45 px with 8 px side padding and a bottom hairline `--color-border-subtle` (m:chat/ChatTabStrip.tsx:L9-L12, m:chat/ChatTabStrip.tsx:L63-L67) (iPhone: 45 pt bar)
- [x] Banners over the top of chat (hype train, drops, pins) start 45 px lower while the strip shows (m:screens/WatchScreen.tsx:L1183-L1188) (iPhone: inset banners below the strip) (iPhone now: pins sit under the strip)
- [x] The rooms sit in one flat glazed capsule that scrolls horizontally inside its fixed rim, 4 px padding and 4 px gaps (m:chat/ChatTabStrip.tsx:L68-L71) (iPhone: horizontal `ScrollView` inside a Liquid Glass capsule) (iPhone now: Liquid Glass capsule that hugs its rooms; past the bar it compresses in place instead of scrolling (Brandon: overflow never scrolls, clips or wraps): rooms behind the one on screen fold to their faces one at a time, longest name first, and the room on screen keeps its name and close button; seen with 3 and 4 tabs and a 19-letter name)
- [x] The active room is a darker pill `rgb(0 0 0 / 0.72)` with full radius (m:chat/ChatTabStrip.tsx:L75-L80, s:styles/globals.css:L834-L837) (iPhone: dark capsule behind the selected tab)
- [x] Each tab is 32 px tall with 10 px left and 4 px right padding; tab content max width 136 px (m:chat/ChatTabStrip.tsx:L75-L84) (iPhone: same)
- [x] The phone strip uses its own 32 px pills, 13 px labels and 20 px faces; the desktop chat-header scale (26 px capsules, 22 px pills, 11.5 px text, 18 px faces) is a desktop header rule (pref: chat-header-scale.md) (m:chat/ChatTabStrip.tsx:L75-L113) (iPhone: phone sizes; see Decisions for Brandon)
- [~] Tab avatar: 20 px round (the 7TV animated avatar for Twitch rooms) or the label's first letter in 10 px semibold text-muted on a surface circle (m:chat/ChatTabStrip.tsx:L85-L99) (iPhone: `AsyncImage` circle or initial) (iPhone now: the channel's profile picture or initial; not the 7TV animated avatar)
- [x] The stream-following tab shows an 11 px filled broadcast icon in accent (m:chat/ChatTabStrip.tsx:L100-L102) (iPhone: `dot.radiowaves.left.and.right`)
- [~] A provider mark (11 px) shows on non-Twitch tabs only when the open tabs span platforms (pref: platform-marks-only-where-mixed.md) (m:chat/ChatTabStrip.tsx:L54-L56, m:chat/ChatTabStrip.tsx:L103-L106) (iPhone: same rule) (iPhone now: built from Rust `mixed_platforms`; no Kick rooms on the iPhone yet)
- [x] The label is 13 px, truncated; active label semibold text-primary, others text-secondary (m:chat/ChatTabStrip.tsx:L107-L113) (iPhone: same)
- [x] Tapping a tab makes it active (m:chat/ChatTabStrip.tsx:L81-L84, m:chat/chatTabsStore.ts:L183) (iPhone: tap selects)
- [x] Every non-stream tab has a 24 px close button (12 px bold X, text-muted, aria-label "Close <label> chat"); the stream tab has a 6 px spacer instead (m:chat/ChatTabStrip.tsx:L115-L126) (iPhone: `xmark` button, accessibilityLabel "Close <label> chat")
- [~] The strip does not re-render on each arriving message (memoized, reads only the tab store) (m:chat/ChatTabStrip.tsx:L49-L52, m:chat/ChatTabStrip.tsx:L136) (iPhone: strip observes only tab state) (iPhone now: built: it reads only the tab list)
- [x] Background tabs show no unread or mention marks (the desktop dock has them) (m:chat/ChatTabStrip.tsx:L72-L129) (iPhone: none, matching the phone; see backlog)

### 10.2 Tab rules

- [x] Closing a tab releases its room; if it was active, the first remaining tab becomes active (m:chat/chatTabsStore.ts:L170-L181) (iPhone: same) (iPhone now: the log shows the room left and the stream tab back on screen)
- [x] The stream tab cannot be closed (m:chat/chatTabsStore.ts:L174) (iPhone: same) (iPhone now: no close button; Rust `services/chat_tabs.rs` test)
- [x] There is no reordering and no maximum number of tabs (m:chat/chatTabsStore.ts:L42-L68) (iPhone: no reorder, no cap)
- [x] Tabs are not persisted; they live for the app session (m:chat/chatTabsStore.ts:L80-L84) (iPhone: in memory; see Decisions for Brandon) (iPhone now: every launch starts with the stream tab alone)
- [x] The stream tab is always first; manual tabs follow in the order opened (m:chat/chatTabsStore.ts:L118-L121, m:chat/chatTabsStore.ts:L142-L145) (iPhone: same order)
- [x] Twitch rooms are keyed by the bare lowercased login and carry the numeric Twitch id; other platforms are keyed `provider:slug` and carry NO channel id, so Twitch `xqc` and Kick `xqc` are two tabs (m:chat/chatTabsStore.ts:L20-L40, m:chat/chatTabsStore.ts:L70-L78, s:utils/providerKey.ts:L16-L34, m:chat/chatTabsStore.test.ts:L26-L62) (iPhone: Rust tab keys; never a bare Kick id) (iPhone now: Rust keys tabs (`chat_tabs.rs`, tests cover Kick))
- [~] When the watched stream changes, the old stream room is released and the new one acquired, unless it is already open as a manual tab, which is promoted instead of duplicated (m:chat/chatTabsStore.ts:L104-L121) (iPhone: Rust `sync_stream_tab`) (iPhone now: Rust `sync_stream` with tests; a stream switch was not driven)
- [~] The active tab follows the stream on a switch unless I am sitting on another room (m:chat/chatTabsStore.ts:L122-L129) (iPhone: same) (iPhone now: Rust with tests; not driven)
- [~] When the stream closes, its tab drops but manual tabs stay open, so moderating several rooms survives closing the player (m:chat/chatTabsStore.ts:L89-L102) (iPhone: same) (iPhone now: Rust with tests; not driven)
- [~] Adding a room already open just switches to it (m:chat/chatTabsStore.ts:L132-L138) (iPhone: same) (iPhone now: Rust with tests; not driven)
- [~] A Kick room opened from a bare slug (no avatar, or label equal to the slug) asks `invoke provider_channel_meta {provider, channel}` and fills in the real name and avatar when it answers (m:chat/chatTabsStore.ts:L146-L161, m:chat/chatTabsStore.test.ts:L64-L77) (iPhone: Rust fills tab metadata) (iPhone A9: Rust fills tab metadata (A9: tab label "Maherco" seen))
- [x] Opening the first room starts the chat bridge (`invoke start_chat`); later Twitch rooms join (`invoke join_chat_channel`); Kick rooms connect their adapter (`invoke provider_chat_connect`) (s:stores/chatConnectionStore.ts:L3386-L3423) (iPhone: Rust subscribe per room) (iPhone now: first room `start_chat`, later rooms `join_chat_channel`, one socket)
- [x] Closing a Twitch room leaves at once (`invoke leave_chat_channel`); a Kick room waits a 15 s grace before `invoke provider_chat_disconnect` (s:stores/chatConnectionStore.ts:L3428-L3504) (iPhone: Rust unsubscribe) (iPhone A9: Kick grace built in `ChatRooms.leaveKick` (A9))
- [ ] A room whose connect fails retries alone after 30 s, doubling up to 5 minutes (s:stores/chatConnectionStore.ts:L1922-L1965) (iPhone: Rust retry)
- [~] Stream-scoped surfaces (hype train, drops, polls, predictions) show only while the active tab is the stream tab of a Twitch stream; pinned messages follow the active tab's channel id (m:chat/ChatTabStrip.tsx:L19-L47, m:screens/WatchScreen.tsx:L169-L176) (iPhone: same scoping) (iPhone now: pins follow the room on screen; `ChatRooms.viewingStream` is there for the stream-scoped surfaces, which the iPhone does not draw over chat yet)
- [x] The composer menu rows that drive tabs ("Add a chat", "Reload this chat", "Close this chat" on manual tabs only) belong to chat-input-emotes.md (m:chat/ComposerMenuSheet.tsx:L145-L205) (iPhone: see chat-input-emotes.md) (iPhone now: Add a chat, Reload this chat, Close this chat (rooms opened beside the stream))

## 11. Add a chat sheet

- [x] A bottom sheet titled "Add a chat", at most 70 percent of screen height (m:chat/AddChatSheet.tsx:L133) (iPhone: `.sheet` with `.presentationDetents([.fraction(0.7)])`)
- [x] A search field with a 16 px magnifier and placeholder "Search, or paste a kick.com link", 15 px text, autocapitalize and autocorrect off (m:chat/AddChatSheet.tsx:L134-L144) (iPhone: `.textInputAutocapitalization(.never)`, `.autocorrectionDisabled()`, `magnifyingglass`) (iPhone now: placeholder "Search channels": no Kick links on the iPhone yet) (iPhone A9: seen 2026-10-16 (A9))
- [x] Under 2 characters, the sheet lists live follows from Twitch and Kick merged, without any network call (m:chat/AddChatSheet.tsx:L46-L48, m:chat/AddChatSheet.tsx:L130) (iPhone: list from the Rust home snapshot)
- [x] That list carries a header "Live now" (12 px semibold uppercase, wide tracking, text-muted) (m:chat/AddChatSheet.tsx:L146-L150) (iPhone: section header)
- [~] Two or more characters search Twitch (`invoke search_channels {query}`) and Kick (`invoke provider_search {provider: 'kick', query}`) in parallel after a 320 ms debounce (m:chat/AddChatSheet.tsx:L77-L111) (iPhone: 0.32 s debounce, Rust merged search) (iPhone now: Twitch results seen through Rust `search_chat_rooms` after the 0.32 s pause; the iPhone drops Kick rows until it has Kick) (iPhone A9: Kick rows now kept from Rust `search_chat_rooms`)
- [~] One platform failing never empties the other platform's results (m:chat/AddChatSheet.tsx:L86-L104) (iPhone: independent results) (iPhone now: Rust `search_chat_rooms`)
- [x] Search includes offline channels (chat is joinable while offline) (m:chat/AddChatSheet.tsx:L1-L8) (iPhone: same) (iPhone now: an offline channel came back from the search)
- [~] Results order: live channels by viewers descending, then offline ones in platform order (m:chat/searchOrder.ts:L1-L16, m:chat/searchOrder.test.ts:L8-L23) (iPhone: Rust orders, see moves) (iPhone now: Rust `order_rooms` with a test)
- [x] Pasting a kick.com link (scheme, `www.` and `@` optional) or typing `kick:slug` / `kick/slug` shows a single row for that room with the subline "Open this Kick chat", without searching (m:chat/AddChatSheet.tsx:L25-L32, m:chat/AddChatSheet.tsx:L49-L70, m:chat/AddChatSheet.tsx:L214) (iPhone: Rust URL classifier) (iPhone A9: seen 2026-10-16 (A9): k-add.png)
- [x] "Searching…" shows while a search runs with nothing listed (m:chat/AddChatSheet.tsx:L152-L153) (iPhone: same text)
- [~] "No channels found." shows for an empty search (m:chat/AddChatSheet.tsx:L154-L157) (iPhone: same text) (iPhone now: built; not captured)
- [~] "Nobody you follow is live. Search above to open any channel." shows when no follow is live (m:chat/AddChatSheet.tsx:L154-L159) (iPhone: same text) (iPhone now: built; not captured)
- [~] Empty-state text is 14 px text-muted, centred, 24 px vertical padding (m:chat/AddChatSheet.tsx:L152-L159) (iPhone: same) (iPhone now: built; not captured)
- [x] Rows: 12 px gap, 8 px by 4 px padding, pressed state 70 percent opacity (m:chat/AddChatSheet.tsx:L169-L174) (iPhone: list row)
- [x] Row avatar: 36 px round (7TV avatar) or the first letter in 13 px semibold on a surface circle (m:chat/AddChatSheet.tsx:L175-L189) (iPhone: circle image)
- [x] A live row adds a 12 px dot in `--color-live` with a 2 px ring in the background colour at the avatar's bottom right (m:chat/AddChatSheet.tsx:L190-L204) (iPhone: overlay dot)
- [x] Name 15 px truncated, with a 12 px provider mark on non-Twitch rows (m:chat/AddChatSheet.tsx:L206-L212) (iPhone: same)
- [x] Subline 12.5 px text-muted: the game name or "Live" when live, "Offline" when not (m:chat/AddChatSheet.tsx:L213-L215) (iPhone: same)
- [x] Live rows with viewers show the count with a 13 px people icon in tabular figures at the trailing edge (m:chat/AddChatSheet.tsx:L219-L226) (iPhone: `person.2.fill`)
- [x] A room already open is disabled at 45 percent opacity with "Open" at the trailing edge (m:chat/AddChatSheet.tsx:L163-L173, m:chat/AddChatSheet.tsx:L217-L218) (iPhone: disabled row)
- [x] Follow rows without an `is_live` field count as live (m:chat/AddChatSheet.tsx:L164-L166) (iPhone: Rust supplies explicit liveness)
- [~] Picking a row opens it as a tab, makes it active, clears the query and closes the sheet (m:chat/AddChatSheet.tsx:L118-L128) (iPhone: same) (iPhone now: drive steps open rooms through the same `ChatRooms.add`; a tap in the sheet was not driven)

## 12. Pinned messages banner

- [x] Pins for the ACTIVE tab's Twitch channel are fetched with `invoke get_pinned_chat_messages {channelId}` on open and every 30 s, skipped while the app is backgrounded (m:screens/WatchScreen.tsx:L550-L589) (iPhone: read the Rust `channel_state` pinned section instead of polling, see moves) (iPhone now: from the core's channel state per open Twitch room (30 s poll in Rust, pushed as `channel-state`))
- [~] A pin from the fan refreshes the pins at once (m:chat/MobileChatPane.tsx:L601-L602, m:screens/WatchScreen.tsx:L589) (iPhone: Rust `settle_pin` then a state push) (iPhone now: Rust `settle_pin`; no pin was made from a drive)
- [x] Each answer is stamped with its channel, so a slow answer never shows under another room (m:screens/WatchScreen.tsx:L553-L576) (iPhone: key pins by room) (iPhone now: two rooms each showed their own pin)
- [x] Kick rooms never show pins on the phone (no Twitch id; the slice's Kick pin is not read) (m:screens/WatchScreen.tsx:L174-L176, s:stores/chatConnectionStore.ts:L156-L157) (iPhone: none) (iPhone A9: seen 2026-10-16 (A9): pins 0 in a Kick room)
- [x] The banner floats over the top of chat (below the tab strip when shown), inset 10 px at the sides and 6 px at the top, beneath the hype train and drops bar in a reversed stack with 4 px gaps (m:screens/WatchScreen.tsx:L1164-L1212) (iPhone: overlay at the top of the list)
- [~] It is hidden in the landscape overlay (m:screens/WatchScreen.tsx:L1166-L1170) (iPhone: same) (iPhone now: built; not seen)
- [x] It is one container that grows downward, never a bar plus a second card (m:watch/PinnedBanner.tsx:L1-L11) (iPhone: one view that expands)
- [x] Its surface is the static popover tint (90 percent, opaque at 0 percent glassiness) with no backdrop blur, since a blur over repainting chat costs every frame (pref: glassiness-zero-contract.md) (m:watch/PinnedBanner.tsx:L44-L46, m:mobile.css:L103-L112) (iPhone: solid tinted rounded rect, no blur)
- [x] Collapsed header (10 px by 6 px padding): a 12 px filled pin icon in accent (m:watch/PinnedBanner.tsx:L47-L52) (iPhone: `pin.fill`)
- [~] Then the sender's name in their paint with at most their highest Twitch role badge (broadcaster, staff, moderator, VIP), capped at 50 percent width (m:watch/PinnedBanner.tsx:L53-L72, s:components/chat/PinnedChatter.tsx:L66-L88, s:utils/chatterBadges.ts:L133-L169) (iPhone: same) (iPhone now: name colour seen; the role badge (Rust `sender_view.role_badge`) was not on the pins seen)
- [x] Then a one-line preview of the text in 12 px at text-primary 85 percent (m:watch/PinnedBanner.tsx:L73-L78) (iPhone: one-line label)
- [~] "+<n>" in 11 px semibold text-secondary when more than one message is pinned (m:watch/PinnedBanner.tsx:L79-L83) (iPhone: same) (iPhone now: built; only single pins seen)
- [x] A 12 px caret that rotates 180 degrees over 180 ms `cubic-bezier(0.2,0.8,0.2,1)` when expanded (m:watch/PinnedBanner.tsx:L84-L90) (iPhone: `chevron.down` rotation)
- [x] An unpainted pinned name gets a text shadow `0 1px 2px rgba(0,0,0,0.8)`; a painted one brings its own (m:watch/PinnedBanner.tsx:L37-L38) (iPhone: shadow on plain names)
- [x] Tapping the header toggles expanded; the body unrolls by height and opacity over 200 ms `cubic-bezier(0.16,1,0.3,1)` (m:watch/PinnedBanner.tsx:L47-L50, m:watch/PinnedBanner.tsx:L93-L104) (iPhone: animated height)
- [x] Expanded, the header shows the first pin's author with every badge on one line (badges that do not fit hide whole) and drops the preview (m:watch/PinnedBanner.tsx:L53-L78) (iPhone: same) (iPhone now: the pin author's badge art from Rust `sender_view`)
- [x] Below, the first pin's full text at 13 px relaxed leading (m:watch/PinnedBanner.tsx:L106-L110) (iPhone: same)
- [~] Then a hairline and each further pin as "<badges> <name>: <text>" (m:watch/PinnedBanner.tsx:L111-L131) (iPhone: same) (iPhone now: built; only single pins seen)
- [x] The banner starts collapsed every time; desktop's `pinned_start_collapsed` and `pinned_collapsed_style` are not read (m:screens/WatchScreen.tsx:L167) (iPhone: start collapsed)
- [x] Tapping a pin never jumps to the message, and there is no unpin control (m:watch/PinnedBanner.tsx:L40-L137) (iPhone: none)

## 13. Streak banners (above the composer)

- [~] Opening a Twitch room asks `invoke get_watch_streak {channelId}` and `invoke get_resub_notification {channelId}` once each, no polling (m:chat/StreakBanners.tsx:L63-L85) (iPhone: Rust commands on room open) (iPhone now: built; no followed live channel had a streak to share during the checks)
- [ ] The banners sit directly above the composer with 10 px side and 6 px top padding, 4 px apart; resub above watch streak (m:chat/StreakBanners.tsx:L149-L153, m:chat/MobileChatInput.tsx:L368-L375) (iPhone: VStack above the composer)
- [ ] Resub banner: popover surface, 10 px by 6 px padding, a 13 px filled star in accent (m:chat/StreakBanners.tsx:L153-L155) (iPhone: `star.fill`)
- [ ] Resub title 12 px: "Gift sub from <gifter>" for a gift with a known gifter, else "<n> month subscribed" / "<n> months subscribed" (m:chat/StreakBanners.tsx:L157-L161) (iPhone: same strings)
- [ ] When the streak exceeds 1 month, a 10.5 px text-muted toggle "Include <n>-month streak", prefixed "✓ " while on (default on) (m:chat/StreakBanners.tsx:L162-L170) (iPhone: tappable line with a checkmark)
- [ ] Resub "Share" chip: 11.5 px semibold, accent/20 fill, accent text, full radius, 10 px by 4 px (m:chat/StreakBanners.tsx:L146-L147, m:chat/StreakBanners.tsx:L172-L178) (iPhone: capsule button)
- [ ] Resub dismiss: 12 px X, aria-label "Dismiss resub" (m:chat/StreakBanners.tsx:L179-L185) (iPhone: `xmark`, label "Dismiss resub")
- [ ] Watch streak banner: amber-500/5 ground, amber-500/20 border, radius 8 px (m:chat/StreakBanners.tsx:L192-L194) (iPhone: same)
- [ ] A 24 px amber-500/10 chip holds a 13 px amber-400 flame (stroke 2.5) glowing 8 px in the warning colour at 20 percent (m:chat/StreakBanners.tsx:L195-L197) (iPhone: `flame.fill` in amber)
- [ ] Title 12 px: "<n> stream watch streak" or "Watch streak milestone" (m:chat/StreakBanners.tsx:L198-L202) (iPhone: same)
- [ ] Watch streak "Share" chip is solid amber-500 with black text, a 10 px warning glow, and scales to 0.95 when pressed (m:chat/StreakBanners.tsx:L203-L209) (iPhone: same)
- [ ] Watch streak dismiss: amber-400/60 X, aria-label "Dismiss watch streak" (m:chat/StreakBanners.tsx:L210-L216) (iPhone: same)
- [ ] Share sends the composer text along (null when empty): `invoke share_watch_streak {channelId, milestoneId, message}` (m:chat/StreakBanners.tsx:L95-L116) (iPhone: Rust command)
- [ ] Resub share: `invoke use_resub_token {channelLogin, message, includeStreak, tokenId}` (m:chat/StreakBanners.tsx:L118-L140) (iPhone: Rust command)
- [ ] A successful share hides that banner (the token is consumed) and clears the composer (m:chat/StreakBanners.tsx:L104-L107, m:chat/StreakBanners.tsx:L128-L131) (iPhone: same)
- [ ] Failures toast "Could not share that watch streak." or "Could not share that resub." (m:chat/StreakBanners.tsx:L108-L113, m:chat/StreakBanners.tsx:L132-L137) (iPhone: same)
- [ ] Share chips disable while a share is running (m:chat/StreakBanners.tsx:L58, m:chat/StreakBanners.tsx:L172-L176) (iPhone: disabled state)
- [ ] Dismissals are remembered per channel for the session, so leaving and coming back does not resurrect a dismissed banner (m:chat/StreakBanners.tsx:L54-L61, m:chat/StreakBanners.tsx:L87-L93) (iPhone: per-room dismissed set)
- [x] Android quirk: moving from a Twitch room with a streak to a Kick room keeps the old banner, because the fetch is skipped without clearing state (m:chat/StreakBanners.tsx:L63-L66) (iPhone: clear banners on room change; see Decisions for Brandon) (iPhone now: the iPhone keeps streaks per room, so a room change never shows another room's banner)

## 14. Connection, recovery and watchdog

- [~] The phone never shows the connection error text or a connected indicator; the only visible connection state is the system row "Chat connection lost, reconnecting..." posted to each Twitch room when an outage outlives a 10 s grace (m:chat/MobileChatPane.tsx:L96-L105, s:stores/chatConnectionStore.ts:L2194-L2218, s:stores/chatConnectionStore.ts:L328-L332) (iPhone: same row, no status chrome) (iPhone now: Rust `services/chat_outage.rs` posts the row after the 10 s grace while a native shell draws rows; no outage happened during the checks)
- [~] Fast backend rebuilds (inside 10 s) stay completely silent (s:stores/chatConnectionStore.ts:L2198-L2204) (iPhone: same) (iPhone now: Rust grace timer; not observed)
- [~] After an outage the gap is backfilled from 30 s before the outage to 30 s after now, deduped, inserted in time order (s:stores/chatConnectionStore.ts:L2162-L2185, s:stores/chatConnectionStore.ts:L1060-L1083) (iPhone: Rust backfill) (iPhone now: Rust `chat-gap` (30 s either side), rows placed in time order (`ChatRoomBuffer.insertInTimeOrder`, tested); not observed)
- [ ] Rust sends a heartbeat every 30 s and stops once the IRC reader has heard nothing for 90 s, so silence means a dead connection rather than a quiet room (r:services/irc_service.rs:L125-L132, r:services/irc_service.rs:L1997-L2019) (iPhone: Rust health signal)
- [ ] The store's health check runs every 30 s; with no live socket it schedules a reconnect (s:stores/chatConnectionStore.ts:L1459-L1472) (iPhone: Rust supervisor)
- [ ] 2 to 3 minutes without frames marks rooms stale with the text "Connection may be stale", a dash, then "no data for <n>s" (s:stores/chatConnectionStore.ts:L62, s:stores/chatConnectionStore.ts:L1473-L1478) (iPhone: Rust stale event, never parsed text)
- [ ] Past 3 minutes it first nudges the backend to re-JOIN (`invoke nudge_chat_channels`) and escalates only after a second stale window (s:stores/chatConnectionStore.ts:L1479-L1500) (iPhone: Rust ladder)
- [ ] Escalation checks `invoke check_stream_online {userLogin}` (Twitch only): online reconnects, and a second consecutive time first runs `invoke restart_chat_bridge`; offline hands over to the stream-offline flow (s:stores/chatConnectionStore.ts:L1502-L1553) (iPhone: Rust ladder)
- [~] The local socket reconnects forever, backoff 1 s doubling to 30 s (s:stores/chatConnectionStore.ts:L1877-L1900) (iPhone: the local WebSocket bridge does not exist natively: Swift reads Rust through UniFFI; only the Rust IRC reconnect remains)
- [ ] Watchdog: when the ACTIVE Twitch room goes stale, chat is rebuilt once per quiet spell, re-arming when frames flow again (m:chat/useChatWatchdog.ts:L19-L49, m:chat/MobileChatPane.tsx:L149-L153) (iPhone: Rust rebuilds itself on its own stale signal, see moves)
- [~] Kick rooms are not watched by the phone watchdog; their adapter watches itself (m:chat/MobileChatPane.tsx:L149-L153) (iPhone: Rust adapter health) (iPhone A9: built (A9))
- [x] Rebuild order: release every Twitch tab, `invoke stop_chat`, then re-acquire every Twitch tab (re-join plus recent history) (m:chat/chatRecovery.ts:L59-L91) (iPhone: Rust `rebuild_chat`) (iPhone now: Rust `rebuild_chat` (`services/chat_recovery.rs`): the reload rebuilt the connection on a new port and the rooms carried on)
- [~] Automatic rebuilds run at most once per 60 s, never two at once, and only when a Twitch tab is open; a manual reload is never throttled (m:chat/chatRecovery.ts:L27-L51) (iPhone: same throttle in Rust) (iPhone now: Rust single-flight and 60 s cooldown with a test)
- [~] The rebuild on return after 90 s or more in the background (PiP never counts as background) follows system.md § 5; the rebuild itself is the throttled cycle above (m:lifecycle.ts:L47-L73) (iPhone now: built (`rebuild_chat` with force off after 90 s away, never in PiP); not driven)
- [x] "Reload this chat" on a Twitch tab runs the forced rebuild (m:chat/chatTabsStore.ts:L212-L235) (iPhone: Rust forced rebuild)
- [~] "Reload this chat" on a Kick tab drops and retakes the adapter (`invoke provider_chat_disconnect` then `invoke provider_chat_connect`) (m:chat/chatTabsStore.ts:L189-L211, m:chat/chatTabsStore.test.ts:L79-L92) (iPhone: Rust per-room reload) (iPhone A9: built: `ChatRooms.reload` Kick branch (A9))
- [x] The per-room `reloadNonce` that reloads bump is never read by a phone view (m:chat/chatTabsStore.ts:L44-L46, m:chat/chatRecovery.ts:L93-L99) (iPhone: not needed)
- [~] Recovery logs at warn level so it shows in release logs on a device (m:chat/chatRecovery.ts:L55-L57) (iPhone: Rust warn log) (iPhone now: Rust `log::warn!` in `chat_recovery`)

## 15. Empty, loading and offline rooms

- [x] A room with no rows yet is an empty list with no placeholder text and no spinner (m:chat/MobileChatPane.tsx:L658-L677) (iPhone: empty list; see backlog)
- [ ] The first paint arrives with history and live rows together, at most 1500 ms after the first live row (s:stores/chatConnectionStore.ts:L1112, s:stores/chatConnectionStore.ts:L3362-L3369) (iPhone: Rust snapshot)
- [x] An offline Twitch room still shows recent history from the backfill (s:stores/chatConnectionStore.ts:L3362-L3369) (iPhone: Rust history) (iPhone now: an offline room opened beside the stream loaded 18 history rows)
- [~] An offline Kick room is seeded from Kick's own scrollback (s:stores/chatConnectionStore.ts:L3371-L3378) (iPhone: Rust history) (iPhone A9: built (A9))

## 16. Landscape overlay chat (row rendering only)

- [x] The overlay renders the same list read-only: no composer, no tap-to-reply, no "Tap a message to reply" pill (m:chat/MobileChatPane.tsx:L54-L59, m:chat/MobileChatPane.tsx:L612-L618, m:chat/MobileChatPane.tsx:L680-L684, m:chat/MobileChatPane.tsx:L695) (iPhone: read-only flag)
- [ ] The fan in the overlay drops Reply (and Profile on non-Twitch rooms) (m:chat/MobileChatPane.tsx:L729-L737) (iPhone: same)
- [x] "Resume live" still shows in the overlay when paused (m:chat/MobileChatPane.tsx:L678-L691) (iPhone: same)
- [x] The overlay hides pins, drops, hype train, polls and predictions (m:screens/WatchScreen.tsx:L1164-L1170, m:screens/WatchScreen.tsx:L1217) (iPhone: same)
- [ ] The overlay column's background, width, opacity and side are watch-player's (m:mobile.css:L114-L133) (iPhone: see watch-player.md)

## 17. Settings that change chat rendering (keys and defaults; UI in settings.md)

- [x] `chat_design.font_size` 14 and `message_spacing` 8 (phone first run), `font_weight` 400 (r:models/settings.rs:L630-L648, r:models/settings.rs:L552-L562) (iPhone: Rust settings)
- [x] `chat_design.message_entrance` `slide` and `deleted_message_style` `italic` on a fresh phone install; `none` and `strikethrough` when the block is absent (r:models/settings.rs:L630-L648, r:models/settings.rs:L1921-L1946) (iPhone: same)
- [x] `chat_design.show_dividers` false, `alternating_backgrounds` false (r:models/settings.rs:L555-L556) (iPhone: same)
- [x] `chat_design.show_timestamps` false, `show_timestamp_seconds` false, `timestamp_format` `12h` (r:models/settings.rs:L566-L568) (iPhone: same)
- [x] `chat_design.emote_scale` 1, `emote_margin` 0.125, `animate_emotes` `always` (r:models/settings.rs:L569-L574) (iPhone: same)
- [ ] `chat_design.show_chat_gifs` true, `gif_scale` 1 (r:models/settings.rs:L571-L572) (iPhone: same)
- [ ] `chat_design.show_personal_emotes` true, `ffz_emote_effects` true, `bttv_emote_modifiers` true, `giant_emotes` true, `giant_emote_align` `center`, `emoji_style` `apple` (r:models/settings.rs:L580-L600) (iPhone: same)
- [x] `chat_design.badge_scale` 1, `show_badges` true, `show_third_party_badges` true, `hidden_badge_providers` empty, `show_avatars` true (r:models/settings.rs:L592-L601) (iPhone: same)
- [x] `chat_design.name_color_adjustment` `hsl_loop`, `username_separator` `none`, `username_style` `plain`, `username_accent_source` `user`, `show_at_sign` false (r:models/settings.rs:L589-L602, r:models/settings.rs:L620) (iPhone: same)
- [ ] `chat_design.mention_color` `#ff4444`, `reply_color` `#ff6b6b`, `mention_animation` true, `paint_mentions_in_body` true; `mention_weight` medium, `mention_italic` inherit, `mention_style` plain, `mention_text_color` unset when absent (r:models/settings.rs:L563-L578, d:components/chat/mentionStyle.ts:L19-L26) (iPhone: same)
- [x] `chat_design.reply_style` `full`, `backfill_opacity` 100, `hide_shared_chat` false, `seventv_emote_notices` true (r:models/settings.rs:L573-L603) (iPhone: same)
- [ ] `chat_design.link_previews` true, `link_preview_keep_link` false, `shorten_links` true, `link_color` empty, `link_underline` true, `link_preview_trusted_domains` empty (r:models/settings.rs:L585-L605) (iPhone: same)
- [x] `chat_render.message_buffer_cap` 100 (phone range 50 to 300), `chat_render.smooth_scroll_on_resume` true (s:types/index.ts:L604-L614, s:stores/chatConnectionStore.ts:L1037-L1046) (iPhone: same)
- [ ] `chat_events`: `event_style` cards, `event_animation` none, `cheer_display` card, `superchat_currency` empty, `hidden_provider_events` empty, `event_templates` empty (s:types/index.ts:L1001-L1020) (iPhone: same)
- [ ] `chat_highlights.appearance` standard at 20 percent; built-in first-time chatter on (d:components/ChatMessage.tsx:L3225-L3227, d:components/ChatMessage.tsx:L2276-L2282) (iPhone: same)
- [ ] `message_repeat.mode` off, `collapse_gift_subs` true, `moderation.show_mod_messages` false, `moderation.ignore_clear_chat` false, `cosmetics.paint_shadows` all (s:stores/chatConnectionStore.ts:L2871-L2874, s:types/index.ts:L1195, s:stores/chatConnectionStore.ts:L2366-L2370) (iPhone: same)
- [ ] Phone-only pref `mentionHaptic` default on, localStorage key `sn-mention-haptic` (m:phonePrefs.ts:L13-L31) (iPhone: UserDefaults or a Rust setting; the row belongs to settings.md)
- [ ] Keys with no effect on the phone path: `scrollbar_marks`, `scrollbar_mark_kinds`, `mod_action_style`, `mod_drag_layout`, `mod_pin_style`, `moderation.timeout_presets`, `moderation.saved_ban_reasons`, `compact_emote_tooltips`, `emote_hover_size`, `pinned_start_collapsed`, `pinned_collapsed_style`, `chat_highlights.appearance.flash_title_when_unfocused`, `chat_design.mention_sound` (d:components/ChatMessageList.tsx:L609, d:components/ChatMessage.tsx:L1325-L1361, s:components/ui/Tooltip.tsx:L63-L67) (iPhone: ignore these keys)
- [ ] A chat look setting whose name cannot convey it gets a live preview that uses the real row treatment (pref: visual-settings-get-previews.md) (d:components/chat/deletedMessage.ts:L1-L4, d:components/chat/eventCard.ts:L1-L5, d:components/chat/mentionStyle.ts:L1-L5) (iPhone: settings.md previews reuse the chat row view)

## 18. Accessibility

- [ ] Labelled controls: "Close <label> chat", "Dismiss resub", "Dismiss watch streak", "Always trust <host>"; every other control is labelled by its visible text ("Resume live", "Share", "Open", tab names) (m:chat/ChatTabStrip.tsx:L122, m:chat/StreakBanners.tsx:L182, m:chat/StreakBanners.tsx:L213, d:components/chat/LinkPreviewCard.tsx:L318) (iPhone: `accessibilityLabel` with the same strings)
- [ ] Decorative icons and accent bars are hidden from assistive tech (d:components/chat/StyledChatName.tsx:L104-L134, d:components/ChatMessage.tsx:L3316-L3339) (iPhone: `accessibilityHidden(true)`)
- [ ] Arrival animations and glints stop under reduced motion (s:styles/globals.css:L3329-L3333, s:styles/globals.css:L3440-L3443) (iPhone: honour `isReduceMotionEnabled`)
- [ ] The long-press fan has no non-gesture alternative on the phone (m:chat/ChatFanOut.tsx:L1-L19) (iPhone: add `accessibilityCustomActions` per row (Reply, Profile, Copy and the mod actions) so VoiceOver reaches the same actions; the sighted gesture is unchanged)
- [x] Chat text size follows the chat font setting, not the system text size (d:components/ChatMessage.tsx:L3430-L3433) (iPhone: settings font; Dynamic Type is backlog)

## 19. Rust commands and events this area depends on

- [x] `invoke start_chat`, `invoke join_chat_channel`, `invoke leave_chat_channel`, `invoke stop_chat` (Twitch room lifecycle) (s:stores/chatConnectionStore.ts:L3386-L3504, m:chat/chatRecovery.ts:L77-L83) (iPhone: UniFFI equivalents)
- [x] `invoke provider_chat_connect`, `invoke provider_chat_disconnect` (Kick room lifecycle and reload) (m:chat/chatTabsStore.ts:L198-L211) (iPhone: UniFFI) (iPhone A9: seen 2026-10-16 (A9): live Kick rows flow (66 -> 100 rows in 24 s) after the Pusher key fix)
- [x] `invoke load_channel_history`, `invoke kick_chat_history` (history on join and after outages) (s:stores/chatConnectionStore.ts:L908-L949, s:stores/chatConnectionStore.ts:L1121-L1135) (iPhone: UniFFI) (iPhone A9: seen 2026-10-16 (A9))
- [ ] `invoke nudge_chat_channels`, `invoke restart_chat_bridge`, `invoke check_stream_online` (health ladder) (s:stores/chatConnectionStore.ts:L1489-L1553) (iPhone: Rust-internal)
- [ ] `invoke provider_channel_meta`, `invoke search_channels`, `invoke provider_search` (tabs and add-chat) (m:chat/chatTabsStore.ts:L150-L153, m:chat/AddChatSheet.tsx:L89-L92) (iPhone: UniFFI)
- [~] `invoke delete_chat_message`, `invoke ban_user`, `invoke unban_user`, `invoke pin_chat_message`, `invoke get_pinned_chat_messages` (mod actions and pins) (m:chat/modActions.ts:L15-L64, m:screens/WatchScreen.tsx:L566-L568) (iPhone: UniFFI) (iPhone now: wired; never run from a drive)
- [~] `invoke get_watch_streak`, `invoke share_watch_streak`, `invoke get_resub_notification`, `invoke use_resub_token` (streak banners) (m:chat/StreakBanners.tsx:L63-L140) (iPhone: UniFFI) (iPhone now: wired; nothing to share during the checks)
- [ ] `invoke get_user_by_login`, `invoke get_user_by_id` (mention resolution, shared-chat source channel), `invoke fetch_link_preview` (d:components/ChatMessage.tsx:L442-L445, d:components/ChatMessage.tsx:L2339-L2341, s:services/linkPreviewService.ts:L210-L211) (iPhone: Rust should stamp these instead, see moves)
- [ ] `listen 7tv://emote-set-update`, `listen 7tv://cosmetic-update` (m:boot/useMobileBoot.ts:L168-L172) (iPhone: UniFFI callbacks)

## 20. Cross-references

- [x] The composer, reply bar ("Replying to @<name>"), emote picker, autocomplete, slash commands, room-mode chips, chat gating and send rules follow chat-input-emotes.md (m:chat/MobileChatInput.tsx:L355-L385, m:chat/MobileChatPane.tsx:L158-L173) (iPhone: see chat-input-emotes.md) (iPhone now: see chat-input-emotes.md)
- [ ] The profile sheet opened from names, mentions and the fan follows profile-cosmetics.md (m:chat/MobileChatPane.tsx:L714-L721) (iPhone: see profile-cosmetics.md)
- [ ] Channel points, hype train, drop progress, polls and predictions over chat follow rewards-badges-drops.md and watch-player.md (m:screens/WatchScreen.tsx:L1190-L1228) (iPhone: see those files)
- [ ] Toast chrome, including the Undo action, follows shell-design.md (m:chat/MobileChatPane.tsx:L573-L597) (iPhone: see shell-design.md)
- [ ] The landscape overlay chrome (resize, opacity, hide) follows watch-player.md (m:screens/WatchScreen.tsx:L1231-L1244) (iPhone: see watch-player.md)
- [ ] The settings rows for every key in § 17 follow settings.md (iPhone: see settings.md)
- [ ] App lifecycle beyond the chat rebuild (flushes, notification kicks) follows system.md (m:lifecycle.ts:L71-L104) (iPhone: see system.md)

## Decisions for Brandon

Not parity lines: open questions found while reading, each to be settled before its screen is built.

- Tapping an emote or a 7TV badge does nothing on the phone (it sets desktop-only state). The iPhone line says no tap action; or should it open an emote or badge detail view?
- The phone's resume pill shows no count; desktop shows "Chat Paused (N new)". Add the count on iPhone?
- Chat tabs are not saved between launches on Android. Persist them on iPhone?
- The phone tab strip uses its own sizes (32 px pills, 13 px text), not the chat header scale (26 px capsules, 22 px pills, 11.5 px text). Which one for iPhone?

## Beyond Android (backlog, Brandon picks)

- [ ] (backlog, not parity) A "<n> new" count on the Resume live button while paused, as the desktop pill shows
- [ ] (backlog, not parity) Unread and mention dots on background chat tabs, from Rust per-room counters (the desktop dock's marks)
- [ ] (backlog, not parity) Persist open chat tabs across launches (Rust-owned, like the desktop chat dock's saved list), with drag to reorder in the strip
- [ ] (backlog, not parity) A visible chat connection state (a small "Reconnecting" capsule at the top of the list) beyond the inline system row
- [ ] (backlog, not parity) An empty-room placeholder ("No messages yet") and a brief shimmer while history loads
- [ ] (backlog, not parity) Long-press preview of an emote or badge (name, provider, large image), which needs a gesture split with the fan, since the phone has no tooltips
- [ ] (backlog, not parity) Chat delay synced to stream latency so chat lines up with the video
- [ ] (backlog, not parity) Mention notifications while backgrounded (local notification or APNs) and a mention sound option on the phone, from Rust's existing `chat-mention-ping`
- [ ] (backlog, not parity) Dynamic Type for chat rows when the user opts in
- [ ] (backlog, not parity) A VoiceOver rotor of mentions and highlights in the current room
- [ ] (backlog, not parity) A Live Activity for a room you moderate, showing mentions or held AutoMod messages
- [ ] (backlog, not parity) Share a message via `ShareLink` from the fan
- [ ] (backlog, not parity) Unpin from the pinned banner and jump-to-message from a pin, for moderators
- [ ] (backlog, not parity) Saved ban and timeout reasons in the fan for moderators
- [ ] (backlog, not parity) iPad: two chat rooms side by side and hardware keyboard shortcuts for tab switching
- [ ] (backlog, not parity) Handoff of the open room between iPhone and Mac (open the same chat on desktop)
