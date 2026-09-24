//! Each platform's categories, cached and shared by every surface that shows
//! them: the platform's own Categories tab, and the unified view's "On other
//! platforms" row.
//!
//! Categories change slowly, and building one costs a real fetch (Kick ranks a
//! sample of its live directory, YouTube reads its games page), so a page is
//! kept for `TTL` and served to whoever asks. Each platform has its own slot: a
//! second caller for the same platform waits for the fetch already running
//! instead of starting another, the last page stays readable while a new one
//! is fetched, and no platform ever waits on another.

use std::collections::HashMap;
use std::sync::Arc;

use once_cell::sync::Lazy;
use tokio::time::{Duration, Instant};

use crate::models::provider_stream::ProviderCategory;
use crate::services::providers::registry;

const TTL: Duration = Duration::from_secs(600);
/// Fetched per platform whatever the caller asked for: the most any surface
/// shows, so one fetch serves them all.
const FETCH_LIMIT: u32 = 40;

#[derive(Default)]
struct Slot {
    /// The last page and when it landed. Never held across an await.
    page: std::sync::Mutex<Option<(Instant, Vec<ProviderCategory>)>>,
    /// Held for the length of a fetch, so there is one at a time.
    fetching: tokio::sync::Mutex<()>,
}

impl Slot {
    fn fresh(&self, limit: u32) -> Option<Vec<ProviderCategory>> {
        let page = self.page.lock().unwrap_or_else(|p| p.into_inner());
        let (at, categories) = page.as_ref()?;
        (at.elapsed() < TTL && limit <= FETCH_LIMIT)
            .then(|| categories.iter().take(limit as usize).cloned().collect())
    }
}

static SLOTS: Lazy<std::sync::Mutex<HashMap<String, Arc<Slot>>>> =
    Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

fn slot(provider: &str) -> Arc<Slot> {
    let mut slots = SLOTS.lock().unwrap_or_else(|p| p.into_inner());
    slots.entry(provider.to_string()).or_default().clone()
}

/// `provider`'s top `limit` categories, most watched first: the cached page
/// while it is fresh, else a fetch.
pub async fn get(provider: &str, limit: u32) -> Result<Vec<ProviderCategory>, String> {
    let slot = slot(provider);
    if let Some(hit) = slot.fresh(limit) {
        return Ok(hit);
    }
    let _one = slot.fetching.lock().await;
    // Fetched by the caller this one waited behind.
    if let Some(hit) = slot.fresh(limit) {
        return Ok(hit);
    }
    let source = registry()
        .await
        .get_source(provider)
        .ok_or_else(|| format!("provider '{}' has no browse support", provider))?;
    let page = source
        .categories(None, limit.max(FETCH_LIMIT))
        .await
        .map_err(|e| e.to_string())?;
    let top = page.categories.iter().take(limit as usize).cloned().collect();
    *slot.page.lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), page.categories));
    Ok(top)
}

/// `provider`'s last page, however old, without fetching.
pub fn cached(provider: &str, limit: u32) -> Vec<ProviderCategory> {
    let slot = slot(provider);
    let page = slot.page.lock().unwrap_or_else(|p| p.into_inner());
    page.as_ref()
        .map(|(_, c)| c.iter().take(limit as usize).cloned().collect())
        .unwrap_or_default()
}

/// Whether `provider` has no page yet, or only one older than `TTL`.
pub fn is_stale(provider: &str) -> bool {
    let slot = slot(provider);
    let page = slot.page.lock().unwrap_or_else(|p| p.into_inner());
    page.as_ref().map_or(true, |(at, _)| at.elapsed() >= TTL)
}
