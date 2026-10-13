# Parity contract: Shell, navigation and design system

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

Cross-referenced areas: `following-browse-search.md`, `watch-player.md`, `chat-view.md`, `chat-input-emotes.md`, `rewards-badges-drops.md`, `profile-cosmetics.md`, `settings.md`, `accounts-signin.md`, `system.md`.

## 1. Shell selection and document setup

- [~] The phone shell is picked synchronously before first paint from the user agent (`/android|iphone|ipad|ipod/i`); a native app has no shell choice to make (s:main.tsx:L103-L107) (s:utils/platform.ts:L10-L15) (iPhone: the SwiftUI `App` scene is the phone shell; nothing to detect)
- [~] A legacy "in-place" shell (the adapted desktop App with its own back chain) is reachable by setting localStorage `sn-legacy-shell` = "1"; it is an Android escape hatch only (s:main.tsx:L107) (m:inPlaceBack.ts:L1-L32) (s:styles/mobile.css:L17-L44) (iPhone: not ported; there is one shell)
- [~] `html[data-mobile="true"]`, `data-platform="mobile"` and `data-orientation` (portrait when height >= width, updated on resize and orientationchange) are set on the document root for CSS (s:main.tsx:L57-L64) (s:utils/platform.ts:L93-L101) (s:utils/platform.ts:L125-L139) (iPhone: CSS hooks, not needed; orientation comes from the scene geometry, see § 17)
- [~] The viewport is fixed at scale 1 with pinch zoom disabled (`maximum-scale=1.0, user-scalable=no`), drawn edge to edge (`viewport-fit=cover`) and asks the engine to resize content for the keyboard (`interactive-widget=resizes-content`) (p:index.html:L11-L12) (iPhone: native views do not zoom; edge to edge is the SwiftUI default with safe areas, see § 15)
- [ ] The whole app is wrapped in the Motion scope so the motion preference applies before any screen renders (s:main.tsx:L187-L198) (d:components/MotionScope.tsx:L20-L32) (iPhone: read `motion_mode` at the root and inject it through the SwiftUI environment, see § 19)
- [ ] The settings file read (`invoke load_settings`) starts as soon as the entry module runs, overlapping the rest of startup, and the first `loadSettings` consumes that result exactly once (s:bootPreload.ts:L1-L21) (iPhone: call `load_settings` on the Rust core the moment the app process starts, before the first view body)
- [~] On load the app deletes the stale localStorage keys `plyr` and `streamnook_whatsnew_cache_v2` (s:main.tsx:L85-L91) (iPhone: no WebView storage; nothing to clean)
- [~] React render errors are logged with their component stack to the Rust log (s:main.tsx:L164-L176) (iPhone: Swift has no render errors; log Rust-side failures and crashes through the existing logger)

## 2. Boot sequence and loading screen

### 2.1 What the screen shows while booting

- [ ] While `isBooting` is true the whole shell shows only a centred loader: the Penrose mark at 56 px wide and the line "Loading StreamNook" under it, 24 px gap, 14 px medium `textSecondary` (m:MobileApp.tsx:L114-L117) (d:components/LoadingWidget.tsx:L157-L274) (iPhone: a full-screen view with the Penrose mark drawn natively at 56 pt and the caption; no tab bar, no navigation)
- [ ] The boot loader shows no rotating funny messages on the phone (`useFunnyMessages` off); the caption is fixed (m:MobileApp.tsx:L116) (d:components/LoadingWidget.tsx:L162-L180) (iPhone: static caption)
- [ ] The Penrose mark is nine isometric cubes marching one slot around the triangle on a 1.5 s loop; it keeps marching even when Motion is Off, because a frozen loader reads as a hung app (d:components/PenroseMarch.tsx:L1-L20) (d:styles/globals.css:L3033-L3044) (iPhone: native Canvas/TimelineView animation at full frame rate, exempt from the motion setting) (pref: never-drop-fps.md)
- [ ] The boot loader is replaced instantly when booting ends (no fade on the phone); desktop fades its boot veil out over 0.45 s easeInOut (m:MobileApp.tsx:L114-L141) (d:App.tsx:L2420-L2432) (iPhone: swap the root view; a short opacity crossfade is acceptable but Android has none)
- [x] The Android launch splash is the OS default (no custom splash resource); the app paints its theme background from the first frame (t:gen/android/app/src/main/res/values-v27/themes.xml:L1-L10) (iPhone: `UILaunchScreen` in Info.plist with the default theme background `#0c0c0d` and no logo, so launch hands straight to the boot loader)
- [ ] The tab shell, tab bar and every full-screen layer mount only after boot, setup and sign-in gates pass; the device-code login overlay and the toast host are always mounted (m:MobileApp.tsx:L114-L181) (iPhone: root view switches between Boot, Setup, SignedOut and Main; the toast overlay sits above all four)

### 2.2 Gate order after boot

- [~] Gate order is: booting -> first-run setup -> signed out -> main shell. First-run setup WINS over sign-in because the wizard performs the Twitch sign-in itself; a signed-out first-run user must land in the wizard, never on the bare sign-in screen (m:MobileApp.tsx:L114-L141) (d:App.tsx:L1199-L1226) (iPhone: same order in the root switch; built as booting -> setup -> tabs: the tabs stay browsable signed out, with the landing shown inside Following and sign-in notes elsewhere (per the iPhone brief; open question for Brandon); the wizard gate itself was seen on the phone with the real core)
- [~] Setup is needed when `settings.setup_complete` is false AND the local backstop marker `streamnook-setup-complete` is not "true" AND the wizard was not closed this session (m:MobileApp.tsx:L85-L92) (d:App.tsx:L1199-L1226) (iPhone: read `setup_complete` from Rust; see moves file for folding the backstop into Rust; reads `setup_complete` from `load_settings` only; the localStorage backstop is a Rust request (ios/docs/rust-requests.md 4))
- [ ] Closing the wizard without finishing (`onClose`) drops the user out of setup for the rest of the session (the escape hatch) and on to the sign-in gate (m:MobileApp.tsx:L60) (m:MobileApp.tsx:L134) (iPhone: a session-only flag in the root view model)
- [~] Signed out AFTER setup (sign-out, token expiry, forced re-auth) shows the signed-out landing (§ 4), never the wizard again (m:MobileApp.tsx:L136-L140) (iPhone: same; signed out the tabs show the landing inside Following, never the wizard)

### 2.3 Boot steps, in order

- [ ] Theme, glassiness, glass blur and interface font apply from settings on mount and whenever any of `theme`, `custom_themes`, `glass_transparency`, `glass_blur`, `font`, `font_custom`, `oled_accent` changes (s:boot/useThemeBoot.ts:L22-L46) (iPhone: an observable theme model fed by the Rust settings snapshot and `settings-changed` pushes; see § 21 and § 22)
- [ ] Step 1: `await loadSettings()` (`invoke load_settings`, preloaded), THEN step 2: `await checkAuthStatus()`; the phone runs them in sequence while desktop runs both in parallel with `Promise.allSettled` (m:boot/useMobileBoot.ts:L73-L78) (d:App.tsx:L896-L910) (iPhone: run them in sequence like Android; they gate the root view)
- [ ] `checkAuthStatus` calls `invoke has_stored_credentials`, fires `invoke verify_token_health` without awaiting it, then `invoke get_user_info` (success = signed in), then `invoke ffz_local_user_status`, and on success `invoke start_whisper_listener` (s:stores/AppStore.ts:L3370-L3495) (iPhone: same commands through the core; owned in detail by accounts-signin.md)
- [ ] `isBooting` becomes false in a `finally`, so the loader always drops whether auth resolved signed in, signed out, or a boot step threw (m:boot/useMobileBoot.ts:L78-L82) (iPhone: same, never leave the boot view up on an error)
- [~] If `loadSettings` throws, the `finally` drops the loader but the rest of `initialize` (every listener below) is skipped for the session; this looks unintended (m:boot/useMobileBoot.ts:L73-L82) (iPhone: register every event handler regardless of a settings read failure; flagged for Brandon)
- [~] A network failure inside `checkAuthStatus` (DNS, timeout, 5xx) still sets `isAuthenticated: false`, so booting offline lands on the signed-out landing; it only skips the "session expired" toast (s:stores/AppStore.ts:L3496-L3545) (iPhone: flagged for Brandon; matching Android means an offline launch shows "Sign in with Twitch")
- [ ] The phone never shows the desktop "Welcome back, {name}!" toast on a restored session (s:stores/AppStore.ts:L3474-L3485) (iPhone: no welcome toast)
- [ ] Step 3 (fire and forget, after auth): `invoke validate_drops_token`; a false result logs that the drops token was rejected and cleared; transport errors never clear the token (m:boot/useMobileBoot.ts:L84-L106) (d:App.tsx:L318-L323) (iPhone: same call after auth)
- [ ] Step 4: load the active drops cache (1 h TTL) immediately; desktop defers it to a 2000 ms boot tier (m:boot/useMobileBoot.ts:L108-L109) (d:App.tsx:L924-L927) (iPhone: owned by rewards-badges-drops.md; trigger it after auth)
- [ ] Step 5, only when signed in: 8000 ms after boot, check the 7TV session and silently re-mint it when it is within 7 days of expiry and no attempt was made in the last 12 h (m:boot/useMobileBoot.ts:L127-L138) (s:services/sevenTvSession.ts:L1-L89) (iPhone: owned by profile-cosmetics.md; keep the 8 s deferral so it misses the boot burst)
- [ ] Step 5 continued: list every stored account (falls back to the active account if the registry is not ready), register them as "own" for cosmetics, identities and atmospheres, revalidate the active account's cosmetics then fetch its full profile, and warm identity for the active and every other own account (m:boot/useMobileBoot.ts:L116-L165) (d:App.tsx:L956-L1000) (iPhone: one Rust call after auth; see moves file; keyed by Twitch numeric user id)
- [ ] Step 6: listen `7tv://emote-set-update` and `7tv://cosmetic-update` (live 7TV pushes from the Rust EventAPI socket) (m:boot/useMobileBoot.ts:L167-L173) (iPhone: HostSink events routed to the emote and cosmetics models; owned by chat-input-emotes.md and profile-cosmetics.md)
- [ ] Step 7: listen `channel-points-earned`; a positive `points` adds to the lifetime stat `channel_points_collected` for the signed-in user when Supabase is configured; no toast and no notification here (m:boot/useMobileBoot.ts:L175-L204) (d:hooks/useWatchedChestStat.ts:L15-L29) (iPhone: same; note desktop counts only `watched-chest-claimed`, so the two shells count different sources, see moves file; the points pulse is owned by chat-input-emotes.md)
- [ ] Step 8: listen `ad-pivot`; a payload with `url` replaces the playing stream URL (and region) so the player reloads clean (m:boot/useMobileBoot.ts:L206-L211) (iPhone: owned by watch-player.md)
- [ ] Step 9: listen `refresh-following-list` and reload followed streams on it (m:boot/useMobileBoot.ts:L213-L216) (iPhone: owned by following-browse-search.md)
- [ ] Step 10: hydrate the Kick follows store, pull current liveness once, then listen `provider-live-update` (provider + stream rows) (m:boot/useMobileBoot.ts:L218-L228) (iPhone: owned by following-browse-search.md)
- [ ] Step 11: listen `provider-stream-offline`; when the provider and the lower-cased channel login match the stream being watched, run the stream-offline handling at once instead of waiting for the slower liveness poll (m:boot/useMobileBoot.ts:L229-L239) (iPhone: owned by watch-player.md; key is provider + login, never mixed with Twitch numeric ids)
- [ ] Step 12: register the notification categories (idempotent) before anything can post (m:boot/useMobileBoot.ts:L258-L261) (iPhone: register `UNNotificationCategory` set at launch; owned by system.md)
- [ ] Step 13: ask for notification permission once at boot ONLY when setup is already complete and the OS still reports "not determined"; a fresh install is asked by the wizard instead (m:boot/useMobileBoot.ts:L263-L275) (iPhone: `UNUserNotificationCenter.getNotificationSettings`, request only when `.notDetermined` and `setup_complete`)
- [~] Step 14: sync the background live-check worker to the current notification prefs on every launch (m:boot/useMobileBoot.ts:L277-L279) (iPhone: no WorkManager; APNs is the live-alert lane, owned by system.md)
- [ ] Step 15: unless notifications are disabled (`live_notifications.enabled === false` or `push_notifications === false`), `invoke push_register` to upload the push token and follow list, fire and forget (m:boot/useMobileBoot.ts:L281-L292) (iPhone: APNs device token registration through the same Rust command; owned by system.md)
- [ ] Step 16: open a channel handed in from outside: first a tapped notification's pending channel, otherwise `invoke take_pending_watch_link` (cold-start deep link); the login is trimmed and lower-cased and started with `startStream`; `window.__SN_OPEN_CHANNEL__` is exposed for warm hand-offs (m:boot/useMobileBoot.ts:L294-L316) (iPhone: `onOpenURL` and `UNUserNotificationCenterDelegate` route to the same start-stream call; owned by system.md)
- [ ] There is deliberately no `streamer-went-live` listener on the phone; background push/poll is the only live-alert lane (m:boot/useMobileBoot.ts:L318-L322) (iPhone: same, APNs only)
- [ ] Step 17: listen `badge-notification` (an array); unless notifications are off or `show_badge_notifications` is false, post one system notification per badge titled "New badge: {badge_name}" with the badge description as body, id hashed from `badge:{set_id}-v{version}` so duplicates collapse (m:boot/useMobileBoot.ts:L324-L347) (iPhone: owned by system.md)
- [ ] Step 18: listen `drop-ready` -> "Drop ready to claim" and `drop-claimed` -> "Drop claimed", body `{name} · {campaign}`, gated by `show_drops_notifications` (m:boot/useMobileBoot.ts:L349-L372) (iPhone: owned by system.md)
- [ ] Step 19 (Supabase configured only): track presence for the signed-in user, subscribe to the StreamNook, cosmetics and atmospheres registries, and re-pull entitlement registries whenever the app becomes visible, at most once per 10 000 ms (m:boot/useMobileBoot.ts:L374-L414) (d:App.tsx:L795-L860) (iPhone: on `scenePhase == .active`, throttled 10 s; see moves file)
- [ ] Auth is rechecked every 5 min while the app is visible (skipped while hidden), whatever is on screen; desktop rechecks only while signed in AND watching (m:boot/useMobileBoot.ts:L418-L425) (d:App.tsx:L1155-L1165) (iPhone: a 5 min timer gated on `scenePhase == .active`)
- [ ] All listeners, timers and subscriptions are torn down when the shell unmounts, including ones registered after an await (m:boot/useMobileBoot.ts:L63-L71) (m:boot/useMobileBoot.ts:L427-L430) (iPhone: tie subscriptions to the app model lifetime)
- [ ] Never await an optional external IPC (Discord, OBS) on the boot path; the Discord connect is fire and forget (s:stores/AppStore.ts:L1757-L1764) (iPhone: no Discord on iPhone; keep any optional call off the boot path) (pref: discord-connect-boot-hang.md)
- [ ] Shell-level hooks also mounted once at the root: Kick account sync (accounts-signin.md), the Twitch sign-in prompt (accounts-signin.md), keyboard insets (§ 16), current stream stats (watch-player.md), the back handler (§ 8), lifecycle (system.md) and the video capability report (watch-player.md) (m:MobileApp.tsx:L44-L55) (m:MobileApp.tsx:L104-L110) (iPhone: the matching app-level models start once in the `App` init)
- [~] Desktop-only boot steps are deliberately not replicated: session resume, whisper import, `eventsub://channel-moderate`, drop-progress automation, snippet sync, window sizing, universal-cache disk sync, cross-window settings sync (m:boot/useMobileBoot.ts:L11-L13) (d:App.tsx:L866-L885) (iPhone: not ported)

## 3. First-run setup (the shared wizard on the phone)

The phone runs the desktop `SetupWizard` itself through its `IS_MOBILE` branches.

- [x] The wizard is a full-screen layer (z 60) on the theme background with a faint accent radial wash and a film-grain texture, fading in over 0.45 s (d:components/SetupWizard.tsx:L1218-L1251) (iPhone: a full-screen root view; wash as a `RadialGradient` stack; grain optional; seen on the iPhone: wash built, no grain)
- [~] The phone shows 7 pages: 0 Welcome, 1 Components (auto), 2 Twitch and accounts, 3 Drops, 5 Theme, 7 Notifications, 8 Done; page 4 (whisper import) and page 6 (sidebar mode) are skipped on mobile because those features do not exist there (d:components/SetupWizard.tsx:L46-L79) (iPhone: same 7 pages; 5 pages on iPhone: Components (no-op on iOS) and Theme (waits for the Rust theme list, rust-requests.md 1) are left out)
- [ ] Every page change goes through the skip-aware next/previous helpers, including programmatic advances after a sign-in (d:components/SetupWizard.tsx:L64-L79) (d:components/SetupWizard.tsx:L402) (iPhone: one page list without the skipped pages)
- [ ] Page content enters with opacity 0 -> 1 and y 14 -> 0 and leaves with y -14, 0.4 s, ease [0.16, 1, 0.3, 1], one page at a time (d:components/SetupWizard.tsx:L1327-L1338) (iPhone: `.transition(.opacity.combined(with: .offset(y:)))` 0.4 s)
- [ ] Pages swipe: a horizontal drag anywhere between the back control and the footer with 0.15 elastic rubber band; release past 70 px left advances, past 70 px right goes back; a second swipe within 400 ms of a page change is ignored (d:components/SetupWizard.tsx:L1285-L1325) (iPhone: a horizontal `DragGesture` with a 70 pt threshold, or a paged `TabView` with swipe disabled where forward is locked)
- [ ] Forward swipe is allowed on page 0 and pages 2 to 7, never on page 1 (it advances itself) or page 8 (finishing is a deliberate tap), and never on page 2 while Twitch is not signed in (d:components/SetupWizard.tsx:L686-L699) (iPhone: same rules)
- [x] Back (swipe right or the top-left "Back" control with a chevron, 14 px `textSecondary`, top inset safe-top + 8 px) is offered only on pages 2 to 7 (d:components/SetupWizard.tsx:L685) (d:components/SetupWizard.tsx:L1271-L1282) (iPhone: a leading "Back" button with `chevron.left`)
- [ ] A gesture hint (a 7 px accent dot travelling right to left with a 2 px tail, 1.6 s loop, 0.7 s pause) sits above the button until the user swipes once; with Motion below Full it is the text "Swipe sideways to move between pages" (d:components/SetupWizard.tsx:L135-L189) (d:components/SetupWizard.tsx:L1386-L1390) (iPhone: same; text variant when `motion_mode != full`)
- [x] Footer: centred step dots above a full-width primary button (py 16, 16 px text), padding bottom safe-bottom + 20 px; dots are 8 px high, the active one 40 px wide in accent with a glow, past ones 8 px at accent 50% and tappable to jump back, future ones `borderSubtle` and disabled; aria-label "Step {n}" (d:components/SetupWizard.tsx:L1340-L1384) (iPhone: custom page indicator; tappable past dots)
- [x] Primary button labels: page 0 "Get started", pages 2 (signed in) to 7 "Continue", page 8 "Start watching", page 1 none (d:components/SetupWizard.tsx:L620-L663) (iPhone: same copy)
- [~] Page 0: StreamNook logo, title "Welcome to StreamNook" (two lines, 48 px medium), body "Yeah yeah, another setup wizard. A few clicks and we'll get out of your way, promise." and an animated 7TV emote (webp, not avif) (d:components/SetupWizard.tsx:L720-L745) (iPhone: same copy; emote through the image pipeline; copy and the Penrose mark seen; the animated 7TV emote waits for the image pipeline)
- [~] Page 1: `invoke check_components_installed` (always true now) shows "Components ready" / "The ad blocker is in place. Moving on." and advances; otherwise "Setting things up" / "Installing the ad blocker. This only takes a moment." while `invoke extract_bundled_components` runs (d:components/SetupWizard.tsx:L247-L310) (d:components/SetupWizard.tsx:L748-L780) (iPhone: [~] candidate to drop the page, since nothing installs on iOS; flagged for Brandon; page left out on iPhone (flagged))
- [~] Page 2 unsigned: Twitch glyph 56 px in `#9146FF`, title "Connect your accounts", body "Connect the platforms you watch on to see your follows, chat, and use channel features.", then the Kick/YouTube account rows; the primary button is "Sign in with Twitch" on Twitch purple `#9146FF` with the glyph, becoming "Waiting for sign-in" with a spinner while the sign-in runs (d:components/SetupWizard.tsx:L636-L651) (d:components/SetupWizard.tsx:L836-L906) (iPhone: same; sign-in sheet owned by accounts-signin.md; glyph, copy and the Twitch-purple "Sign in with Twitch" seen; the Kick and YouTube rows are not on the page yet)
- [ ] Page 2 signed in: green check, "You're signed in" / "Your follows, chat, and channel actions are connected.", button "Continue"; a successful sign-in advances to page 3 after 500 ms (d:components/SetupWizard.tsx:L427-L483) (iPhone: same)
- [ ] Closing the native sign-in view without finishing (`sn:login-cancelled`) returns the step to its idle state instead of waiting forever (d:components/SetupWizard.tsx:L463-L475) (iPhone: sheet dismissal resets the step)
- [~] Page 3: "Drops and inventory" / "Sign in to track Twitch Drops, watch your inventory, and auto-claim rewards." with "Sign in for Drops"; while authorizing a card says "Approve the request on Twitch to turn on drops and channel points." and "Waiting for authorization" with "Reopen sign-in window"; success shows "You're in for Drops" / "Drops will claim themselves while you watch." and advances after 500 ms; Drops is skippable (d:components/SetupWizard.tsx:L335-L425) (d:components/SetupWizard.tsx:L782-L835) (iPhone: flow owned by rewards-badges-drops.md and accounts-signin.md; copy seen; "Sign in for Drops" waits for the drops sign-in sheet (accounts lane))
- [~] Page 5: "Pick your look" / "Choose a theme to start with. Tap one to try it on. There are more options, fonts, and a theme builder in Settings later." with a 2-column grid of theme cards (each painted on its own background with five palette dots and the name) in a vertical scroller capped at 46% of the viewport height; tapping applies the theme live and saves `theme` (d:components/SetupWizard.tsx:L527-L534) (d:components/SetupWizard.tsx:L996-L1040) (iPhone: `LazyVGrid` of 2 columns; whole wizard repaints live; waits for the Rust theme list (rust-requests.md 1))
- [x] Page 7 (phone): "Stay in the loop" / "Get a notification when a followed streamer goes live, a whisper lands, or a drop is claimed. You can fine-tune each type in Settings." with a full-width "Enable notifications" button that becomes disabled "Notifications enabled" once granted; a refusal shows "Notifications are turned off for StreamNook. You can turn them back on in Android Settings whenever you like." (d:components/SetupWizard.tsx:L556-L591) (d:components/SetupWizard.tsx:L1083-L1113) (iPhone: `requestAuthorization`; the refusal copy says "Settings" instead of "Android Settings"; seen on the iPhone: requestAuthorization wired; the refusal copy says "Settings")
- [x] Page 8: logo, "You're all set" / "StreamNook is ready. Pick a stream and dive in.", and a summary of rows "Drops sign-in", "Twitch sign-in", "Other platforms", each a green check or "skipped"; no keyboard hint and no whisper row on the phone (d:components/SetupWizard.tsx:L1146-L1206) (iPhone: same)
- [ ] "Start watching" saves `setup_complete: true`, writes the local marker `streamnook-setup-complete` = "true", flags the announcements baseline for a first-time setup, then closes; a failed save shows the error toast "Failed to save settings" (d:components/SetupWizard.tsx:L485-L523) (iPhone: one Rust call; see moves file)

## 4. Signed-out landing

- [x] Centred column, 32 px side padding, safe-top and safe-bottom padding; never shrunk to one pane on a large or folded screen (m:MobileOnboarding.tsx:L14-L26) (iPhone: centred `VStack` in the safe area; seen on the iPhone: inside the Following tab while signed out)
- [x] Title "StreamNook" 24 px bold `textPrimary`, 8 px below it a 14 px `textMuted` centred line, max 280 px wide, then 40 px gap (m:MobileOnboarding.tsx:L27-L32) (iPhone: same)
- [~] The line reads "Watch Twitch with your emotes, badges, and cosmetics everywhere." unless StreamNook signed the user out itself, in which case it states the reason: "Twitch needs you to sign in again for new features." (m:MobileOnboarding.tsx:L28-L32) (s:utils/twitchSignIn.ts:L12-L17) (iPhone: reason read from the Rust account roster; the usual line seen; the sign-in reason waits for Rust (rust-requests.md 5))
- [x] One glass button, full width up to 320 px, 48 px tall, 15 px semibold: "Sign in with Twitch", "Sign in again" when a reason is pending, "Signing in…" and 60% opacity while a sign-in is running (m:MobileOnboarding.tsx:L33-L40) (iPhone: `.buttonStyle(.glass)`; sign-in flow owned by accounts-signin.md)

## 5. Shell layout and layer stacking

- [x] The shell fills the dynamic viewport height as a column, clips overflow, base font size 14 px (m:mobile.css:L10-L21) (iPhone: root `ZStack` filling the scene)
- [x] The tab content area pads its top by the safe-top inset and fills the space; tab screens scroll under the floating tab bar (m:MobileApp.tsx:L143-L147) (iPhone: safe area top respected; content extends under the tab bar)
- [ ] Scrolling lists reserve bottom clearance of tab bar height (56 px) + safe-bottom + 64 px so the last row is clear of the floating bar (m:mobile.css:L46-L55) (iPhone: `.contentMargins(.bottom, ...)` or system tab-bar inset handling)
- [ ] Scrollbars are hidden everywhere on the phone (m:mobile.css:L23-L28) (iPhone: `.scrollIndicators(.hidden)`, or the system default auto-hiding indicator)
- [ ] Layer order, bottom to top: tab screens, tab bar (z 30), category drill-in (z 35), watch layer incl. the mini player (z 40), Settings and Cosmetics drill-ins (z 50), toasts (z 50, mounted later), setup wizard (z 60), sheets (z 9000), the watch layer while in system PiP (z 9500) (m:MobileApp.tsx:L165-L175) (m:screens/CategoryStreamsScreen.tsx:L36-L41) (m:screens/WatchScreen.tsx:L955) (iPhone: Browse category is a push in the Browse NavigationStack; the player is an overlay above the TabView; Settings/Cosmetics are pushes from You; sheets are `.sheet`)
- [x] A category drill-in covers the tab bar but sits under the mini player, so a shrunk stream floats over the category list (m:screens/CategoryStreamsScreen.tsx:L39) (m:screens/WatchScreen.tsx:L955) (iPhone: hide the tab bar on that push with `.toolbar(.hidden, for: .tabBar)`; mini player overlay stays above)
- [ ] Settings and Cosmetics drill-ins cover the mini player too (m:screens/SettingsScreen.tsx:L68-L73) (m:screens/CosmeticsScreen.tsx:L449-L454) (iPhone: pushes that hide the tab bar; the mini player overlay hides while they are open)
- [x] The tab shell stays mounted while the full player covers it, so scroll positions and state survive (m:MobileApp.tsx:L63-L77) (iPhone: the TabView stays in the hierarchy under the player overlay)
- [ ] While the full player covers the shell, infinite decorative animations in the shell stop (`animate-ping` live dots, hype-train shine) and glass panels drop their backdrop blur; in mini they keep running because the shell is visible (m:MobileApp.tsx:L63-L77) (m:mobile.css:L57-L72) (m:mobile.css:L95-L101) (iPhone: pause `TimelineView`/repeating animations and drop material effects in covered tabs, driven by a `shellCovered` flag)
- [ ] Badge-wall tiles skip layout and decode until scrolled near (intrinsic height 118 px) (m:mobile.css:L86-L93) (iPhone: `LazyVGrid`; owned by rewards-badges-drops.md)
- [ ] The clip sheet is mounted at the root so a clip tapped in chat opens over any layer (m:MobileApp.tsx:L170-L175) (iPhone: owned by chat-view.md; presented from the root)

## 6. Tab bar

- [x] Four tabs in this order, icon only, no visible labels: Following (heart), Browse (compass), Rewards (package), You (user circle); accessibility labels "Following", "Browse", "Rewards", "You"; the active tab carries `aria-current="page"` (m:MobileTabBar.tsx:L12-L17) (m:MobileTabBar.tsx:L50-L61) (iPhone: system `TabView` with SF Symbols `heart`, `safari`, `shippingbox`, `person.crop.circle`; system tab bars show labels, see ambiguity note at the end; seen on the iPhone: system tab bar with its labels showing (decision for Brandon))
- [x] The bar floats detached from the edges: 20 px + safe-left/right from each side, 14 px + safe-bottom above the bottom edge, max width 520 px centred, height 56 px, 6 px inner horizontal padding, capsule shape, shadow `0 8px 24px -12px rgba(0,0,0,0.45)` (m:MobileTabBar.tsx:L30-L45) (m:mobile.css:L16) (iPhone: the iOS 26 floating Liquid Glass tab bar matches this shape natively)
- [~] The bar wears the "dock glaze": clear glass with a 4 px blur (scaled by glassiness), saturate 110%, a 16% black film, an accent hairline rim and a masked specular; at Glassiness 0 its floor is the opaque tertiary colour (d:styles/globals.css:L170-L292) (d:styles/globals.css:L838-L854) (iPhone: `glassEffect(.regular)` on the bar; opaque `backgroundTertiary` at Glassiness 0, see § 20; the system Liquid Glass bar stays glass at Glassiness 0 (Brandon: system chrome is Apple's glass))
- [x] Each tab is an equal-width column; the selected tab shows a darker capsule 62 x 44 px, `rgb(0 0 0 / 0.72)`, hugging the glyph and gliding between tabs on a shared spring (stiffness 350, damping 30), exactly like desktop Home's tab strip (m:MobileTabBar.tsx:L62-L73) (d:styles/globals.css:L834-L837) (d:components/Home.tsx:L2702-L2715) (iPhone: the system selection pill; custom bar uses `matchedGeometryEffect` with `.spring(response: 0.34, dampingFraction: 0.8)`; seen on the iPhone: the system selection pill)
- [x] Glyphs are 25 px; the active glyph is filled and coloured `accent`, inactive glyphs are regular weight in `textPrimary` (not muted, so they stay visible over bright thumbnails) (m:MobileTabBar.tsx:L53-L57) (m:MobileTabBar.tsx:L87) (iPhone: `.symbolVariant(.fill)` on the selected tab, tint `accent`, unselected `textPrimary`)
- [ ] Once signed in with a profile picture, the You tab shows the user's 26 px round avatar (7TV avatar when they have one) instead of the glyph, with a 2 px accent ring when active (m:MobileTabBar.tsx:L48-L49) (m:MobileTabBar.tsx:L75-L85) (iPhone: a custom tab item image from the avatar pipeline; owned look in profile-cosmetics.md)
- [x] Touch targets are at least 44 x 44 px (m:MobileTabBar.tsx:L56) (m:mobile.css:L40-L44) (iPhone: system tab bar targets)
- [x] No badges or counts on any tab (m:MobileTabBar.tsx:L45-L94) (iPhone: no `.badge`)
- [ ] Tapping the already-selected tab does nothing (no scroll to top, no pop) (m:MobileTabBar.tsx:L52) (m:navStore.ts:L59) (iPhone: system reselect pops to root/scrolls to top; acceptable, see backlog)
- [x] The bar is hidden (kept out of the compositor) while the full-screen player covers the shell; it is visible with the mini player (m:MobileApp.tsx:L75-L77) (m:MobileApp.tsx:L165) (m:MobileTabBar.tsx:L36) (iPhone: the full player overlay covers the TabView; tab bar visible under mini)
- [~] The bar does not exist during boot, setup or the signed-out landing (m:MobileApp.tsx:L114-L165) (iPhone: TabView only in the Main root state; absent during boot and setup; the signed-out landing lives inside the tabs, so the bar shows there)

## 7. Tab switching and start tab

- [~] Switching tabs slides the new screen in from the side of the newly selected tab: opacity 0 -> 1 and x +-28 px -> 0, the old one leaves the opposite way, 0.16 s, ease [0.2, 0.8, 0.2, 1] (m:MobileApp.tsx:L94-L102) (m:MobileApp.tsx:L148-L162) (iPhone: system TabView switches without a slide; optional 0.16 s crossfade, flagged for Brandon; system TabView switches without the slide (decision for Brandon))
- [ ] The first tab does not animate in on boot (m:MobileApp.tsx:L148) (iPhone: no transition on first appearance)
- [ ] The app opens on the chosen start tab (phone pref `sn-start-tab`, default "following"), read synchronously so the first frame is already right (m:navStore.ts:L48-L52) (m:phonePrefs.ts:L13-L25) (iPhone: initial `selection` from the stored pref; see moves file)
- [ ] Tapping the live drop progress while watching shrinks the player to mini, switches to Rewards and asks that screen to scroll to and flash the campaign; the request clears once consumed (m:navStore.ts:L27-L30) (m:navStore.ts:L61-L67) (iPhone: same, owned by rewards-badges-drops.md)

## 8. Navigation state and back handling

- [ ] Navigation state: active tab, open sheet stack (bottom first, de-duplicated by id), open settings panel id, open Browse category, cosmetics open, player mode (`full` / `mini`), drop campaign focus (m:navStore.ts:L13-L46) (m:navStore.ts:L48-L80) (iPhone: an `@Observable` navigation model with per-tab `NavigationPath`s)
- [ ] Back chain, first match wins: 1 close the top sheet; 2 close the settings panel; 3 close cosmetics; 4 if a stream is playing or loading in full mode, shrink to mini (closing is the mini player's X); 5 close the Browse category; 6 go to the start tab if on another tab; otherwise not consumed (m:navStore.ts:L83-L119) (iPhone: see the next lines for each step's native gesture)
- [~] Android back is delivered by the activity calling `window.__SN_BACK__()`; an unconsumed press moves the task to the background, never kills it (m:navStore.ts:L123-L129) (k:MainActivity.kt:L726-L740) (iPhone: no back button; each step maps to swipe-back or swipe-down below)
- [ ] Back step 1 (sheet): the top sheet closes (m:navStore.ts:L86-L90) (iPhone: swipe the sheet down)
- [x] Back steps 2, 3 and 5 (settings panel, cosmetics, category): the drill-in slides out (m:navStore.ts:L91-L98) (m:navStore.ts:L106-L109) (iPhone: interactive edge swipe back on the NavigationStack, plus the back button; seen on the iPhone: NavigationStack push with the system back button seen; edge swipe not finger-tested)
- [~] Back step 4 (full player): the player shrinks to the in-app mini player and browsing continues with the stream alive (m:navStore.ts:L99-L105) (iPhone: swipe down on the player or its collapse button; owned by watch-player.md; mini mode seen through a drive step; the minimize button and the drag are built but not finger-tested)
- [~] Back step 6 (unwind to the start tab) and "not consumed -> background" have no iOS counterpart (m:navStore.ts:L110-L118) (iPhone: not needed; there is no global back, and apps never background themselves)

## 9. Drill-in screens

- [x] Settings panels, Cosmetics and a Browse category open as full-screen layers that push in from the trailing edge (x 100% -> 0) and slide back out the same way, 0.22 s, ease [0.2, 0.8, 0.2, 1] (m:ui/DrillInScreen.tsx:L17-L18) (m:ui/DrillInScreen.tsx:L39-L53) (iPhone: NavigationStack push/pop, which also gives interactive swipe back)
- [x] Each layer is opaque `background`, pads its top by safe-top and keeps showing its last content while sliding out (m:screens/SettingsScreen.tsx:L62-L73) (m:screens/CategoryStreamsScreen.tsx:L30-L41) (iPhone: native push keeps the outgoing view)
- [x] Drill-in header: a 44 px back button (arrow, 22 px, `textSecondary`, aria-label "Back"; category uses "Back to Browse") then an 18 px bold title, 8 px padding, a hairline `borderSubtle` under Settings and Cosmetics headers (m:screens/SettingsScreen.tsx:L74-L83) (m:screens/CosmeticsScreen.tsx:L455-L464) (m:screens/CategoryStreamsScreen.tsx:L153-L160) (iPhone: inline navigation title with the system back button)
- [ ] Drill-in bodies pad their bottom by safe-bottom + 16 px (m:screens/SettingsScreen.tsx:L85-L88) (iPhone: safe area)
- [~] Tab screen headers: 20 px bold title ("Following", "Browse", "Rewards") at 16 px side padding, 12 px top, 8 px bottom (m:screens/FollowingScreen.tsx:L127-L129) (m:screens/BrowseScreen.tsx:L278-L280) (iPhone: large navigation title or a matching custom header; screen contents owned by their areas; inline large navigation titles in Satoshi (32 pt bold) with the toolbar on the same row)

## 10. Sheets

- [x] Sheets replace desktop popovers, dropdowns and context menus: a bottom panel portalled above everything, `role="dialog"`, `aria-modal` (m:ui/MobileSheet.tsx:L1-L10) (m:ui/MobileSheet.tsx:L75-L83) (iPhone: `.sheet` with `presentationDetents`)
- [ ] Backdrop: black at 50% with a 6 px blur scaled by glassiness; tapping it closes the sheet (m:ui/MobileSheet.tsx:L84-L93) (iPhone: system dimming; sheets dismiss on tap outside only for non-large detents, so allow `.presentationBackgroundInteraction` off and rely on swipe/tap)
- [ ] The backdrop fades in and out over 0.13 s linear; the panel slides up from 100% on a spring (stiffness 560, damping 42, mass 0.7) and back down on exit (m:ui/MobileSheet.tsx:L32-L34) (m:ui/MobileSheet.tsx:L79-L82) (m:ui/MobileSheet.tsx:L113-L115) (iPhone: system sheet animation)
- [x] Panel: full width capped at 640 px and centred, `glass-modal` with 18 px top corners and square bottom corners, column layout (m:ui/MobileSheet.tsx:L94-L97) (d:styles/globals.css:L1660-L1683) (iPhone: system sheet with Liquid Glass background; corner radius system)
- [x] Height is content-sized up to `maxHeightFraction` of the viewport (default 0.72) and never taller than the viewport minus keyboard minus safe-top minus 12 px; per-sheet fractions used: 0.5 (composer menu), 0.6 (clip), 0.7 (add chat), 0.8 (user profile), default elsewhere (m:ui/MobileSheet.tsx:L17-L21) (m:ui/MobileSheet.tsx:L108) (m:chat/ComposerMenuSheet.tsx:L83) (m:watch/MobileClipSheet.tsx:L92) (m:chat/AddChatSheet.tsx:L133) (m:profile/UserProfileSheet.tsx:L254) (iPhone: `.presentationDetents([.height(min(content, fraction*screen))])` single detent; seen on the iPhone: `fittedSheet(maxFraction:)`)
- [x] One detent only: the sheet cannot be dragged taller (m:ui/MobileSheet.tsx:L51-L70) (iPhone: a single detent)
- [x] Grabber: 36 x 4 px capsule, `textMuted` at 40%, 10 px from the top; optional title under it, 14 px semibold `textPrimary`, 8 px below the grabber (m:ui/MobileSheet.tsx:L117-L126) (iPhone: `.presentationDragIndicator(.visible)` plus an inline title)
- [ ] Dragging starts only on the grabber/title strip, follows the finger 1:1 downward only (no upward travel), and on release closes when dragged more than 96 px, otherwise springs back; no velocity flick (m:ui/MobileSheet.tsx:L24) (m:ui/MobileSheet.tsx:L51-L70) (m:ui/MobileSheet.tsx:L117-L123) (iPhone: system swipe-down dismiss, which also honours velocity; acceptable)
- [ ] Body scrolls inside the sheet with 16 px side and bottom padding; the panel pads its bottom by safe-bottom (m:ui/MobileSheet.tsx:L109) (m:ui/MobileSheet.tsx:L127) (iPhone: `ScrollView` inside the sheet; safe area automatic)
- [ ] With the keyboard open the sheet rises by the keyboard height (0.15 s ease-out) and its height cap shrinks by the same amount, so a search field is never covered (m:ui/MobileSheet.tsx:L98-L111) (iPhone: sheets avoid the keyboard natively)
- [ ] Each open sheet registers on the navigation sheet stack and closes when back targets it (m:ui/MobileSheet.tsx:L138-L150) (iPhone: not needed; sheet dismissal is native)
- [ ] Drag state resets every time the sheet reopens (m:ui/MobileSheet.tsx:L36-L38) (iPhone: native)
- [ ] Under reduced or off motion the slide is dropped and only the backdrop fade remains (m:ui/MobileSheet.tsx:L7-L9) (d:components/MotionScope.tsx:L27-L31) (iPhone: system sheets already respect Reduce Motion; also honour `motion_mode`)

## 11. Pull to refresh

- [ ] Used on Following, Browse (Twitch and Kick), a Browse category and Rewards (drops and badges tabs) (m:screens/FollowingScreen.tsx:L175) (m:screens/BrowseScreen.tsx:L386) (m:screens/KickBrowseBody.tsx:L118) (m:screens/CategoryStreamsScreen.tsx:L162) (m:screens/RewardsScreen.tsx:L490) (iPhone: `.refreshable` on each list)
- [ ] The pull only engages when the list is at its very top and the finger moves down; anything else scrolls normally (m:ui/PullToRefresh.tsx:L48-L72) (iPhone: native)
- [ ] The list follows at half the finger distance, capped at 120 px of travel (m:ui/PullToRefresh.tsx:L13-L14) (m:ui/PullToRefresh.tsx:L69-L71) (iPhone: native rubber band)
- [ ] Releasing at 36 px of travel or more (72 px of finger) triggers the refresh; less springs back over 0.2 s cubic-bezier(0.2, 0.8, 0.2, 1) (m:ui/PullToRefresh.tsx:L74-L90) (m:ui/PullToRefresh.tsx:L133-L144) (iPhone: native trigger distance)
- [ ] Indicator: from 4 px of travel a bare 20 px bold down arrow fades in (opacity travel / 28.8) and grows (scale 0.6 + travel / 120, max 1), `textMuted` until armed, then `accent` and rotated 180 degrees (0.15 s); it rides above the content (top = max(-30, travel - 46), 34 px tall) (m:ui/PullToRefresh.tsx:L104-L132) (iPhone: system `UIRefreshControl` spinner; arrow optional)
- [ ] While refreshing the list holds 36 px down and the Penrose mark (30 px) marches until the refresh promise settles, then the list springs back; a second pull during a refresh is ignored (m:ui/PullToRefresh.tsx:L78-L86) (m:ui/PullToRefresh.tsx:L117-L118) (iPhone: `.refreshable { await ... }`; Penrose as a custom indicator is optional, flagged)
- [ ] No haptic on arm or release (m:ui/PullToRefresh.tsx:L74-L90) (iPhone: whatever the system refresh control does)
- [ ] Overscroll glow/bounce is suppressed on these lists (`overscroll-behavior-y: contain`) (m:ui/PullToRefresh.tsx:L136-L139) (iPhone: native bounce is fine)

## 12. Settle-in list animation

- [ ] The first time a list shows real items, each item fades and rises in: opacity 0 -> 1, y 14 px -> 0, scale 0.97 -> 1, 0.34 s, ease [0.16, 1, 0.3, 1], staggered 45 ms per index, capped at 400 ms delay (m:ui/SettleIn.tsx:L34-L35) (m:ui/SettleIn.tsx:L83-L124) (iPhone: per-item `.opacity`/`.offset`/`.scaleEffect` animated with `.timingCurve(0.16, 1, 0.3, 1, duration: 0.34).delay(min(i * 0.045, 0.4))`)
- [ ] It plays once per list identity and latches: refresh, infinite-scroll pages and single new items just appear; a new reset key (layout switch, new search, filter change) replays it (m:ui/SettleIn.tsx:L50-L80) (iPhone: an `@State` "settled for key" latch)
- [ ] Skeletons and empty states never consume the settle (it waits for `ready`), and a list mounting from a warm cache starts already settled (m:ui/SettleIn.tsx:L50-L68) (m:screens/RewardsScreen.tsx:L469) (iPhone: same flags)
- [ ] It still plays on the first screen after launch or setup even though tab switches suppress initial animations (m:ui/SettleIn.tsx:L7-L13) (iPhone: trigger on first non-empty data, not on view appear)
- [ ] After an item finishes its entrance it becomes a plain, non-animating view (m:ui/SettleIn.tsx:L89-L105) (iPhone: no lingering animation state)
- [ ] Under reduced or off motion items appear in place with no movement (m:ui/SettleIn.tsx:L26-L28) (iPhone: skip offset/scale; opacity optional)
- [ ] Desktop Home has its own, smaller card entrance (opacity, y 8 px) and skips it under the boot veil (d:components/Home.tsx:L1883-L1891) (iPhone: no entrance while the boot view is up)

## 13. Skeletons

- [ ] First-load placeholders shaped like stream cards: 3 by default, stacked with 12 px gaps, 16 px side and bottom padding; each a glass panel with a 16:9 `surface` block, then a 16 px bar at 75% width and a 12 px bar at 33% width (8 px apart) inside 12 x 10 px padding (m:ui/SkeletonCards.tsx:L5-L17) (iPhone: `.redacted(reason: .placeholder)` cards of the same geometry)
- [ ] Skeletons pulse quietly (Tailwind pulse: opacity 1 -> 0.5 -> 1, 2 s), no shimmer gradients; the pulse stops under reduced or off motion (m:ui/SkeletonCards.tsx:L1-L2) (d:styles/globals.css:L3058-L3062) (iPhone: repeating opacity animation; none when motion is reduced)
- [ ] Shown by Following, Browse (Twitch and Kick) and category screens while the first page loads (m:screens/FollowingScreen.tsx:L180) (m:screens/BrowseScreen.tsx:L389) (m:screens/KickBrowseBody.tsx:L136) (m:screens/CategoryStreamsScreen.tsx:L170) (iPhone: same screens; owned by following-browse-search.md)

## 14. Haptics map

Haptics mark state changes only, never motion; every call is a no-op where unsupported.

- [ ] Arm (15 ms, the heaviest single tick): a chat message long-press arms after 420 ms (fan-out opens) (m:ui/haptics.ts:L19-L22) (m:chat/useLongPressDrag.ts:L22) (m:chat/useLongPressDrag.ts:L69-L74) (iPhone: `UIImpactFeedbackGenerator(style: .medium).impactOccurred()`)
- [ ] Tick (8 ms, light): the fan-out selection moves to a different bucket; the landscape chat overlay handle is tapped (strip toggles); the split handle snaps onto the seam (within 20 px) (m:ui/haptics.ts:L24-L27) (m:chat/ChatFanOut.tsx:L223-L228) (m:watch/ChatOverlayChrome.tsx:L110) (m:watch/SplitHandle.tsx:L49-L56) (iPhone: `UISelectionFeedbackGenerator().selectionChanged()`)
- [ ] Step (5 ms, lightest): the timeout dial crosses a magnitude tier (seconds -> minutes -> hours ...); the brightness or volume swipe crosses each tenth; the split handle is grabbed (m:ui/haptics.ts:L29-L32) (m:chat/ChatFanOut.tsx:L229-L237) (m:screens/WatchScreen.tsx:L700-L708) (m:watch/SplitHandle.tsx:L79) (iPhone: `UIImpactFeedbackGenerator(style: .light).impactOccurred(intensity: 0.6)`)
- [ ] Commit (20 ms): a fan-out action is committed; a chat message mentions you (when the "mention buzz" pref is on, the app is visible, the message is not your own and is under 8 s old) (m:ui/haptics.ts:L34-L37) (m:chat/ChatFanOut.tsx:L247-L249) (m:chat/MobileChatPane.tsx:L136-L146) (iPhone: `UIImpactFeedbackGenerator(style: .rigid)` for commits; mention could use `UINotificationFeedbackGenerator().notificationOccurred(.success)`, decide once)
- [ ] Destructive (15 ms, 40 ms gap, 15 ms): a destructive fan-out action (ban, timeout, delete) is committed, distinct from a copy (m:ui/haptics.ts:L39-L43) (m:chat/ChatFanOut.tsx:L248) (iPhone: `UINotificationFeedbackGenerator().notificationOccurred(.warning)`)
- [ ] Mention haptic pref `sn-mention-haptic`, default on (m:phonePrefs.ts:L14) (m:phonePrefs.ts:L27-L33) (iPhone: same pref; row owned by settings.md)
- [ ] No haptics on tab switches, sheet open/close, pull to refresh, or drill-in navigation (m:ui/haptics.ts:L1-L9) (iPhone: none added there either)

## 15. Toasts at the shell level

- [x] On the phone toasts always anchor bottom-centre, floating tab bar height (56 px) + safe-bottom + 12 px above the bottom edge, newest lowest, sliding up 32 px with a fade (d:components/ToastManager.tsx:L188-L198) (d:components/ToastManager.tsx:L150-L180) (d:components/ToastManager.tsx:L444-L447) (iPhone: an overlay toast host above the tab bar; seen on the iPhone: drawn in their own window, so they float above sheets (seen over a sheet))
- [x] The phone shows only error toasts and toasts that carry an action (e.g. Undo, "Log In"); everything else is suppressed (d:components/ToastManager.tsx:L337-L346) (iPhone: same filter; seen on the iPhone: SNKit `ToastKind.shows` with tests)
- [~] Toast card: glass panel, 16 px padding, width min(380 px, viewport - 24 px), 14 px medium message, a 16 px close button (aria-label "Dismiss"), an accent action button under the text; auto-dismiss after 5000 ms (live 8000 ms, sticky never) (d:components/ToastManager.tsx:L453-L487) (s:stores/AppStore.ts:L1630) (iPhone: same timings; error toast with its Dismiss button seen; the action toast is built but not screenshotted)
- [ ] Session-loss toasts (both with a "Log In" action): "Your session has expired. Please log in again to continue." and "Your login session expired while away. Please log in again." (s:stores/AppStore.ts:L3526-L3544) (iPhone: owned by accounts-signin.md)

## 16. Safe areas, insets and keyboard

- [~] Insets come from a native bridge: the activity pushes `--sn-inset-t/r/b/l` (system bars + display cutout), `--sn-kb` (keyboard minus the bottom bar inset) and the gesture strips `--sn-gesture-l/r/t/b` on every inset change, and the page pulls the same values synchronously once on load (`SNInsets.get()`), because the WebView reports 0 for env() safe areas (m:nativeInsets.ts:L18-L39) (k:MainActivity.kt:L908-L959) (s:main.tsx:L68-L72) (iPhone: SwiftUI safe area insets; no bridge)
- [x] Safe-area variables fall back to env() when the bridge is absent (m:mobile.css:L17-L20) (iPhone: native safe area)
- [ ] Layout math that cannot be expressed declaratively (the mini player clamp) reads numeric insets including the gesture strips (m:nativeInsets.ts:L60-L82) (iPhone: `GeometryProxy.safeAreaInsets`; owned by watch-player.md)
- [x] Content draws into the display cutout (short edges) and behind transparent status and navigation bars (t:gen/android/app/src/main/res/values-v27/themes.xml:L1-L10) (k:MainActivity.kt:L686-L690) (iPhone: full-bleed backgrounds with `.ignoresSafeArea()` on the background only)
- [~] A visualViewport fallback computes `--sn-kb` = innerHeight - viewport height - viewport offsetTop when the native bridge is absent (dev only) (m:ui/useKeyboardInsets.ts:L7-L26) (iPhone: not needed; keyboard safe area / `keyboardLayoutGuide`)
- [ ] The keyboard height lifts sheets and the chat composer; with the keyboard closed nothing moves (m:ui/MobileSheet.tsx:L98-L111) (iPhone: keyboard safe area; composer owned by chat-input-emotes.md)

## 17. Window shape, size classes and orientation

- [ ] Size class by window WIDTH in points: compact below 600, medium below 840, expanded from 840 (m:ui/useWindowShape.ts:L68-L72) (iPhone: compute from `GeometryReader` width, not from UIKit size classes, so iPhones land in the same buckets: portrait compact; landscape 667 to 839 medium; 840 and wider expanded)
- [ ] Landscape means width greater than height (m:ui/useWindowShape.ts:L77) (iPhone: same from geometry)
- [ ] Two panes are possible when expanded, or medium in landscape (m:ui/useWindowShape.ts:L79-L81) (iPhone: every iPhone in landscape qualifies)
- [ ] "Large screen" means the SMALLEST side is at least 600; a phone on its side is still a phone (m:ui/useWindowShape.ts:L56-L66) (m:ui/useWindowShape.ts:L95) (iPhone: always false on iPhone)
- [ ] The default two-pane split falls at 62% of the width (player wider than chat) (m:ui/useWindowShape.ts:L83-L88) (iPhone: same ratio; owned by watch-player.md)
- [~] Hinge handling: a vertical hinge moves the split onto the hinge centre, a half-open horizontal hinge gives a top/bottom split at the hinge centre, a flat horizontal hinge is ignored; hinge data arrives as `sn:fold` from the activity (m:ui/useWindowShape.ts:L83-L98) (m:ui/useWindowShape.ts:L119-L123) (k:MainActivity.kt:L845-L885) (iPhone: no foldable iPhones; `fold` is always null)
- [~] The shape recomputes on resize and orientation change (m:ui/useWindowShape.ts:L100-L118) (iPhone: geometry changes drive SwiftUI automatically)
- [x] The Android app is not orientation-locked: every screen, including the tab shell, rotates with the device (t:gen/android/app/src/main/AndroidManifest.xml:L31) (iPhone: Brandon to decide; the brief expects a portrait app that turns landscape only while watching, see ambiguity note; seen on the iPhone: Brandon: portrait everywhere, landscape only while watching full; seen: the tabs stay portrait and minimizing from landscape turns back)
- [x] Landscape watching is always immersive: no status or navigation bars in any layout; mini player and PiP keep the bars (m:screens/WatchScreen.tsx:L512-L516) (iPhone: `.statusBarHidden(true)` + `.persistentSystemOverlays(.hidden)` on the full player in landscape; owned by watch-player.md) (pref: landscape-watch-always-immersive.md)
- [ ] Layout fixes compress in place and never push UI into a second row or move content down (m:ui/AdaptiveGrid.tsx:L1-L14) (iPhone: same rule for every SwiftUI layout) (pref: no-ui-moves-down.md)
- [ ] The tests pin the derivation: no split without a hinge; tabletop split at 457 for a hinge at y 450 + 14/2; a flat horizontal hinge ignored; a vertical hinge at x 415 + 10/2 gives splitX 420 and two panes at 840 x 757; 700 x 500 two panes, 700 x 900 none; 915 x 411 is expanded with two panes but not a large screen; 840 x 757 and 1280 x 800 are large screens, 412 x 915 is not (m:ui/windowShape.test.ts:L1-L60) (iPhone: port the non-hinge cases as Swift unit tests of the shape function)

## 18. Adaptive grid

- [ ] Column counts: compact = 1 column for cards, 1 for rows in portrait and 2 for rows in landscape; medium = 2; expanded = 3 cards or 2 rows (m:ui/AdaptiveGrid.tsx:L28-L36) (iPhone: `LazyVGrid` with `GridItem(.flexible())` x count from the same table)
- [ ] Default 16 px horizontal padding and 12 px gaps (m:ui/AdaptiveGrid.tsx:L38-L44) (iPhone: same spacing)
- [~] With a vertical hinge and a 2-column layout, the left column ends at the hinge and the gutter IS the hinge (at least as wide as the hinge) (m:ui/AdaptiveGrid.tsx:L48-L69) (iPhone: no hinge on iPhone)
- [ ] Used by Following, Browse (Twitch and Kick) and category stream lists (m:screens/FollowingScreen.tsx:L191) (m:screens/BrowseScreen.tsx:L398) (m:screens/KickBrowseBody.tsx:L145) (m:screens/CategoryStreamsScreen.tsx:L183) (iPhone: same table; owned by following-browse-search.md)

## 19. System bars

- [~] Status and navigation bar icons follow the StreamNook THEME, not the phone's dark mode: light icons when the theme background's luminance (0.2126 R + 0.7152 G + 0.0722 B, over 255) is below 0.5, dark icons otherwise (m:systemBars.ts:L34-L59) (d:themes/index.ts:L1510-L1513) (iPhone: status bar style `.lightContent` / `.darkContent` from the same test, via the hosting controller's `preferredStatusBarStyle`; light content from the default theme seen; other themes wait for the resolved palette (rust-requests.md 1))
- [ ] The colour is applied from the palette already on the document at install, then on every theme change (`sn:theme-applied`) (m:systemBars.ts:L43-L54) (d:themes/index.ts:L1681-L1697) (iPhone: recompute whenever the theme model changes)
- [x] Before any palette loads the bars assume a dark background (light icons) (k:MainActivity.kt:L673-L690) (iPhone: launch with light status bar content to match the default dark theme)
- [ ] Showing the bars again after immersive playback re-asserts the icon colour and resets the transient-bars behaviour (k:MainActivity.kt:L313-L336) (iPhone: native; nothing to re-assert)
- [~] Immersive hides both bars with "show transiently by swipe" behaviour (k:MainActivity.kt:L313-L324) (iPhone: `.statusBarHidden(true)` and `.persistentSystemOverlays(.hidden)`; the home indicator auto-hides and returns on touch)

## 20. Motion settings

- [ ] Setting `motion_mode`: "full" (default when unset), "reduced" or "off"; it applies app-wide and immediately (d:components/MotionScope.tsx:L6-L32) (s:types/index.ts:L1210-L1215) (iPhone: read from settings into the environment; settings row owned by settings.md)
- [ ] Reduced and Off drop transform and layout motion in JS-driven animations (tab slide, drill-in slide, sheet slide, settle-in offsets, the tab highlight glide) while keeping fades (d:components/MotionScope.tsx:L27-L31) (iPhone: replace offsets/scales with `.opacity` transitions or none)
- [ ] Off collapses every CSS transition and animation to instant, except spinners and the Penrose loader (d:styles/globals.css:L3015-L3044) (iPhone: `withAnimation(nil)` / `.transaction { $0.animation = nil }` except loaders)
- [ ] Reduced stops looping decorations (live-dot pings, skeleton pulses, shimmers), shortens glass-button transitions to 0.18 s ease-out and turns scale-ins into a 0.18 s fade (d:styles/globals.css:L3046-L3062) (iPhone: same rules)
- [ ] Some loops also stop when the OS asks for reduced motion, independent of the app setting (d:styles/globals.css:L1995-L1999) (iPhone: also honour `accessibilityReduceMotion` for decorative loops)
- [ ] Animated decorations animate only opacity or transform; never colour or filter keyframes, which repaint every frame (pref: repaint-animation-and-boot-stutter.md) (d:styles/globals.css:L204-L240) (iPhone: animate opacity/transform; avoid animating blur radius or colour on loops)
- [ ] Animations keep full frame rate and clean loops (pref: never-drop-fps.md) (d:components/PenroseMarch.tsx:L1-L20) (iPhone: `CADisplayLink`/`TimelineView(.animation)` at the display rate)

## 21. Glass and glassiness

- [x] Setting `glass_transparency` 0 to 100 (default 100 when unset) sets the global glass strength (value / 100) (d:themes/index.ts:L1703-L1723) (r:models/settings.rs:L989) (iPhone: a 0 to 1 strength in the theme model; seen on the iPhone: Theme panel slider writes it through `patch_settings`)
- [ ] Setting `glass_blur` (default on, every platform): off caps the effective strength at 0.45 and strips every backdrop blur while surfaces stay tinted (d:themes/index.ts:L1572-L1574) (d:themes/index.ts:L1725-L1738) (d:styles/globals.css:L2958-L2969) (r:models/settings.rs:L994-L995) (iPhone: blur off = solid tinted fills, no `glassEffect`)
- [x] Every frosted surface is a two-layer composite: an opaque `backgroundTertiary` floor whose opacity is (1 - strength), under the surface's own tint; blur radius scales by strength (panel 10 px, button 8 px, modal 28 px, chrome 20 px, dock 4 px, sheet backdrop 6 px) (d:styles/globals.css:L2443-L2470) (d:styles/globals.css:L2795-L2827) (d:styles/globals.css:L1660-L1683) (iPhone: Liquid Glass at full strength; overlay a `backgroundTertiary` fill at opacity (1 - strength) for intermediate values)
- [ ] Translucent surface tints (`surface`, `surface-hover`, `surface-active`) are recomputed toward opaque as strength drops (tint composited over tertiary, colour and alpha interpolated) (d:themes/index.ts:L1576-L1605) (iPhone: compute blended colours in Rust; see moves file)
- [~] 0% contract: at Glassiness 0 no surface is see-through and no backdrop blur remains anywhere (except a member's chat atmosphere frost, the floating chat overlay and the full-screen chat column, which have their own opacity settings) (d:styles/globals.css:L2950-L2969) (iPhone: no `glassEffect`/`Material` at 0; solid fills) (pref: glassiness-zero-contract.md; every StreamNook surface seen opaque at 0; the system tab bar, sheets and menus stay Liquid Glass (Brandon))
- [ ] Menus over live video use the flyout recipe (92% base at full glass, opaque at 0%), never the chat popover tint, and are never frosted inside another frosted parent (d:styles/globals.css:L2871-L2895) (iPhone: menus over video get a stronger tint behind the glass; owned by watch-player.md) (pref: glassiness-zero-contract.md)
- [ ] Popovers over scrolling chat (drop bar, pinned banner) drop the blur and use a 90% tint (opaque at 0%) because the chat repaints under them (m:mobile.css:L103-L112) (iPhone: owned by chat-view.md)
- [ ] The landscape chat overlay over video uses no blur: a background tint at the overlay opacity setting (default 55%), a faint top sheen and a 1 px inner hairline on the video edge (m:mobile.css:L114-L133) (iPhone: owned by watch-player.md)
- [x] Every new surface uses the glass primitives (panel, button, modal, glaze, flyout), drops chrome it does not need, and gets a visual pass before it is shown to Brandon (pref: design-pass-before-presenting.md) (d:styles/globals.css:L2443-L2470) (iPhone: shared SwiftUI modifiers `snGlassPanel`, `snGlassButton`, `snGlassChrome` built once in the design system; seen on the iPhone: `snPanel`, `snMediaCard`, `snGlass`, `snChipVeil`)
- [x] Platform logos appear only where platforms mix (pref: platform-marks-only-where-mixed.md) (m:ui/AdaptiveGrid.tsx:L1-L14) (iPhone: same rule on every list; owned per area; seen on the iPhone: only the Accounts card (it mixes) draws them)

## 22. Theme tokens

- [ ] The phone reads these theme variables: `background`, `background-secondary`, `background-tertiary`, `surface` (+ hover, active), `text-primary`, `text-secondary`, `text-muted`, `accent` (+ hover, muted, neon), `border`, `border-light`, `border-subtle`, `success`, `warning`, `error`, `info`, the eight `highlight-*` colours, and the fixed LIVE red `#ef4444` (d:themes/index.ts:L1608-L1679) (p:tailwind.config.js:L15-L37) (d:styles/globals.css:L1788) (iPhone: a `Palette` struct delivered by Rust; see moves file)
- [x] Default theme "Frosted Glass" (`winters-glass`): background `#0c0c0d`, secondary `rgba(255, 255, 255, 0.03)`, tertiary `#1a1a1b`, surface `rgba(151, 177, 185, 0.15)` (hover 0.25, active 0.35), text `#ffffff` / `#97b1b9` / `rgba(151, 177, 185, 0.6)`, accent `#97b1b9` (hover `#adc4cc`), borders at 0.3 / 0.2 / 0.1 of `rgb(151, 177, 185)`, success `#22c55e`, warning `#eab308`, error `#ef4444`, info `#6b9dff` (d:themes/index.ts:L208-L244) (d:themes/index.ts:L1464) (iPhone: same values)
- [ ] 22 built-in themes plus user custom themes; the OLED theme takes its accent from `oled_accent` (invalid hex falls back to the default) and derives hover (+20% lighter) and muted (50% alpha) from it; the retired second OLED theme migrates to OLED with `#ff9933` (d:themes/index.ts:L1515-L1535) (s:stores/AppStore.ts:L1706-L1712) (iPhone: resolved by Rust, not re-implemented in Swift)
- [~] Interface font from `font` (default "satoshi"; also twitch/Inter, geist, manrope, outfit, space-grotesk, serif/Fraunces, system, or a custom family name); chat body weight 300, 200 for Inter (d:themes/index.ts:L1740-L1822) (iPhone: bundle the same font files; "system" maps to SF Pro; Satoshi bundled and seen, `system` maps to SF Pro; other ids draw in Satoshi (rust-requests.md 3))
- [ ] Shared elevation tokens: `--elev-1` `0 1px 2px rgba(0,0,0,0.20), 0 2px 6px rgba(0,0,0,0.12)`; hairline contour `0 0 0 0.5px rgba(0,0,0,0.4)` (d:styles/globals.css:L1829) (d:styles/globals.css:L1853) (iPhone: `.shadow` modifiers in the design system)

## 23. Typography and touch targets

- [~] Base text 14 px; tab screen titles 20 px bold; drill-in titles 18 px bold; onboarding title 24 px bold; sheet titles 14 px semibold; section labels 11 to 12 px semibold uppercase with wide tracking in `textMuted`; body rows mostly 12 to 13.5 px; primary sign-in button 15 px semibold (m:mobile.css:L15) (m:screens/FollowingScreen.tsx:L129) (m:screens/SettingsScreen.tsx:L82) (m:MobileOnboarding.tsx:L27) (m:ui/MobileSheet.tsx:L125) (m:screens/AdFreeSettings.tsx:L33) (iPhone: fixed point sizes matching these; Dynamic Type support is backlog; scale built as Font tokens; tab titles are 32 pt inline large navigation titles)
- [x] Interactive rows and buttons are at least 44 x 44 px (m:mobile.css:L40-L44) (iPhone: HIG 44 pt minimum)

## Decisions for Brandon

Not parity lines: open questions found while reading, each to be settled before its screen is built.

- Orientation: Android rotates every screen; the iPhone brief expects portrait with landscape only while watching. Pick one (§ 17).
- Tab labels: Android's bar is icon-only; the iOS system tab bar shows labels. Keep labels (system look) or hide them (Android look) (§ 6).
- Glassiness 0 vs system Liquid Glass: the system tab bar and sheets stay glass; honouring the 0% contract may need a custom tab bar and sheet background (§ 21).
- Offline launch: Android shows the sign-in screen when the auth check fails for network reasons (§ 2.3). Match it, or keep the last signed-in state offline?
- Boot listener skip: a settings read failure skips every boot listener on Android (§ 2.3). The iPhone should register them regardless.
- Setup page 1 ("Setting things up", ad blocker install) is a no-op on iOS; drop it from the iPhone wizard (§ 3)?
- Tab switch slide (0.16 s) vs the system TabView's instant switch (§ 7).

## Beyond Android (backlog, Brandon picks)

- [ ] (backlog, not parity) The mini player docks as the iOS 26 tab bar bottom accessory (`tabViewBottomAccessory`), the way system media apps show now playing, instead of floating free
- [ ] (backlog, not parity) The tab bar minimises while scrolling down a list (`tabBarMinimizeBehavior(.onScrollDown)`) and returns on scroll up
- [ ] (backlog, not parity) Tapping the active tab scrolls its list to the top and pops any pushed screen (system default; Android does nothing)
- [ ] (backlog, not parity) Home Screen quick actions on the app icon: "Following", "Browse", and "Watch {last channel}" (`UIApplicationShortcutItem`)
- [ ] (backlog, not parity) App Intents / Shortcuts: "Open StreamNook to a channel", "Show who is live"; Siri and Spotlight suggestions from them
- [ ] (backlog, not parity) Spotlight indexing of followed channels so a search for a streamer's name opens their channel (`CSSearchableItem`)
- [ ] (backlog, not parity) Handoff: a stream watched on the phone appears as a continuation on a Mac running StreamNook (`NSUserActivity`)
- [ ] (backlog, not parity) Live Activity / Dynamic Island while a stream plays or a drop is progressing (needs a widget extension; current xtool builds produce an empty extension, so blocked until that is fixed)
- [ ] (backlog, not parity) Home Screen widget showing live followed channels (same extension blocker)
- [ ] (backlog, not parity) Alternate app icons chosen in Settings (`setAlternateIconName`)
- [ ] (backlog, not parity) Dynamic Type: the type scale in § 23 scales with the user's text size
- [ ] (backlog, not parity) System "Reduce Transparency" forces Glassiness 0 behaviour and system "Reduce Motion" forces motion Reduced, on top of the app settings
- [ ] (backlog, not parity) Focus filter: a "Streaming" focus can silence live alerts except favourites
- [ ] (backlog, not parity) Search as the iOS 26 search tab role (`Tab(role: .search)`) in the tab bar
