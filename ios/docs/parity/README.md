# StreamNook for iPhone: parity contract

Read from StreamNook dev 577f3c02 + working tree of 2026-10-12 (the Android phone shell in `src/mobile`, the shared stores and services in `src`, and the Rust core in `src-tauri/src`). This folder is phase 4 of the plan (`C:\Users\Brandon\.claude\plans\streamnook-ios\PLAN.md`). The phase 3 input, every piece of logic the phone runs in TypeScript that has to move to Rust first, is in [`../rust-moves.md`](../rust-moves.md).

The contract is the list of everything the Android phone app does, area by area, that the native SwiftUI iPhone app has to do too. Builders tick lines as they see them working on the phone; the verify round ends with no `[ ]` left and `[~]` only where iOS genuinely cannot (or need not) do the thing.

## How to read a line

```markdown
- [ ] Tapping a live card opens Watch with the player at 16:9 and chat below (m:screens/FollowingScreen.tsx:L120-L148) (iPhone: NavigationStack push, AVPlayer view, UIKit chat list)
```

- The behavior comes first, with real numbers (sizes, timings, limits) and user-facing text copied verbatim in double quotes.
- Then the source lines it was read from, then the iPhone equivalent.
- `(pref: name.md)` cites a file in the StreamNook memory folder (a standing preference of Brandon's or a recorded trap).

Path shorthand used in the citations:

- `m:` is `src/mobile/` (the Android phone shell)
- `s:` is `src/` outside `src/mobile/` (shared stores, services, utils, styles)
- `d:` is desktop code under `src/` consulted for RULE ZERO (usually `src/components/`)
- `r:` is `src-tauri/src/` (the Rust core)
- `k:` is `src-tauri/gen/android/app/src/main/java/app/streamnook/` (the Android Kotlin layer)
- `t:` is `src-tauri/` outside `src/` (config, Android resources)
- `p:` is the repository root
- `pref:` is a file in the StreamNook memory folder (`C:\Users\Brandon\.claude\projects\C--StreamNook\memory\`): Brandon's standing preferences and recorded traps

Units: CSS px map 1:1 to iOS points. Tailwind spacing is converted to px (one unit is 4 px).

Legend:

- `[ ]` not built on the iPhone yet
- `[x]` seen working on the iPhone
- `[~]` built but needs work, or impossible or replaced on iOS (always with the reason)
- `- [ ] (backlog, not parity)` lines sit under `## Beyond Android (backlog, Brandon picks)` at the end of an area: things other iPhone Twitch apps do that the Android app does not. They are not part of parity; Brandon picks which (if any) get built.

## Rules every area follows

- **RULE ZERO.** Every derived surface derives from the desktop client. Each area grepped the desktop counterpart first; where Android deliberately diverged the line says so.
- **Swift only presents.** Logic still living in TypeScript moves to Rust (desktop and Android switched in the same change) before the Swift screen that needs it is built. See `../rust-moves.md`; items marked P1 block Swift work.
- **iPhone look.** System tab bar, NavigationStack, sheets with detents, system menus and popovers, SF Symbols, Apple's real Liquid Glass. Never a port of the Android/Lucide/Phosphor icon or a glass imitation.
- **Android-only mechanisms** get the iOS replacement or `[~]` with the reason: SNInsets bridge to SwiftUI safe areas, scene phase and UIKit APIs; MediaPlaybackService to AVAudioSession + MPNowPlayingInfoCenter; Cast and TV screen to AirPlay through the LAN relay; Firebase push and WorkManager polling to APNs; APK update check to TestFlight; battery optimisation and notification channels to none; the back button to swipe back and sheet dismiss; FileProvider share to ShareLink; Downloads save to a Files export.
- **Native is not WebView.** Audio Boost and Song Identification are possible in a native app (MTAudioProcessingTap on AVPlayer, or the sample-buffer engine), so they are `[ ]`, not `[~]`. WebView-only workarounds are `[~]` only when the observable behavior is not needed natively.

## Areas

Each area file ends with two unnumbered sections: **Decisions for Brandon** (plain bullets, not parity lines: open questions and Android behaviors that may be bugs, each to settle before that screen is built) and **Beyond Android (backlog, Brandon picks)**.

| Area | File | Lines | `[ ]` | `[~]` | Backlog | Decisions | Covers |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| Shell, navigation and design system | [`shell-design.md`](shell-design.md) | 333 | 185 | 18 | 14 | 7 | boot, onboarding, tab bar, layers and navigation, drill-ins, sheets, pull to refresh, haptics map, insets, orientation, system bars, motion, glass, theme tokens |
| Following, Browse and search | [`following-browse-search.md`](following-browse-search.md) | 266 | 167 | 6 | 18 | 5 | Following list and freshness, Browse Live and Categories, category streams, Kick browse, search, the stream card |
| Watch screen and player | [`watch-player.md`](watch-player.md) | 610 | 393 | 46 | 16 | 5 | opening Watch, portrait and immersive landscape layouts, player chrome and gestures, quality, latency, recovery, PiP, lock screen audio, AirPlay in place of Cast and TV screen, clips, offline room, Audio Boost and Song ID |
| Chat view | [`chat-view.md`](chat-view.md) | 650 | 464 | 6 | 16 | 4 | chat rows (names, badges, emotes, paints, mentions, events, moderation display), scroll and buffer rules, fan-out, mod actions, chat tabs, add chat, pinned banner, streak banners, recovery and watchdog |
| Chat composer and emotes | [`chat-input-emotes.md`](chat-input-emotes.md) | 478 | 337 | 5 | 13 | 12 | composer, send rules, room-mode gating, autocomplete, emote carousel and picker, chat menu, the full commands list |
| Rewards, badges, drops and channel points | [`rewards-badges-drops.md`](rewards-badges-drops.md) | 454 | 285 | 4 | 11 | 4 | Rewards tab, drops (campaigns, progress, claims, eligibility), badge gallery and detail, in-player drop bar, channel points button and menu, predictions and polls |
| Profiles, user cards and cosmetics | [`profile-cosmetics.md`](profile-cosmetics.md) | 273 | 167 | 2 | 15 | 3 | user profile sheet, own identity header, Cosmetics screen (StreamNook, 7TV and Twitch identity choices) |
| Settings | [`settings.md`](settings.md) | 767 | 479 | 15 | 22 | 11 | the eight settings rows and every panel they open: Player, Ad-free, Chat (the longest), Theme, Notifications and its channels sheet, Cache, Support; phone prefs; copy and structure rules |
| You tab, accounts and sign-in | [`accounts-signin.md`](accounts-signin.md) | 294 | 156 | 5 | 7 | 5 | You tab layout, accounts card, the in-app sign-in sheet that replaces the Android login overlay, Twitch, Drops, Kick, 7TV and Subscribe flows, credentials at rest |
| Notifications, lifecycle and platform services | [`system.md`](system.md) | 323 | 147 | 34 | 13 | 8 | notification kinds and lanes (APNs in place of Firebase and WorkManager), taps and deep links, lifecycle and background gate, update check, What's New, share, logs, version reporting, the SNInsets bridge inventory |
| **Total** | | **4448** | **2780** | **141** | **145** | **64** | |

## Ownership map (every `src/mobile` file has exactly one owner)

- **shell-design** (19): `MobileApp.tsx`, `MobileOnboarding.tsx`, `MobileTabBar.tsx`, `boot/useMobileBoot.ts`, `mobile.css`, `navStore.ts`, `inPlaceBack.ts`, `nativeInsets.ts`, `systemBars.ts`, `ui/AdaptiveGrid.tsx`, `ui/DrillInScreen.tsx`, `ui/MobileSheet.tsx`, `ui/PullToRefresh.tsx`, `ui/SettleIn.tsx`, `ui/SkeletonCards.tsx`, `ui/haptics.ts`, `ui/useKeyboardInsets.ts`, `ui/useWindowShape.ts`, `ui/windowShape.test.ts`
- **following-browse-search** (9): `screens/FollowingScreen.tsx`, `screens/BrowseScreen.tsx`, `screens/CategoryStreamsScreen.tsx`, `screens/KickBrowseBody.tsx`, `ui/MobileStreamCard.tsx`, `followRefresh.ts`, `followingMerge.ts`, `followingMerge.test.ts`, `useHypeTrains.ts`
- **watch-player** (17): `screens/WatchScreen.tsx`, `player/MobilePlayer.tsx`, `player/QualitySheet.tsx`, `player/TogetherSheet.tsx`, `player/liveVideo.ts`, `player/lockScreenAudio.ts`, `player/useMobileHlsEngine.ts`, `watch/ChatOverlayChrome.tsx`, `watch/DialIndicator.tsx`, `watch/MobileClipSheet.tsx`, `watch/SplitHandle.tsx`, `watch/watchLayout.ts`, `cast/CastCard.tsx`, `cast/CastSheet.tsx`, `cast/castState.ts`, `cast/tvScreenState.ts`, `videoCap.ts`
- **chat-view** (14): `chat/MobileChatPane.tsx`, `chat/ChatFanOut.tsx`, `chat/ChatTabStrip.tsx`, `chat/AddChatSheet.tsx`, `chat/StreakBanners.tsx`, `chat/chatRecovery.ts`, `chat/chatTabsStore.ts`, `chat/chatTabsStore.test.ts`, `chat/useChatWatchdog.ts`, `chat/useLongPressDrag.ts`, `chat/modActions.ts`, `chat/searchOrder.ts`, `chat/searchOrder.test.ts`, `watch/PinnedBanner.tsx`
- **chat-input-emotes** (7): `chat/MobileChatInput.tsx`, `chat/ComposerMenuSheet.tsx`, `chat/MobileCommandSheet.tsx`, `chat/MobileEmoteCarousel.tsx`, `chat/chatGating.ts`, `chat/useEmoteOwnerNames.ts`, `chat/useFollowStatus.ts`
- **rewards-badges-drops** (9): `screens/RewardsScreen.tsx`, `rewards/MobileMissingNow.tsx`, `rewards/badgeGalleryStore.ts`, `rewards/badgeSort.ts`, `rewards/badgeSort.test.ts`, `dropsCampaigns.ts`, `dropsEligibility.ts`, `watch/DropProgressBar.tsx`, `chat/useChannelPoints.ts`
- **profile-cosmetics** (3): `profile/UserProfileSheet.tsx`, `profile/OwnIdentityHeader.tsx`, `screens/CosmeticsScreen.tsx`
- **settings** (5): `screens/SettingsScreen.tsx`, `screens/AdFreeSettings.tsx`, `screens/MobileNotificationsSettings.tsx`, `screens/NotifiedChannelsSheet.tsx`, `phonePrefs.ts`
- **accounts-signin** (3): `screens/YouScreen.tsx`, `profile/AccountsCard.tsx`, `cosmetics/sevenTvConnect.ts`
- **system** (8): `notifications.ts`, `notifyChannels.ts`, `lifecycle.ts`, `backgroundGate.ts`, `updateCheck.ts`, `nativeBridge.ts`, `screens/MobileWhatsNew.tsx`, `acl_parity.test.ts`

94 files in `src/mobile`, each owned once. Test files are owned with the module they test; their cases are listed as the acceptance tests the Rust port must keep.

Shared and desktop files are not owned: any area may cite them. When two areas describe the same screen, the owner of the source file writes the behavior and the other writes one cross-reference line (for example, the points button lives in the composer file but is a channel points feature, so chat-input-emotes points at rewards-badges-drops § 10). A duplicate pass ran at merge; repeat it whenever a builder adds lines.

## Brandon's preferences encoded as behaviors

| Preference | Rule | Encoded in |
| --- | --- | --- |
| `landscape-watch-always-immersive.md` | no system bars in landscape watching, any layout | watch-player, shell-design, settings, system |
| `never-drop-fps.md` | animated renders keep full frame rate and clean loops | every area that animates |
| `no-ui-moves-down.md` | overflow compresses in place, never a second row | shell-design, watch-player, settings |
| `platform-marks-only-where-mixed.md` | platform logos only where platforms mix | following-browse-search, chat-view, watch-player, accounts-signin, shell-design, settings |
| `no-mini-player-previews.md` | no thumbnail-and-play tiles in the player; status is a dot and words | watch-player |
| `visual-settings-get-previews.md` | a setting whose look its name cannot convey gets a live preview | settings, chat-view, profile-cosmetics |
| `settings-reset-arrows.md` | every preference row gets a reset arrow | settings |
| `settings-copy-standard.md` | a section per feature, short noun rows, dependents nested, speak to the viewer | settings |
| `chat-header-scale.md` | 26 px capsules, 22 px pills, 11.5 px text, 18 px faces | chat-view, profile-cosmetics |
| `glassiness-zero-contract.md` | no see-through surface at 0% | shell-design, settings, chat-view, chat-input-emotes, watch-player |
| `no-total-user-count-public.md` | member numbers are fine, "of N" totals never ship | rewards-badges-drops, profile-cosmetics, settings |
| `users-vocabulary-not-api-categories.md` | never surface an API split in UI copy | rewards-badges-drops, settings |
| `no-competitor-references-user-facing.md` | no competitor names in user-facing text (backlog lines describe features without naming apps) | every area |
