//! Default chat name colour for a Twitch chatter who never picked one.
//!
//! Twitch sends such users with an EMPTY `color` tag and leaves the client to
//! choose. Twitch's own web chat picks a random palette colour per session, so
//! there is nothing to match exactly; StreamNook needs a rule that is stable
//! across sessions, viewers, the app and the hosted overlay. This follows
//! Chatterino: Twitch's 15-colour palette indexed by the numeric user id.
//!
//! MIRRORED BY HAND in the site: `C:\streamnook.app\src\overlay\defaultNameColor.ts`
//! (the hosted overlay has no Rust). When this changes, change that too and
//! re-run the `cases` vectors below against it in node.

/// Twitch's default name palette, in Twitch's order.
pub const TWITCH_NAME_PALETTE: [&str; 15] = [
    "#FF0000", // Red
    "#0000FF", // Blue
    "#00FF00", // Green
    "#B22222", // FireBrick
    "#FF7F50", // Coral
    "#9ACD32", // YellowGreen
    "#FF4500", // OrangeRed
    "#2E8B57", // SeaGreen
    "#DAA520", // GoldenRod
    "#D2691E", // Chocolate
    "#5F9EA0", // CadetBlue
    "#1E90FF", // DodgerBlue
    "#FF69B4", // HotPink
    "#8A2BE2", // BlueViolet
    "#00FF7F", // SpringGreen
];

/// Pick the palette entry for a chatter. `user_id` is the Twitch numeric id;
/// `fallback_name` (login or display name) only matters when the id is missing
/// or not numeric, which Twitch itself never sends.
pub fn default_name_color(user_id: &str, fallback_name: &str) -> &'static str {
    let seed: u64 = match user_id.trim().parse::<u64>() {
        Ok(n) => n,
        Err(_) => {
            let key = if user_id.trim().is_empty() { fallback_name } else { user_id };
            key.bytes().fold(0u64, |acc, b| acc.wrapping_add(b as u64))
        }
    };
    TWITCH_NAME_PALETTE[(seed % TWITCH_NAME_PALETTE.len() as u64) as usize]
}

/// The tag value as Twitch sent it, or the derived default when the tag is
/// empty. Never changes a colour the user actually set.
pub fn resolve_name_color(tag: Option<&str>, user_id: &str, fallback_name: &str) -> String {
    match tag {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        _ => default_name_color(user_id, fallback_name).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (user_id, fallback_name, expected). Replay these in node against the
    /// site's `defaultNameColor.ts` after touching either side.
    const CASES: &[(&str, &str, &str)] = &[
        ("519930659", "vezix__yt_", "#00FF7F"),  // 519930659 % 15 = 14 SpringGreen
        ("1286638783", "runtzyvl", "#8A2BE2"),   // % 15 = 13 BlueViolet
        ("132477205", "rajdhani", "#5F9EA0"),    // % 15 = 10 CadetBlue
        ("15", "x", "#FF0000"),                  // exact multiple -> index 0
        ("0", "x", "#FF0000"),
        ("14", "x", "#00FF7F"),
        ("18446744073709551615", "x", "#FF0000"), // u64::MAX % 15 = 0
        ("", "abc", "#D2691E"),                  // bytes 97+98+99 = 294 % 15 = 9
        ("not-a-number", "abc", "#B22222"),      // byte sum 1173 % 15 = 3
        ("", "", "#FF0000"),
    ];

    #[test]
    fn vectors() {
        for (id, name, want) in CASES {
            assert_eq!(default_name_color(id, name), *want, "{id:?}/{name:?}");
            println!("VECTOR	{id}	{name}	{want}");
        }
    }

    #[test]
    fn resolve_keeps_a_set_colour_and_fills_an_empty_one() {
        assert_eq!(resolve_name_color(Some("#25B8A7"), "1", "a"), "#25B8A7");
        assert_eq!(resolve_name_color(Some(""), "519930659", "a"), "#00FF7F");
        assert_eq!(resolve_name_color(None, "519930659", "a"), "#00FF7F");
    }
}
