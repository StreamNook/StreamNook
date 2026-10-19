# Parity contract: Notifications, lifecycle and platform services

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

Kotlin citations use `r:gen/android/app/src/main/java/app/streamnook/<File>.kt` (the path resolves under `src-tauri/`). The manifest is `src-tauri/gen/android/app/src/main/AndroidManifest.xml`, cited by name.

Owned files: `m:notifications.ts`, `m:notifyChannels.ts`, `m:lifecycle.ts`, `m:backgroundGate.ts`, `m:updateCheck.ts`, `m:nativeBridge.ts`, `m:screens/MobileWhatsNew.tsx`, `m:acl_parity.test.ts`. Cross-referenced areas: shell-design (boot, nav, insets, system bars), watch-player (player PiP, lock-screen audio, gestures), settings (Notifications panel, NotifiedChannelsSheet, Support rows), accounts-signin, rewards-badges-drops, chat-view.

## 1. Notification kinds and their copy

### 1.1 A followed Twitch channel went live

- [ ] Title is "{user_name} is live" (display name, not login) (r:android_notify.rs:L734-L741) (iPhone: APNs alert `title`, built by Rust or the Worker sender, never by Swift)
- [ ] Body is the stream title; when the title is empty it is "Playing {game_name}"; when both are empty the body is empty (r:android_notify.rs:L727-L733) (iPhone: APNs alert `body`, same fallback order)
- [ ] The push lane uses the server's `title` and `body` fields, falling back to "{login} is live" and an empty body when the payload omits them (k:SNMessagingService.kt:L39-L53) (iPhone: the Worker always sends both fields; the Notification Service Extension keeps the same fallback)
- [ ] The streamer's Twitch profile image is shown as the notification's large picture, fetched from Helix `users?id=` for only the channels being announced (up to 100 ids per call), and a failed fetch costs the picture, never the notification (r:android_notify.rs:L583-L627, L754-L762) (k:NotifyRenderer.kt:L73, L96-L106) (iPhone: `UNNotificationAttachment` added by a Notification Service Extension with `mutable-content: 1`; on failure deliver without it)
- [ ] Only Twitch follows from Helix `streams/followed` are announced, fully paged at 100 per page with a hard stop of 10 pages (r:android_notify.rs:L526-L576) (iPhone: same rule in the Worker / Rust collector)
- [ ] Category is "Channels going live" with heads-up importance (Android IMPORTANCE_HIGH, 4), description "A channel you follow started streaming" (m:notifications.ts:L133-L139) (k:NotifyChannels.kt:L27) (iPhone: `interruptionLevel = .active`, default sound, `threadIdentifier` "live-channels")
- [ ] Desktop announces the same event in-app as "{streamer_name} is now live!" through the Dynamic Island / toast and its native OS notification is a stub; the phone deliberately uses the system shade instead and its copy is "{user_name} is live" (d:components/DynamicIsland.tsx:L504-L508, L595-L701) (r:services/live_notification_service.rs:L65-L104) (iPhone: system notification, phone copy)

### 1.2 New badge

- [ ] Title is "New badge: {badge_name}", body is the badge description or empty (r:android_notify.rs:L134-L143) (m:boot/useMobileBoot.ts:L328-L347) (iPhone: same copy in the local notification)
- [ ] The background lane shows the badge image as the large picture; the in-app lane shows no picture because the Tauri plugin cannot load a remote image (r:android_notify.rs:L140) (m:notifications.ts:L188-L198) (iPhone: attach the badge image in BOTH lanes; a native app can, so show it always)
- [ ] Tapping a badge notification only brings the app up; it opens nothing (k:NotifyRenderer.kt:L50-L52) (iPhone: no `login` in `userInfo`, so the tap handler just foregrounds)
- [ ] Only badges the Rust classifier marks NotifyNew or NotifyAvailable notify; corrections and redundant re-pushes store silently (r:android_notify.rs:L111-L147) (r:commands/badge_service.rs:L537-L550) (iPhone: same classifier, Rust owned)
- [ ] A fresh install absorbs the whole relay backlog silently (seed when the feed store is empty) instead of announcing every badge ever dropped (r:android_notify.rs:L106-L119) (iPhone: same, Rust owned)
- [ ] Category "New badges", default importance (3), description "A new global badge became available" (m:notifications.ts:L146-L151) (iPhone: `interruptionLevel = .active`, default sound, `threadIdentifier` "badges")
- [ ] The background lane reads the edge feed `https://modroom.streamnook.app/badges/latest.json` (an array of relay drops each wrapping a `badge`) (r:android_notify.rs:L79-L104) (iPhone: Rust collector; see 2.1 for when it runs)

### 1.3 Drops

- [ ] "Drop ready to claim" posts when Rust emits `drop-ready`, body "{drop_name} · {campaign_name}" with empty parts dropped and no body when both are empty (m:boot/useMobileBoot.ts:L349-L358) (iPhone: local `UNNotificationRequest` from the Rust event)
- [ ] "Drop claimed" posts when Rust emits `drop-claimed`, body "{benefit_name} · {campaign_name}" with the same joining rule (m:boot/useMobileBoot.ts:L360-L372) (iPhone: local notification)
- [ ] Rust emits `drop-ready` once per drop per session and only while `drops.notify_on_drop_available` (default true); `drop-claimed` only after the claim is confirmed granted and while `drops.notify_on_drop_claimed` (default true) (r:services/drops_service.rs:L2055-L2077, L2105-L2111) (r:models/drops.rs:L422-L423) (iPhone: same Rust gates)
- [ ] Drop notifications are in-app lane only: they post only while the app process is alive, never from the background poll (m:boot/useMobileBoot.ts:L349-L372) (r:android_notify.rs:L646-L664) (iPhone: same; there is no background Drops on iOS)
- [ ] Drop notifications carry no stable id, so a repeat stacks a second row rather than replacing (m:notifications.ts:L201-L207, L243-L258) (iPhone: give them an identifier "drop:{drop_id}" so a repeat replaces; this is a fix, see Decisions)
- [ ] Tapping a drop notification only brings the app up (m:notifications.ts:L243-L258) (iPhone: foreground only)
- [ ] Category "Drops", default importance (3), description "Rewards ready to claim, and rewards claimed for you" (m:notifications.ts:L140-L145) (iPhone: `threadIdentifier` "drops")

### 1.4 Channel points (reserved, never posted)

- [~] A "Channel points" category exists at LOW importance ("Bonus chests collected in the background") but nothing posts to it on the phone: points gains show only as the in-app "+N" float (m:notifications.ts:L152-L159) (m:chat/useChannelPoints.ts:L91-L104) (m:screens/MobileNotificationsSettings.tsx:L60-L69) (iPhone: no notification; reserve `threadIdentifier` "channel-points" with `interruptionLevel = .passive` should the Rust watch heartbeat ever report credits)

### 1.5 Not notified on the phone

- [~] Whispers, mentions, gift subs received and Twitch rewards notifications are desktop settings that the phone never posts (no listener, no panel row) (m:screens/MobileNotificationsSettings.tsx:L60-L64) (s:types/index.ts:L705-L751) (iPhone: none, parity with Android; see Beyond Android)
- [~] Kick follows and favourite channels going live raise `streamer-went-live` in Rust on Android too, but the phone has no listener, so they never notify (r:lib.rs:L1230-L1247) (m:boot/useMobileBoot.ts:L318-L322) (iPhone: none until Brandon decides, see Decisions)

### 1.6 Test notification

- [ ] The Notifications panel's "Send a test notification" posts title "StreamNook", body "Notifications are working." with no category (m:screens/MobileNotificationsSettings.tsx:L385-L395) (iPhone: local notification; the panel row is owned by settings.md)

## 2. Delivery lanes

### 2.1 Background poll lane (Android WorkManager)

- [~] A periodic WorkManager job named "sn_live_notifications" runs every `background_interval_minutes` (15, 30 or 60; floor 15; default 15), only with a network connection, policy UPDATE so a changed interval applies without pushing the next run out (k:NotifyScheduler.kt:L22-L47) (m:screens/MobileNotificationsSettings.tsx:L72) (r:models/settings.rs:L733-L736) (iPhone: no periodic background execution exists; APNs replaces it. Optional `BGAppRefreshTask` fallback is backlog)
- [~] Each run calls Rust `pollOnce` over JNI in a process with no app around it; Rust resolves the data dir by probing for `<base>/StreamNook` then `<base>/files/StreamNook`, returns `[]` for every "nothing to do" case, and the worker always reports success (no retries) (r:android_notify.rs:L192-L205, L296-L339) (k:NotifyWorker.kt:L22-L47) (iPhone: replaced by APNs; the collector logic stays in Rust for the foreground check below)
- [ ] On every return to the foreground, at most once per 5 minutes, one immediate notification check runs (Android: a one-shot worker, policy KEEP so a burst of resumes coalesces) (m:lifecycle.ts:L40-L45, L86-L91) (k:NotifyScheduler.kt:L64-L77) (iPhone: on `scenePhase == .active`, throttled 5 min, call the Rust collector and post its items as local notifications; catches anything a missed push left out)
- [ ] The lane is on whenever the master switch `live_notifications.enabled` is on and off only when it is off; `background_checks` is retired and never read (m:notifications.ts:L224-L241) (iPhone: the same master switch drives APNs registration)
- [ ] The schedule is re-synced at every launch and whenever a notification setting changes, so scheduled work never outlives the preference (m:boot/useMobileBoot.ts:L277-L279) (m:screens/MobileNotificationsSettings.tsx:L140-L162) (iPhone: re-sync push registration at launch and on every settings change)

### 2.2 Push lane (Android FCM) to APNs

- [ ] At every launch, when `enabled` and `push_notifications` (default true) are both on, the app registers the device with `https://notify.streamnook.app/register`, fire and forget, never blocking boot (m:boot/useMobileBoot.ts:L281-L292) (pref: discord-connect-boot-hang.md) (iPhone: `UIApplication.registerForRemoteNotifications()` at launch, then Rust `push_register` with the APNs device token; boot never awaits it)
- [ ] Registration sends the device token plus every followed channel's Twitch user id (paged 100 per page, hard stop 20 pages) with `Authorization: OAuth {user_token}`; the server replaces the follow set wholesale; success answers "registered:{count}" (r:android_notify.rs:L425-L479) (iPhone: same body with `apns_token`, bundle topic `app.streamnook` and the APNs environment (sandbox for development-signed builds), Worker sender in plan phase 8)
- [ ] "No token yet" is a normal outcome answered as "no-token", not an error; a non-2xx is an error "register failed: {status}" logged at debug (r:android_notify.rs:L438-L444, L474-L477) (m:boot/useMobileBoot.ts:L288-L290) (iPhone: Swift passes the token straight to Rust instead of a token file, so "no-token" means `didFailToRegisterForRemoteNotifications`)
- [ ] Turning the master or push toggle off unregisters the device (`/unregister` with the token); turning either back on registers again (m:screens/MobileNotificationsSettings.tsx:L153-L159) (r:android_notify.rs:L481-L508) (iPhone: same Rust commands)
- [ ] A token rotation is persisted for the next registration (Android: `onNewToken` writes `<data>/StreamNook/fcm_token`; the token is also refetched once per launch) (k:SNMessagingService.kt:L56-L75) (k:MainActivity.kt:L705-L714) (iPhone: `didRegisterForRemoteNotificationsWithDeviceToken` fires each launch; pass it to Rust every time)
- [ ] The follow set on the server is refreshed only at launch and on toggle changes, not on follow or unfollow mid-session (m:boot/useMobileBoot.ts:L281-L292) (iPhone: same, or better re-register on `refresh-following-list`; see Decisions)
- [ ] Only pushes with data `type: "live"` are handled; anything else is dropped (k:SNMessagingService.kt:L21-L26) (iPhone: the Notification Service Extension ignores other types)
- [ ] Each live push is shown only if Rust `claimShow` allows it: master and live toggles on, channel login not muted (case-insensitive), and this `(channel_id, started_at)` not already recorded (r:android_notify.rs:L341-L423) (iPhone: either the Worker applies the same gates per device or the Notification Service Extension links the Rust claim; see Decisions)
- [ ] On a fresh install with no state file, a push is shown but nothing is recorded, so the first poll still seeds silently (r:android_notify.rs:L350-L356, L400-L404) (iPhone: same asymmetry wherever the claim lives)
- [ ] A device with no push support keeps delivering through the poll lane alone (k:MainActivity.kt:L705-L714) (iPhone: if APNs registration fails, only the foreground check of 2.1 delivers)

### 2.3 In-app lane

- [ ] While the app runs, Rust events `badge-notification`, `drop-ready` and `drop-claimed` post system notifications from the shell, each gated by `enabled !== false` and its own category toggle (`show_badge_notifications`, `show_drops_notifications`, default true) (m:boot/useMobileBoot.ts:L241-L256, L328-L372) (iPhone: Swift subscribes to the same Rust events and posts `UNNotificationRequest`s; gating moves to Rust, see moves)
- [ ] There is no in-app `streamer-went-live` listener on the phone: live alerts come only from the background/push lanes, app open or closed (m:boot/useMobileBoot.ts:L318-L322) (iPhone: same; live alerts are APNs plus the foreground check only)
- [ ] Desktop's 60 s live-notification service is gated off Android only, so as written it would arm on iOS and diff the Home snapshot's followed list into `streamer-went-live` events nobody listens to (r:lib.rs:L1220-L1223, L1291-L1304, L1306-L1311) (r:services/live_notification_service.rs:L57-L104) (iPhone: gate it `cfg(desktop)`; see Decisions)
- [ ] Notifications post while the app is in the foreground too, landing in the shade (iPhone: implement `userNotificationCenter(_:willPresent:)` returning `[.banner, .list, .sound]` so foreground posts show)
- [ ] The notification plugin's own JS wrapper is never used (it throws without the Web Notifications API); commands are invoked directly: `invoke plugin:notification|is_permission_granted`, `request_permission`, `create_channel`, `notify` (m:notifications.ts:L1-L8, L19-L26, L66-L78, L162-L173, L243-L258) (iPhone: `[~]` WebView workaround, not needed natively: Swift calls `UNUserNotificationCenter` directly)

### 2.4 Dedupe, identity and grouping

- [ ] A live alert is announced at most once per broadcast: state `notify_state.json` keyed by Twitch user id holds the stream's `started_at`; a different `started_at` for the same channel is a new broadcast and does notify (r:android_notify.rs:L48-L61, L696-L726) (iPhone: same Rust state; key space is the Twitch user id)
- [ ] Every sighting of an ongoing stream refreshes its `seen_at`, and entries age out only after 7 days unseen, so a 24/7 stream is never re-announced (r:android_notify.rs:L31-L34, L703-L714, L744-L751) (iPhone: same)
- [ ] The very first poll after install (no state file) records everyone currently live and announces nothing (r:android_notify.rs:L227-L237, L686-L726) (iPhone: same)
- [ ] All state read-modify-writes are serialized by one process lock, with the network done before the lock (r:android_notify.rs:L41-L46, L679-L689) (iPhone: same lock in Rust; if the Notification Service Extension runs the claim, it is a second process and needs a file lock)
- [ ] A muted login is skipped before dedupe, compared lowercase (r:android_notify.rs:L666-L699) (iPhone: same)
- [ ] Notification identity is the item's `channel_id` string ("{twitch_user_id}" for live, "badge:{set_id}-v{version}" for badges); a second post for the same identity replaces the first instead of stacking (r:android_notify.rs:L153-L171) (k:NotifyRenderer.kt:L75-L77) (m:boot/useMobileBoot.ts:L339-L345) (m:notifications.ts:L210-L222) (iPhone: `UNNotificationRequest.identifier` = the same string, and `apns-collapse-id` = the same string on pushes; no Java hash needed)
- [ ] Notifications are grouped per category ("app.streamnook.GROUP.{category}") with one summary row per category that produced something, so several arrivals collapse into one row (k:NotifyRenderer.kt:L69-L71, L80-L93) (iPhone: `threadIdentifier` = category id; iOS stacks automatically)
- [ ] Tapping a notification dismisses it (auto-cancel) (k:NotifyRenderer.kt:L68) (iPhone: default)
- [~] Every notification carries the monochrome status-bar icon `ic_stat_notify` (m:notifications.ts:L175-L182) (k:NotifyRenderer.kt:L65) (iPhone: iOS always uses the app icon; nothing to set)
- [ ] The desktop gate dedupes go-live per login for 5 minutes and merges the same person across platforms within 10 minutes; the phone's lane uses the persistent `started_at` dedupe instead (r:services/live_announce.rs:L1-L31) (iPhone: phone rule)

### 2.5 Quiet rules and per-channel opt-out

- [ ] Master switch `live_notifications.enabled` (default true) silences every lane; category toggles `show_live_notifications`, `show_badge_notifications`, `show_drops_notifications` (default true) silence one each; an absent field reads as ON (r:android_notify.rs:L629-L664) (r:models/settings.rs:L746-L776) (iPhone: same Rust-read settings)
- [ ] Per-channel opt-out is an EXCLUSION list `muted_live_channels` of lowercase Twitch logins: empty means every follow notifies, so an upgrade changes nothing (m:notifyChannels.ts:L1-L26) (r:models/settings.rs:L718-L722) (iPhone: same list, same key space)
- [ ] Toggling a channel writes the whole settings object with the login added or removed and returns the new muted state for optimistic rendering; with settings not yet loaded it writes nothing and returns the current state (m:notifyChannels.ts:L28-L48) (iPhone: a Rust read-modify-write command, see moves)
- [ ] The player shows a bell next to a Twitch channel's name to toggle its live alerts (filled bell on, slashed bell at 45% white off; aria-labels "Turn off live alerts for {user_name}" / "Turn on live alerts for {user_name}") (m:player/MobilePlayer.tsx:L134, L410-L428) (iPhone: owned by watch-player.md; SF Symbols `bell.fill` / `bell.slash`)
- [ ] The full per-channel list lives in the NotifiedChannelsSheet (m:screens/NotifiedChannelsSheet.tsx:L95, L129) (iPhone: owned by settings.md)
- [~] No quiet hours, no snooze; the OS category importance (heads-up, default, silent) is the only volume control (m:notifications.ts:L107-L160) (iPhone: Focus modes cover quiet hours; per-category importance maps to `interruptionLevel` only)

## 3. Permission and OS delivery state

- [~] First run: the setup wizard's Notifications step (Android step 7, the iPhone page before "You're all set") shows a bell, title "Notifications", body "Choose what StreamNook alerts you about. You can change these any time in Settings.", and a switch per alert type Rust marks for setup on the platform (`get_alert_types`, `setup: true`: Android Live channels and Mentions and replies, iPhone Mentions and replies), every one off; the filled button reads "Skip" until one is on, then "Continue". The step is left out when Rust offers no type (m:setup/NotificationsStep.tsx) (d:components/SetupWizard.tsx) (iPhone: SetupWizard.swift `.notifications`) (v3-notify 2026-10-19: built and gated, not yet seen on either phone)
- [~] Turning a type on writes its setting (and the master switch on) and only then asks the OS: Android `ensureNotificationPermission`, iPhone `requestAuthorization` when `.notDetermined`. A refusal shows "Notifications are turned off for StreamNook in Android settings / iPhone Settings, so these alerts will not arrive until you allow them there." with an "Open Android settings" / "Open iPhone Settings" button; its room is kept while hidden so the switches never move (v3-notify 2026-10-19: built, not seen)
- [~] Leaving the step forward writes every offered type as shown (chosen on, the rest off), so a skipped step turns those alerts off (v3-notify 2026-10-19: built, not seen)
- [~] After setup, at boot, the app asks exactly once by itself when setup is complete, the permission has never been asked ('default'), the master switch is on and some listed type is on, for people who updated in place and never saw the wizard step; someone who chose no type in the wizard is never asked (m:boot/useMobileBoot.ts) (iPhone: no boot ask; it asks when a type is turned on)
- [ ] Permission has four states: 'granted'; 'default' (never asked); 'denied' (asked, can ask again); 'blocked' (the OS will no longer show the dialog), derived from the rationale flag plus a local "asked" marker `sn-notif-asked` (m:notifications.ts:L28-L63) (iPhone: `[~]` iOS reports this directly: `.notDetermined` = default, `.denied` = blocked (iOS never re-prompts), `.authorized`/`.provisional`/`.ephemeral` = granted; there is no re-askable 'denied')
- [ ] Asking is skipped when already granted, and the asked marker is set before the system dialog (m:notifications.ts:L65-L78) (iPhone: not needed; the OS tracks it)
- [ ] The panel reads everything that can silence delivery in one go: permission, the app-level switch, and which categories are silenced in system settings (importance 0; -1 means not created yet, null means unknown) (m:notifications.ts:L80-L105) (m:nativeBridge.ts:L211-L241) (iPhone: `UNNotificationSettings` gives `authorizationStatus`, `alertSetting`, `lockScreenSetting`, `notificationCenterSetting`; per-category blocking is `[~]` iOS has no per-category switches)
- [ ] Four OS categories are created at boot before anything can post, idempotent, with permanent ids `live-channels`, `drops`, `badges`, `channel-points` that are never renamed (m:notifications.ts:L107-L173) (m:boot/useMobileBoot.ts:L258-L261) (k:NotifyChannels.kt:L6-L39) (iPhone: `[~]` no notification channels on iOS; register `UNNotificationCategory`s with the same ids only if actions are added, and use them as `threadIdentifier`s)
- [~] "Open notification settings" and "open this category's settings" jump to the OS screens (m:nativeBridge.ts:L262-L276) (k:MainActivity.kt:L551-L580) (iPhone: `UIApplication.open(URL(string: UIApplication.openNotificationSettingsURLString)!)`; no per-category screen exists)
- [~] Battery-optimisation exemption status and request (Android withholds network from background jobs in Rare/Restricted buckets) (m:nativeBridge.ts:L278-L306) (k:MainActivity.kt:L582-L615) (iPhone: none on iOS; the settings row is not shown)
- [ ] The panel re-reads delivery state whenever the app returns to the foreground, so a fix made in system settings shows at once (m:screens/MobileNotificationsSettings.tsx:L109-L135) (iPhone: owned by settings.md; refresh on `scenePhase == .active`)

## 4. Notification taps and deep links

- [ ] Tapping a live alert opens Watch on that channel: the login is trimmed and lowercased and passed to `startStream`, which opens the player full (m:boot/useMobileBoot.ts:L294-L316) (m:screens/WatchScreen.tsx:L546-L549) (iPhone: `userNotificationCenter(_:didReceive:)` reads `userInfo["login"]` and calls the Rust start-stream path; Watch presented full)
- [ ] Cold start (the tap launched the app): the login is held natively and drained once by the shell after boot (settings and auth loaded), so it opens the channel as soon as the app is ready (k:MainActivity.kt:L85-L88, L699-L703, L633-L646) (m:nativeBridge.ts:L333-L347) (m:boot/useMobileBoot.ts:L305-L310) (iPhone: set the `UNUserNotificationCenter` delegate before launch finishes; hold the login until boot completes, then open)
- [ ] Warm (app already running): the tap reaches the running shell directly through `window.__SN_OPEN_CHANNEL__(login)`; a tap that lands before that handler exists stays held and is drained on mount (k:MainActivity.kt:L649-L671) (m:boot/useMobileBoot.ts:L305-L307) (iPhone: delegate call; if boot is still running, queue it)
- [ ] Each notification's tap intent is distinct per item so two alerts never open the same stream (k:NotifyRenderer.kt:L48-L62) (iPhone: per-request `userInfo`)
- [ ] A pending notification tap wins over a pending deep link at cold start; only when there is no tap is `take_pending_watch_link` drained (m:boot/useMobileBoot.ts:L307-L316) (iPhone: same priority)
- [ ] `streamnook://watch/<channel>`, `streamnook://w/<channel>` and bare `streamnook://<channel>` resolve in Rust to a login (lowercase, `[a-z0-9_]`, at most 25 chars); warm links emit `streamnook:watch`, cold links are parked for `invoke take_pending_watch_link` (r:lib.rs:L69-L117, L1138-L1174) (iPhone: `CFBundleURLTypes` scheme `streamnook` + `.onOpenURL`, parsed by the same Rust function, then start-stream)
- [~] On Android the scheme is configured for desktop only, so the manifest's deep-link block is empty and `streamnook://` never opens the phone app; the phone also has no `streamnook:watch` listener (desktop has one) (t:tauri.conf.json:L55-L59) (AndroidManifest.xml L44-L46) (d:App.tsx:L728-L756) (iPhone: register the scheme and handle warm and cold; see Decisions)
- [ ] Desktop defers a link that arrives mid-boot until boot finishes, then plays it (d:App.tsx:L733-L760) (iPhone: same deferral)

## 5. App lifecycle

### 5.1 Going to the background

- [ ] The handler is installed once at shell mount and is idempotent (m:lifecycle.ts:L122-L136) (m:MobileApp.tsx:L105) (iPhone: observe `scenePhase` in the App scene)
- [ ] Going hidden while in picture-in-picture is NOT backgrounding: nothing below runs (m:lifecycle.ts:L59-L72) (iPhone: while `AVPictureInPictureController.isPictureInPictureActive`, treat `.background` as foreground for these rules)
- [ ] Going hidden stamps the time, sets the background flag before any await, and fires `invoke flush_persistent_stores` without waiting (m:lifecycle.ts:L71-L83) (iPhone: on `scenePhase == .background` call the Rust flush; wrap in `beginBackgroundTask` so it completes)
- [ ] The flush writes settings, the universal cache manifest, the mod log, whispers, VOD progress and chat logs now, because the OS kills a background process with no exit event; each is a no-op when clean (r:commands/app.rs:L297-L314) (iPhone: same command)
- [ ] Playback is deliberately not paused or stopped by backgrounding: a stream keeps playing audio in the background (m:lifecycle.ts:L22-L26) (m:backgroundGate.ts:L13-L14) (iPhone: owned by watch-player.md, AVAudioSession `.playback` + `UIBackgroundModes: audio`)
- [ ] The Rust badge-drop feed keeps its socket up while backgrounded (m:lifecycle.ts:L18-L20) (iPhone: iOS suspends the process when no audio plays; the socket dies and must reconnect on foreground, Rust-side)

### 5.2 Coming back

- [ ] Coming back clears the background flag first (m:lifecycle.ts:L85-L86) (iPhone: `scenePhase == .active`)
- [ ] One notification check runs, at most once per 5 minutes (m:lifecycle.ts:L87-L91) (iPhone: see 2.1)
- [ ] Entitlement registries (membership, cosmetics, atmospheres) are pulled once, and again from boot's own listener throttled to 10 s (m:lifecycle.ts:L92-L99) (m:boot/useMobileBoot.ts:L402-L413) (iPhone: one Rust-side refresh per foreground, 10 s throttle; the double call is a TS duplicate)
- [ ] The followed list reloads if it is older than 60 s and the user is signed in, the stamp set up front so a second resume does not start a second load; it also bumps the preview-image generation and asks Rust to refresh the `hype_trains` and `collaborations` Home sections (m:followRefresh.ts:L21-L73) (iPhone: Rust `refresh_home_section` + followed refresh on foreground; owned by the Following area for the list itself)
- [ ] After 90 s or more away, chat is rebuilt: every open Twitch room is released, `invoke stop_chat`, then re-acquired, and each pane's reload nonce bumps so rooms refill with recent history; shorter absences are left to the 2-minute staleness watchdog (m:lifecycle.ts:L47-L57, L105-L119) (m:chat/chatRecovery.ts:L27-L105) (iPhone: Rust-owned on foreground; chat-view.md owns the panes)
- [ ] A chat rebuild is single-flight and skipped if one ran in the last 60 s (a manual reload bypasses this), and Kick rooms are never cycled by it (m:chat/chatRecovery.ts:L27-L51) (iPhone: same rule in Rust)
- [ ] A return from PiP does not count as an absence (no hidden stamp was taken), so it never triggers a chat rebuild (m:lifecycle.ts:L71-L73, L110-L112) (iPhone: same)
- [ ] The chat socket schedules an immediate reconnect when the app becomes visible or the network comes back and the socket is not open (s:stores/chatConnectionStore.ts:L4002-L4027) (iPhone: Rust reconnect on foreground and on `NWPathMonitor` satisfied)
- [ ] The auth recheck runs every 5 minutes and skips ticks while hidden (m:boot/useMobileBoot.ts:L418-L425) (iPhone: Rust timer that skips while backgrounded)
- [ ] Platform accounts (Kick, YouTube, TikTok roster) refresh on every return to the foreground (s:hooks/usePlatformAccountSync.ts:L103-L106) (iPhone: owned by accounts-signin.md)
- [ ] Foreground resume never opens EventSub sockets from the UI; only Rust's watch session aims the watch socket (pref: eventsub-socket-ownership.md) (iPhone: Swift never starts a socket on resume)

### 5.3 Background gate (polls that skip a tick)

- [ ] One process-wide flag "backgrounded" is true only while hidden and not in PiP (m:backgroundGate.ts:L16-L26) (iPhone: a Rust flag set from `scenePhase`, see moves)
- [ ] Polls skip a tick while backgrounded instead of tearing their intervals down; the next tick after resume re-reads (m:backgroundGate.ts:L7-L11) (iPhone: same rule in each Rust poller)
- [ ] Pinned chat messages poll every 30 s and skip while backgrounded (m:screens/WatchScreen.tsx:L561-L589) (iPhone: chat-view.md / Rust channel_state)
- [ ] Channel points balance polls every 60 s and skips while backgrounded (m:chat/useChannelPoints.ts:L35, L206-L214) (iPhone: rewards area / Rust channel_state)
- [ ] Drop progress polls every 60 s and skips while backgrounded (m:watch/DropProgressBar.tsx:L31, L193-L203) (iPhone: rewards-badges-drops.md)
- [ ] Playback is never gated (m:backgroundGate.ts:L13-L14) (iPhone: watch-player.md)

### 5.4 Picture-in-picture at app level

- [ ] PiP is told apart from backgrounding by a synchronous native read, with the page-side mirror `data-sn-pip` only as fallback, because the async PiP event races the visibility change (m:lifecycle.ts:L59-L69) (m:nativeBridge.ts:L141-L160) (iPhone: `isPictureInPictureActive` is synchronous)
- [ ] Leaving the app while a stream plays enters system PiP automatically unless the background mode setting is 'audio' (m:screens/WatchScreen.tsx:L489-L499) (k:MainActivity.kt:L171-L176, L743-L755) (iPhone: `canStartPictureInPictureAutomaticallyFromInline`; owned by watch-player.md)
- [ ] Closing PiP with its X (not expanding it) ends the stream; the closed flag is read once on the way back in and cleared, so the stream is not left loaded and does not reappear next open (k:MainActivity.kt:L798-L815) (m:nativeBridge.ts:L162-L175) (m:screens/WatchScreen.tsx:L451-L465) (iPhone: `pictureInPictureControllerDidStopPictureInPicture` without a restore call = closed; tear the stream down)
- [ ] Nothing may revive a closed stream: a late quality swap that resolves after the close must not remount audio (pref: android-phantom-audio-transient-swap.md) (s:stores/AppStore.ts:L2512-L2525) (iPhone: owned by watch-player.md; Rust start sequence check)
- [ ] Returning from PiP restores the system bar appearance (k:MainActivity.kt:L787-L789) (iPhone: `[~]` not needed, SwiftUI owns status bar style)
- [~] The WebView is kept resumed while in PiP and while the media service runs, undoing wry's pause, or PiP freezes and audio dies (k:MainActivity.kt:L757-L786) (iPhone: WebView workaround, not needed natively: AVPlayerLayer keeps rendering in PiP and AVAudioSession keeps audio alive)

### 5.5 Screen, display, fold, back, teardown

- [~] Screen off and on raise `sn:screen-off` / `sn:screen-on` so the player drops to the audio-only rendition (Chromium tears the media pipeline down without a surface) (k:MainActivity.kt:L977-L1011) (m:player/lockScreenAudio.ts:L49-L60, L165-L184) (iPhone: WebView workaround, not needed natively; AVPlayer keeps audio when locked. Whether to drop to audio-only for data is watch-player.md's call)
- [~] Display mode changes (including a 1440p/1080p resolution switch that leaves CSS size unchanged) raise `sn:display-changed` so the rendition cap is re-reported (k:MainActivity.kt:L963-L976, L1013-L1030) (m:videoCap.ts:L42-L60, L89) (iPhone: an iPhone's panel mode is fixed; read `UIScreen.main.nativeBounds` once; watch-player.md owns the cap)
- [~] Foldable posture publishes `--sn-fold-x`, `--sn-fold-w`, `data-sn-fold`, `data-sn-fold-posture` and `sn:fold` (k:MainActivity.kt:L832-L887) (m:ui/useWindowShape.ts:L121) (iPhone: no foldable iPhones; nothing to build)
- [~] The hardware back button asks the page first through `window.__SN_BACK__()` and backgrounds the task (never finishes it) when the page does not consume it (k:MainActivity.kt:L725-L740) (m:navStore.ts:L124-L125) (iPhone: swipe back and sheet dismiss; owned by shell-design.md)
- [~] Configuration changes never recreate the activity (orientation, uiMode, density, locale, screen size) and re-assert the bar icon colour (AndroidManifest.xml L31) (k:MainActivity.kt:L818-L830) (iPhone: SwiftUI handles rotation without recreation)
- [ ] When the task is swiped away (the activity really finishing), playback stops through the same "stop" media command the lock-screen card uses, the media service is stopped, and the WebView is destroyed so no audio survives without an app (k:MainActivity.kt:L1032-L1111) (iPhone: termination kills the process; additionally on `applicationWillTerminate` stop playback and flush, so no Now Playing card lingers)
- [~] While casting, the volume keys step the TV volume (k:MainActivity.kt:L1113-L1126) (iPhone: Chromecast is a non-goal; AirPlay volume follows the system)

### 5.6 iOS-only lifecycle duties

- [ ] On every return to the foreground, the loopback relay re-binds its listening port (iOS tears down a suspended app's listeners) before the player resumes (no Android counterpart; plan phase 2 "relay re-bind on foreground") (iPhone: `willEnterForeground` to a Rust `relay_rebind`, checked before `play()`)
- [ ] A memory warning trims decoded image and emote caches (no Android counterpart; plan phase 2 "decoded size caches + memory warnings") (iPhone: `UIApplication.didReceiveMemoryWarningNotification` to a Rust trim command plus the Swift image cache)
- [ ] Rust background pollers (home snapshot, channel state, badge feed, version report) skip ticks while the scene is backgrounded and not playing or in PiP (no Android counterpart in Rust; Android freezes the process instead) (iPhone: scene phase pushed to Rust)

## 6. Update check

- [~] On opening the You tab (never at boot), the app fetches `https://streamnook.app/api/v1/update-android` with `cache: no-store`; a non-2xx (503 is "nothing published") or any error is silent and retried next time; it logs "[Update] running {current}, published {version}" at warn (m:updateCheck.ts:L17, L67-L89) (m:screens/YouScreen.tsx:L24-L37) (iPhone: updates come from TestFlight; no in-app check)
- [~] "Newer" is a numeric per-segment compare where missing segments are 0, never a string compare (m:updateCheck.ts:L50-L65) (iPhone: not needed with TestFlight)
- [~] The running version comes from the merged Tauri config (Android 0.1.x), not `get_current_app_version` which is the desktop 8.x number (m:updateCheck.ts:L20-L40) (iPhone: `CFBundleShortVersionString`, which must equal Rust `package_info().version` on iOS, plan phase 2)
- [~] When newer, a row shows "Update to {latest}" (15 px semibold) over "You are on {current}" plus " · {size} MB" when the manifest gives a size (12.5 px muted), accent tint 10% background, down-arrow icon and a chevron; tapping opens `https://streamnook.app/download/android` in the browser and a refusal toasts "Could not open the download page." (m:screens/YouScreen.tsx:L72-L99) (m:updateCheck.ts:L91-L101) (iPhone: none; TestFlight notifies testers)
- [~] F-Droid installs see the same prompt today (pref: fdroid-self-hosted-repo.md) (iPhone: not applicable)
- [ ] Desktop's updater checks `check_for_bundle_update` and records the outcome that the version report carries; the phone's TS check records nothing (s:services/updateStatus.ts:L46) (r:services/version_report.rs:L124-L152) (iPhone: no check; the report sends no update fields)

## 7. What's New

- [ ] Reached from the Settings list row "What's New" ("Recent releases") (m:screens/SettingsScreen.tsx:L21-L24, L52) (iPhone: row owned by settings.md; pushes this screen in the NavigationStack)
- [ ] Desktop's What's New lists GitHub releases (8.x); the phone deliberately shows its own line's notes instead, because desktop notes describe changes that never shipped to the phone (m:screens/MobileWhatsNew.tsx:L1-L15) (d:components/ChangelogOverlay.tsx:L103) (iPhone: show the iOS version line's notes only)
- [ ] Data comes from `invoke get_android_changelog`: Rust fetches the update manifest, splits `notes` into paragraphs on blank lines, takes each paragraph's first sentence (a stop followed by whitespace, so "0.1.9" does not split) as the title with the trailing "." removed and the rest as the body (r:commands/settings.rs:L736-L741) (r:services/changelog.rs:L480-L547) (iPhone: an iOS twin of this Rust command reading the iOS notes source; see Decisions)
- [ ] Each change gets a topic chosen by keywords in priority order: audio, power, playback, profile, link, chat, notifications, fix, build, other (r:services/changelog.rs:L486-L510) (iPhone: icons as SF Symbols: `speaker.wave.2`, `battery.100.bolt`, `play.circle`, `person.crop.circle`, `link`, `bubble.left.and.bubble.right`, `bell`, `wrench.and.screwdriver`, `hammer`, `sparkles`)
- [ ] Only the current release is shown (one entry, no history) (m:screens/MobileWhatsNew.tsx:L12-L15) (iPhone: same)
- [ ] The running version and the notes load in parallel (m:screens/MobileWhatsNew.tsx:L88-L113) (iPhone: `.task` with `async let`)
- [ ] Loading shows a designed skeleton, never the word "Loading": a 44 px rounded-12 tile with two bars (20 px x 80 px, 12 px x 144 px) and three 12 px-radius glass cards each with a 32 px rounded-8 tile and three lines (14 px at 40% width, 12 px full, 12 px at 80%), all pulsing; aria-label "Loading release notes" (m:screens/MobileWhatsNew.tsx:L58-L81) (iPhone: `.redacted(reason: .placeholder)` on the real layout, accessibility label "Loading release notes")
- [ ] Unavailable (no manifest, an error, or offline) shows a centred block, 48 px top and bottom padding: a 56 px rounded-16 surface tile with a 24 px muted sparkle, "No release notes right now" (15 px semibold), "These load from the update service, so this usually just means no connection." (13 px muted, max 260 px wide), and "You are on {version}" (12.5 px, version in textSecondary medium) when the version is known (m:screens/MobileWhatsNew.tsx:L117-L134) (iPhone: `ContentUnavailableView` with `sparkles`, same copy)
- [ ] The hero shows a 44 px rounded-12 surface tile with a filled 22 px accent sparkle, the version at 26 px bold tight, and the publish date beside it (12.5 px muted, long month format like "October 12, 2026") (m:screens/MobileWhatsNew.tsx:L136-L163) (iPhone: `Text(date, format: .dateTime.year().month(.wide).day())`)
- [ ] Under the version: "You are on this version" with a filled check (success colour) when the running version equals the release, else "Update available" with a download icon in accent plus " · you are on {running}" muted (m:screens/MobileWhatsNew.tsx:L143-L177) (iPhone: `checkmark.circle.fill` / `arrow.down.circle`; on iOS the update comes from TestFlight)
- [ ] Each change is a 12 px-radius glass card (13 px horizontal, 12 px vertical padding, 12 px gap) with a 32 px rounded-8 surface tile holding the 17 px accent topic icon, the title at 14.5 px semibold snug, and the body at 13 px with 1.5 line height in textSecondary, 3 px below; cards 8 px apart (m:screens/MobileWhatsNew.tsx:L181-L206) (iPhone: List rows or cards with `glassEffect`)
- [ ] A release with no notes shows "No notes for this release." (14 px muted) (m:screens/MobileWhatsNew.tsx:L207-L209) (iPhone: same copy)
- [ ] Release notes are facts only; no promotional framing around them (pref: no-marketing-copy-on-download-page.md) (iPhone: same)

## 8. Share

- [ ] The player's share button opens the system share sheet with text "{user_name} is live: {link}" and subject = stream title or user name; Twitch links are `https://streamnook.app/w/{login}?d={MMDD}` (the landing page hands off to the app or falls back to Twitch), other platforms use their own watch URL (m:player/MobilePlayer.tsx:L511-L534) (s:utils/shareLink.ts:L1-L22) (iPhone: `ShareLink(item: url, subject:, message:)`; button owned by watch-player.md)
- [ ] Without a share target the text is copied to the clipboard instead (m:nativeBridge.ts:L196-L209) (k:MainActivity.kt:L478-L493) (iPhone: `UIActivityViewController` always exists; no fallback needed)
- [ ] Desktop copies the same link to the clipboard rather than sharing (d:components/StreamContextMenu.tsx:L317) (d:components/VideoPlayer.tsx:L627) (iPhone: phone shares, like Android)
- [ ] Log export can share a zip through the share sheet (see 9) (r:commands/logs.rs:L218-L221) (iPhone: `ShareLink` with the zip file URL)

## 9. Logs and diagnostics

- [ ] The app log is `<app_data>/logs/streamnook.log`, with `errors.log` beside it; at 5,000,000 bytes it rotates to `streamnook.log.old`; each start writes a "==== StreamNook {version} started ====" banner (r:services/file_log.rs:L1-L5, L30, L327, L344-L350) (iPhone: same Rust file log under Application Support; the banner is spike criterion 1)
- [ ] Release logging level is Info with the "Detailed log" toggle on (`error_reporting_enabled`, default on) and Warn when off (r:services/file_log.rs:L13-L18) (d:components/settings/SupportSettings.tsx:L265-L289) (iPhone: same; row owned by settings.md)
- [ ] Log export zips every non-empty file in the logs folder (name order) into the cache as `streamnook-logs.zip`, answers "empty" when nothing was logged, and otherwise saves `streamnook-logs-YYYY-MM-DD-HHMM.zip` to Downloads ("saved") or opens the share sheet ("shared"); on Android 9 and older a save falls back to share (r:commands/logs.rs:L141-L226) (iPhone: Rust builds the zip and returns its path; "Save to Files" uses `fileExporter`, "Share" uses `ShareLink`; plan phase 2 export_logs arm)
- [ ] Toasts: "Saved to your Downloads folder." on save, "Nothing has been logged yet." when empty, "Could not get the logs. Try again." on error (d:components/settings/SupportSettings.tsx:L252-L263, L290-L311) (iPhone: owned by settings.md; say "Saved to Files.")
- [~] Desktop opens the logs folder in the file manager instead (d:components/settings/SupportSettings.tsx:L313-L330) (r:commands/logs.rs:L116-L139) (iPhone: no file manager handoff; export only)
- [ ] Frontend warnings go into the same log file (the phone has no devtools, so lifecycle and update lines log at warn on purpose) (r:commands/logs.rs:L15-L53) (m:updateCheck.ts:L77-L80) (m:chat/chatRecovery.ts:L55-L57) (iPhone: Swift logs through a Rust log command so lines interleave)

## 10. Version reporting

- [ ] Rust reports the running client 90 s after start and every 15 minutes: `app_version`, `platform`, `arch`, `target`, `channel`, `report_failures`, plus update-check fields only when a check ran, to `/api/v1/user/sync` (r:services/version_report.rs:L56-L172) (pref: version-reporting-measures-logins.md) (iPhone: same Rust service; `platform` "ios" from Rust's own target, never a UA sniff)
- [ ] A failed send retries after 2 minutes up to 3 times in a row, then falls back to 15 minutes; failures are counted and reported on recovery; no token is not a failure (r:services/version_report.rs:L64-L116) (iPhone: same)
- [ ] `app_version` is the merged-config version (desktop 8.x, Android 0.1.x) (r:services/client_identity.rs:L64-L101) (iPhone: the iOS line from 0.1.0; tauri-headless must return it from `package_info`)
- [ ] The report is named a version report, never telemetry, and what the account records is disclosed in Settings > Support "What your account records" with no toggle (pref: version-report-naming-and-disclosure.md) (iPhone: rows owned by settings.md)

## 11. Native bridge inventory (`window.SNInsets`)

Every bridge call is best-effort and silently absent outside Android (m:nativeBridge.ts:L1-L38). One line per method; behavior lives in the owning area.

### 11.1 Methods

- [ ] `get()`: last insets JSON (top, right, bottom, left, kb, gesture edges) in CSS px (k:MainActivity.kt:L223-L224, L908-L960) (m:nativeInsets.ts:L20) (iPhone: SwiftUI safe area + keyboard safe area; shell-design.md)
- [ ] `setImmersive(on)`: hide/show system bars, swipe shows them transiently (m:nativeBridge.ts:L40-L46) (k:MainActivity.kt:L312-L336) (iPhone: `.statusBarHidden` + `.persistentSystemOverlays(.hidden)`; watch-player.md) (pref: landscape-watch-always-immersive.md)
- [~] `setBarsLightContent(light)`: bar icon colour from the theme (m:nativeBridge.ts:L48-L56) (m:systemBars.ts:L40) (iPhone: `preferredColorScheme` / status bar style; shell-design.md)
- [ ] `setKeepScreenOn(on)`: awake while a stream plays or the TV owns it (m:nativeBridge.ts:L58-L64) (iPhone: `UIApplication.shared.isIdleTimerDisabled`; watch-player.md)
- [ ] `setPreferredRefreshRate(fps)`: 60 while playing, 0 restores (m:nativeBridge.ts:L66-L74) (k:MainActivity.kt:L400-L425) (iPhone: `CADisplayLink.preferredFrameRateRange`; watch-player.md) (pref: never-drop-fps.md)
- [ ] `getScreenBrightness()` / `setScreenBrightness(level)`: window-only brightness, floor 0.02, -1 hands back to the system; 0.5 when unknown (m:nativeBridge.ts:L76-L100) (k:MainActivity.kt:L344-L375) (iPhone: `UIScreen.main.brightness`, restored on leave; watch-player.md)
- [~] `getMediaVolume()` / `setMediaVolume(level)`: device media volume 0..1, 1 when unknown (m:nativeBridge.ts:L102-L123) (k:MainActivity.kt:L377-L398) (iPhone: no app-settable system volume; MPVolumeView slider or AVPlayer volume; watch-player.md)
- [ ] `setPipEligible(on)`: auto-enter PiP on leaving (m:nativeBridge.ts:L125-L131) (iPhone: `canStartPictureInPictureAutomaticallyFromInline`; watch-player.md)
- [~] `enterPip()`: kept but unused by any UI (m:nativeBridge.ts:L133-L139) (k:MainActivity.kt:L495-L513) (iPhone: `startPictureInPicture()` available but not wired to a button)
- [ ] `isInPip()`: synchronous PiP read, null without a bridge (m:nativeBridge.ts:L141-L160) (iPhone: `isPictureInPictureActive`; this area 5.4)
- [ ] `consumePipClosed()`: one-shot "PiP was closed, not expanded" (m:nativeBridge.ts:L162-L175) (iPhone: delegate callback; this area 5.4)
- [~] `setPipSourceRect(l,t,r,b)`: device-pixel video rect for the PiP animation (m:nativeBridge.ts:L177-L185) (m:screens/WatchScreen.tsx:L518-L544) (iPhone: not needed; AVPictureInPictureController animates from the player layer)
- [ ] `setPipMuted(muted)`: PiP window mute action icon (m:nativeBridge.ts:L187-L194) (m:player/MobilePlayer.tsx:L267-L302) (iPhone: `[~]` AVKit's PiP window has no custom actions; mute follows the player)
- [ ] `share(text, subject)`: system share sheet, clipboard fallback (m:nativeBridge.ts:L196-L209) (iPhone: `ShareLink`; this area 8)
- [ ] `areNotificationsEnabled()`, `channelImportance(id)`, `shouldShowNotificationRationale()` (m:nativeBridge.ts:L222-L260) (iPhone: `UNNotificationSettings`; this area 3)
- [~] `openNotificationSettings()`, `openChannelSettings(id)` (m:nativeBridge.ts:L262-L276) (iPhone: `openNotificationSettingsURLString`; no per-channel screen)
- [~] `isIgnoringBatteryOptimizations()`, `requestIgnoreBatteryOptimizations()` (m:nativeBridge.ts:L288-L306) (iPhone: none)
- [~] `scheduleBackgroundChecks(min)`, `cancelBackgroundChecks()` (m:nativeBridge.ts:L308-L322) (iPhone: replaced by APNs registration / unregistration)
- [ ] `runNotifyCheckNow()` (m:nativeBridge.ts:L324-L331) (iPhone: foreground Rust check; this area 2.1)
- [ ] `consumePendingChannel()` (m:nativeBridge.ts:L333-L347) (iPhone: notification-tap queue; this area 4)
- [ ] `mediaSessionStart(title, artist, artUrl, playing)` / `mediaSessionStop()`: lock-screen card and the foreground service that keeps audio alive (m:nativeBridge.ts:L349-L379) (m:player/lockScreenAudio.ts:L60, L231) (iPhone: AVAudioSession `.playback` + MPNowPlayingInfoCenter + MPRemoteCommandCenter; watch-player.md)
- [ ] `displayShortEdge()`: physical short edge of the active display mode for the rendition cap (k:MainActivity.kt:L291-L296) (m:videoCap.ts:L42-L60) (iPhone: `UIScreen.main.nativeBounds` short side; watch-player.md)

### 11.2 Native-to-page signals

- [ ] `sn:pip` (with `data-sn-pip`), `sn:pip-closed`, `sn:pip-mute` (k:MainActivity.kt:L90-L99, L780-L816) (iPhone: AVPictureInPictureController delegate; watch-player.md)
- [ ] `sn:media-cmd` (play, pause, stop from the lock screen, headset or focus loss) (k:MainActivity.kt:L896-L906) (iPhone: MPRemoteCommandCenter + AVAudioSession interruption; watch-player.md)
- [~] `sn:screen-off`, `sn:screen-on`, `sn:display-changed`, `sn:fold` (k:MainActivity.kt:L845-L887, L991-L1030) (iPhone: see 5.5)
- [ ] `sn:login-cancelled`, `sn:drops-redirect`, `sn:login-storage` (k:MainActivity.kt:L101-L149) (iPhone: WKWebView sign-in sheet callbacks; accounts-signin.md)
- [~] CSS variables `--sn-inset-t/r/b/l`, `--sn-kb`, `--sn-gesture-l/r/t/b` and `data-sn-native-insets` (k:MainActivity.kt:L908-L960) (iPhone: SwiftUI safe areas; shell-design.md)
- [~] Page hooks called by native code: `window.__SN_BACK__()` and `window.__SN_OPEN_CHANNEL__(login)` (m:navStore.ts:L124-L125) (m:inPlaceBack.ts:L10) (m:boot/useMobileBoot.ts:L305-L306) (iPhone: no page; back is navigation, open-channel is the notification delegate)

## 12. ACL parity test

- [~] A vitest checks that every command registered in `lib.rs`'s `generate_handler!` is allowed by exactly one of `permissions/app-commands.toml` or `permissions/mobile-commands.toml` (more than 300 each), and that the mobile file lists nothing stale; no user-visible behavior (m:acl_parity.test.ts:L1-L75) (iPhone: Swift calls Rust through `ios_ffi`, not the Tauri ACL; keep the equivalent guarantee as the plan phase 2 "event registry test" that every command and event Swift uses is registered on iOS)

## Decisions for Brandon

Not parity lines: open questions found while reading, each to be settled before its screen is built.

- Live alert gating on iPhone: the Worker applies master, live and mute rules per device (needs the mute list uploaded with registration), or a Notification Service Extension links the Rust `claim_show` (second process, needs a file lock). Recommended: the Worker sends, the extension claims, matching Android's single Rust authority.
- Desktop live poll on iOS (lib.rs L1222, L1295): gate it `cfg(desktop)` (recommended, APNs is the lane), or keep it to post in-app foreground alerts.
- Kick and favourite go-live alerts: Android emits them and drops them; iPhone matches (none) unless you want them.
- `streamnook://` links do not open the Android app (no mobile deep-link config). iPhone: register the scheme (and fix Android in the same change)?
- What's New source for the iOS line: an iOS manifest at `/api/v1/update-ios`, or a section in CHANGELOG.md (note Penrose builds Discord cards from CHANGELOG.md at a tag, pref: release-announcements-penrose.md).
- Drop notifications have no stable id on Android (repeats stack). Give them `drop:{drop_id}` on both?
- Wizard copy says "a whisper lands" but the phone never notifies whispers. Drop that phrase on both?
- Re-register the push follow set on follow/unfollow, or keep launch-only like Android?

## Beyond Android (backlog, Brandon picks)

- [ ] (backlog, not parity) A Live Activity on the Lock Screen and Dynamic Island for the stream being watched (channel, title, live dot + words), needs a widget extension (xtool widget extensions are currently broken)
- [ ] (backlog, not parity) A Home Screen widget listing followed channels that are live, fed by the Rust followed snapshot through an App Group
- [ ] (backlog, not parity) App Intents / Shortcuts: "Watch {channel}" and "Who is live" so Siri and Shortcuts can open a stream
- [ ] (backlog, not parity) Universal links for `https://streamnook.app/w/{login}` (apple-app-site-association on streamnook.app) so shared links open the app without the landing page hop
- [ ] (backlog, not parity) Notification actions on a live alert: "Watch" (default) and "Mute alerts for this channel" (writes `muted_live_channels` through Rust)
- [ ] (backlog, not parity) Time Sensitive interruption level as an opt-in per favourite channel, so chosen alerts break through Focus
- [ ] (backlog, not parity) A Focus filter that silences live alerts for chosen channels during a Focus
- [ ] (backlog, not parity) Mention notifications while the app is backgrounded with audio playing (local notifications from the Rust chat stream)
- [ ] (backlog, not parity) Drop notification tap opens the Rewards tab on that campaign
- [ ] (backlog, not parity) `BGAppRefreshTask` running the Rust collector as a best-effort fallback when push is unavailable
- [ ] (backlog, not parity) Spotlight indexing of followed channels so searching a streamer's name opens their channel
- [ ] (backlog, not parity) Handoff: a stream watched on the phone offered on a Mac running StreamNook at the same channel
- [ ] (backlog, not parity) MetricKit crash and hang diagnostics added to the log export zip
