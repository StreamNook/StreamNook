# Profiles and cosmetics: the data side

What the core serves for the profile area (You header, Cosmetics editor, chatter profile sheet,
chat-row member looks). The view layer is the design lane's; everything here is decided in Rust
(`src-tauri/src/services/profile_view.rs`, `cosmetics_registry.rs`, `member_theme.rs`, `css_fill.rs`)
and decoded by `ios/Sources/SNKit/CosmeticsModels.swift`. FakeCore answers every command below
(`SNKit/FakeCosmetics.swift`), and its writes only change fixture state.

## Commands

| Command | Args | Answer (SNKit type) | Notes |
| --- | --- | --- | --- |
| `user_profile_sheet` | `userId, login, displayName, color?, channelId?, channelLogin?` | `ProfileSheetData` | groups in order Twitch, 7TV, Chat clients (images only); `streamnook` only for members; `picture` is the 7TV avatar pick for a 56 pt box; `atmosphere` (with its `look`) or `cologne` is what the member wears on their profile, wear-gated, shown even when kept out of chat |
| `user_profile_sheet_cached` | `userId, channelId?` | `ProfileSheetData?` | the last answer for that person and room, no network |
| `get_sub_standing` | `login, channel, wornBadges` | `SubStanding?` | every word is Rust's |
| `get_user_message_history_limited` | `userId, limit` | `[ProfileHistoryRow]` | then `watch_user_history { userKey }` / `user-history-message` / `unwatch_user_history` |
| `own_identity_header` | `userId, login, displayName` | `OwnIdentityData` | worn badges (max 8), paint, 7TV picture, StreamNook badge, worn `atmosphere` / `cologne`, Twitch `banner_url` (nil when none); saved to disk |
| `own_identity_header_cached` | `userId` | `OwnIdentityData?` | first-frame paint |
| `cosmetics_picker_state` | `userId` | `StreamNookPickerState` | owned badges with `worn`; atmospheres with `allowed`/`worn` (Default `tier` first); `show_chat_toggle` |
| `external_cosmetics_state` | `userId, login` | `ExternalPickerState` | 7TV paints/badges with `worn`, `seventv_user_id`, Twitch selectable/earned badges, `chat_color`, `named_colors` |
| `sn_atmospheres` | | `[Atmosphere]` | each `look` as layers; refresh on `sn-registry-changed` (CosmeticsStore does) |
| `equip_streamnook_badge` | `userId, slug?` | | nil takes it off; error text is user-facing |
| `set_profile_theme` | `userId, theme` | | `tier` or an atmosphere id; failure: "Could not apply that atmosphere." |
| `set_chat_atmosphere` | `userId, show` | `Bool` | false = not saved |
| `set_seventv_paint` / `set_seventv_badge` | `userId` (7TV id), `paintId?` / `badgeId?` | `{success}` | `SESSION_EXPIRED` in the error = signed out |
| `update_chat_identity_gql` | `badgeId, badgeVersion` | | `NEEDS_DROPS_AUTH` in the error |
| `update_user_chat_color` | `targetUserId, color` | | a `named_colors[].value`, or hex for Prime/Turbo |
| `get_twitch_bio` / `update_twitch_bio` | `userId` / `description` | `String` | 300 characters max, checked in Rust; the write goes through the twitch.tv web session (no extra scope); answers what Twitch stored |
| `update_twitch_display_name` | `displayName` | `{display_name, description}` | only the capitalization may change (checked in Rust, then by Twitch); error text is user-facing |
| `upload_twitch_profile_image` | `kind` (`picture` or `banner`), `dataBase64` | | PNG, JPG or GIF up to 10 MB, read from the bytes; Twitch shows it a few seconds later, so re-read the header after a short wait |
| `get_seventv_auth_status`, `get_seventv_login_url`, `store_seventv_token` (`accessToken`, `userId: ""`, `twitchId: ""`), `logout_seventv` | | | the core reads the 7TV account from the token |

## Events

- `sn-registry-changed` `{version}`: members, cosmetics or atmospheres changed (re-read pickers).
- `member-theme-changed` `MemberTheme`: a member's theme resolved or changed.
- Chat frames: rows carry `view.atmosphere`, `view.cologne` and a `provider: "streamnook"` badge;
  `ROW_MEMBER` repaints a member's rows everywhere.

## Rendering helpers (data in, pixels out)

- `PaintedText`, `PaintFillView`, `PaintPainter`, `PaintedRun`: a `ChatPaint` with every layer.
- `FillView`, `FillPainter`: an `SNFill` (Atmosphere swatches, edges, rims).
- `AtmosphereWashLayer`, `WashView`: a chat row's wash (base, curtains or blurred image, rim, edge,
  Cologne drift).
- `CosmeticImage`: badge and avatar images with fallbacks, animation, `streamnook-house:` art.

## Drive steps (DEBUG)

`profile:LOGIN`, `profile:row`, `profile:close`, `cosmetics:open:KEY`, `cosmetics:scroll:ID`,
`cosmetics:dump`, `wash:find`, and FakeCore-only writes `cosmetics:equip:badge|theme|paint|7tvbadge|twitch:ID`,
`cosmetics:equip:color:VALUE` (a `named_colors[].value`), `cosmetics:equip:bio:TEXT` (`_` for spaces),
`cosmetics:chat:on|off`.
