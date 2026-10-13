# Parity contract: You tab, accounts and sign-in

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

Term used in this area:

- "the sign-in sheet" means the one iOS WKWebView sheet that replaces Android's native login overlay (`k:TwitchLoginPlugin.kt`). Section 3 specifies it once; every flow after that reuses it.

## 1. You tab layout

### 1.1 Order and frame

- [ ] The You tab is one vertical scroll with, in order: identity header, update banner (only when an update exists), Cosmetics row, "Accounts" section, "Open the app on" section, "Settings" section; no intermediate menu between You and the settings panels (m:screens/YouScreen.tsx:L1-L2, L39-L167) (iPhone: `List` or `ScrollView` + `LazyVStack` inside the You tab's `NavigationStack`, `.listStyle(.insetGrouped)` sections in this order)
- [ ] The scroll content clears the floating tab bar at its bottom (`sn-tabbar-clearance`) (m:screens/YouScreen.tsx:L40) (iPhone: system `TabView` safe area handles it; no manual clearance)
- [ ] Desktop counterpart: the account rows live in Settings > Profile > "Accounts" (`d:components/settings/LinkedAccountsSection.tsx`); the phone deliberately lifts them onto the You tab so sign-in and sign-out sit beside the account they act on and are never hidden behind the tab bar (m:screens/YouScreen.tsx:L117-L119; d:components/settings/LinkedAccountsSection.tsx:L111-L117) (iPhone: same placement on the You tab)

### 1.2 Identity header

- [ ] Header row: avatar left, identity block right, `items-start` (top aligned, not centred) so a two-line name plus badge row never drags the avatar off the name; gap 12 px, padding 16 px left/right, 16 px top, 12 px bottom (m:screens/YouScreen.tsx:L41-L44) (iPhone: `HStack(alignment: .top, spacing: 12)` with `.padding(.horizontal, 16).padding(.top, 16).padding(.bottom, 12)`)
- [ ] Avatar is 56 px, circular, the signed-in Twitch profile image rendered through the 7TV avatar component (animated 7TV avatar when the user has one, `motion="animate"`), keyed by the Twitch user id; not draggable (m:screens/YouScreen.tsx:L45-L54) (iPhone: 56 pt circular `AsyncImage`/animated image view from the shared image pipeline, 7TV avatar override keyed by the bare Twitch user id)
- [ ] No profile image: a 56 px `bg-surface` circle placeholder (m:screens/YouScreen.tsx:L55-L57) (iPhone: 56 pt `Circle().fill(surface)`)
- [ ] Identity block (display name, login, badges, paint) is `OwnIdentityHeader`, owned by profile-cosmetics.md (m:screens/YouScreen.tsx:L58-L63) (iPhone: see profile-cosmetics.md)
- [ ] Display-name fallback chain passed to the header: `display_name`, then `username`, then the literal "Signed in" (m:screens/YouScreen.tsx:L61)
- [ ] No user id yet (profile still loading): a single line "Signed in", 18 px bold `textPrimary`, truncated (m:screens/YouScreen.tsx:L64-L68) (iPhone: `Text("Signed in").font(.system(size: 18, weight: .bold)).lineLimit(1)`)

### 1.3 Update banner

- [~] The update check runs once each time the You tab mounts (not at boot, never mid-stream) and the banner shows only when a newer build exists; mechanism owned by system.md (m:screens/YouScreen.tsx:L24-L37, L72-L99) (iPhone: `[~]` updates come from TestFlight; no banner, no check)
- [~] The APK update banner's look and copy follow system.md § 6; on iPhone it is never shown (updates come from TestFlight) (m:screens/YouScreen.tsx:L72-L99)

### 1.4 Cosmetics row

- [ ] Row: paintbrush icon 20 px in accent, title "Cosmetics" 15 px medium, subtitle "Your badges, atmosphere, and 7TV paint" 12.5 px muted truncated, muted chevron 16 px; padding 12 px, 8 px radius, pressed `bg-surface-active`; outer inset 12 px left/right, 8 px below (m:screens/YouScreen.tsx:L101-L115) (iPhone: `NavigationLink` row with SF Symbol `paintbrush.pointed.fill` tinted accent, subtitle as secondary text)
- [ ] Tapping opens the Cosmetics screen layer (`setCosmeticsOpen(true)`); the screen itself is profile-cosmetics.md (m:screens/YouScreen.tsx:L103) (iPhone: NavigationStack push of the Cosmetics view)

### 1.5 Section headers

- [ ] Section headers "Accounts", "Open the app on", "Settings": 12 px semibold, `textMuted`, uppercase, wide tracking, 16 px left inset, 4 px below (m:screens/YouScreen.tsx:L120-L122, L130-L132, L147-L149) (iPhone: `Section(header:)` default grouped header style, which is already small uppercase secondary)

### 1.6 Start tab ("Open the app on")

- [ ] Full-width segmented control with four options in order "Following", "Browse", "Rewards", "You"; 16 px side inset, 16 px below (m:screens/YouScreen.tsx:L127-L145) (iPhone: `Picker` with `.pickerStyle(.segmented)`)
- [ ] The choice persists per device under localStorage key `sn-start-tab`; default "following"; an unknown stored value reads as "following"; a storage failure keeps the choice for the session only (m:phonePrefs.ts:L13-L25, L47-L54) (iPhone: `@AppStorage("sn-start-tab")` default `following`; phone-only shell pref, never in the Rust settings file)
- [ ] The app opens on this tab, and the back gesture unwinds to it (m:navStore.ts:L51, L113) (iPhone: initial `TabView` selection from the stored value; back unwind is shell-design.md)
- [ ] Segmented control look (shared desktop primitive): track `rgba(151,177,185,0.06)` with inset shadow, 8 px radius, 2 px padding; a single sliding thumb `rgba(151,177,185,0.15)`, 6 px radius, moving with `cubic-bezier(0.34,1.4,0.5,1)` over 280 ms (instant when reduce-motion is on, and instant on first placement); labels 13 px medium, active `textPrimary`, inactive `textMuted` (d:components/settings/_primitives.tsx:L176-L262) (iPhone: system segmented picker; it already animates and honours Reduce Motion)

### 1.7 Settings rows

- [ ] One row per `SETTINGS_ROWS` entry, in its order (Player, Ad-free, Chat, Theme, Notifications, Cache, Support, What's New); labels, icons and descriptions are owned by settings.md (m:screens/YouScreen.tsx:L147-L165; m:screens/SettingsScreen.tsx:L29-L53) (iPhone: see settings.md)
- [ ] Row look: icon 20 px `textSecondary`, label 15 px medium, description 12.5 px muted truncated, muted chevron 16 px, 12 px padding, 8 px radius, pressed `bg-surface-active`, 12 px outer inset (m:screens/YouScreen.tsx:L150-L164) (iPhone: `NavigationLink` rows with SF Symbols chosen in settings.md)
- [ ] Tapping a row opens that settings panel directly (`openSettings(id)`), no intermediate list (m:screens/YouScreen.tsx:L154) (iPhone: NavigationStack push of the panel)

## 2. Accounts card

### 2.1 Card frame

- [ ] One glass panel holding one row per platform the phone can sign in to: Twitch, then Kick; rows separated by a `borderSubtle` hairline; card inset 16 px left/right, 16 px below (m:profile/AccountsCard.tsx:L128-L196; m:screens/YouScreen.tsx:L123-L125) (iPhone: a grouped `Section` with two rows; Liquid Glass background only if the shell's card style uses it, see shell-design.md)
- [~] No YouTube or TikTok rows on the phone: their connect is desktop only and errors on mobile (desktop shows Kick, YouTube and TikTok under "Other platforms") (d:components/settings/PlatformAccountRows.tsx:L28-L50; r:services/youtube_auth_service.rs:L656; r:services/tiktok_auth_service.rs:L462) (iPhone: `[~]` non-goal per the iOS plan; YouTube and TikTok sign-in stay desktop only)
- [ ] No linked secondary Twitch accounts, no "Add account", no "Make main", no "Send chat as", no credential-storage notice on the phone (desktop has all of them) (d:components/settings/LinkedAccountsSection.tsx:L50-L88, L143-L306) (iPhone: same omission for parity; account switching is in the backlog section)
- [ ] Each row: 14 px padding, 12 px gap, avatar, then a title (15 px semibold `textPrimary`, truncated) over a subtitle (12.5 px `textMuted`, snug line height), then the row's action on the right (m:profile/AccountsCard.tsx:L131-L140, L150-L161) (iPhone: custom row `HStack(spacing: 12)` with `.padding(14)`)

### 2.2 Row avatar

- [ ] Avatar 44 px circle (object-cover) when the account has a picture, rendered through the 7TV avatar component so an animated 7TV avatar replaces it; not draggable (m:profile/AccountsCard.tsx:L40-L54) (iPhone: 44 pt circular image from the shared pipeline)
- [ ] With a picture, the platform's bare mark (15 px) sits on the avatar's bottom-right corner (2 px below, 4 px right of the edge), lifted by a `drop-shadow(0 1px 2px rgba(0,0,0,0.7))`, no disc behind it (m:profile/AccountsCard.tsx:L60-L65) (iPhone: platform logo image 15 pt in an `.overlay(alignment: .bottomTrailing)` with `.shadow(color: .black.opacity(0.7), radius: 2, y: 1)`) (pref: platform-marks-only-where-mixed.md, the card mixes platforms so marks are allowed)
- [ ] Without a picture, a 44 px `bg-surface` circle with the platform mark at 20 px centred (m:profile/AccountsCard.tsx:L55-L58)
- [ ] The Twitch avatar's 7TV key is the bare Twitch user id; the Kick avatar's 7TV key is `sevenTvIdForStream({provider: 'kick', user_id: kick.id})` and only while connected (m:profile/AccountsCard.tsx:L132, L151-L155) (iPhone: ask Rust for the namespaced cosmetics key; never pass a bare Kick id, Kick and Twitch ids are both numeric)

### 2.3 Twitch row

- [ ] Title: Twitch `display_name`, else `login`, else "Twitch" (m:profile/AccountsCard.tsx:L134-L136)
- [ ] Subtitle "Your StreamNook account"; while the sign-out is armed it reads "Signs you out of StreamNook" (m:profile/AccountsCard.tsx:L137-L139)
- [ ] The Twitch row offers only signing out (Twitch is the account the whole app runs on, so there is no connect action here) (m:profile/AccountsCard.tsx:L4-L7, L141-L146)
- [ ] Sign-out pill: 12 px horizontal, 6 px vertical padding, fully rounded, 12.5 px semibold, no wrap, `glass-button` with `textSecondary` text, label "Sign out"; the tap target stays at least 44 x 44 px around the smaller pill (m:profile/AccountsCard.tsx:L69-L88; m:mobile.css:L41-L44) (iPhone: `Button("Sign out")` with `.buttonStyle(.bordered)` + `.controlSize(.small)` + `.buttonBorderShape(.capsule)`, `.contentShape` padded to 44 pt)
- [ ] Two-tap confirm: the first tap arms it (pill turns `bg-error/15` with `text-error`, label "Confirm", colour transition), a second tap within 3000 ms signs out; with no second tap it disarms itself after 3000 ms; the timer is cleared on unmount (m:profile/AccountsCard.tsx:L20-L37, L81-L86, L141-L146) (iPhone: keep the inline arm-then-confirm pill with a 3 s auto-disarm, `UINotificationFeedbackGenerator(.warning)` on arm; do not swap in an alert, the row itself explains the consequence)
- [ ] Confirmed sign-out calls the main-account sign-out: `invoke sign_out_active_twitch_account`, which promotes the most recently added linked account if any, else fully signs out; either way Rust clears the web-session cache, entitlement caches, the GIF config cache and the drops credential (s:stores/AppStore.ts:L3350-L3368; s:services/accountService.ts:L55-L57; r:commands/accounts.rs:L132-L153) (iPhone: same Rust command through `invoke`)
- [ ] Promoted outcome: identity is re-established (auth status, follows, account list, chat reconnect) and toast "Signed out. Now watching as @{login}" (success) (s:stores/AppStore.ts:L3315-L3334, L3355-L3358)
- [ ] Full sign-out outcome: signed-out state, follows emptied, toast "Successfully signed out from Twitch" (success); the whole shell then shows the signed-out landing (section 4.1) (s:stores/AppStore.ts:L3359-L3363; m:MobileApp.tsx:L136-L140)
- [ ] Failure toast "Failed to sign out. Please try again." (error) (s:stores/AppStore.ts:L3364-L3367)
- [ ] Signing out of Twitch signs the embedded sign-in browser out too: every cookie and all web storage are removed, so the next sign-in starts at Twitch's login page instead of continuing as the person who left, and the silent 7TV re-mint stops minting for them (s:stores/AppStore.ts:L3302-L3305; k:TwitchLoginPlugin.kt:L427-L445; r:twitch_login_plugin.rs:L130-L141) (iPhone: `WKWebsiteDataStore.removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast)` on the sign-in store, then tell Rust to drop its cookie jar). Android divergence for Brandon: only `logoutFromTwitch` clears the jar, and the You tab calls `signOutActiveAccount`, which does not, so on Android today a You-tab sign-out leaves the old twitch.tv, kick.com and 7tv.app sessions in the jar (s:stores/AppStore.ts:L3350-L3368)

### 2.4 Kick row

- [ ] Title: the Kick account name when connected (else "Kick"); "Kick" when signed out (m:profile/AccountsCard.tsx:L157-L159)
- [ ] Subtitle, first match wins: the in-flight step message; connected with Kick follows: "{N} follow" / "{N} follows" (N counts every Kick follow, imported or hand-added); connected with none: "Signed in"; signed out: "Not signed in"; one line, truncated (m:profile/AccountsCard.tsx:L97, L120-L126, L160)
- [ ] "Connected" means Rust holds a Kick OAuth token (the roster row is `connected`); an `expired` roster row reads as signed out (s:stores/platformAccountStore.ts:L206-L232; r:services/account_roster.rs:L160-L178)
- [ ] Signed out: a Kick-branded pill "Sign in with Kick" in Kick's colour `#53fc18` with black 12.5 px bold text, 14 px horizontal, 6 px vertical padding, fully rounded, so it reads as signing in WITH Kick; while busy it reads "Signing in" and is disabled at 60% opacity (m:profile/AccountsCard.tsx:L180-L194; s:types/providers.ts:L36) (iPhone: capsule `Button` with `.tint(Color(hex: 0x53fc18))`, `.foregroundStyle(.black)`, bold; disabled state while busy)
- [ ] Tapping it runs the one Kick connect action (`connect('kick')`, section 6); a second tap while one is in flight does nothing (s:stores/platformAccountStore.ts:L264-L266)
- [ ] While connecting, the subtitle reads "Waiting for you to sign in…" (s:stores/platformAccountStore.ts:L266)
- [ ] Connected: a refresh icon button (arrows-clockwise 18 px, muted, 44 x 44 tap target, aria-label "Update your Kick follows") then the Sign out pill (m:profile/AccountsCard.tsx:L162-L179) (iPhone: SF Symbol `arrow.clockwise` button with `.accessibilityLabel("Update your Kick follows")`)
- [ ] The refresh icon spins while a re-read runs and both buttons are disabled at 50% opacity while syncing or while a connect/disconnect is in flight (m:profile/AccountsCard.tsx:L166-L171, L174) (iPhone: `.symbolEffect(.rotate, isActive: syncing)` or a `ProgressView` in its place)
- [ ] Refresh logic: ask Rust whether a kick.com site session is still readable (`invoke kick_account_is_synced`, false on error); if yes, re-read silently, if not (the embedded browser was cleared) re-run the sign-in sheet; `invoke kick_account_sync` with `interactive = !synced` (m:profile/AccountsCard.tsx:L103-L118; s:stores/followsStore.ts:L178-L188; r:commands/provider_browse.rs:L342-L357, L428-L431)
- [ ] Refresh success toast "Found {N} Kick channel you follow" / "Found {N} Kick channels you follow" (success); failure toast "Could not load your Kick follows. Try again in a moment." (error) (m:profile/AccountsCard.tsx:L110-L114)
- [ ] After any Kick import the follow list is replaced from Rust's answer and live state is re-read at once rather than at the next sweep (s:stores/followsStore.ts:L182-L185; r:commands/provider_browse.rs:L405-L419)
- [ ] Kick Sign out uses the same two-tap 3000 ms confirm pill (disabled while busy); confirmed, it calls `invoke kick_disconnect`, which clears the Kick OAuth token, expires only kick.com and id.kick.com cookies in the sign-in browser (Twitch stays signed in), and removes imported Kick follows; hand-added Kick follows stay (m:profile/AccountsCard.tsx:L172-L178; s:stores/platformAccountStore.ts:L312-L328; r:commands/chat.rs:L361-L389) (iPhone: Rust command, plus Swift deletes every cookie whose domain ends in `kick.com` from the sign-in store's `httpCookieStore`)
- [ ] Kick sign-out does not stop an open Kick player or a joined read-only Kick chat; sending goes read-only (r:commands/chat.rs:L361-L369)
- [ ] After a Kick sign-out the Kick follow rows vanish immediately, then the roster and follows are re-read (s:stores/platformAccountStore.ts:L316-L327)
- [ ] A Kick session that dies on its own (not a sign-out) keeps the Kick follow rows and live rows (Kick liveness runs on an app token) and only re-reads the follow list (s:stores/platformAccountStore.ts:L209-L222)

### 2.5 Account state sync (no polling per surface)

- [ ] Account rows read one store fed by Rust's account roster (`invoke get_account_roster`) and repaint on `listen account-roster-changed` (full roster payload) and `listen platform-account-changed` (m:MobileApp.tsx:L48; s:hooks/usePlatformAccountSync.ts:L52-L74) (iPhone: Swift `@Observable` roster model updated from the `account-roster-changed` HostSink event; never poll per row)
- [ ] The roster is re-read whenever the app becomes visible, and on a 30 000 ms backstop timer that only runs while visible (s:hooks/usePlatformAccountSync.ts:L38, L103-L106, L118-L121) (iPhone: re-read on `scenePhase == .active`; backstop timer only while active)
- [ ] An IPC failure while reading the roster keeps the previous state (never treated as a sign-out) (s:stores/platformAccountStore.ts:L243-L250)
- [ ] `listen platform-session-expired` (payload is the provider id): toast "Your {Kick|YouTube|TikTok} session expired." (warning) with a "Reconnect" action that runs that platform's connect; for YouTube the live rows are cleared, Kick keeps its rows (s:hooks/usePlatformAccountSync.ts:L76-L101) (iPhone: banner/toast with a "Reconnect" button, owned visually by shell-design.md toasts)
- [ ] Connecting or disconnecting Kick is reported to the StreamNook account database keyed by the Twitch user id (changes only, never a heartbeat; skipped while signed out of Twitch and backfilled after the next Twitch sign-in; a lost claim waits 15 min, an unreachable server 5 min before retrying) so member cosmetics can follow the user into Kick chat (s:stores/platformAccountStore.ts:L118-L203) (iPhone: same, but see the moves file; this decision should run in Rust)

## 3. The sign-in sheet (replaces the Android login overlay)

### 3.1 Why a WKWebView sheet, never ASWebAuthenticationSession

- [ ] Every web sign-in on the phone runs in ONE native in-app browser whose cookie jar and web storage are shared app-wide and persist across launches: Twitch main sign-in, Drops, Kick, 7TV and Subscribe all use it, and later flows rely on the session an earlier one left (Drops and 7TV ride the Twitch session; Subscribe rides Twitch and Kick) (k:TwitchLoginPlugin.kt:L150-L163, L209-L221; r:commands/twitch.rs:L1322-L1340) (iPhone: one `WKWebView` sheet component over one persistent `WKWebsiteDataStore` dedicated to sign-in, iOS 17 `WKWebsiteDataStore(forIdentifier:)` or `.default()`)
- [ ] Which flow needs what: Twitch main needs the twitch.tv cookie jar (`auth-token`, `unique_id`); Drops needs the live Twitch session plus interception of a `https://www.twitch.tv/#access_token=` redirect; Kick needs kick.com's HttpOnly `session_token` plus interception of `http://localhost:3000/callback`; 7TV needs to read 7tv.app's `localStorage`; Subscribe needs the signed-in Twitch/Kick session (k:TwitchLoginPlugin.kt:L597-L666, L737-L792; r:services/twitch_auth_service.rs:L559-L602; r:services/providers/kick_account.rs:L571-L668) (iPhone: all five use the WKWebView sheet; `ASWebAuthenticationSession` is unsuitable for every one of them because its cookies live in Safari's jar the app cannot read, it cannot read page storage, and it cannot complete on an `https://www.twitch.tv/` or `http://localhost` callback the app does not own)
- [ ] Cookies reach Rust: Rust reads the jar it needs (twitch.tv for the web session, kick.com for the Kick follow list) on demand (r:twitch_login_plugin.rs:L193-L200; r:services/twitch_auth_service.rs:L561-L602) (iPhone: Swift exports `httpCookieStore.getAllCookies()` for the relevant domains, HttpOnly included, to Rust through `set_web_cookies` after every navigation finish in the sheet, on sheet close, and at launch; the Rust iOS arms read that jar. Planned in the iOS plan, phase 2)
- [ ] Never call WKWebView cookie APIs in a way that blocks the main thread waiting on itself (r:platform/cookies.rs:L52; r:services/twitch_auth_service.rs:L395-L432; memory macos-main-thread-deadlock.md) (iPhone: `httpCookieStore.allCookies()` async on the main actor, hand the result to Rust off-main; the iOS core must not compile the desktop `harvest` path that reads the main window's jar, r:services/twitch_auth_service.rs:L267-L328, L389-L393)

### 3.2 Look

- [ ] Full-screen surface over the app: page background `#0E0E10`; a top bar 48 px tall (plus the status-bar inset), background `#18181B`, centred title 15 px `#EFEFF1`, and a close "✕" at the leading edge (18 px, 16 px left / 20 px right padding); the page starts below the bar and is kept clear of the gesture bar and, in landscape, the cutout (k:TwitchLoginPlugin.kt:L32-L34, L201-L207, L330-L365, L388-L417) (iPhone: `.fullScreenCover` or `.sheet` with `.presentationDetents([.large])`, a `NavigationStack` toolbar with `.navigationTitle(title)` inline and a leading `Button` with SF Symbol `xmark`; WKWebView respects the safe area)
- [ ] Bar title defaults to "Sign in to Twitch"; each flow passes its own: "Sign in to Kick", "Sign in to 7TV", "Connect drops", "Subscribe to {name}", "Gift subs to {login}" (k:TwitchLoginPlugin.kt:L65-L66, L338-L340; r:services/providers/kick_account.rs:L575; m:cosmetics/sevenTvConnect.ts:L91; m:screens/RewardsScreen.tsx:L357; s:hooks/useChannelSocial.ts:L416-L419; s:utils/extendedCommands.ts:L392-L396)
- [ ] No URL bar, no other chrome (k:TwitchLoginPlugin.kt:L330-L331)

### 3.3 Browser behavior

- [ ] JavaScript and DOM storage on; cookies accepted, third-party cookies accepted (k:TwitchLoginPlugin.kt:L209-L221) (iPhone: WKWebView defaults; third-party cookies are allowed within the same `WKWebsiteDataStore` for these first-party flows, verify Kick's id.kick.com to kick.com hop on device)
- [ ] Android presents a current mobile Chrome user agent (Chrome 140 on a Pixel 6) so Twitch's browser-version gate does not show its "unsupported browser" wall on an old system WebView (k:TwitchLoginPlugin.kt:L159-L163, L216) (iPhone: set `customUserAgent` to a current Mobile Safari UA, never a Chrome UA; a spoofed Chrome UA is known to break Twitch login inside WKWebView)
- [~] A document-start script fakes `navigator.userAgentData` (Client Hints: Chromium/Google Chrome 140, mobile, Android, Pixel 6) on twitch.tv and kick.com origins only (k:TwitchLoginPlugin.kt:L102-L135, L223-L234) (iPhone: not needed; WebKit has no `userAgentData`, so the gate reads only the UA string)
- [ ] Navigations to anything other than `http://` or `https://` (app-store links, `intent://`, an "Open in app" hop) are swallowed so the sign-in page stays instead of an unknown-scheme error page; logged by scheme only (k:TwitchLoginPlugin.kt:L247-L253) (iPhone: `webView(_:decidePolicyFor:)` returns `.cancel` for non-http(s) schemes, including `twitch://`, `itms-apps://`; never hand them to `UIApplication.open`)
- [ ] Logged navigation URLs never include the fragment (that is where a credential rides) (k:TwitchLoginPlugin.kt:L313-L316) (iPhone: log `url` with `fragment` stripped)
- [ ] Only one sign-in surface exists at a time; opening while one is open re-navigates the existing one and restarts any storage watch for the new key (Kick's second leg reuses the same surface) (k:TwitchLoginPlugin.kt:L190-L199) (iPhone: one sheet instance; a second open request navigates it)
- [ ] A hidden re-mint never takes over a visible sign-in (the hidden open is a no-op); a visible sign-in replaces a running hidden re-mint silently, without firing a cancel (the replaced re-mint hears the new token or runs out its own deadline) (k:TwitchLoginPlugin.kt:L171-L184) (iPhone: same arbitration in the sheet coordinator)
- [ ] Opening the surface resets any captured Kick redirect left from an abandoned attempt (stale `state`) (k:TwitchLoginPlugin.kt:L185-L189)

### 3.4 Closing and cancel

- [ ] Tapping the close button dismisses the surface and tells the app the sign-in was cancelled (`sn:login-cancelled`), so every waiting flow (Twitch, Drops, 7TV) stops spinning; Kick notices by polling whether the surface is still open (k:TwitchLoginPlugin.kt:L356-L363; k:MainActivity.kt:L113-L120; r:services/providers/kick_account.rs:L588-L592) (iPhone: Close button and interactive swipe-down dismissal both report a cancel to Rust and to the waiting flow; for Kick, the sheet's close is reported to Rust directly instead of polled)
- [~] The Android back button does NOT close the overlay: it is routed to the app underneath (`__SN_BACK__`) (k:MainActivity.kt:L729-L740) (iPhone: no back button; a swipe-down on the sheet cancels the sign-in, which is the better behavior)
- [ ] Every flow that opens the surface closes it in its finally path, so a failed or expired flow never leaves it covering the app (`invoke close_mobile_login`) (s:stores/AppStore.ts:L3216-L3225, L3255-L3264; m:screens/RewardsScreen.tsx:L375-L377; d:components/SetupWizard.tsx:L410-L419) (iPhone: the coordinator dismisses the sheet on every terminal outcome)

## 4. Twitch main sign-in

### 4.1 Signed-out landing

- [ ] When setup is complete but Twitch is signed out (sign-out, token expiry, forced re-auth), the whole app shows a centred landing instead of the tabs; first run shows the setup wizard instead, which runs the same Twitch sign-in as one of its steps (m:MobileApp.tsx:L117-L141) (iPhone: root view switches on the Rust auth state; first-run wizard is owned by whichever area holds onboarding, cross-ref system.md)
- [ ] The signed-out landing (title, line, the "Sign in with Twitch" / "Sign in again" / "Signing in…" button and the re-sign-in reason copy) follows shell-design.md § 4; this area owns what the button starts (m:MobileOnboarding.tsx:L1-L43) (m:MobileApp.tsx:L136-L141)

### 4.2 Device code + in-app activation

- [ ] Sign-in starts a single Twitch device-code flow (`invoke twitch_login` returns the verification URI and user code from the same flow the Rust poller waits on) (s:stores/AppStore.ts:L3161-L3173; r:commands/twitch.rs:L20-L32; r:services/twitch_service.rs:L442-L462)
- [ ] An info toast "Enter code {CODE} at twitch.tv/activate" appears as the flow starts (on Android it lands under the overlay) (s:stores/AppStore.ts:L3175-L3176) (iPhone: show it only on the fallback path, section 4.4; Brandon may prefer parity)
- [ ] The activation page (`https://www.twitch.tv/activate?device-code=...`) opens in the sign-in sheet titled "Sign in to Twitch"; the user signs in to Twitch there and approves; the spinner on the landing stops once the sheet is up (s:stores/AppStore.ts:L3178-L3191; r:twitch_login_plugin.rs:L74-L96) (iPhone: sheet with the URL Rust returned; the iPhone app decides "mobile" by platform, not a UA regex)
- [ ] If Twitch's mobile site lands on the bare home page mid sign-in (under an "Open in App" prompt), the sheet sends it back to the activation URL once; a second arrival at home is left alone (k:TwitchLoginPlugin.kt:L675-L694) (iPhone: same one-shot bounce in `decidePolicyFor`/`didCommit`)
- [ ] The sheet closes itself the moment approval lands, without waiting for the next poll tick: the destination is read off the `redirect_uri` of the `auth.twitch.tv/authorize` hop (today `www.twitch.tv/settings/connections`, also the fallback when the authorize hop was skipped); not done while a storage watch is active (7TV also passes through authorize) (k:TwitchLoginPlugin.kt:L271-L327, L707-L735) (iPhone: same host+path match in the navigation delegate)
- [ ] Rust polls the device token on Twitch's advertised interval until the code expires, stores the token in the primary slot, clears any owed re-sign-in reason, resets the web-session cache, then emits `twitch-login-complete`; storage or poll failure emits `twitch-login-error` with the message (r:services/twitch_service.rs:L464-L556)
- [ ] `listen twitch-login-complete`: close the sheet, re-read auth status, toast "Login successful! You are now authenticated with Twitch." (success), load followed streams, clear the loading and code state (s:stores/AppStore.ts:L3213-L3246) (iPhone: same sequence driven by the HostSink event; `focus_window` is desktop only and skipped)
- [ ] `listen twitch-login-error`: toast "Login failed: {message}" (error), clear loading and code state, close the sheet (s:stores/AppStore.ts:L3249-L3268)
- [ ] Closing the sheet before approving stops the spinner and clears the code; the Rust device-code poll is left to expire on its own (no command calls it off) (s:stores/AppStore.ts:L3270-L3288) (iPhone: same; consider a Rust cancel command, see moves file)
- [ ] Starting the flow failing outright: toast "Login failed: {message}. Please try again." (error) (s:stores/AppStore.ts:L3290-L3295)

### 4.3 The Twitch web session (cookies)

- [ ] After the in-app sign-in, the twitch.tv `auth-token` and `unique_id` cookies in the sign-in browser are the app's Twitch web session, used for playback entitlement (Turbo, subs), GQL and Recap; Rust reads them only when asked and caches a found session for 3600 s and an empty read for 5 s (r:services/twitch_auth_service.rs:L48-L62, L154-L250, L559-L602) (iPhone: Swift feeds the jar to Rust via `set_web_cookies`; Rust's iOS arm of `harvest` reads that jar; never the main webview)
- [ ] Sign-out flags the web session as logged out so reads return "not logged in" until the next sign-in or account change (r:services/twitch_auth_service.rs:L190-L204)
- [ ] No feature asks for an extra sign-in for data Twitch serves publicly, and private data rides this existing in-app twitch.tv session silently; a feature that cannot get it skips quietly instead of prompting (pref: no-extra-signin-for-public-data.md) (iPhone: never add a sign-in step beyond the ones in this file)

### 4.4 Fallback: sign-in by code

- [ ] If the in-app sheet cannot open, the app shows a code panel instead: dimmed blurred backdrop (`rgba(0,0,0,0.72)`, 8 px blur), panel 88% wide up to 420 px, 30/24 px padding, 20 px radius, `rgba(24,24,27,0.94)` (s:stores/AppStore.ts:L3188-L3191; d:components/DeviceLoginOverlay.tsx:L37-L84; m:MobileApp.tsx:L180) (iPhone: a `.sheet` with `.presentationDetents([.medium])`)
- [ ] Panel copy: title "Log in to Twitch" (19 px bold); line "Open the link below and enter this code to authorize StreamNook." (13.5 px, 55% white); the code in a 34 px monospaced bold box with 6 px letter spacing; hint "Tap the code to copy", "Copied" for 1500 ms after a tap (d:components/DeviceLoginOverlay.tsx:L86-L101, L26-L34) (iPhone: code in `.monospaced()` with `UIPasteboard` copy and `.sensoryFeedback(.success)`)
- [ ] Primary button "Open {verification URI without scheme}" in `#9147ff`, 12 px radius, 15 px semibold; it opens the page in the system browser and does nothing visible if none exists (d:components/DeviceLoginOverlay.tsx:L19-L25, L103-L107) (iPhone: `UIApplication.shared.open(url)`; note this path cannot capture the web session, so Drops/Subscribe/7TV re-mint will need the sheet later)
- [ ] Status line with a pulsing 8 px `#9147ff` dot (1.4 s ease-in-out, opacity 0.35 to 1) and "Waiting for you to authorize…"; "Cancel" text button clears the panel and the loading state (d:components/DeviceLoginOverlay.tsx:L109-L131) (iPhone: `ProgressView` or a pulsing dot with `.symbolEffect(.pulse)`, `Button("Cancel")`)
- [ ] Approving the code on any device completes the sign-in through the same `twitch-login-complete` event, which also clears this panel (d:components/DeviceLoginOverlay.tsx:L7-L13; s:stores/AppStore.ts:L3234)

## 5. Drops sign-in (the redirect grant)

- [ ] The flow is owned here; the prompt ("Twitch Drops" card, "Connect drops to track campaign progress and earn while you watch.", "Connect drops" / "Approve on Twitch…" button and the error line) lives on the Rewards screen, see rewards-badges-drops.md (m:screens/RewardsScreen.tsx:L594-L616)
- [ ] Twitch closed the device-code grant on the drops client, so Drops uses the implicit grant: `invoke start_drops_login` returns `https://id.twitch.tv/oauth2/authorize?client_id=<drops client>&response_type=token&redirect_uri=https%3A%2F%2Fwww.twitch.tv%2F&scope=`; never the device flow, never the Android-TV client id (m:screens/RewardsScreen.tsx:L331-L340; r:commands/drops.rs:L741-L743; r:services/drops_auth_service.rs:L105-L111; memory twitch-android-client-id-revoked.md)
- [ ] The authorize page opens in the sign-in sheet titled "Connect drops"; because the sheet already holds the Twitch session from the main sign-in, an account that approved before sees no consent screen at all and the sheet closes on its own almost immediately (m:screens/RewardsScreen.tsx:L331-L357)
- [ ] The listener for the outcome is attached BEFORE the sheet opens, so an instant redirect is never missed (m:screens/RewardsScreen.tsx:L341-L357)
- [ ] Capture: a navigation to `https://www.twitch.tv/...#...access_token=<token>...` is cancelled, the token is held natively, the sheet closes, and the app is told only that it landed (`sn:drops-redirect`); the token never travels through page code, an event payload, a URL bar or a log (k:TwitchLoginPlugin.kt:L597-L637; k:MainActivity.kt:L122-L131) (iPhone: `decidePolicyFor` sees the `www.twitch.tv` URL with an `access_token` fragment, returns `.cancel`, and Swift passes the URL straight to Rust, whose `access_token_from_redirect` parses it, r:services/drops_auth_service.rs:L113-L125; also check `didCommit`/`didStartProvisionalNavigation` since a fragment landing can skip the policy call, k:TwitchLoginPlugin.kt:L309-L311)
- [ ] Rust stores it as the drops credential (`invoke finish_mobile_drops_login`; empty token fails with "Twitch did not hand back a drops credential") (r:twitch_login_plugin.rs:L108-L128) (iPhone: an iOS arm that takes the redirect URL or token from Swift, since there is no Kotlin holder)
- [ ] Success: toast "Drops connected!" (success), the panel flips to connected and the inventory loads (m:screens/RewardsScreen.tsx:L362-L367)
- [ ] Closing the sheet first, or 5 min passing with no redirect: error line "Sign-in was closed before Twitch approved drops." (m:screens/RewardsScreen.tsx:L341-L360)
- [ ] Any other failure: the backend's own reason is shown in the error line and as toast "Drops connection failed: {reason}" (error) (m:screens/RewardsScreen.tsx:L368-L374)
- [ ] The sheet is always closed afterwards, and the button leaves its busy state (m:screens/RewardsScreen.tsx:L375-L378)
- [ ] At launch Rust validates the stored drops token once (`invoke validate_drops_token`); a 401 deletes it so the app honestly shows "not connected"; offline never clears it (m:boot/useMobileBoot.ts:L84-L107) (iPhone: same call at launch, ideally moved into Rust boot, see moves file)
- [ ] There is no separate Drops sign-out on the phone: the drops credential is cleared by a Twitch sign-out or account switch and by a rejected token at launch (r:commands/accounts.rs:L119-L122, L145-L151) (iPhone: same; no extra control)
- [ ] Setup wizard Drops step on the phone opens the same authorize URL in the sign-in sheet and waits for `drops-login-complete` / `drops-login-error` or a cancel (d:components/SetupWizard.tsx:L200-L211, L313-L415). Android divergence for Brandon: those two events are only emitted by the desktop overlay (r:commands/twitch.rs:L928-L957) and nothing in the wizard calls `finish_mobile_drops_login` after `sn:drops-redirect`, so on Android the wizard's Drops step appears to never finish after approval (iPhone: make the wizard use the same Rust-owned Drops flow as Rewards)

## 6. Kick sign-in

- [ ] One action, "Sign in with Kick", obtains both Kick credentials the user never hears about: the kick.com site session (for the follow list) and the OAuth token (chat, moderation); no user-facing string says "import" or "sync" (s:stores/platformAccountStore.ts:L30-L35, L268-L273; r:services/providers/kick_account.rs:L416-L432)
- [ ] It runs `invoke kick_account_sync {interactive: true}`, which drives the sign-in sheet from Rust (open, poll, collect, close), titled "Sign in to Kick" (s:stores/followsStore.ts:L178-L181; r:commands/provider_browse.rs:L342-L357; r:services/providers/kick_account.rs:L567-L580) (iPhone: Rust asks the Swift host to present the sheet through `host_request`; today the iOS arm is a stub that errors "Kick sign-in is not available on this platform", r:services/providers/kick_account.rs:L670-L673)
- [ ] Leg 1, site login first (consent at id.kick.com does not leave kick.com cookies): the sheet opens `https://kick.com/login` (r:services/providers/kick_account.rs:L567-L580)
- [ ] Kick's mobile site ignores /login and shows its home page with the form closed, so on that page the sheet presses Kick's own Log In button (`button[data-testid="login"]`) every 500 ms, up to 20 tries, stopping as soon as a password field exists (k:TwitchLoginPlugin.kt:L137-L148, L257-L269) (iPhone: a `WKUserScript` at document end, or `evaluateJavaScript` in `didFinish`, only when host+path is `kick.com/login`)
- [ ] Rust polls the sign-in browser's kick.com cookies every 2 s for up to 150 polls (5 min); Cloudflare clearance cookies (`cf_clearance`, `__cf_bm`, `_cfuvid`) are dropped before replay; each poll tries to read the follow list with the session (r:services/providers/kick_account.rs:L582-L611, L767-L775) (iPhone: Swift pushes the kick.com cookies to Rust via `set_web_cookies` on every navigation finish and on a 2 s timer while this flow runs)
- [ ] Signed in but the follow list refuses three times in a row: stop waiting and continue to consent rather than leave the person on kick.com for 5 min (r:services/providers/kick_account.rs:L584-L609)
- [ ] The follow list is read 20 pages at most with the percent-decoded `session_token` as the bearer plus the cookie header and `X-XSRF-TOKEN` (r:services/providers/kick_account.rs:L787-L870)
- [ ] Leg 2, consent in the same sheet: Rust builds the PKCE authorize URL (`begin_auth`) and re-navigates the sheet to it; for an already signed-in user this is usually one tap (r:services/providers/kick_account.rs:L616-L621; r:services/kick_auth_service.rs:L385-L433)
- [ ] The consent redirect `http://localhost:3000/callback?code=...&state=...` is cancelled (nothing need listen on port 3000) and handed to Rust once; the sheet closes; Rust checks `state` and exchanges the code; a binding failure on port 3000 never blocks the in-app flow (k:TwitchLoginPlugin.kt:L639-L666; r:services/kick_auth_service.rs:L328-L358, L401-L413, L437-L465; memory kick-signin-port-3000.md) (iPhone: `decidePolicyFor` matches the prefix `http://localhost:3000/callback?` (also `http://127.0.0.1:3000/callback`), returns `.cancel`, and Swift calls Rust's `capture_redirect` directly; no 400 ms relay polling; ATS never sees the request because it is cancelled before load)
- [ ] The redirect is stored before the sheet closes, and Rust checks "closed" before "redirect", so a capture is never lost to the race (k:TwitchLoginPlugin.kt:L643-L645; r:services/providers/kick_account.rs:L621-L641) (iPhone: deliver the redirect to Rust synchronously before dismissing)
- [ ] Consent waits at most 180 s ("Kick login timed out (no redirect received)"); a state mismatch aborts ("Kick login state mismatch, aborting") (r:services/kick_auth_service.rs:L447-L463)
- [ ] Closing the sheet during leg 1 fails the connect with "Sign-in was cancelled"; closing during leg 2 ends the wait quietly (r:services/providers/kick_account.rs:L588-L592, L643-L649)
- [ ] The sheet failing to open: "The Kick sign-in window could not open." (r:services/providers/kick_account.rs:L577-L580)
- [ ] No follow list was read: "Signed in, but couldn't read your Kick follows" (the OAuth leg may still have succeeded, so the row can read connected) (r:services/providers/kick_account.rs:L662-L667)
- [ ] An import that ended without an ok status: "Sign-in did not finish on kick.com." (r:commands/provider_browse.rs:L353-L358)
- [ ] Outcome copy on the row and as a toast: the backend sentence with its first letter capitalised; any message mentioning "cancel" reads "Sign-in was cancelled"; an empty, "undefined" or over-160-character message reads "Sign-in was not completed"; the row shows it for 6000 ms unless something replaced it (s:stores/platformAccountStore.ts:L82-L116, L296-L304) (iPhone: Rust should return the user-facing sentence, see moves file)
- [ ] After any connect attempt the roster and the follow list are re-read and the Following tab repaints without waiting (s:stores/platformAccountStore.ts:L305-L309)
- [ ] Edge to keep: if leg 1 imported follows but consent was abandoned, the follows exist but the row reads "Not signed in" (connected means OAuth token) (r:services/account_roster.rs:L160-L163; r:services/providers/kick_account.rs:L643-L667)
- [ ] Other surfaces that start this same flow: the Kick chat composer's "Sign in to Kick to chat" pill ("Waiting for you to sign in" while busy), owned by chat-input.md; and the "Reconnect" toast action in 2.5 (m:chat/MobileChatInput.tsx:L112, L438-L446)

## 7. 7TV sign-in and the 30-day session

### 7.1 Visible sign-in

- [ ] The 7TV row ("7TV" + "Signed in"/"Not signed in" + "Sign in"/"Sign out"/"Signing in…") and the "tap a tile to sign in first" entry live on the Cosmetics screen, owned by profile-cosmetics.md; this file owns the flow they call (m:screens/CosmeticsScreen.tsx:L311-L343, L574-L589)
- [ ] The sign-in page URL comes from Rust (`invoke get_seventv_login_url`, `https://api.7tv.app/v4/auth/login?platform=twitch`) and opens in the sign-in sheet titled "Sign in to 7TV" (m:cosmetics/sevenTvConnect.ts:L83-L92; r:services/seventv_auth_service.rs:L293-L295)
- [ ] 7TV's sign-in round-trips through Twitch (using the sheet's Twitch session) and lands on 7tv.app, which writes its session token into its own localStorage under `7tv-token`; there is no redirect to intercept, so the sheet reads that key from the page (m:cosmetics/sevenTvConnect.ts:L1-L16; k:TwitchLoginPlugin.kt:L49-L63)
- [ ] The waiting listener is registered BEFORE the sheet opens so an already-authorised sign-in that completes instantly is not missed (m:cosmetics/sevenTvConnect.ts:L85-L87)
- [ ] Storage watch: every 700 ms the sheet evaluates `localStorage.getItem('7tv-token')` on the current page; null is normal until the flow lands on 7tv.app (k:TwitchLoginPlugin.kt:L36-L37, L746-L792) (iPhone: `evaluateJavaScript` on a 0.7 s timer while the sheet is open)
- [ ] A value of 50 characters or fewer is a placeholder and is ignored (k:TwitchLoginPlugin.kt:L42-L43, L779)
- [ ] A JWT whose `exp` is past or within 60 s is ignored (last month's token still sitting in 7tv.app storage) and the watch keeps polling for the new one (k:TwitchLoginPlugin.kt:L770-L778, L794-L812)
- [ ] A good token stops the watch, is handed to the app (`sn:login-storage` with `{key, value}` as JSON), and the sheet closes (k:TwitchLoginPlugin.kt:L779-L783; k:MainActivity.kt:L133-L149) (iPhone: Swift hands the token straight to Rust, never through a page)
- [ ] The 7TV user id is the JWT `sub` (or `user_id`) claim; a token that does not decode to one is rejected ("token captured but carried no user id" in the log) and the sign-in returns false (m:cosmetics/sevenTvConnect.ts:L32-L49, L97-L101) (iPhone: Rust derives it, see moves file)
- [ ] Stored with `invoke store_seventv_token {accessToken, userId, twitchId}` where `twitchId` is the signed-in Twitch user id (m:cosmetics/sevenTvConnect.ts:L103-L108; r:commands/seventv_cosmetics.rs:L279-L288)
- [ ] The visible watch gives up after 5 min (both the sheet and the app side), and a close/cancel resolves "not signed in" at once (m:cosmetics/sevenTvConnect.ts:L18-L19, L51-L74; k:TwitchLoginPlugin.kt:L39-L40)
- [ ] Outcome toasts on the Cosmetics screen: captured but status not authenticated "7TV did not finish signing you in. Try again." (error); a thrown failure "Could not sign in to 7TV." (error); no 7TV id "7TV did not identify your account." (error); sign-out failure "Could not sign out of 7TV." (error); an expired session on a write "Your 7TV sign-in expired. Tap again to sign back in." (error) (m:screens/CosmeticsScreen.tsx:L316-L341, L377-L385, L392-L401)
- [ ] 7TV sign-out is `invoke logout_seventv`; it removes the stored token but leaves 7tv.app's own storage in the sign-in browser (m:screens/CosmeticsScreen.tsx:L377-L385; r:commands/seventv_cosmetics.rs:L296-L303) (iPhone: same; Brandon may want the 7tv.app storage cleared too)

### 7.2 Status and the 30-day session

- [ ] 7TV tokens are 30-day JWTs; Rust reports `{is_authenticated, user_id, twitch_id, expires_at}` from the JWT `exp` with no network; an expired token is kept (not deleted) so a re-mint can be attempted; only an explicit sign-out or a rejected request clears it (m:cosmetics/sevenTvConnect.ts:L21-L27, L111-L113; r:services/seventv_auth_service.rs:L188-L226; memory seventv-30-day-session-and-mobile-resume.md)
- [ ] At launch, 8 s after boot and only when signed in to Twitch, the app checks whether to re-mint: never when no token is stored; never within 12 h of the previous attempt; otherwise when the token expires within 7 days or already has (s:services/sevenTvSession.ts:L11-L42; m:boot/useMobileBoot.ts:L127-L138)
- [ ] The attempt time is stamped (localStorage `sn-7tv-refresh-attempt`) BEFORE the attempt, so a crash mid-attempt still waits 12 h (s:services/sevenTvSession.ts:L15, L44-L60, L78-L79) (iPhone: Rust-owned stamp, see moves file)
- [ ] Silent re-mint: the sign-in surface runs hidden (no bar, never drawn, never touchable, page fills the frame) at the 7TV login URL; while the Twitch session in the jar is alive, 7TV completes on its own and the new token is captured exactly like a visible sign-in (m:cosmetics/sevenTvConnect.ts:L115-L150; k:TwitchLoginPlugin.kt:L68-L79, L369-L378) (iPhone: an offscreen `WKWebView` on the same data store, attached to the window at alpha 0 and user-interaction disabled so timers and storage run normally; never presented)
- [ ] The hidden surface gives itself 30 s, then dismisses itself and reports a cancel; the app side waits 35 s and closes it on its own deadline (`invoke close_mobile_login`) (k:TwitchLoginPlugin.kt:L99-L100, L749-L763; m:cosmetics/sevenTvConnect.ts:L29-L30, L127-L137)
- [ ] A successful re-mint stores the token with the same `store_seventv_token` call; outcomes are logged only ("[7TV] session re-minted silently" / "[7TV] silent re-mint did not complete"), never shown (s:services/sevenTvSession.ts:L80-L88; m:cosmetics/sevenTvConnect.ts:L139-L149)
- [ ] Desktop parity: desktop re-mints the primary through a hidden window at boot with the same scheduler and a ~12 s capture budget (s:App.tsx:L953; r:commands/seventv_cosmetics.rs:L638-L682) (iPhone: same scheduler)

## 8. Subscribe checkout

- [ ] Subscribe (and the `/gift` command) opens the platform's own checkout in the sign-in sheet as the signed-in account: Twitch `https://www.twitch.tv/subs/{login}`, Kick `https://kick.com/{slug}/subscribe`; title "Subscribe to {name}" (or "Gift subs to {login}") (r:commands/twitch.rs:L1322-L1365; s:hooks/useChannelSocial.ts:L413-L429; s:utils/extendedCommands.ts:L385-L402) (iPhone: Rust returns the URL (today `open_subscribe_window` is not registered on iOS, r:lib.rs:L1609-L1611) and Swift presents the sheet; buttons live in watch/composer areas, cross-ref)
- [ ] `/gift` posts a local system line: "Opened {login}'s sub page. Choose \"Gift a sub\" and pick {N}. Payment happens on Twitch." (or without the count); in your own channel it toasts "You can't gift subs in your own channel" (info) instead (s:utils/extendedCommands.ts:L385-L402)
- [ ] When chat reports the user's own subscription in that channel while the sheet is open, toast "Subscription successful! Thank you for subscribing!" (or "Subscription successful! Gift sent!" for a gift) (success) and close the sheet (s:hooks/useChannelSocial.ts:L375-L407)
- [ ] Payment happens on the platform's page inside the sheet; StreamNook never sees payment details (r:commands/twitch.rs:L1322-L1328) (iPhone: same; internal TestFlight only, so no App Store payment-rule review applies)

## 9. Credentials at rest

- [ ] Every credential (Twitch primary token, drops token, 7TV token, Kick/YouTube/TikTok) is sealed by the token vault (AES-256-GCM files); on phones the key is a file `.token_vault_key` in the app sandbox, and the credential-storage answer is "the app's private storage" (r:services/token_vault/mod.rs:L13, L63, L117-L127; memory token-vault-dev-migrates-real-tokens.md) (iPhone: move the vault key to the Keychain, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, per the iOS plan phase 2; Keychain items survive app deletion, so a fresh install must not resurrect old tokens)
- [ ] The page never holds a Twitch token; all Helix/GQL calls are Rust's (memory rust-owned-chat-state.md) (iPhone: Swift never holds tokens either, except the transient hand-off of a captured drops/7TV token to Rust)
- [ ] App data lives under one base on phones (`app_paths::set_base`); sign-in state must not live in a purgeable cache folder (memory account-registry-empty-trap.md) (iPhone: `Library/Application Support`, not `Library/Caches`)

## 10. Signed-out states elsewhere (cross-references)

- [ ] Whole app signed out of Twitch: the landing in 4.1 (m:MobileApp.tsx:L136-L141)
- [ ] Drops not connected: Rewards prompt, rewards-badges-drops.md (m:screens/RewardsScreen.tsx:L594-L616)
- [ ] Drops not connected on Cosmetics: "Sign in to Drops to choose which badge you wear." and the write error "Sign in to Drops to change your Twitch badge.", profile-cosmetics.md (m:screens/CosmeticsScreen.tsx:L364-L369, L726-L730)
- [ ] Kick not connected in a Kick chat: composer pill, chat-input.md (m:chat/MobileChatInput.tsx:L438-L446)
- [ ] 7TV not connected: Cosmetics 7TV row, profile-cosmetics.md (m:screens/CosmeticsScreen.tsx:L574-L589)

## Decisions for Brandon

Not parity lines: open questions found while reading, each to be settled before its screen is built.

- Twitch sign-out: on Android the You tab's sign-out never clears the sign-in browser, so the old twitch.tv, kick.com and 7tv.app sessions survive and the next sign-in can open already signed in as the previous person. This contract has the iPhone clear it on every sign-out (Rust does it, rust-moves.md). Confirm.
- Setup wizard Drops step: on Android it waits for `drops-login-complete`, which only desktop sends, so it likely never finishes after approval. The iPhone wires it to the redirect-grant finish. Confirm, and fix Android in the same change?
- 7TV sign-out: also clear 7tv.app's stored token inside the sign-in browser?
- Keep the "Enter code … at twitch.tv/activate" toast that shows under the sign-in sheet, or show the code only in the fallback panel?
- Kick: closing the sheet after the follow import but before the app consent keeps the follows while the row reads "Not signed in". Acceptable?

## Beyond Android (backlog, Brandon picks)

- [ ] (backlog, not parity) Switch between several Twitch accounts from the You tab (the desktop already keeps linked accounts with "Make main" and "Send chat as"); one tap moves the main slot and reconnects chat
- [ ] (backlog, not parity) Check at launch that the twitch.tv web session belongs to the same Twitch account as the app's token, and re-run the sign-in sheet if they differ, so drops and points never credit the wrong account
- [ ] (backlog, not parity) A clear message in the sign-in sheet when the user taps "Continue with Google" or "Continue with Apple" on Twitch's page, since those providers refuse embedded browsers
- [ ] (backlog, not parity) Password and one-time-code AutoFill in the sign-in sheet (QuickType bar from Passwords), verified on device for twitch.tv and kick.com
- [ ] (backlog, not parity) On the first launch after a reinstall, wipe Keychain-held credentials left by the previous install before reading any
- [ ] (backlog, not parity) Optional Face ID confirmation for signing out or opening Subscribe checkout, in place of the two-tap pill
- [ ] (backlog, not parity) An App Intent "Open StreamNook on Rewards/Following" for Shortcuts that respects the start-tab choice
