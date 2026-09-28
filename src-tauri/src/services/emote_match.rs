//! Emote name matching for the chat composer.
//!
//! The Tab cycle and the emote search list both rank here, over the sets Rust
//! already holds, so no window keeps its own copy of the matching rules or of
//! the emote sets they run over.
//!
//! Two profiles:
//! - `Cycle` is the Tab cycle: provider tier, then favorites, then name, with
//!   names deduplicated case-insensitively. A leading `:` moves Twitch to the
//!   front; a leading `@` is a chatter query and matches no emote.
//! - `Search` is the list: how well the name matches what was typed comes
//!   first, so an exact Twitch match is never buried under provider order. It
//!   always looks inside names too, because a Twitch channel emote starts with
//!   the streamer's prefix (`vulpLove`), so "starts with lo" could never find it.
//!   Names deduplicate EXACTLY, because `LOADING` and `Loading` are different
//!   tokens that render as different emotes.
//!
//! Callers hand names in with `offer`; the rest of a row (`Seed`) is only built
//! for names that matched, so a scan over thousands of names allocates for the
//! handful that survive.

use serde::Serialize;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use crate::models::settings::Settings;
use crate::services::emote_service::{Emote, EmoteProvider, EmoteSet};

/// Marks a BTTV or FFZ row that came from the provider's global set.
pub const GLOBAL_EMOTE_TYPE: &str = "global";

/// Marks a 7TV row from the viewer's own personal set, usable in every channel.
pub const PERSONAL_EMOTE_TYPE: &str = "personal";

/// The most rows one call returns.
pub const MAX_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Twitch,
    SevenTv,
    Bttv,
    Ffz,
    Kick,
    YouTube,
}

impl Slot {
    const COUNT: usize = 6;

    fn index(self) -> usize {
        self as usize
    }

    /// The provider string the page uses for this slot.
    pub fn wire(self) -> &'static str {
        match self {
            Slot::Twitch => "twitch",
            Slot::SevenTv => "7tv",
            Slot::Bttv => "bttv",
            Slot::Ffz => "ffz",
            Slot::Kick => "kick",
            Slot::YouTube => "youtube",
        }
    }

    /// The disk-cache provider, where the slot has one. YouTube emoji are
    /// never cached to disk.
    pub fn cache_provider(self) -> Option<EmoteProvider> {
        match self {
            Slot::Twitch => Some(EmoteProvider::Twitch),
            Slot::SevenTv => Some(EmoteProvider::SevenTV),
            Slot::Bttv => Some(EmoteProvider::BTTV),
            Slot::Ffz => Some(EmoteProvider::FFZ),
            Slot::Kick => Some(EmoteProvider::Kick),
            Slot::YouTube => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Cycle,
    Search,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// 7TV, BTTV, FFZ, then Twitch: the Tab cycle's long-standing order.
    Default,
    /// Twitch first, for a query that started with a colon.
    TwitchFirst,
}

impl Order {
    fn slots(self) -> [Slot; Slot::COUNT] {
        match self {
            Order::Default => [Slot::SevenTv, Slot::Bttv, Slot::Ffz, Slot::Twitch, Slot::Kick, Slot::YouTube],
            Order::TwitchFirst => [Slot::Twitch, Slot::SevenTv, Slot::Bttv, Slot::Ffz, Slot::Kick, Slot::YouTube],
        }
    }
}

// "Contains" instead of "starts with", from chat_input.emote_tab_complete_match_mode.
static CONTAINS_MODE: AtomicBool = AtomicBool::new(false);

/// Pick up the match mode from saved settings. Runs on boot and every save.
pub fn refresh_settings(settings: &Settings) {
    let includes = settings
        .extra
        .get("chat_input")
        .and_then(|v| v.get("emote_tab_complete_match_mode"))
        .and_then(|v| v.as_str())
        == Some("includes");
    CONTAINS_MODE.store(includes, AtomicOrdering::Relaxed);
}

pub fn contains_mode() -> bool {
    CONTAINS_MODE.load(AtomicOrdering::Relaxed)
}

/// How a name matched. Lower is better.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    ExactCase,
    Exact,
    Prefix,
    /// Starts a word inside the name: "lo" in `vulpLove` or `peepo_lol`. Channel
    /// emotes all carry the streamer's prefix, so this is how they are found.
    WordStart,
    Contains,
}

pub struct Spec<'a> {
    pub query: &'a str,
    pub profile: Profile,
    /// Search only. The cycle derives its order from a leading colon.
    pub order: Order,
    pub contains: bool,
    pub limit: usize,
}

pub struct Context<'a> {
    /// The channel being typed in, as its platform's user id.
    pub channel_id: Option<&'a str>,
    pub favorites: &'a HashSet<String>,
    /// Ids in 7TV's global set.
    pub seventv_globals: &'a HashSet<String>,
    pub ffz_subwoofer: bool,
}

/// Everything a row needs besides its name.
pub struct Seed {
    pub id: String,
    pub url: String,
    pub insert_text: Option<String>,
    pub emote_type: Option<String>,
    pub is_zero_width: Option<bool>,
    pub modifier_flags: Option<u32>,
    /// Available in every channel (a provider's global set).
    pub global: bool,
    /// Belongs to the channel being typed in.
    pub own: bool,
}

struct Hit {
    slot: Slot,
    name: String,
    kind: Kind,
    /// Byte offset of the match in `name`, where it is known (ASCII names).
    at: Option<usize>,
    favorite: bool,
    seed: Seed,
}

struct Ranked {
    tier: u8,
    hit: Hit,
}

#[derive(Serialize, Debug, Clone)]
pub struct Row {
    #[serde(skip)]
    pub slot: Slot,
    pub id: String,
    pub name: String,
    /// What to put in the message when it differs from `name` (a YouTube
    /// unicode emoji inserts the character itself).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    pub provider: &'static str,
    pub source_label: &'static str,
    /// The label without the provider ("Global", "Channel Sub", "Personal"),
    /// for surfaces that show the provider as a logo.
    pub source_detail: &'static str,
    /// Where the typed text sits in `name` (byte offset and length), for
    /// highlighting. Absent while browsing and for names it cannot place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_at: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_len: Option<usize>,
    /// Section heading, only when browsing with nothing typed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_zero_width: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_flags: Option<u32>,
}

pub struct Collector<'a> {
    ctx: Context<'a>,
    profile: Profile,
    order: Order,
    contains: bool,
    limit: usize,
    /// The query with any colon trigger removed, case kept.
    needle: String,
    needle_lower: String,
    /// Search with nothing typed: everything matches, grouped.
    browse: bool,
    /// Nothing can match (an @ query, a bare colon, an empty cycle query).
    inert: bool,
    hits: [Vec<Hit>; Slot::COUNT],
}

impl<'a> Collector<'a> {
    pub fn new(spec: Spec<'_>, ctx: Context<'a>) -> Self {
        let limit = spec.limit.clamp(1, MAX_LIMIT);
        let (needle, order, inert) = match spec.profile {
            Profile::Cycle => {
                if spec.query.starts_with('@') {
                    (String::new(), Order::Default, true)
                } else if let Some(rest) = spec.query.strip_prefix(':') {
                    let n = rest.strip_suffix(':').unwrap_or(rest).to_string();
                    let inert = n.is_empty();
                    (n, Order::TwitchFirst, inert)
                } else {
                    (spec.query.to_string(), Order::Default, spec.query.is_empty())
                }
            }
            Profile::Search => {
                let rest = spec.query.strip_prefix(':').unwrap_or(spec.query);
                let n = rest.strip_suffix(':').unwrap_or(rest).to_string();
                (n, spec.order, false)
            }
        };
        let browse = spec.profile == Profile::Search && needle.is_empty();
        Collector {
            ctx,
            profile: spec.profile,
            order,
            contains: spec.contains || spec.profile == Profile::Search,
            limit,
            needle_lower: needle.to_lowercase(),
            needle,
            browse,
            inert,
            hits: Default::default(),
        }
    }

    fn kind_of(&self, name: &str) -> Option<(Kind, Option<usize>)> {
        if self.browse {
            return Some((Kind::Prefix, None));
        }
        let q = self.needle_lower.as_str();
        if name.is_ascii() && q.is_ascii() {
            let (nb, qb) = (name.as_bytes(), q.as_bytes());
            if nb.len() < qb.len() {
                return None;
            }
            if nb[..qb.len()].eq_ignore_ascii_case(qb) {
                if nb.len() == qb.len() {
                    let kind = if name == self.needle { Kind::ExactCase } else { Kind::Exact };
                    return Some((kind, Some(0)));
                }
                return Some((Kind::Prefix, Some(0)));
            }
            if !self.contains {
                return None;
            }
            let mut found = None;
            for i in 1..=nb.len() - qb.len() {
                if nb[i..i + qb.len()].eq_ignore_ascii_case(qb) {
                    if starts_word(nb, i) {
                        return Some((Kind::WordStart, Some(i)));
                    }
                    if found.is_none() {
                        found = Some((Kind::Contains, Some(i)));
                    }
                }
            }
            return found;
        }
        // Lowercasing can change byte lengths outside ASCII, so a match in a
        // non-ASCII name is ranked but not placed.
        let lower = name.to_lowercase();
        if lower == q {
            let kind = if name == self.needle { Kind::ExactCase } else { Kind::Exact };
            Some((kind, None))
        } else if lower.starts_with(q) {
            Some((Kind::Prefix, None))
        } else if self.contains && lower.contains(q) {
            Some((Kind::Contains, None))
        } else {
            None
        }
    }

    /// Consider one emote. `seed` runs only when the name matched.
    pub fn offer(
        &mut self,
        slot: Slot,
        name: &str,
        id: &str,
        ffz_sub_only: bool,
        seed: impl FnOnce() -> Seed,
    ) {
        if self.inert || (ffz_sub_only && !self.ctx.ffz_subwoofer) {
            return;
        }
        // YouTube emoji names are `:shortcut:`; what people type is the inside.
        let (key, lead) = if slot == Slot::YouTube {
            let inner = name.trim_start_matches(':');
            (inner.trim_end_matches(':'), name.len() - inner.len())
        } else {
            (name, 0)
        };
        let Some((kind, at)) = self.kind_of(key) else {
            return;
        };
        let favorite = self.ctx.favorites.contains(id);
        self.hits[slot.index()].push(Hit {
            slot,
            name: name.to_string(),
            kind,
            at: at.map(|a| a + lead),
            favorite,
            seed: seed(),
        });
    }

    pub fn offer_emote(&mut self, slot: Slot, e: &Emote) {
        let channel_id = self.ctx.channel_id;
        let globals = self.ctx.seventv_globals;
        self.offer(slot, &e.name, &e.id, e.ffz_sub_only.unwrap_or(false), || {
            let global = match slot {
                Slot::Twitch => twitch_type_is_global(e.emote_type.as_deref()),
                Slot::SevenTv => {
                    globals.contains(&e.id) || e.emote_type.as_deref() == Some(PERSONAL_EMOTE_TYPE)
                }
                Slot::Kick => kick_set_is_global(e.emote_type.as_deref().unwrap_or_default()),
                _ => e.emote_type.as_deref() == Some(GLOBAL_EMOTE_TYPE),
            };
            let own = match slot {
                Slot::Twitch => channel_id.is_some() && e.owner_id.as_deref() == channel_id,
                _ => !global,
            };
            Seed {
                id: e.id.clone(),
                url: e.url.clone(),
                insert_text: None,
                emote_type: e.emote_type.clone(),
                is_zero_width: e.is_zero_width,
                modifier_flags: e.modifier_flags,
                global,
                own,
            }
        });
    }

    pub fn offer_set(&mut self, set: &EmoteSet) {
        for e in &set.twitch {
            self.offer_emote(Slot::Twitch, e);
        }
        for e in &set.seven_tv {
            self.offer_emote(Slot::SevenTv, e);
        }
        for e in &set.bttv {
            self.offer_emote(Slot::Bttv, e);
        }
        for e in &set.ffz {
            self.offer_emote(Slot::Ffz, e);
        }
        for e in &set.kick {
            self.offer_emote(Slot::Kick, e);
        }
    }

    /// Whether a 7TV id is in the global set, for callers building their own seeds.
    pub fn is_seventv_global(&self, id: &str) -> bool {
        self.ctx.seventv_globals.contains(id)
    }

    /// Deduplicate, rank and cap. Returns the rows and how many matched in all.
    pub fn finish(self) -> (Vec<Row>, usize) {
        let match_len = (!self.needle.is_empty()).then(|| self.needle.len());
        let Collector { profile, order, limit, browse, mut hits, .. } = self;
        let mut seen: HashSet<String> = HashSet::new();
        let mut merged: Vec<Ranked> = Vec::new();
        // Walking in tier order is what makes the better provider keep a
        // name both of them define.
        for (tier, slot) in order.slots().iter().enumerate() {
            for hit in std::mem::take(&mut hits[slot.index()]) {
                let key = match profile {
                    Profile::Cycle => hit.name.to_lowercase(),
                    Profile::Search => hit.name.clone(),
                };
                if seen.insert(key) {
                    merged.push(Ranked { tier: tier as u8, hit });
                }
            }
        }
        let total = merged.len();
        let cmp = |a: &Ranked, b: &Ranked| compare(profile, browse, a, b);
        if browse {
            merged.sort_by(cmp);
            if merged.len() > limit {
                merged = share_between_groups(merged, limit);
            }
        } else {
            if merged.len() > limit {
                merged.select_nth_unstable_by(limit - 1, cmp);
                merged.truncate(limit);
            }
            merged.sort_by(cmp);
        }
        let rows = merged
            .into_iter()
            .map(|r| {
                let h = r.hit;
                let label = source_label(h.slot, &h.seed);
                let detail = source_detail(h.slot, &h.seed);
                Row {
                    slot: h.slot,
                    group: browse.then(|| GROUPS[group_rank(&h) as usize]),
                    id: h.seed.id,
                    name: h.name,
                    insert_text: h.seed.insert_text,
                    url: h.seed.url,
                    local_path: None,
                    provider: h.slot.wire(),
                    source_label: label,
                    source_detail: detail,
                    match_at: h.at.filter(|_| match_len.is_some()),
                    match_len: h.at.and(match_len),
                    is_zero_width: h.seed.is_zero_width,
                    modifier_flags: h.seed.modifier_flags,
                }
            })
            .collect();
        (rows, total)
    }
}

/// Whether byte `i` of an ASCII name begins a word: a capital after a lowercase
/// letter (`vulp|Love`), a letter after a digit or the reverse, or anything
/// after punctuation (`peepo_|lol`).
fn starts_word(name: &[u8], i: usize) -> bool {
    let (prev, cur) = (name[i - 1], name[i]);
    !prev.is_ascii_alphanumeric()
        || (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
        || (prev.is_ascii_digit() != cur.is_ascii_digit())
}

/// Emote types from Twitch's user-emotes endpoint that every account has.
fn twitch_type_is_global(t: Option<&str>) -> bool {
    matches!(t, None | Some("" | "globals" | "smilies" | "twofactor" | "owl2019" | "none"))
}

/// Kick's shared sets, as opposed to a channel's own emotes.
pub fn kick_set_is_global(set: &str) -> bool {
    set == "Global" || set == "Emojis"
}

fn source_label(slot: Slot, seed: &Seed) -> &'static str {
    match slot {
        Slot::Twitch => match seed.emote_type.as_deref() {
            Some("subscriptions") if seed.own => "Channel Sub",
            Some("subscriptions") => "Sub",
            Some("follower") => "Follower",
            Some("bitstier") => "Bits",
            Some("channelpoints") => "Channel Points",
            Some("hypetrain") => "Hype Train",
            Some("prime") => "Prime",
            Some("turbo") => "Turbo",
            Some("rewards" | "limitedtime") => "Reward",
            _ => "Twitch Global",
        },
        Slot::SevenTv if seed.emote_type.as_deref() == Some(PERSONAL_EMOTE_TYPE) => "7TV Personal",
        Slot::SevenTv if seed.global => "7TV Global",
        Slot::SevenTv => "7TV",
        Slot::Bttv if seed.global => "BTTV Global",
        Slot::Bttv => "BTTV",
        Slot::Ffz if seed.global => "FFZ Global",
        Slot::Ffz => "FFZ",
        Slot::Kick => match seed.emote_type.as_deref() {
            Some("Global") => "Kick Global",
            Some("Emojis") => "Kick Emoji",
            _ => "Kick",
        },
        Slot::YouTube if seed.global => "Emoji",
        Slot::YouTube => "YouTube",
    }
}

/// The source label with the provider left off, for a surface that shows the
/// provider as its logo.
fn source_detail(slot: Slot, seed: &Seed) -> &'static str {
    let label = source_label(slot, seed);
    let provider = match slot {
        Slot::Twitch => "Twitch",
        Slot::SevenTv => "7TV",
        Slot::Bttv => "BTTV",
        Slot::Ffz => "FFZ",
        Slot::Kick => "Kick",
        Slot::YouTube => "YouTube",
    };
    match label.strip_prefix(provider).map(str::trim_start) {
        Some("") => "Channel",
        Some(rest) => rest,
        None => label,
    }
}

const GROUPS: [&str; 8] = ["Favorites", "This channel", "Twitch", "7TV", "BTTV", "FFZ", "Kick", "YouTube"];

fn group_rank(h: &Hit) -> u8 {
    if h.favorite {
        return 0;
    }
    if h.seed.own {
        return 1;
    }
    match h.slot {
        Slot::Twitch => 2,
        Slot::SevenTv => 3,
        Slot::Bttv => 4,
        Slot::Ffz => 5,
        Slot::Kick => 6,
        Slot::YouTube => 7,
    }
}

/// Cut a browse list (already sorted by group) to `limit` so that every group
/// keeps some rows. Without this, someone subscribed to many channels fills the
/// whole list with Twitch emotes and never sees a 7TV, BTTV or FFZ global.
/// Rows are shared evenly, and whatever a small group does not need passes to
/// the others.
fn share_between_groups(sorted: Vec<Ranked>, limit: usize) -> Vec<Ranked> {
    let mut counts = [0usize; GROUPS.len()];
    for r in &sorted {
        counts[group_rank(&r.hit) as usize] += 1;
    }
    let mut quota = [0usize; GROUPS.len()];
    let mut left = limit;
    loop {
        let open: Vec<usize> = (0..GROUPS.len()).filter(|&g| quota[g] < counts[g]).collect();
        if open.is_empty() || left == 0 {
            break;
        }
        let share = (left / open.len()).max(1);
        for g in open {
            let add = share.min(counts[g] - quota[g]).min(left);
            quota[g] += add;
            left -= add;
            if left == 0 {
                break;
            }
        }
    }
    let mut taken = [0usize; GROUPS.len()];
    sorted
        .into_iter()
        .filter(|r| {
            let g = group_rank(&r.hit) as usize;
            taken[g] += 1;
            taken[g] <= quota[g]
        })
        .collect()
}

fn compare(profile: Profile, browse: bool, a: &Ranked, b: &Ranked) -> Ordering {
    let (x, y) = (&a.hit, &b.hit);
    match profile {
        Profile::Cycle => a
            .tier
            .cmp(&b.tier)
            .then((!x.favorite).cmp(&!y.favorite))
            .then_with(|| alpha(&x.name, &y.name)),
        Profile::Search if browse => group_rank(x)
            .cmp(&group_rank(y))
            .then_with(|| alpha(&x.name, &y.name)),
        Profile::Search => x
            .kind
            .cmp(&y.kind)
            .then((!x.favorite).cmp(&!y.favorite))
            .then(a.tier.cmp(&b.tier))
            .then((!x.seed.own).cmp(&!y.seed.own))
            .then(x.name.chars().count().cmp(&y.name.chars().count()))
            .then_with(|| alpha(&x.name, &y.name)),
    }
}

/// Case-insensitive name order, lowercase first on a tie, which is how the
/// page's own collation ordered the cycle before it moved here.
fn alpha(a: &str, b: &str) -> Ordering {
    let fold = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<Vec<_>>();
    fold(a)
        .cmp(&fold(b))
        .then_with(|| a.chars().map(char::is_uppercase).cmp(b.chars().map(char::is_uppercase)))
        .then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emote(provider: EmoteProvider, id: &str, name: &str, emote_type: Option<&str>, owner: Option<&str>) -> Emote {
        Emote {
            id: id.to_string(),
            name: name.to_string(),
            url: format!("https://example.invalid/{id}"),
            provider,
            is_zero_width: None,
            local_url: None,
            emote_type: emote_type.map(String::from),
            owner_id: owner.map(String::from),
            owner_name: None,
            width: None,
            modifier_flags: None,
            ffz_sub_only: None,
        }
    }

    fn set() -> EmoteSet {
        let mut s = EmoteSet::new();
        s.twitch = vec![
            emote(EmoteProvider::Twitch, "t1", "Kappa", Some("globals"), Some("0")),
            emote(EmoteProvider::Twitch, "t2", "vulpLove", Some("subscriptions"), Some("42")),
            emote(EmoteProvider::Twitch, "t3", "tlouLol", Some("subscriptions"), Some("99")),
            emote(EmoteProvider::Twitch, "t4", "LUL", Some("globals"), Some("0")),
        ];
        s.seven_tv = vec![
            emote(EmoteProvider::SevenTV, "s1", "Kappa", None, None),
            emote(EmoteProvider::SevenTV, "s2", "Loading", None, None),
            emote(EmoteProvider::SevenTV, "s3", "glorp", None, None),
            emote(EmoteProvider::SevenTV, "s4", "lol", None, None),
        ];
        s.bttv = vec![
            emote(EmoteProvider::BTTV, "b1", "LOADING", Some(GLOBAL_EMOTE_TYPE), None),
            emote(EmoteProvider::BTTV, "b2", "blobDance", None, None),
        ];
        s.ffz = vec![emote(EmoteProvider::FFZ, "f1", "LoL", None, None)];
        s
    }

    struct Fixture {
        favorites: HashSet<String>,
        globals: HashSet<String>,
    }

    impl Fixture {
        fn new() -> Self {
            Fixture {
                favorites: HashSet::new(),
                globals: ["s3".to_string()].into_iter().collect(),
            }
        }

        fn run(&self, query: &str, profile: Profile, order: Order, contains: bool) -> (Vec<Row>, usize) {
            let mut c = Collector::new(
                Spec { query, profile, order, contains, limit: 50 },
                Context {
                    channel_id: Some("42"),
                    favorites: &self.favorites,
                    seventv_globals: &self.globals,
                    ffz_subwoofer: false,
                },
            );
            c.offer_set(&set());
            c.finish()
        }
    }

    fn names(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn cycle_keeps_provider_tier_then_name() {
        let f = Fixture::new();
        let (rows, _) = f.run("lo", Profile::Cycle, Order::Default, false);
        // 7TV first (Loading, lol), then the BTTV/FFZ names not already taken
        // case-insensitively (LOADING and LoL both are), then Twitch.
        assert_eq!(names(&rows), vec!["Loading", "lol"]);
    }

    #[test]
    fn cycle_colon_puts_twitch_first() {
        let f = Fixture::new();
        let (rows, _) = f.run(":ka", Profile::Cycle, Order::Default, false);
        assert_eq!(rows[0].provider, "twitch");
        assert_eq!(rows.len(), 1, "the 7TV Kappa folds into the Twitch one");
        let (rows, _) = f.run(":Kappa:", Profile::Cycle, Order::Default, false);
        assert_eq!(names(&rows), vec!["Kappa"]);
    }

    #[test]
    fn cycle_at_and_bare_colon_match_nothing() {
        let f = Fixture::new();
        assert!(f.run("@lo", Profile::Cycle, Order::Default, true).0.is_empty());
        assert!(f.run(":", Profile::Cycle, Order::Default, true).0.is_empty());
        assert!(f.run("", Profile::Cycle, Order::Default, true).0.is_empty());
    }

    #[test]
    fn cycle_favorite_never_jumps_a_tier() {
        let mut f = Fixture::new();
        f.favorites.insert("t2".to_string());
        let (rows, _) = f.run("l", Profile::Cycle, Order::Default, true);
        let pos = |n: &str| rows.iter().position(|r| r.name == n).unwrap();
        assert!(pos("Loading") < pos("vulpLove"));
    }

    #[test]
    fn search_ranks_match_quality_first() {
        let f = Fixture::new();
        let (rows, _) = f.run(":lol", Profile::Search, Order::TwitchFirst, true);
        // Exact-case "lol" leads, then the case-insensitive exact "LoL", then
        // prefix, then contains.
        assert_eq!(rows[0].name, "lol");
        assert_eq!(rows[1].name, "LoL");
        assert!(names(&rows).contains(&"tlouLol"));
    }

    #[test]
    fn search_keeps_names_that_differ_only_by_case() {
        let f = Fixture::new();
        let (rows, total) = f.run("load", Profile::Search, Order::Default, false);
        assert_eq!(total, 2);
        assert!(names(&rows).contains(&"LOADING"));
        assert!(names(&rows).contains(&"Loading"));
    }

    #[test]
    fn starts_with_mode_drops_contains_matches_in_the_cycle() {
        let f = Fixture::new();
        let (rows, _) = f.run("lo", Profile::Cycle, Order::Default, false);
        assert!(!names(&rows).contains(&"blobDance"));
        let (rows, _) = f.run("lo", Profile::Cycle, Order::Default, true);
        assert!(names(&rows).contains(&"blobDance"));
    }

    #[test]
    fn the_list_finds_channel_emotes_by_the_word_after_the_prefix() {
        let f = Fixture::new();
        // Even with the "starts with" setting, the list looks inside names.
        let (rows, _) = f.run(":lo", Profile::Search, Order::TwitchFirst, false);
        let pos = |n: &str| rows.iter().position(|r| r.name == n).unwrap();
        // Prefix matches, then the word inside a name, then plain contains.
        assert!(pos("lol") < pos("vulpLove"));
        assert!(pos("vulpLove") < pos("blobDance"));
        assert!(pos("tlouLol") < pos("blobDance"));
    }

    #[test]
    fn rows_say_where_the_text_matched_and_what_kind_of_emote_it_is() {
        let f = Fixture::new();
        let (rows, _) = f.run(":lo", Profile::Search, Order::TwitchFirst, false);
        let row = |n: &str| rows.iter().find(|r| r.name == n).unwrap();
        assert_eq!((row("vulpLove").match_at, row("vulpLove").match_len), (Some(4), Some(2)));
        assert_eq!(row("lol").match_at, Some(0));
        assert_eq!(row("vulpLove").source_detail, "Channel Sub");
        assert_eq!(row("Loading").source_detail, "Channel");
        let (rows, _) = f.run("", Profile::Search, Order::TwitchFirst, false);
        let row = |n: &str| rows.iter().find(|r| r.name == n).unwrap();
        assert_eq!(row("glorp").source_detail, "Global");
        assert_eq!(row("Kappa").source_detail, "Global");
        assert_eq!(row("glorp").match_at, None);
    }

    #[test]
    fn word_starts() {
        assert!(starts_word(b"vulpLove", 4));
        assert!(starts_word(b"peepo_lol", 6));
        assert!(starts_word(b"x2Lo", 2));
        assert!(!starts_word(b"blobDance", 1));
        assert!(!starts_word(b"tlouWTF", 1));
    }

    #[test]
    fn labels_say_where_an_emote_comes_from() {
        let f = Fixture::new();
        // Twitch first, so the shared name "Kappa" is Twitch's row.
        let (rows, _) = f.run("", Profile::Search, Order::TwitchFirst, false);
        let label = |n: &str| rows.iter().find(|r| r.name == n).unwrap().source_label;
        assert_eq!(label("Kappa"), "Twitch Global");
        assert_eq!(label("vulpLove"), "Channel Sub");
        assert_eq!(label("tlouLol"), "Sub");
        assert_eq!(label("glorp"), "7TV Global");
        assert_eq!(label("Loading"), "7TV");
        assert_eq!(label("LOADING"), "BTTV Global");
        assert_eq!(label("blobDance"), "BTTV");
    }

    #[test]
    fn browse_groups_favorites_then_channel_then_providers() {
        let mut f = Fixture::new();
        f.favorites.insert("t4".to_string());
        let (rows, _) = f.run("", Profile::Search, Order::Default, false);
        assert_eq!(rows[0].name, "LUL");
        assert_eq!(rows[0].group, Some("Favorites"));
        let groups: Vec<&str> = rows.iter().filter_map(|r| r.group).collect();
        let first = |g: &str| groups.iter().position(|x| *x == g).unwrap();
        assert!(first("This channel") < first("Twitch"));
        assert!(first("Twitch") < first("7TV"));
    }

    #[test]
    fn browse_keeps_every_group_when_twitch_alone_could_fill_it() {
        let favorites = HashSet::new();
        let globals: HashSet<String> = (0..5).map(|i| format!("g{i}")).collect();
        let mut c = Collector::new(
            Spec { query: "", profile: Profile::Search, order: Order::Default, contains: false, limit: 100 },
            Context { channel_id: Some("42"), favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        );
        for i in 0..400 {
            c.offer_emote(Slot::Twitch, &emote(EmoteProvider::Twitch, &format!("t{i}"), &format!("sub{i:03}"), Some("subscriptions"), Some("99")));
        }
        for i in 0..5 {
            c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, &format!("g{i}"), &format!("glob{i}"), None, None));
        }
        c.offer_emote(Slot::Bttv, &emote(EmoteProvider::BTTV, "b1", "bttvOne", Some(GLOBAL_EMOTE_TYPE), None));
        let (rows, total) = c.finish();
        assert_eq!(total, 406);
        assert_eq!(rows.len(), 100);
        let count = |g: &str| rows.iter().filter(|r| r.group == Some(g)).count();
        assert_eq!(count("7TV"), 5);
        assert_eq!(count("BTTV"), 1);
        assert_eq!(count("Twitch"), 94);
    }

    #[test]
    fn personal_emotes_are_labelled() {
        let favorites = HashSet::new();
        let globals = HashSet::new();
        let mut c = Collector::new(
            Spec { query: "my", profile: Profile::Search, order: Order::Default, contains: false, limit: 10 },
            Context { channel_id: None, favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        );
        c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, "p1", "myEmote", Some(PERSONAL_EMOTE_TYPE), None));
        assert_eq!(c.finish().0[0].source_label, "7TV Personal");
    }

    #[test]
    fn ffz_sub_only_offered_to_subscribers_only() {
        let favorites = HashSet::new();
        let globals = HashSet::new();
        let mut e = emote(EmoteProvider::FFZ, "f9", "ffzWiggle", None, None);
        e.ffz_sub_only = Some(true);
        for (sub, expected) in [(false, 0), (true, 1)] {
            let mut c = Collector::new(
                Spec { query: "ffz", profile: Profile::Search, order: Order::Default, contains: false, limit: 10 },
                Context { channel_id: None, favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: sub },
            );
            c.offer_emote(Slot::Ffz, &e);
            assert_eq!(c.finish().0.len(), expected);
        }
    }

    #[test]
    fn youtube_shortcuts_match_without_their_colons() {
        let favorites = HashSet::new();
        let globals = HashSet::new();
        let mut c = Collector::new(
            Spec { query: "face", profile: Profile::Cycle, order: Order::Default, contains: false, limit: 10 },
            Context { channel_id: None, favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        );
        c.offer(Slot::YouTube, ":face-blue-smiling:", "UCx/abc", false, || Seed {
            id: "UCx/abc".into(),
            url: String::new(),
            insert_text: None,
            emote_type: None,
            is_zero_width: None,
            modifier_flags: None,
            global: false,
            own: true,
        });
        let (rows, _) = c.finish();
        assert_eq!(rows[0].name, ":face-blue-smiling:");
    }

    #[test]
    fn limit_caps_rows_and_total_counts_all() {
        let favorites = HashSet::new();
        let globals = HashSet::new();
        let mut c = Collector::new(
            Spec { query: "e", profile: Profile::Search, order: Order::Default, contains: false, limit: 500 },
            Context { channel_id: None, favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        );
        for i in 0..300 {
            c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, &format!("x{i}"), &format!("e{i:03}"), None, None));
        }
        let (rows, total) = c.finish();
        assert_eq!(rows.len(), MAX_LIMIT);
        assert_eq!(total, 300);
        assert_eq!(rows[0].name, "e000");
        assert_eq!(rows[MAX_LIMIT - 1].name, format!("e{:03}", MAX_LIMIT - 1));
    }

    #[test]
    fn a_large_set_ranks_quickly() {
        let mut big = EmoteSet::new();
        big.seven_tv = (0..10_000)
            .map(|i| emote(EmoteProvider::SevenTV, &format!("s{i}"), &format!("emote{i}Lo"), None, None))
            .collect();
        let favorites = HashSet::new();
        let globals = HashSet::new();
        let started = std::time::Instant::now();
        let mut c = Collector::new(
            Spec { query: "lo", profile: Profile::Search, order: Order::Default, contains: true, limit: 60 },
            Context { channel_id: None, favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        );
        c.offer_set(&big);
        let (rows, total) = c.finish();
        assert_eq!(total, 10_000);
        assert_eq!(rows.len(), 60);
        // A guard against an accidental quadratic path, generous for debug builds.
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
    }

    #[test]
    fn alpha_is_case_insensitive_with_lowercase_first() {
        assert_eq!(alpha("apple", "Banana"), Ordering::Less);
        assert_eq!(alpha("kappa", "Kappa"), Ordering::Less);
    }
}
