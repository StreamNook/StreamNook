//! Native "what song is this" recognition.
//!
//! Fingerprints a short mono 16 kHz PCM clip captured from the player and looks
//! it up against Shazam's catalog. The fingerprinting (algorithm + signature
//! encoder + Hanning table) is vendored from shazamio-core (MIT, see
//! LICENSE-shazamio-core in this directory); the lookup request is ours
//! (shazam.rs). No API key or account is involved, and only the fingerprint
//! (not audio) leaves the machine. The song's links on each music service
//! are found from its ISRC and Apple Music id (links.rs), and its cover's
//! colors are read in the background (cover_tint.rs).

mod algorithm;
pub mod cover_tint;
mod hanning;
pub mod history;
mod links;
mod services;
mod shazam;
mod signature_format;

pub use shazam::{Provider, SongMatch};

use algorithm::SignatureGenerator;

/// How long an identification waits for the song's links. Lookups it cuts
/// short are finished by a refresh right after the song is filed.
const LINKS_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

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

    // Turn Shazam's hub links, ISRC and Apple id into the track's own link on
    // every service the finder reaches, and a search page on the rest. A match
    // with no title names nothing and is dropped by the caller.
    if let Some(song) = result.as_mut().filter(|s| !s.title.trim().is_empty()) {
        let input = links::ResolveInput {
            title: song.title.clone(),
            artist: song.artist.clone(),
            album: song.album.clone(),
            isrc: song.isrc.clone(),
            isrc_from: song.isrc_from,
            apple_id: song.apple_id.clone(),
            known: song.providers.clone(),
        };
        let found = links::resolve(&input, LINKS_BUDGET).await;
        song.links_are_searches = !found.providers.is_empty() && found.providers.iter().all(|p| p.search);
        song.providers = found.providers;
        song.isrc = found.isrc;
        song.isrc_from = found.isrc_from;
        song.links_complete = found.complete;
        // Deezer's album cover when Shazam had none.
        if song.album_art.is_none() {
            song.album_art = found.cover;
        }
    }

    Ok(result)
}
