//! Channel links: which channels on different platforms are the same streamer.
//!
//! A multistreamer's community is split across Twitch, Kick and YouTube, and
//! nothing else in the app knows those rooms belong together. This is the one
//! place that does, so the chat panel can merge them into whichever one the
//! viewer is actually watching.
//!
//! A group is addressed by ANY of its members rather than by a designated
//! primary, so the same link resolves whether the viewer arrives from Twitch or
//! from Kick. A channel belongs to at most one group; linking a channel that
//! already belongs elsewhere moves it, because two groups claiming the same
//! channel would make lookup order decide the answer.
//!
//! Storage and lookup only. The probe that SUGGESTS links lives alongside this
//! and is Kick-only on purpose: every YouTube channel lookup is a full
//! watch-page fetch, and a burst of those gets the IP challenged.

use crate::models::settings::{ChannelLinkGroup, LinkMember, Settings};
use crate::services::providers::key::{make_key, normalize_channel, same_channel};
use log::debug;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// The canonical address of one member, and the form `dismissed` stores.
///
/// One helper on purpose: reads and writes must derive this identically, and a
/// second implementation is how a lookup starts silently missing its own rows.
/// Note this is `make_key`'s composite space (`twitch:xqc`), NOT the bare-Twitch
/// persisted space — nothing here predates multi-platform, so there is no legacy
/// shape to stay compatible with.
pub fn member_key(provider: &str, channel: &str) -> String {
    make_key(provider, channel)
}

fn matches(member: &LinkMember, provider: &str, channel: &str) -> bool {
    member.provider == provider && same_channel(provider, &member.channel, channel)
}

/// Index of the group holding this channel, if any.
fn index_of(links: &[ChannelLinkGroup], provider: &str, channel: &str) -> Option<usize> {
    links
        .iter()
        .position(|g| g.members.iter().any(|m| matches(m, provider, channel)))
}

/// The group this channel belongs to, addressed by any member.
pub fn group_for(settings: &Settings, provider: &str, channel: &str) -> Option<ChannelLinkGroup> {
    index_of(&settings.channel_links, provider, channel).map(|i| settings.channel_links[i].clone())
}

/// The other platforms' channels for whatever is being watched, which is what
/// the chat panel actually asks for.
pub fn companions_of(settings: &Settings, provider: &str, channel: &str) -> Vec<LinkMember> {
    group_for(settings, provider, channel)
        .map(|g| {
            g.members
                .into_iter()
                .filter(|m| !matches(m, provider, channel))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a suggestion for `candidate` was already refused for this streamer,
/// so a probe never offers the same wrong answer twice.
pub fn is_dismissed(
    settings: &Settings,
    provider: &str,
    channel: &str,
    candidate_provider: &str,
    candidate_channel: &str,
) -> bool {
    let key = member_key(candidate_provider, candidate_channel);
    group_for(settings, provider, channel)
        .map(|g| g.dismissed.iter().any(|d| d.eq_ignore_ascii_case(&key)))
        .unwrap_or(false)
}

/// Attach `member` to the streamer that owns (provider, channel).
///
/// Creates the group if this is the first link. If `member` already belongs to
/// a different group it is MOVED, not copied: a channel in two groups makes
/// `group_for` answer by position, which is not an answer.
pub fn link(
    settings: &mut Settings,
    provider: &str,
    channel: &str,
    mut member: LinkMember,
) -> ChannelLinkGroup {
    // Writes are canonical; reads stay loose (see `same_channel`). Never fold a
    // YouTube id's case here: `normalize_channel` carves it out because the id
    // is case-sensitive and is handed straight to the chat connect.
    member.channel = normalize_channel(&member.provider, &member.channel);

    // Detach the incoming member from wherever it lived before.
    if let Some(prev) = index_of(&settings.channel_links, &member.provider, &member.channel) {
        settings.channel_links[prev]
            .members
            .retain(|m| !matches(m, &member.provider, &member.channel));
        if settings.channel_links[prev].members.len() < 2 {
            settings.channel_links.remove(prev);
        }
    }

    let idx = match index_of(&settings.channel_links, provider, channel) {
        Some(i) => i,
        None => {
            settings.channel_links.push(ChannelLinkGroup {
                id: member_key(provider, channel),
                members: vec![LinkMember {
                    provider: provider.to_string(),
                    channel: normalize_channel(provider, channel),
                    display_name: None,
                    avatar: None,
                }],
                dismissed: vec![],
            });
            settings.channel_links.len() - 1
        }
    };

    let group = &mut settings.channel_links[idx];
    // Accepting a link cancels any earlier refusal of the same platform.
    let key = member_key(&member.provider, &member.channel);
    group.dismissed.retain(|d| !d.eq_ignore_ascii_case(&key));
    if let Some(existing) = group
        .members
        .iter_mut()
        .find(|m| matches(m, &member.provider, &member.channel))
    {
        // Re-linking refreshes the display fields rather than duplicating.
        existing.display_name = member.display_name.or(existing.display_name.clone());
        existing.avatar = member.avatar.or(existing.avatar.clone());
    } else {
        group.members.push(member);
    }
    group.clone()
}

/// Detach one platform from its streamer. A group with fewer than two members
/// links nothing, so it is dropped rather than left as a stub that would make
/// `group_for` return a "linked" answer with no companions.
pub fn unlink(settings: &mut Settings, provider: &str, channel: &str) {
    let Some(idx) = index_of(&settings.channel_links, provider, channel) else {
        return;
    };
    settings.channel_links[idx]
        .members
        .retain(|m| !matches(m, provider, channel));
    if settings.channel_links[idx].members.len() < 2 {
        settings.channel_links.remove(idx);
    }
}

/// Record that `candidate` is NOT this streamer, so the probe stops offering it.
///
/// Refusing a suggestion for a channel with no group yet still has to persist,
/// or every app start re-asks the same question. A group is created to hold the
/// refusal even though it links nothing.
pub fn dismiss(
    settings: &mut Settings,
    provider: &str,
    channel: &str,
    candidate_provider: &str,
    candidate_channel: &str,
) {
    let key = member_key(candidate_provider, candidate_channel);
    let idx = match index_of(&settings.channel_links, provider, channel) {
        Some(i) => i,
        None => {
            settings.channel_links.push(ChannelLinkGroup {
                id: member_key(provider, channel),
                members: vec![LinkMember {
                    provider: provider.to_string(),
                    channel: normalize_channel(provider, channel),
                    display_name: None,
                    avatar: None,
                }],
                dismissed: vec![],
            });
            settings.channel_links.len() - 1
        }
    };
    let group = &mut settings.channel_links[idx];
    if !group.dismissed.iter().any(|d| d.eq_ignore_ascii_case(&key)) {
        group.dismissed.push(key);
    }
}

/// Emitted when a probe turns up a channel that might be the same streamer.
pub const EVENT: &str = "channel-links";

#[derive(Serialize, Clone)]
pub struct LinkSuggestion {
    /// The channel being watched, so the frontend can ignore a suggestion that
    /// arrives after the viewer has moved on.
    pub provider: String,
    pub channel: String,
    /// What we found. Never linked without the viewer saying so: a same-named
    /// stranger is a real possibility, and their chat pouring into the feed
    /// under this streamer's name is worse than no suggestion at all.
    pub candidate: LinkMember,
    pub title: String,
    pub is_live: bool,
}

/// When each probed channel becomes due again, so opening the same stream twice
/// does not ask twice.
///
/// A negative answer is held far longer than a positive one: a channel that does
/// not exist on Kick is not going to start existing this session, while a
/// positive one is only re-read in case the viewer linked or refused it
/// elsewhere.
static PROBED: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
const PROBE_TTL_MISS: Duration = Duration::from_secs(12 * 60 * 60);
const PROBE_TTL_HIT: Duration = Duration::from_secs(30 * 60);
/// Bounded so a long session browsing many channels cannot grow the map without
/// limit. Entries are advisory, so dropping due ones early costs one extra probe.
const PROBED_MAX: usize = 512;

/// Stored as the instant it becomes DUE, not the instant it was probed, so the
/// two lifetimes need no arithmetic at the comparison.
fn due_after(now: Instant, found: bool) -> Instant {
    now + if found { PROBE_TTL_HIT } else { PROBE_TTL_MISS }
}

fn is_due_at(due: Option<&Instant>, now: Instant) -> bool {
    match due {
        Some(at) => now >= *at,
        None => true,
    }
}

fn probe_is_due(key: &str) -> bool {
    let map = PROBED.get_or_init(|| Mutex::new(HashMap::new()));
    let map = map.lock().unwrap_or_else(|e| e.into_inner());
    is_due_at(map.get(key), Instant::now())
}

fn mark_probed(key: &str, found: bool) {
    let map = PROBED.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = map.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    map.insert(key.to_string(), due_after(now, found));
    if map.len() > PROBED_MAX {
        map.retain(|_, due| now < *due);
    }
}

/// Whether a name is worth asking Kick about.
///
/// Only a NAME. A YouTube channel is addressed by a `UC…` id, which is not a
/// name on any other platform, so probing one would be asking about a string
/// nobody chose.
fn is_probeable_name(provider: &str, channel: &str) -> bool {
    if provider == "youtube" || channel.starts_with('@') || channel.starts_with("UC") {
        return false;
    }
    !channel.is_empty() && channel.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Two channels are the same person if the names match once the punctuation
/// people sprinkle differently across platforms is removed.
///
/// A Twitch login has no spaces and a YouTube display name often does ("The
/// Burnt Peanut" against `theburntpeanut`), so a strict compare would miss most
/// real pairs. Loose is safe here precisely because nothing is ever linked
/// without the viewer confirming it.
fn names_match(a: &str, b: &str) -> bool {
    let fold = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>()
    };
    let (a, b) = (fold(a), fold(b));
    !a.is_empty() && a == b
}

/// A Kick channel of this name, if one exists.
///
/// Kick's channel endpoint is a plain API call, so this is cheap, and it
/// resolves EXISTENCE rather than liveness: a link that could only be made
/// while both platforms happened to be live at once would almost never be made.
async fn find_on_kick(channel: &str) -> Option<(LinkMember, String, bool)> {
    let slug = channel.to_lowercase();
    let source = crate::services::providers::registry().await.get_source("kick")?;
    let row = source.channel_meta(&slug).await.ok()?;
    Some((
        LinkMember {
            provider: "kick".into(),
            channel: row.user_login.clone(),
            display_name: Some(row.user_name.clone()),
            avatar: row.profile_image_url.clone(),
        },
        row.title.clone(),
        row.is_live,
    ))
}

/// A YouTube channel of this name that is live right now.
///
/// Through SEARCH, which is one InnerTube JSON call, and deliberately NOT
/// through `channel_meta`: that fetches a whole watch page, and
/// `youtube_media`'s own live check warns that a burst of those is what gets an
/// IP challenged — which would break YouTube playback and chat across the app,
/// not just this feature.
///
/// Two things about search results that are easy to get wrong:
/// it ranks by relevance rather than identity, so the top hit is frequently
/// somebody else entirely and the name has to match before anything is offered;
/// and a row's `user_login` is the VIDEO id, which names one broadcast and never
/// comes back, so what gets stored is the `UC…` channel id.
///
/// The cost of going through search is that it only finds a channel that is
/// live, since there is no cheap way to resolve an offline one by name.
async fn find_on_youtube(name: &str) -> Option<(LinkMember, String, bool)> {
    let source = crate::services::providers::registry().await.get_source("youtube")?;
    let page = source.search(name).await.ok()?;
    let row = page
        .streams
        .into_iter()
        .find(|r| r.is_live && names_match(&r.user_name, name) && r.user_id.starts_with("UC"))?;
    Some((
        LinkMember {
            provider: "youtube".into(),
            // The channel id, verbatim: YouTube ids are case-sensitive and this
            // is handed straight to the chat connect.
            channel: row.user_id.clone(),
            display_name: Some(row.user_name.clone()),
            avatar: row.profile_image_url.clone(),
        },
        row.title.clone(),
        true,
    ))
}

/// Look for this streamer on the other platforms, and emit a suggestion for
/// each one found.
///
/// Nothing here is ever linked on its own. A same-named stranger is a real
/// possibility on both platforms, and their chat arriving under this streamer's
/// name would be worse than no suggestion at all.
pub async fn probe(app: &AppHandle, settings: &Settings, provider: &str, channel: &str) {
    if !settings.chat_blend.enabled || !settings.chat_blend.suggest_links {
        return;
    }
    if !is_probeable_name(provider, channel) {
        return;
    }

    // Every platform searched at once: a slow YouTube lookup must not hold
    // back the Kick suggestion, or the other way round.
    futures::future::join_all(
        ["kick", "youtube"]
            .into_iter()
            .map(|candidate| probe_one(app, settings, provider, channel, candidate)),
    )
    .await;
}

/// Search `candidate` for this streamer, and emit a suggestion if found.
async fn probe_one(app: &AppHandle, settings: &Settings, provider: &str, channel: &str, candidate: &str) {
    if provider == candidate {
        return; // already the platform we would be suggesting
    }
    if settings.chat_blend.platforms.get(candidate) == Some(&false) {
        return; // excluded everywhere, so finding one would be noise
    }
    if group_for(settings, provider, channel)
        .map(|g| g.members.iter().any(|m| m.provider == candidate))
        .unwrap_or(false)
    {
        return; // already linked
    }
    // Cached per CANDIDATE platform, not per watched channel: finding one
    // must not suppress the search for the other.
    let cache_key = format!("{}|{}", member_key(provider, channel), candidate);
    if !probe_is_due(&cache_key) {
        return;
    }

    let found = match candidate {
        "kick" => find_on_kick(channel).await,
        "youtube" => find_on_youtube(channel).await,
        _ => None,
    };
    // One line per probe, at info. Whether a platform was searched and what
    // came back is the first question asked when a suggestion does not turn
    // up, and answering it from the outside means a rebuild.
    log::info!(
        "[ChannelLinks] {candidate} probe for {channel}: {}",
        found
            .as_ref()
            .map(|(m, _, live)| format!("found {} (live={live})", m.channel))
            .unwrap_or_else(|| "nothing".into()),
    );

    let Some((member, title, is_live)) = found else {
        // No such channel, it is offline where only live ones are visible,
        // or the platform was unreachable. Either way there is nothing to
        // offer, and re-asking on every channel open would be a request per
        // channel forever.
        mark_probed(&cache_key, false);
        return;
    };
    mark_probed(&cache_key, true);
    if is_dismissed(settings, provider, channel, &member.provider, &member.channel) {
        return; // the viewer already said this is not them
    }

    let suggestion = LinkSuggestion {
        provider: provider.to_string(),
        channel: channel.to_string(),
        candidate: member,
        title,
        is_live,
    };
    if let Err(e) = app.emit(EVENT, &suggestion) {
        debug!("[ChannelLinks] emit failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(provider: &str, channel: &str) -> LinkMember {
        LinkMember {
            provider: provider.into(),
            channel: channel.into(),
            display_name: None,
            avatar: None,
        }
    }

    fn linked() -> Settings {
        let mut s = Settings::default();
        link(&mut s, "twitch", "xqc", member("kick", "xqc"));
        s
    }

    /// The whole point of a group: a viewer arriving from Kick must find the same
    /// streamer a viewer arriving from Twitch finds. Addressing by a designated
    /// primary would work in one direction only.
    #[test]
    fn a_group_resolves_from_any_member() {
        let s = linked();
        assert!(group_for(&s, "twitch", "xqc").is_some());
        assert!(group_for(&s, "kick", "xqc").is_some(), "and from the member that was added");
        assert!(group_for(&s, "twitch", "someone_else").is_none());
    }

    #[test]
    fn companions_exclude_the_channel_being_watched() {
        let s = linked();
        let from_twitch = companions_of(&s, "twitch", "xqc");
        assert_eq!(from_twitch.len(), 1);
        assert_eq!(from_twitch[0].provider, "kick");

        let from_kick = companions_of(&s, "kick", "xqc");
        assert_eq!(from_kick.len(), 1);
        assert_eq!(from_kick[0].provider, "twitch");
    }

    /// YouTube ids are case-SENSITIVE and are handed straight to the chat
    /// connect, so a write must not fold them. Reads stay loose so rows written
    /// before that rule existed still match.
    #[test]
    fn a_youtube_id_is_stored_with_its_case_but_found_without_it() {
        let mut s = Settings::default();
        link(&mut s, "twitch", "xqc", member("youtube", "UCxvT6Dy8OjXlpLFhCAyPWlQ"));

        let stored = &group_for(&s, "twitch", "xqc").unwrap().members[1];
        assert_eq!(stored.channel, "UCxvT6Dy8OjXlpLFhCAyPWlQ", "case survives the write");
        assert!(
            group_for(&s, "youtube", "ucxvt6dy8ojxlplfhcaypwlq").is_some(),
            "a lowercased row from before the casing rule still resolves",
        );
    }

    /// Kick slugs and Twitch logins fold, so a differently-cased lookup is the
    /// same channel and must not create a second group.
    #[test]
    fn a_kick_slug_folds_case() {
        let mut s = linked();
        link(&mut s, "TWITCH", "xqc", member("kick", "XQC"));
        // The uppercase provider does not match, so that call creates nothing new
        // for twitch; what matters is that the kick member did not duplicate.
        let g = group_for(&s, "kick", "xqc").unwrap();
        assert_eq!(
            g.members.iter().filter(|m| m.provider == "kick").count(),
            1,
            "differently-cased slug is the same member",
        );
    }

    /// A channel in two groups makes `group_for` answer by position rather than
    /// by fact, so linking an already-linked channel must move it.
    #[test]
    fn linking_an_already_linked_channel_moves_it() {
        let mut s = linked();
        link(&mut s, "twitch", "someone_else", member("kick", "xqc"));

        assert_eq!(s.channel_links.len(), 1, "the emptied group was dropped, not left as a stub");
        let g = group_for(&s, "kick", "xqc").unwrap();
        assert!(g.members.iter().any(|m| m.channel == "someone_else"));
        assert!(
            group_for(&s, "twitch", "xqc").is_none(),
            "the old group had one member left, which links nothing",
        );
    }

    #[test]
    fn unlinking_the_last_pair_removes_the_group() {
        let mut s = linked();
        unlink(&mut s, "kick", "xqc");
        assert!(s.channel_links.is_empty());
        assert!(group_for(&s, "twitch", "xqc").is_none());
    }

    /// Refusing a suggestion has to persist even when nothing is linked yet, or
    /// the probe asks the same question on every app start.
    #[test]
    fn a_dismissal_persists_with_no_link_and_survives_a_round_trip() {
        let mut s = Settings::default();
        assert!(!is_dismissed(&s, "twitch", "xqc", "kick", "xqc"));

        dismiss(&mut s, "twitch", "xqc", "kick", "xqc");
        assert!(is_dismissed(&s, "twitch", "xqc", "kick", "xqc"));
        assert!(
            companions_of(&s, "twitch", "xqc").is_empty(),
            "a refusal is not a link",
        );

        let json = serde_json::to_value(&s).expect("serialize");
        let back: Settings = serde_json::from_value(json).expect("deserialize");
        assert!(is_dismissed(&back, "twitch", "xqc", "kick", "xqc"));
    }

    /// Changing your mind must actually clear the refusal, or the probe stays
    /// silent forever on a channel the user has now linked by hand.
    #[test]
    fn linking_clears_an_earlier_dismissal() {
        let mut s = Settings::default();
        dismiss(&mut s, "twitch", "xqc", "kick", "xqc");
        link(&mut s, "twitch", "xqc", member("kick", "xqc"));
        assert!(!is_dismissed(&s, "twitch", "xqc", "kick", "xqc"));
        assert_eq!(companions_of(&s, "twitch", "xqc").len(), 1);
    }

    /// Re-linking refreshes the display fields rather than pushing a duplicate
    /// member, which would render the same chat twice in the merged feed.
    #[test]
    fn relinking_updates_rather_than_duplicates() {
        let mut s = linked();
        link(
            &mut s,
            "twitch",
            "xqc",
            LinkMember {
                provider: "kick".into(),
                channel: "xqc".into(),
                display_name: Some("xQc".into()),
                avatar: Some("https://example.invalid/a.png".into()),
            },
        );
        let g = group_for(&s, "twitch", "xqc").unwrap();
        assert_eq!(g.members.len(), 2);
        assert_eq!(g.members[1].display_name.as_deref(), Some("xQc"));
    }

    /// A miss has to be held LONGER than a hit, not shorter. An earlier version
    /// encoded both lifetimes as an offset against one comparison and inverted
    /// them, which made every channel with no Kick counterpart — the common case
    /// — re-probe on every single open.
    #[test]
    fn a_miss_is_remembered_far_longer_than_a_hit() {
        let t0 = Instant::now();
        let hit = due_after(t0, true);
        let miss = due_after(t0, false);
        assert!(miss > hit, "a channel that does not exist must not be re-asked first");

        assert!(!is_due_at(Some(&hit), t0), "neither is due immediately");
        assert!(!is_due_at(Some(&miss), t0));

        assert!(
            is_due_at(Some(&hit), t0 + PROBE_TTL_HIT),
            "a hit is re-read after the short lifetime",
        );
        assert!(
            !is_due_at(Some(&miss), t0 + PROBE_TTL_HIT),
            "a miss is NOT, which is the whole point",
        );
        assert!(is_due_at(Some(&miss), t0 + PROBE_TTL_MISS));
    }

    #[test]
    fn a_channel_never_probed_is_due() {
        assert!(is_due_at(None, Instant::now()));
    }

    /// The probe asks Kick about a NAME. A YouTube channel is addressed by a UC
    /// id or an @handle, which is not a name anyone chose on another platform,
    /// so asking about it would be noise at best.
    #[test]
    fn only_a_real_name_is_worth_probing() {
        assert!(is_probeable_name("twitch", "xqc"));
        assert!(is_probeable_name("twitch", "the_burnt_peanut"));
        assert!(!is_probeable_name("youtube", "xqc"), "never from a YouTube page");
        assert!(!is_probeable_name("twitch", "UCX6OQ3DkcsbYNE6H8uQQuVA"));
        assert!(!is_probeable_name("twitch", "@mrbeast"));
        assert!(!is_probeable_name("twitch", ""));
        assert!(!is_probeable_name("twitch", "has spaces"));
    }

    /// YouTube search ranks by RELEVANCE, not identity, so the top hit for a
    /// streamer's name is frequently somebody else playing the same game. The
    /// name check is the only thing standing between that and a stranger's chat
    /// being offered under this streamer's name.
    #[test]
    fn a_name_matches_across_the_punctuation_platforms_disagree_about() {
        // The real pair this exists for: a Twitch login has no spaces, a YouTube
        // display name usually does.
        assert!(names_match("The Burnt Peanut", "theburntpeanut"));
        assert!(names_match("NICKMERCS", "nickmercs"));
        assert!(names_match("Anthony_Z", "anthonyz"));
        assert!(names_match("xQc", "xqc"));
    }

    #[test]
    fn a_different_streamer_is_not_a_match() {
        assert!(!names_match("Krinkz", "nickmercs"));
        assert!(!names_match("nickmercs2", "nickmercs"), "a near miss is still a miss");
        assert!(!names_match("", "nickmercs"));
        // Folding must not make two different people collapse into one.
        assert!(!names_match("nick mercs tv", "nickmercs"));
    }

    /// Three platforms is the case the feature exists for.
    #[test]
    fn a_third_platform_joins_the_same_group() {
        let mut s = linked();
        link(&mut s, "kick", "xqc", member("youtube", "UCabc"));
        assert_eq!(s.channel_links.len(), 1, "added via kick, not via the creator");
        assert_eq!(companions_of(&s, "twitch", "xqc").len(), 2);
    }
}
