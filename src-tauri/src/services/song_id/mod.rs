//! Native "what song is this" recognition.
//!
//! Fingerprints a short mono 16 kHz PCM clip captured from the player and looks
//! it up against Shazam's catalog. The fingerprinting (algorithm + signature
//! encoder + Hanning table) is vendored from shazamio-core (MIT, see
//! LICENSE-shazamio-core in this directory); the lookup request is ours
//! (shazam.rs). No API key or account is involved, and only the fingerprint
//! (not audio) leaves the machine.

mod algorithm;
mod hanning;
mod odesli;
mod shazam;
mod signature_format;

pub use shazam::{SongMatch};

use algorithm::SignatureGenerator;

/// Identify the song in a mono 16 kHz PCM buffer. Returns `Ok(None)` when
/// nothing matched (talking over the music, or an obscure/unreleased track).
pub async fn identify(samples: Vec<i16>) -> Result<Option<SongMatch>, String> {
    if samples.len() < 128 {
        return Err("Not enough audio captured to identify a song.".to_string());
    }

    // Fingerprinting is a synchronous CPU burst (FFTs over the whole clip), so
    // run it off the async runtime to avoid stalling other tasks.
    let signature =
        tokio::task::spawn_blocking(move || SignatureGenerator::make_signature_from_buffer(samples))
            .await
            .map_err(|e| format!("fingerprinting task failed: {}", e))?;

    let mut result = shazam::recognize(&signature).await?;

    // Turn Shazam's single (sometimes missing) link into a full set of clickable
    // service links via Odesli. Best-effort: failure keeps the Shazam fallback.
    if let Some(song) = result.as_mut() {
        if let Some(seed) = song.seed_url.clone() {
            if let Some(enriched) = odesli::enrich(&seed).await {
                song.song_link = enriched.page_url;
                if !enriched.providers.is_empty() {
                    song.providers = enriched.providers;
                }
                if song.album_art.is_none() {
                    song.album_art = enriched.thumbnail;
                }
            }
        }

        // Guarantee at least some clickable links even when Shazam exposed no
        // real platform URL and Odesli had nothing to resolve.
        if song.providers.is_empty() {
            song.providers = shazam::search_links(&song.artist, &song.title);
        }
    }

    Ok(result)
}
