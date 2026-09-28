//! Frame samples in, a glow colour out. See `services::media_glow`.

use base64::Engine;

use crate::services::media_glow;

/// One downscaled thumbnail, base64 RGBA.
///
/// Base64 rather than a `Vec<u8>`: Tauri's IPC serialises a byte vector as a
/// JSON array of numbers, which turns 576 bytes into roughly 2 KB of text on
/// every sample of every tile. The encoded string is about 770 bytes and costs
/// one allocation to decode.
///
/// This is the STILL path only: a card thumbnail, sampled once when it decodes
/// and then never again. The live path does not come here — a light that tracks
/// the picture has to resample every frame, and sending a frame across IPC that
/// often is the bulk media copy the efficiency standard forbids, so it computes
/// its own colours in the page (see `useMediaGlow`). The two want different
/// answers anyway: a card wants the one colour a stream reads as, which is the
/// modal colour clamped into a legible lightness band; a light wants the mean,
/// and wants to go dark when the picture does.
#[tauri::command]
pub async fn submit_media_frame(
    key: String,
    rgba_b64: String,
    width: usize,
    height: usize,
) -> Result<Option<media_glow::Glow>, String> {
    let rgba = base64::engine::general_purpose::STANDARD
        .decode(rgba_b64.as_bytes())
        .map_err(|e| format!("bad frame sample: {e}"))?;
    Ok(media_glow::submit(&key, &rgba, width, height).await)
}

/// The Linux build's still path: the colour a card should glow, from its
/// thumbnail's URL. Rust fetches the thumbnail, scales it down and chooses the
/// colour (`services::media_glow::sample_url`), cached by URL so every surface
/// showing that stream gets the same answer without a second fetch; the page
/// only sets one CSS variable from the answer. Everywhere else the page keeps
/// the canvas sampler above, whose result is the one users have: Rust's
/// resample of the same thumbnail is not pixel-identical, and a near-tie in
/// the colour's modal bucket can land differently. On Linux it moves the
/// fetch, decode and readback off the page's thread while it is still booting.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub async fn sample_media_glow(url: String) -> Result<Option<media_glow::Glow>, String> {
    Ok(media_glow::sample_url(&url).await)
}

/// The phone has no `image` decoder in its build; a card there keeps the
/// theme accent unless the colour is already cached.
#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub async fn sample_media_glow(url: String) -> Result<Option<media_glow::Glow>, String> {
    Ok(media_glow::cached(&url).await)
}
