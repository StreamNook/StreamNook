//! The emote picker's contents for one room and one tab: which tabs the room
//! has and how many emotes each holds, the selected tab's sections in order
//! with their titles and grid widths, and each cell's flags. Every window that
//! shows the picker (desktop chat, the popped-out menu, the mod room, both
//! phones) draws this answer and groups nothing itself.
//!
//! Sections:
//! - Twitch: "Global Emotes", one section per subscription owner titled with
//!   the owner's display name ("Channel <id>" until it is known), "Bits
//!   Emotes", "Follower Emotes", "Channel Points Emotes", then any other type
//!   capitalized. The current channel's subscription section comes first, then
//!   globals, channel points, other subscriptions, and the rest by title.
//! - 7TV, BetterTTV, FrankerFaceZ and Favorites: by width ("Standard" up to 48
//!   px, 7 columns; "Wide" up to 80 px, 4; "Ultra Wide", 3), zero-width emotes
//!   first, then narrower, then by name. 7TV leads with the viewer's own
//!   "Personal" emotes in a Twitch room.
//! - Kick: one section per set label, in arrival order.
//! - Favorites leads with "Recently used": the emotes the viewer sent last that
//!   this room has, newest first (`recent_emotes`).
//!
//! Search is a case-insensitive substring match on names inside the tab.
//! Cells carry whether they are a favorite, locked (an FFZ subscriber effect
//! for a viewer who is not an FFZ subscriber), `replaced_by` (the viewer's
//! personal emote that stands in for a channel emote of the same name), and,
//! as the sets hold them, zero-width and modifier flags.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::services::emote_service::{Emote, EmoteSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Favorites,
    Twitch,
    Bttv,
    SevenTv,
    Ffz,
    Kick,
}

impl Tab {
    pub fn parse(raw: &str) -> Option<Tab> {
        Some(match raw {
            "favorites" => Tab::Favorites,
            "twitch" => Tab::Twitch,
            "bttv" => Tab::Bttv,
            "7tv" => Tab::SevenTv,
            "ffz" => Tab::Ffz,
            "kick" => Tab::Kick,
            _ => return None,
        })
    }

    pub fn wire(self) -> &'static str {
        match self {
            Tab::Favorites => "favorites",
            Tab::Twitch => "twitch",
            Tab::Bttv => "bttv",
            Tab::SevenTv => "7tv",
            Tab::Ffz => "ffz",
            Tab::Kick => "kick",
        }
    }
}

/// The platform the room is on, which decides its tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    Twitch,
    Kick,
    /// YouTube and any other platform: 7TV only from this answer.
    Other,
}

impl Room {
    pub fn parse(provider: Option<&str>) -> Room {
        match provider.unwrap_or("twitch") {
            "twitch" => Room::Twitch,
            "kick" => Room::Kick,
            _ => Room::Other,
        }
    }

    /// The emote tabs this answer serves for the room, in picker order
    /// (Favorites first; Emoji and GIFs are the page's own).
    fn tabs(self) -> &'static [Tab] {
        match self {
            Room::Twitch => &[Tab::Favorites, Tab::Twitch, Tab::Bttv, Tab::SevenTv, Tab::Ffz],
            Room::Kick => &[Tab::Favorites, Tab::Kick, Tab::SevenTv],
            Room::Other => &[Tab::Favorites, Tab::SevenTv],
        }
    }
}

/// One cell: the emote as the sets hold it, plus what the picker shows on it.
#[derive(Clone, Debug, Serialize)]
pub struct PickerEmote {
    #[serde(flatten)]
    pub emote: Emote,
    pub favorite: bool,
    /// Shown dimmed with a lock and cannot be picked.
    pub locked: bool,
    /// The viewer's personal emote of the same name, which shows instead
    /// when this name is typed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<ReplacedBy>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReplacedBy {
    pub id: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PickerSection {
    /// Stable across answers ("globals", "sub-<owner id>", "standard").
    pub key: String,
    pub title: String,
    /// Grid columns on a phone-width picker.
    pub columns: u8,
    pub emotes: Vec<PickerEmote>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PickerTab {
    pub id: &'static str,
    /// Emotes the tab holds, before any search.
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PickerAnswer {
    /// False while the room's emote set is not in memory yet.
    pub ready: bool,
    pub tabs: Vec<PickerTab>,
    /// The tab the sections belong to.
    pub tab: &'static str,
    pub sections: Vec<PickerSection>,
}

/// What the picker needs besides the set.
pub struct Context<'a> {
    pub room: Room,
    /// The room's own id (a Twitch user id in a Twitch room), for putting the
    /// channel's subscription section first.
    pub channel_id: Option<&'a str>,
    /// Favorite emote ids in the order they were saved.
    pub favorites: &'a [String],
    /// Emote ids the viewer used most recently, newest first.
    pub recent: &'a [String],
    pub ffz_subwoofer: bool,
    /// Twitch user id -> display name for subscription emote owners.
    pub owner_names: &'a HashMap<String, String>,
    /// The viewer's own 7TV personal emotes (name -> emote), Twitch rooms only.
    pub personal: Option<&'a HashMap<String, Emote>>,
}

const STANDARD_MAX: u32 = 48;
const WIDE_MAX: u32 = 80;
/// The width an emote is grouped by when the provider did not say.
const DEFAULT_WIDTH: u32 = 32;

fn matches(name: &str, needle: &str) -> bool {
    needle.is_empty() || name.to_lowercase().contains(needle)
}

/// Personal emotes, in name order so the section is stable.
fn personal_list(personal: Option<&HashMap<String, Emote>>) -> Vec<&Emote> {
    let mut list: Vec<&Emote> = personal.map(|m| m.values().collect()).unwrap_or_default();
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
    list
}

/// The emotes a tab holds, before search.
fn tab_emotes<'a>(set: &'a EmoteSet, tab: Tab, ctx: &Context<'a>) -> Vec<&'a Emote> {
    match tab {
        Tab::Twitch => set.twitch.iter().collect(),
        Tab::Bttv => set.bttv.iter().collect(),
        Tab::Ffz => set.ffz.iter().collect(),
        Tab::Kick => set.kick.iter().collect(),
        Tab::SevenTv => {
            let mut out = personal_list(ctx.personal);
            // A channel emote that is also one of the viewer's own shows once.
            let mine: HashSet<&str> = out.iter().map(|e| e.id.as_str()).collect();
            out.extend(set.seven_tv.iter().filter(|e| !mine.contains(e.id.as_str())));
            out
        }
        Tab::Favorites => in_room(set, ctx, ctx.favorites),
    }
}

/// The emotes with these ids that this room can use, in the order given, once each.
fn in_room<'a>(set: &'a EmoteSet, ctx: &Context<'a>, ids: &[String]) -> Vec<&'a Emote> {
    let mut by_id: HashMap<&str, &Emote> = HashMap::new();
    let all = personal_list(ctx.personal)
        .into_iter()
        .chain(set.twitch.iter())
        .chain(set.seven_tv.iter())
        .chain(set.bttv.iter())
        .chain(set.ffz.iter())
        .chain(set.kick.iter());
    for e in all {
        by_id.entry(e.id.as_str()).or_insert(e);
    }
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(id.as_str()))
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .collect()
}

/// How many recently used emotes the Favorites tab leads with: two rows.
const RECENT_SHOWN: usize = 14;

fn cell(
    e: &Emote,
    ctx: &Context<'_>,
    favorites: &HashSet<&str>,
    replaced: &HashMap<&str, &Emote>,
    personal_ids: &HashSet<&str>,
) -> PickerEmote {
    let is_channel_third_party = !matches!(e.provider, crate::services::emote_service::EmoteProvider::Twitch)
        && !personal_ids.contains(e.id.as_str());
    PickerEmote {
        emote: e.clone(),
        favorite: favorites.contains(e.id.as_str()),
        locked: e.ffz_sub_only.unwrap_or(false) && !ctx.ffz_subwoofer,
        replaced_by: is_channel_third_party
            .then(|| replaced.get(e.name.as_str()))
            .flatten()
            .map(|mine| ReplacedBy { id: mine.id.clone(), url: mine.url.clone() }),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Twitch emotes grouped by type and owner, in picker order.
fn twitch_sections(emotes: Vec<PickerEmote>, ctx: &Context<'_>) -> Vec<PickerSection> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, (String, Vec<PickerEmote>)> = HashMap::new();
    for pe in emotes {
        let e = &pe.emote;
        let kind = e.emote_type.as_deref().unwrap_or("globals");
        let owner = e.owner_id.as_deref().filter(|o| !o.is_empty());
        let (key, title) = match (kind, owner) {
            ("globals", _) | (_, None) => ("globals".to_string(), "Global Emotes".to_string()),
            ("subscriptions", Some(o)) => (
                format!("sub-{o}"),
                ctx.owner_names.get(o).cloned().unwrap_or_else(|| format!("Channel {o}")),
            ),
            ("bitstier", _) => ("bits".to_string(), "Bits Emotes".to_string()),
            ("follower", Some(o)) => (format!("follower-{o}"), "Follower Emotes".to_string()),
            ("channelpoints", Some(o)) => (format!("points-{o}"), "Channel Points Emotes".to_string()),
            (other, _) => (other.to_string(), capitalize(other)),
        };
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_insert_with(|| (title, Vec::new())).1.push(pe);
    }
    let current = ctx.channel_id.map(|id| format!("sub-{id}"));
    let rank = |key: &str| -> u8 {
        if current.as_deref() == Some(key) {
            0
        } else if key == "globals" {
            1
        } else if key.starts_with("points-") {
            2
        } else if key.starts_with("sub-") {
            3
        } else {
            4
        }
    };
    order.sort_by(|a, b| {
        rank(a).cmp(&rank(b)).then_with(|| {
            let ta = &groups[a].0;
            let tb = &groups[b].0;
            ta.to_lowercase().cmp(&tb.to_lowercase()).then_with(|| ta.cmp(tb))
        })
    });
    order
        .into_iter()
        .filter_map(|key| {
            let (title, emotes) = groups.remove(&key)?;
            Some(PickerSection { key, title, columns: 7, emotes })
        })
        .collect()
}

/// Third-party emotes grouped by width, zero-width first, then narrower, then
/// by name. Empty groups are left out.
fn width_sections(emotes: Vec<PickerEmote>) -> Vec<PickerSection> {
    let mut groups: [(&str, &str, u8, Vec<PickerEmote>); 3] = [
        ("standard", "Standard", 7, Vec::new()),
        ("wide", "Wide", 4, Vec::new()),
        ("ultrawide", "Ultra Wide", 3, Vec::new()),
    ];
    for pe in emotes {
        let w = pe.emote.width.filter(|w| *w > 0).unwrap_or(DEFAULT_WIDTH);
        let slot = if w <= STANDARD_MAX {
            0
        } else if w <= WIDE_MAX {
            1
        } else {
            2
        };
        groups[slot].3.push(pe);
    }
    groups
        .into_iter()
        .filter(|g| !g.3.is_empty())
        .map(|(key, title, columns, mut emotes)| {
            emotes.sort_by(|a, b| {
                let za = a.emote.is_zero_width.unwrap_or(false);
                let zb = b.emote.is_zero_width.unwrap_or(false);
                let wa = a.emote.width.filter(|w| *w > 0).unwrap_or(DEFAULT_WIDTH);
                let wb = b.emote.width.filter(|w| *w > 0).unwrap_or(DEFAULT_WIDTH);
                zb.cmp(&za)
                    .then(wa.cmp(&wb))
                    .then_with(|| a.emote.name.to_lowercase().cmp(&b.emote.name.to_lowercase()))
                    .then_with(|| a.emote.name.cmp(&b.emote.name))
            });
            PickerSection { key: key.to_string(), title: title.to_string(), columns, emotes }
        })
        .collect()
}

/// Kick emotes by set label, in arrival order.
fn kick_sections(emotes: Vec<PickerEmote>) -> Vec<PickerSection> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<PickerEmote>> = HashMap::new();
    for pe in emotes {
        let label = pe.emote.emote_type.clone().filter(|l| !l.is_empty()).unwrap_or_else(|| "Emotes".into());
        if !groups.contains_key(&label) {
            order.push(label.clone());
        }
        groups.entry(label).or_default().push(pe);
    }
    order
        .into_iter()
        .filter_map(|label| {
            let emotes = groups.remove(&label)?;
            Some(PickerSection { key: label.clone(), title: label, columns: 7, emotes })
        })
        .collect()
}

/// The picker for `tab` (falling back to the room's default tab when the room
/// has no such tab), with `query` applied.
pub fn build(set: Option<&EmoteSet>, ctx: &Context<'_>, tab: Option<Tab>, query: &str) -> PickerAnswer {
    let tabs_here = ctx.room.tabs();
    let default_tab = match ctx.room {
        Room::Twitch => Tab::Twitch,
        Room::Kick => Tab::Kick,
        Room::Other => Tab::SevenTv,
    };
    let tab = tab.filter(|t| tabs_here.contains(t)).unwrap_or(default_tab);
    let empty = EmoteSet::new();
    let ready = set.is_some();
    let set = set.unwrap_or(&empty);

    let tabs = tabs_here
        .iter()
        .map(|t| PickerTab { id: t.wire(), count: tab_emotes(set, *t, ctx).len() })
        .collect();

    let favorites: HashSet<&str> = ctx.favorites.iter().map(String::as_str).collect();
    let personal_ids: HashSet<&str> =
        ctx.personal.map(|m| m.values().map(|e| e.id.as_str()).collect()).unwrap_or_default();
    // Channel names a personal emote of the same name replaces, unless it is
    // the very same emote.
    let replaced: HashMap<&str, &Emote> = match ctx.personal {
        Some(personal) if ctx.room == Room::Twitch => set
            .seven_tv
            .iter()
            .chain(&set.bttv)
            .chain(&set.ffz)
            .filter_map(|e| {
                let mine = personal.get(&e.name).filter(|mine| mine.id != e.id)?;
                Some((e.name.as_str(), mine))
            })
            .collect(),
        _ => HashMap::new(),
    };

    let needle = query.trim().to_lowercase();
    let picked: Vec<PickerEmote> = tab_emotes(set, tab, ctx)
        .into_iter()
        .filter(|e| matches(&e.name, &needle))
        .map(|e| cell(e, ctx, &favorites, &replaced, &personal_ids))
        .collect();

    let sections = match tab {
        Tab::Twitch => twitch_sections(picked, ctx),
        Tab::Kick => kick_sections(picked),
        Tab::SevenTv if ctx.room == Room::Twitch => {
            let (mine, rest): (Vec<_>, Vec<_>) =
                picked.into_iter().partition(|pe| personal_ids.contains(pe.emote.id.as_str()));
            let mut out = Vec::new();
            if !mine.is_empty() {
                out.push(PickerSection { key: "personal".into(), title: "Personal".into(), columns: 7, emotes: mine });
            }
            out.extend(width_sections(rest));
            out
        }
        Tab::Favorites => {
            let mut out = Vec::new();
            let recent: Vec<PickerEmote> = in_room(set, ctx, ctx.recent)
                .into_iter()
                .filter(|e| matches(&e.name, &needle))
                .take(RECENT_SHOWN)
                .map(|e| cell(e, ctx, &favorites, &replaced, &personal_ids))
                .collect();
            if !recent.is_empty() {
                out.push(PickerSection { key: "recent".into(), title: "Recently used".into(), columns: 7, emotes: recent });
            }
            out.extend(width_sections(picked));
            out
        }
        _ => width_sections(picked),
    };

    PickerAnswer { ready, tabs, tab: tab.wire(), sections }
}

/// Owner ids of the Twitch subscription emotes in `set`, sorted and unique,
/// for one batched name lookup.
pub fn subscription_owners(set: &EmoteSet) -> Vec<String> {
    let mut ids: Vec<String> = set
        .twitch
        .iter()
        .filter(|e| e.emote_type.as_deref() == Some("subscriptions"))
        .filter_map(|e| e.owner_id.clone().filter(|o| !o.is_empty()))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::emote_service::EmoteProvider;

    fn emote(provider: EmoteProvider, id: &str, name: &str) -> Emote {
        Emote {
            id: id.into(),
            name: name.into(),
            url: format!("https://example.invalid/{id}"),
            provider,
            is_zero_width: None,
            local_url: None,
            emote_type: None,
            owner_id: None,
            owner_name: None,
            original_name: None,
            owner_ref: None,
            width: None,
            height: None,
            modifier_flags: None,
            ffz_sub_only: None,
            animated: None,
        }
    }

    fn twitch(id: &str, name: &str, kind: &str, owner: Option<&str>) -> Emote {
        let mut e = emote(EmoteProvider::Twitch, id, name);
        e.emote_type = Some(kind.into());
        e.owner_id = owner.map(Into::into);
        e
    }

    fn wide(provider: EmoteProvider, id: &str, name: &str, width: u32) -> Emote {
        let mut e = emote(provider, id, name);
        e.width = Some(width);
        e
    }

    fn ctx<'a>(names: &'a HashMap<String, String>, favorites: &'a [String]) -> Context<'a> {
        Context {
            room: Room::Twitch,
            channel_id: Some("100"),
            favorites,
            recent: &[],
            ffz_subwoofer: false,
            owner_names: names,
            personal: None,
        }
    }

    fn titles(a: &PickerAnswer) -> Vec<&str> {
        a.sections.iter().map(|s| s.title.as_str()).collect()
    }

    #[test]
    fn twitch_groups_by_owner_with_the_channel_first() {
        let mut set = EmoteSet::new();
        set.twitch = vec![
            twitch("g1", "Kappa", "globals", None),
            twitch("s2", "otherHi", "subscriptions", Some("200")),
            twitch("p1", "pointsHi", "channelpoints", Some("100")),
            twitch("s1", "chanHi", "subscriptions", Some("100")),
            twitch("b1", "bitsHi", "bitstier", Some("100")),
            twitch("f1", "folHi", "follower", Some("100")),
            twitch("s3", "zedHi", "subscriptions", Some("300")),
        ];
        let names: HashMap<String, String> =
            [("100".to_string(), "Chan".to_string()), ("200".to_string(), "Other".to_string())].into();
        let a = build(Some(&set), &ctx(&names, &[]), Some(Tab::Twitch), "");
        assert_eq!(
            titles(&a),
            vec!["Chan", "Global Emotes", "Channel Points Emotes", "Channel 300", "Other", "Bits Emotes", "Follower Emotes"]
        );
        assert!(a.sections.iter().all(|s| s.columns == 7));
        assert_eq!(a.tab, "twitch");
        assert_eq!(
            a.tabs,
            vec![
                PickerTab { id: "favorites", count: 0 },
                PickerTab { id: "twitch", count: 7 },
                PickerTab { id: "bttv", count: 0 },
                PickerTab { id: "7tv", count: 0 },
                PickerTab { id: "ffz", count: 0 },
            ]
        );
    }

    #[test]
    fn third_party_groups_by_width_zero_width_first() {
        let mut set = EmoteSet::new();
        let mut zw = wide(EmoteProvider::SevenTV, "z", "zLast", 40);
        zw.is_zero_width = Some(true);
        set.seven_tv = vec![
            wide(EmoteProvider::SevenTV, "a", "bWide", 70),
            wide(EmoteProvider::SevenTV, "b", "aStd", 32),
            emote(EmoteProvider::SevenTV, "c", "noWidth"),
            zw,
            wide(EmoteProvider::SevenTV, "d", "huge", 200),
            wide(EmoteProvider::SevenTV, "e", "narrow", 20),
        ];
        let names = HashMap::new();
        let a = build(Some(&set), &ctx(&names, &[]), Some(Tab::SevenTv), "");
        assert_eq!(titles(&a), vec!["Standard", "Wide", "Ultra Wide"]);
        let std: Vec<_> = a.sections[0].emotes.iter().map(|e| e.emote.name.as_str()).collect();
        assert_eq!(std, vec!["zLast", "narrow", "aStd", "noWidth"]);
        assert_eq!(
            a.sections.iter().map(|s| s.columns).collect::<Vec<_>>(),
            vec![7, 4, 3]
        );
    }

    #[test]
    fn search_filters_inside_the_tab_and_drops_empty_groups() {
        let mut set = EmoteSet::new();
        set.bttv = vec![wide(EmoteProvider::BTTV, "1", "catJAM", 32), wide(EmoteProvider::BTTV, "2", "wideCAT", 90)];
        let names = HashMap::new();
        let a = build(Some(&set), &ctx(&names, &[]), Some(Tab::Bttv), "jam");
        assert_eq!(titles(&a), vec!["Standard"]);
        assert_eq!(a.tabs[2], PickerTab { id: "bttv", count: 2 }, "counts ignore the search");
        let none = build(Some(&set), &ctx(&names, &[]), Some(Tab::Bttv), "nothing");
        assert!(none.sections.is_empty());
    }

    #[test]
    fn favorites_list_only_what_the_room_has_in_saved_order() {
        let mut set = EmoteSet::new();
        set.twitch = vec![twitch("t1", "Kappa", "globals", None)];
        set.ffz = vec![emote(EmoteProvider::FFZ, "f1", "OMEGALUL")];
        let names = HashMap::new();
        let favorites = vec!["f1".to_string(), "gone".to_string(), "t1".to_string()];
        let a = build(Some(&set), &ctx(&names, &favorites), Some(Tab::Favorites), "");
        let all: Vec<_> = a.sections.iter().flat_map(|s| s.emotes.iter().map(|e| e.emote.name.as_str())).collect();
        assert_eq!(all, vec!["Kappa", "OMEGALUL"], "width order inside the Standard group");
        assert_eq!(a.tabs[0], PickerTab { id: "favorites", count: 2 });
        assert!(a.sections[0].emotes.iter().all(|e| e.favorite));
    }

    #[test]
    fn ffz_subscriber_effects_lock_for_everyone_else() {
        let mut set = EmoteSet::new();
        let mut effect = emote(EmoteProvider::FFZ, "fx", "ffzSpin");
        effect.ffz_sub_only = Some(true);
        set.ffz = vec![effect];
        let names = HashMap::new();
        let mut c = ctx(&names, &[]);
        let a = build(Some(&set), &c, Some(Tab::Ffz), "");
        assert!(a.sections[0].emotes[0].locked);
        c.ffz_subwoofer = true;
        let a = build(Some(&set), &c, Some(Tab::Ffz), "");
        assert!(!a.sections[0].emotes[0].locked);
    }

    #[test]
    fn personal_emotes_lead_7tv_and_mark_what_they_replace() {
        let mut set = EmoteSet::new();
        set.seven_tv = vec![emote(EmoteProvider::SevenTV, "chan", "buh"), emote(EmoteProvider::SevenTV, "same", "glorp")];
        set.bttv = vec![emote(EmoteProvider::BTTV, "b", "buh")];
        let personal: HashMap<String, Emote> = [
            ("buh".to_string(), emote(EmoteProvider::SevenTV, "mine", "buh")),
            ("glorp".to_string(), emote(EmoteProvider::SevenTV, "same", "glorp")),
        ]
        .into();
        let names = HashMap::new();
        let mut c = ctx(&names, &[]);
        c.personal = Some(&personal);
        let a = build(Some(&set), &c, Some(Tab::SevenTv), "");
        assert_eq!(titles(&a), vec!["Personal", "Standard"]);
        let mine: Vec<_> =
            a.sections[0].emotes.iter().map(|e| (e.emote.id.as_str(), e.replaced_by.is_some())).collect();
        assert_eq!(mine, vec![("mine", false), ("same", false)]);
        let chan = a.sections[1].emotes.iter().find(|e| e.emote.id == "chan").unwrap();
        assert_eq!(
            chan.replaced_by,
            Some(ReplacedBy { id: "mine".into(), url: "https://example.invalid/mine".into() }),
            "the channel's buh shows as the personal one"
        );
        let b = build(Some(&set), &c, Some(Tab::Bttv), "");
        assert!(b.sections[0].emotes[0].replaced_by.is_some());
    }

    #[test]
    fn a_tab_the_room_lacks_falls_back_and_no_set_is_not_ready() {
        let names = HashMap::new();
        let mut c = ctx(&names, &[]);
        c.room = Room::Kick;
        let a = build(None, &c, Some(Tab::Twitch), "");
        assert!(!a.ready);
        assert_eq!(a.tab, "kick");
        assert_eq!(a.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), vec!["favorites", "kick", "7tv"]);
    }

    #[test]
    fn kick_groups_by_set_label_in_arrival_order() {
        let mut set = EmoteSet::new();
        let mut a1 = emote(EmoteProvider::Kick, "1", "chanA");
        a1.emote_type = Some("xqc".into());
        let mut g1 = emote(EmoteProvider::Kick, "2", "glob");
        g1.emote_type = Some("Global".into());
        let mut a2 = emote(EmoteProvider::Kick, "3", "chanB");
        a2.emote_type = Some("xqc".into());
        set.kick = vec![a1, g1, a2];
        let names = HashMap::new();
        let mut c = ctx(&names, &[]);
        c.room = Room::Kick;
        let a = build(Some(&set), &c, None, "");
        assert_eq!(titles(&a), vec!["xqc", "Global"]);
        assert_eq!(a.sections[0].emotes.len(), 2);
    }

    #[test]
    fn subscription_owners_are_unique_and_sorted() {
        let mut set = EmoteSet::new();
        set.twitch = vec![
            twitch("1", "a", "subscriptions", Some("9")),
            twitch("2", "b", "subscriptions", Some("3")),
            twitch("3", "c", "subscriptions", Some("9")),
            twitch("4", "d", "follower", Some("7")),
        ];
        assert_eq!(subscription_owners(&set), vec!["3".to_string(), "9".to_string()]);
    }

    #[test]
    fn favorites_lead_with_what_was_used_lately() {
        let mut set = EmoteSet::new();
        set.twitch = vec![twitch("t1", "Kappa", "globals", None)];
        set.bttv = vec![emote(EmoteProvider::BTTV, "b1", "catJAM")];
        let names = HashMap::new();
        let favorites = vec!["t1".to_string()];
        let recent = vec!["b1".to_string(), "gone".to_string(), "t1".to_string()];
        let mut c = ctx(&names, &favorites);
        c.recent = &recent;
        let a = build(Some(&set), &c, Some(Tab::Favorites), "");
        assert_eq!(titles(&a), vec!["Recently used", "Standard"]);
        let used: Vec<_> = a.sections[0].emotes.iter().map(|e| e.emote.name.as_str()).collect();
        assert_eq!(used, vec!["catJAM", "Kappa"]);
        assert_eq!(a.tabs[0], PickerTab { id: "favorites", count: 1 }, "the count is the favorites");
    }
}
