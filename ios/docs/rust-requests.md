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
