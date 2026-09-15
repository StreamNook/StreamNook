//! macOS configuration invariants.
//!
//! HARD RULE: `tauri.macos.conf.json`'s window object must match the base
//! `tauri.conf.json` window object on every key except the intentional
//! macOS-only ones.
//!
//! Tauri merges a platform config into the base one using **JSON Merge Patch
//! (RFC 7396), which REPLACES arrays wholesale rather than merging them**.
//! `app.windows` is an array. So the macOS file cannot override just
//! `decorations` — it has to restate the entire window object, every property,
//! or the omitted ones silently revert to Tauri's built-in defaults rather than
//! to StreamNook's values.
//!
//! That makes the two files a duplicated pair that no compiler checks. Change
//! `minWidth` in the base config and macOS keeps the old one; add a window
//! property and macOS silently runs without it. The failure is invisible on the
//! developer's machine, because the developer is on Windows and the base config
//! is the one that loads.
//!
//! This is the same class of drift `acl_parity.rs` exists to catch, and it gets
//! the same treatment: a test that reads both files and fails the suite the
//! moment they disagree.
//!
//! When a difference is genuinely intended, add the key to `MACOS_ONLY_KEYS`
//! below with a comment explaining why — that list is the documentation of what
//! macOS deliberately does differently.

use serde_json::Value;

const BASE_JSON: &str = include_str!("../tauri.conf.json");
const MACOS_JSON: &str = include_str!("../tauri.macos.conf.json");

/// Keys the macOS window object is allowed to differ on, or to introduce.
///
/// All four exist to give macOS its native window feel back. `decorations:
/// false` (the Windows/Linux value) is what produced BOTH the missing traffic
/// lights and the hard square corners, because macOS only rounds *decorated*
/// windows — no amount of CSS `border-radius` can round an undecorated one.
const MACOS_ONLY_KEYS: &[&str] = &[
    // true on macOS: restores AppKit rounding, drag, snap and window alignment.
    "decorations",
    // "Overlay": webview content still reaches the top edge, but real traffic
    // lights float over it instead of us drawing a fake cluster.
    "titleBarStyle",
    // The custom title bar renders the app name itself; the native one would
    // double it up.
    "hiddenTitle",
    // Overlay leaves the lights centred in a native ~28px title bar, so against
    // our 40px bar they sit high and read as off-kilter next to the other
    // title-bar buttons. See the derivation in the second test: this value
    // resizes the container they centre in, it is not a top offset.
    "trafficLightPosition",
];

fn window_object(raw: &str, which: &str) -> serde_json::Map<String, Value> {
    let root: Value =
        serde_json::from_str(raw).unwrap_or_else(|e| panic!("{which} is not valid JSON: {e}"));
    let windows = root
        .get("app")
        .and_then(|a| a.get("windows"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{which} has no app.windows array"));
    let main = windows
        .iter()
        .find(|w| w.get("label").and_then(Value::as_str) == Some("main"))
        .unwrap_or_else(|| panic!("{which} has no window labelled \"main\""));
    main.as_object()
        .expect("window entry is not an object")
        .clone()
}

#[test]
fn macos_window_config_matches_base_except_for_intentional_keys() {
    let base = window_object(BASE_JSON, "tauri.conf.json");
    let macos = window_object(MACOS_JSON, "tauri.macos.conf.json");

    let mut problems: Vec<String> = Vec::new();

    // Every base key must be restated on macOS with the same value, unless it
    // is one we deliberately override.
    for (key, base_value) in &base {
        if MACOS_ONLY_KEYS.contains(&key.as_str()) {
            continue;
        }
        match macos.get(key) {
            None => problems.push(format!(
                "`{key}` is in tauri.conf.json but MISSING from tauri.macos.conf.json \
                 (RFC 7396 replaces the whole array, so macOS would fall back to \
                 Tauri's default of {base_value}, not to this value)"
            )),
            Some(macos_value) if macos_value != base_value => problems.push(format!(
                "`{key}` differs: base = {base_value}, macOS = {macos_value}. \
                 If that is intended, add `{key}` to MACOS_ONLY_KEYS with a reason."
            )),
            Some(_) => {}
        }
    }

    // And macOS must not invent keys that are neither in the base nor declared.
    for key in macos.keys() {
        if !base.contains_key(key) && !MACOS_ONLY_KEYS.contains(&key.as_str()) {
            problems.push(format!(
                "`{key}` exists only in tauri.macos.conf.json and is not in \
                 MACOS_ONLY_KEYS. Add it there with a reason, or remove it."
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "tauri.macos.conf.json has drifted from tauri.conf.json:\n  - {}",
        problems.join("\n  - ")
    );
}

/// The whole point of the macOS config is the native-feel window. If these go
/// missing the app still builds and runs, but silently reverts to the square,
/// undecorated, traffic-light-less window the port started from.
#[test]
fn macos_window_actually_declares_the_native_chrome() {
    let macos = window_object(MACOS_JSON, "tauri.macos.conf.json");

    assert_eq!(
        macos.get("decorations").and_then(Value::as_bool),
        Some(true),
        "macOS needs `decorations: true`; it is what gives the window its native \
         rounded corners (macOS does not round undecorated windows)"
    );
    assert_eq!(
        macos.get("titleBarStyle").and_then(Value::as_str),
        Some("Overlay"),
        "macOS needs `titleBarStyle: \"Overlay\"` so content still reaches the \
         top edge while AppKit draws the real traffic lights over it"
    );

    let pos = macos
        .get("trafficLightPosition")
        .expect("macOS needs `trafficLightPosition` or the lights sit 6px high");
    let y = pos.get("y").and_then(Value::as_f64).expect("no y position");

    // `y` is NOT an offset from the top of the window. That is the obvious
    // reading and it is wrong; assuming it cost a wasted build. tao's
    // `inset_traffic_lights` (tao-0.35.3, platform_impl/macos/view.rs:1152) does:
    //
    //     title_bar_frame_height = close_button.frame.height + y
    //
    // then pins that container to the top of the window and leaves the buttons
    // centred inside it. So `y` sizes the BOX the lights centre in. Setting it
    // below the native ~28px therefore moves the lights UP, not down.
    //
    // The clean-looking derivation (container == title bar, so
    // y = 40 - 12 = 28) does NOT land correctly, because the visible coloured
    // circle is smaller than, and offset within, the NSWindowButton frame that
    // tao measures. The frame metrics are not knowable from the config, so this
    // value is CALIBRATED against the real window rather than computed:
    //
    //     y = 14  ->  sat ~5px high
    //     y = 28  ->  sat ~2px low
    //     y = 24  ->  still slightly low
    //
    // Those fit `visual_centre = C + y/2` (the lights centre in the container,
    // so 2 units of `y` is 1px of travel), which is what the successive nudges
    // 28 -> 24 -> 22 were stepping along.
    //
    // If the title bar height ever leaves 40px, RE-CALIBRATE by eye; do not try
    // to compute it. Nudge in steps of 2 for 1px at a time.
    //
    // The horizontal partner is `MAC_TRAFFIC_LIGHT_INSET_PX` in
    // `src/utils/platform.ts`: `x` here says where AppKit draws the lights, and
    // that constant says how much leading space the React title bar keeps clear
    // for them. 78 clipped the About/Penrose logo; it is 92 now. Change them together.
    const EXPECTED_Y: f64 = 22.0;
    assert!(
        (y - EXPECTED_Y).abs() < 0.01,
        "trafficLightPosition.y is {y}, expected {EXPECTED_Y}, which is \
         calibrated against a 40px title bar (see the comment above: 2 units of \
         y == 1px of travel). If TitleBar.tsx's `h-[40px]` changed, re-calibrate \
         this and MAC_TRAFFIC_LIGHT_INSET_PX together."
    );
}

/// Deep links register **declaratively** on macOS: the bundler copies
/// `plugins.deep-link.desktop.schemes` into the .app's Info.plist as
/// `CFBundleURLSchemes`. There is no runtime call to fail loudly if the config
/// key goes missing — `streamnook://` share links would simply stop opening the
/// app, with nothing in any log to say why.
///
/// Windows is unaffected either way, because it registers at runtime via
/// `deep_link().register_all()`, so only a test can protect the macOS path.
#[test]
fn deep_link_scheme_is_declared_for_the_bundler() {
    let root: Value = serde_json::from_str(BASE_JSON).expect("tauri.conf.json parses");
    let schemes = root
        .get("plugins")
        .and_then(|p| p.get("deep-link"))
        .and_then(|d| d.get("desktop"))
        .and_then(|d| d.get("schemes"))
        .and_then(Value::as_array)
        .expect(
            "plugins.deep-link.desktop.schemes is missing; macOS deep links are              generated from it at bundle time and would silently stop working",
        );

    assert!(
        schemes
            .iter()
            .any(|s| s.as_str() == Some("streamnook")),
        "the `streamnook` scheme must stay declared; found {schemes:?}"
    );
}

/// The main window is recreated at runtime in two places (Rust
/// `show_main_window` after Go Live or a tray click, JS `ensureMainWindow.ts`
/// from a popout), and neither goes through the config, so neither inherits
/// the macOS chrome above. The first macOS build shipped both with the
/// Windows values: the recreated window came back with no traffic lights,
/// and since the React title bar draws no minimize/close cluster on macOS,
/// no way to close or minimize it at all. Pin both call sites to the config.
#[test]
fn recreated_main_window_restates_the_macos_chrome() {
    const LIB_RS: &str = include_str!("../src/lib.rs");
    const ENSURE_MAIN_TS: &str = include_str!("../../src/utils/ensureMainWindow.ts");

    let macos = window_object(MACOS_JSON, "tauri.macos.conf.json");
    assert_eq!(macos.get("decorations").and_then(Value::as_bool), Some(true));
    assert_eq!(macos.get("titleBarStyle").and_then(Value::as_str), Some("Overlay"));
    assert_eq!(macos.get("hiddenTitle").and_then(Value::as_bool), Some(true));
    let pos = macos
        .get("trafficLightPosition")
        .and_then(Value::as_object)
        .expect("trafficLightPosition object");
    let x = pos.get("x").and_then(Value::as_f64).expect("trafficLightPosition.x");
    let y = pos.get("y").and_then(Value::as_f64).expect("trafficLightPosition.y");

    let rust_needles = [
        ".decorations(true)".to_string(),
        ".title_bar_style(tauri::TitleBarStyle::Overlay)".to_string(),
        ".hidden_title(true)".to_string(),
        format!(".traffic_light_position(tauri::LogicalPosition::new({x:.1}, {y:.1}))"),
    ];
    for needle in &rust_needles {
        assert!(
            LIB_RS.contains(needle.as_str()),
            "src/lib.rs main_window_chrome must restate `{needle}` from tauri.macos.conf.json"
        );
    }

    let js_needles = [
        "decorations: IS_MAC".to_string(),
        "titleBarStyle: 'overlay'".to_string(),
        "hiddenTitle: true".to_string(),
        format!("trafficLightPosition: new LogicalPosition({x}, {y})"),
    ];
    for needle in &js_needles {
        assert!(
            ENSURE_MAIN_TS.contains(needle.as_str()),
            "src/utils/ensureMainWindow.ts must restate `{needle}` from tauri.macos.conf.json"
        );
    }
}
