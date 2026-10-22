//! Emote name matching for the chat composer.
//!
//! The Tab cycle and the emote search list both rank here, over the sets Rust
//! already holds, so no window keeps its own copy of the matching rules or of
//! the emote sets they run over.
//!
//! Two profiles:
//! - `Cycle` is the Tab cycle: provider tier, then favorites, then name. A
//!   leading `:` moves Twitch to the front; a leading `@` is a chatter query
//!   and matches no emote.
//! - `Search` is the list: how well the name matches what was typed comes
//!   first, so an exact Twitch match is never buried under provider order. It
//!   always looks inside names too, because a Twitch channel emote starts with
//!   the streamer's prefix (`vulpLove`), so "starts with lo" could never find it.
//!
//! Both match case-insensitively but keep names EXACTLY: `LULE` and `lule` are
//! different tokens that render as different emotes, so both are offered. Only
//! an identical name defined by two providers collapses to one row, and a
//! Twitch emote always keeps its name (Twitch tags it in the sent message).
//! Names that differ only by case sit next to each other, the one whose case
//! matches what was typed first.
//!
//! Callers hand names in with `offer`; the rest of a row (`Seed`) is only built
//! for names that matched, so a scan over thousands of names allocates for the
//! handful that survive.

use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{OnceLock, RwLock};

use crate::models::settings::Settings;
use crate::services::emote_service::{Emote, EmoteProvider, EmoteSet};

/// Marks a BTTV or FFZ row that came from the provider's global set.
pub const GLOBAL_EMOTE_TYPE: &str = "global";

/// Marks a 7TV row from the viewer's own personal set, usable in every channel.
pub const PERSONAL_EMOTE_TYPE: &str = "personal";

/// A personal row whose name the channel also uses: the channel's emote is
/// hidden behind it in the viewer's own messages.
const PERSONAL_REPLACES_LABEL: &str = "7TV Personal, replaces channel's";
const PERSONAL_REPLACES_DETAIL: &str = "Personal, replaces channel's";

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

/// Which provider keeps a name two of them define, the order chat draws them
/// in (`EmoteLookup::build`): Twitch first (a name Twitch would tag in the sent
/// message is Twitch's emote), then the 7TV extension's 7TV > FFZ > BetterTTV.
const PRECEDENCE: [Slot; Slot::COUNT] = [Slot::Twitch, Slot::SevenTv, Slot::Ffz, Slot::Bttv, Slot::Kick, Slot::YouTube];

/// The emote fill menu's row cap.
pub const FILL_LIMIT: usize = 150;

/// What one `match_emote_tokens` call asks, resolved from what the caller sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub profile: Profile,
    pub order: Order,
    pub limit: usize,
    pub typing: bool,
    /// The room's recent chatters follow the emotes (`chatter_rows`).
    pub chatters: bool,
}

impl Request {
    /// `profile` "fill" is the emote fill menu, asked the same way by the
    /// desktop's colon list and both phones' suggestion strip, so one word gets
    /// one list everywhere: ranked by closeness (`Profile::Search`), Twitch
    /// first, as the viewer types, with the room's chatters after the emotes for
    /// a plain word (a `:` word names none). "cycle" and "search" keep the
    /// caller's own order and typing.
    pub fn resolve(profile: Option<&str>, order: Option<&str>, limit: Option<usize>, typing: Option<bool>) -> Self {
        if profile == Some("fill") {
            return Request {
                profile: Profile::Search,
                order: Order::TwitchFirst,
                limit: limit.unwrap_or(FILL_LIMIT),
                typing: true,
                chatters: true,
            };
        }
        let profile = match profile {
            Some("search") => Profile::Search,
            _ => Profile::Cycle,
        };
        let order = match order {
            Some("twitch_first") => Order::TwitchFirst,
            _ => Order::Default,
        };
        let limit = limit.unwrap_or(match profile {
            Profile::Cycle => 50,
            Profile::Search => 60,
        });
        Request { profile, order, limit, typing: typing.unwrap_or(false), chatters: profile == Profile::Cycle }
    }
}

// "Contains" instead of "starts with", from chat_input.emote_tab_complete_match_mode.
static CONTAINS_MODE: AtomicBool = AtomicBool::new(false);

// Chatter names after the emotes in the Tab cycle and the phone strip, from
// chat_input.emote_tab_complete_include_chatters (on unless turned off).
static INCLUDE_CHATTERS: AtomicBool = AtomicBool::new(true);

/// The viewer's nicknames for chatters (user id -> nickname), from
/// chat_customization.user_overrides, so a nickname finds its chatter too.
fn nicknames() -> &'static RwLock<HashMap<String, String>> {
    static NICKS: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    NICKS.get_or_init(Default::default)
}

/// Pick up the match mode from saved settings. Runs on boot and every save.
pub fn refresh_settings(settings: &Settings) {
    let includes = settings
        .chat_input
        .extra
        .get("emote_tab_complete_match_mode")
        .and_then(|v| v.as_str())
        == Some("includes");
    CONTAINS_MODE.store(includes, AtomicOrdering::Relaxed);
    let chatters = settings
        .chat_input
        .extra
        .get("emote_tab_complete_include_chatters")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    INCLUDE_CHATTERS.store(chatters, AtomicOrdering::Relaxed);
    let nicks: HashMap<String, String> = settings
        .extra
        .get("chat_customization")
        .and_then(|g| g.get("user_overrides"))
        .and_then(|o| o.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(id, v)| {
                    let nick = v.get("nickname")?.as_str()?.trim();
                    (!nick.is_empty()).then(|| (id.clone(), nick.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    if let Ok(mut guard) = nicknames().write() {
        *guard = nicks;
    }
}

/// The most chatter rows one answer adds.
pub const MAX_CHATTERS: usize = 5;
/// The Tab cycle's and the phone strip's row cap, chatters included.
pub const CYCLE_CAP: usize = 50;

/// While the viewer types, the fewest characters (a leading colon aside) that
/// get the full ranked list; a shorter word offers only an emote of exactly
/// that name. One letter is enough: the ranking already puts exact names and
/// prefixes first, ahead of names that only contain the letter.
pub const MIN_PREFIX_CHARS: usize = 1;

/// Whether `word` is under `MIN_PREFIX_CHARS` once a colon trigger is set
/// aside. An `@` word names a chatter and is not short in this sense.
pub fn is_short_word(word: &str) -> bool {
    if word.starts_with('@') {
        return false;
    }
    let rest = word.strip_prefix(':').unwrap_or(word);
    let rest = rest.strip_suffix(':').unwrap_or(rest);
    rest.chars().count() < MIN_PREFIX_CHARS
}

/// What a chatter row inserts: the display name when it differs from the
/// login only by case, else the login (a localized display name mentions
/// nobody), with `@` when the word had one.
fn chatter_insert(login: &str, display: &str, at: bool) -> String {
    let name = if !display.is_empty() && display.eq_ignore_ascii_case(login) { display } else { login };
    format!("{}{name}", if at { "@" } else { "" })
}

/// Chatter rows for the cycle `word` after `emotes`: recent chatters of
/// `channel` (newest first) whose login, display name or nickname starts with
/// the word (contains it, in "includes" mode), minus a leading `@`. None for a
/// `:` word, once `emotes` fills the cap, or while the setting is off. A name
/// an emote row already shows is skipped.
pub fn chatter_rows(
    word: &str,
    emotes: &[Row],
    contains: bool,
    chatters: &[(String, String, String)],
) -> Vec<Row> {
    if !INCLUDE_CHATTERS.load(AtomicOrdering::Relaxed) || word.starts_with(':') || emotes.len() >= CYCLE_CAP {
        return Vec::new();
    }
    let at = word.starts_with('@');
    let needle = word.trim_start_matches('@').to_lowercase();
    // A bare `@` lists the room's chatters, the streamer first; any other empty
    // word lists none.
    if needle.is_empty() && !at {
        return Vec::new();
    }
    let nicks = nicknames().read().map(|g| g.clone()).unwrap_or_default();
    let hit = |s: &str| {
        let s = s.to_lowercase();
        if contains { s.contains(&needle) } else { s.starts_with(&needle) }
    };
    let mut taken: HashSet<String> = emotes.iter().map(|r| r.name.to_lowercase()).collect();
    let room = (CYCLE_CAP - emotes.len()).min(MAX_CHATTERS);
    let mut out = Vec::new();
    for (login, user_id, display) in chatters {
        if out.len() >= room {
            break;
        }
        let shown = if display.is_empty() { login.as_str() } else { display.as_str() };
        let nick = nicks.get(user_id).map(String::as_str);
        if !(hit(login) || hit(shown) || nick.is_some_and(hit)) {
            continue;
        }
        if !taken.insert(shown.to_lowercase()) {
            continue;
        }
        out.push(Row {
            slot: Slot::Twitch,
            id: user_id.clone(),
            name: format!("{}{shown}", if at { "@" } else { "" }),
            insert_text: Some(chatter_insert(login, shown, at)),
            url: String::new(),
            local_path: None,
            provider: "user",
            source_label: "Chatter",
            source_detail: "Chatter",
            match_at: None,
            match_len: None,
            group: None,
            is_zero_width: None,
            modifier_flags: None,
            emote_type: None,
            owner_name: None,
            original_name: None,
            animated: None,
            width: None,
            favorite: false,
            locked: false,
            login: Some(login.clone()),
        });
    }
    out
}

/// The history ring a room's rows are filed under: a Twitch room by its login,
/// any other platform by its composite key (`kick:slug`), as its rows carry it.
pub fn history_room(provider: &str, channel: &str) -> String {
    let channel = channel.trim().trim_start_matches('#');
    if provider == "twitch" || channel.contains(':') {
        channel.to_string()
    } else {
        crate::services::providers::key::make_key(provider, channel)
    }
}

/// The room's streamer among `recent` chatters: the row with the room's login,
/// or with the room's id (a YouTube room is keyed by its channel id).
pub fn find_owner(
    login: &str,
    channel_id: Option<&str>,
    recent: &[(String, String, String)],
) -> Option<(String, String, String)> {
    recent
        .iter()
        .find(|(l, id, _)| l.eq_ignore_ascii_case(login) || channel_id.is_some_and(|c| !c.is_empty() && c == id))
        .cloned()
}

/// `recent` with the room's streamer at the front (moved there when they have
/// chatted, never listed twice), so every chatter list leads with them.
pub fn owner_first(
    owner: Option<(String, String, String)>,
    recent: Vec<(String, String, String)>,
) -> Vec<(String, String, String)> {
    let Some(owner) = owner else { return recent };
    let (login, id) = (owner.0.clone(), owner.1.clone());
    let mut out = Vec::with_capacity(recent.len() + 1);
    out.push(owner);
    out.extend(
        recent
            .into_iter()
            .filter(|(l, i, _)| !(l.eq_ignore_ascii_case(&login) || (!id.is_empty() && *i == id))),
    );
    out
}

/// One name in the composer's `@` list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MentionRow {
    pub login: String,
    pub user_id: String,
    pub display: String,
}

/// The most names the `@` list shows.
pub const MENTION_LIMIT: usize = 5;

/// The composer's `@` list: `chatters` in order (the streamer first, see
/// `owner_first`) whose login, shown name or nickname starts with `query`, a
/// leading `@` set aside. An empty query lists them all. At most `limit`.
/// Nicknames off Twitch are saved under the namespaced chatter id
/// (`kick:<id>`), as the chat rows key them.
pub fn mention_rows(
    provider: &str,
    query: &str,
    chatters: &[(String, String, String)],
    limit: usize,
) -> Vec<MentionRow> {
    let needle = query.trim_start_matches('@').to_lowercase();
    let nicks = nicknames().read().map(|g| g.clone()).unwrap_or_default();
    let hit = |s: &str| s.to_lowercase().starts_with(&needle);
    let nick_key = |id: &str| if provider == "twitch" { id.to_string() } else { format!("{provider}:{id}") };
    chatters
        .iter()
        .filter(|(login, user_id, display)| {
            hit(login) || hit(display) || nicks.get(&nick_key(user_id)).map(String::as_str).is_some_and(hit)
        })
        .take(limit)
        .map(|(login, user_id, display)| MentionRow {
            login: login.clone(),
            user_id: user_id.clone(),
            display: if display.is_empty() { login.clone() } else { display.clone() },
        })
        .collect()
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
#[derive(Default)]
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
    /// Who made it, where the provider says (the emote menu's hover credit).
    pub owner_name: Option<String>,
    /// The emote's own name when this set shows it under an alias.
    pub original_name: Option<String>,
    pub animated: Option<bool>,
    pub width: Option<u32>,
}

struct Hit {
    slot: Slot,
    name: String,
    kind: Kind,
    /// Byte offset of the match in `name`, where it is known (ASCII names).
    at: Option<usize>,
    /// The typed text appears in the name with the same case, so `LU` picks
    /// `LULE` over `lule`.
    case_exact: bool,
    favorite: bool,
    /// An FFZ subscriber effect the viewer cannot use (offered only when the
    /// caller asked for locked rows, so the menu can show it greyed out).
    locked: bool,
    seed: Seed,
}

impl Hit {
    fn is_personal(&self) -> bool {
        self.slot == Slot::SevenTv && self.seed.emote_type.as_deref() == Some(PERSONAL_EMOTE_TYPE)
    }

    /// A channel name lookup the sender's personal set is consulted before.
    fn shadowable(&self) -> bool {
        matches!(self.slot, Slot::SevenTv | Slot::Bttv | Slot::Ffz)
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emote_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub favorite: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// A chatter row's login (provider `user`); absent on emote rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

/// How many emotes matched per provider, counted before the row cap, so the
/// emote menu's tabs can say how many each one holds. `favorites` counts the
/// matches that are favorites, whatever their provider.
pub type ProviderCounts = std::collections::BTreeMap<&'static str, usize>;

/// The `ProviderCounts` key for matches that are favorites.
pub const FAVORITES_COUNT: &str = "favorites";

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
    /// Offer FFZ subscriber effects the viewer cannot use, marked locked.
    include_locked: bool,
    /// A word under `MIN_PREFIX_CHARS` typed while suggestions follow the
    /// typing: only an emote of exactly that name matches.
    exact_only: bool,
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
            include_locked: false,
            exact_only: false,
            hits: Default::default(),
        }
    }

    /// The caller asks as the viewer types (the phones' suggestion strip, the
    /// desktop's colon list), not on Tab or in the emote menu: a word under
    /// `MIN_PREFIX_CHARS` (one letter) then matches only an emote of exactly
    /// that name.
    pub fn typing(mut self, on: bool) -> Self {
        self.exact_only = on && !self.browse && !self.needle.is_empty() && is_short_word(&self.needle);
        self
    }

    /// The emote menu shows FFZ subscriber effects greyed out rather than
    /// hiding them; completion never offers what cannot be sent.
    pub fn include_locked(mut self, on: bool) -> Self {
        self.include_locked = on;
        self
    }

    /// How `name` matches, where, and whether the typed case matches too.
    fn kind_of(&self, name: &str) -> Option<(Kind, Option<usize>, bool)> {
        if self.browse {
            return Some((Kind::Prefix, None, false));
        }
        if self.exact_only {
            // Exact name, exact case: a typed "d" is a word, not the emote "D".
            return (name == self.needle).then_some((Kind::ExactCase, Some(0), true));
        }
        let q = self.needle_lower.as_str();
        if name.is_ascii() && q.is_ascii() && self.needle.len() == q.len() {
            let (nb, qb, typed) = (name.as_bytes(), q.as_bytes(), self.needle.as_bytes());
            if nb.len() < qb.len() {
                return None;
            }
            let same_case = |i: usize| &nb[i..i + typed.len()] == typed;
            if nb[..qb.len()].eq_ignore_ascii_case(qb) {
                if nb.len() == qb.len() {
                    let kind = if name == self.needle { Kind::ExactCase } else { Kind::Exact };
                    return Some((kind, Some(0), same_case(0)));
                }
                return Some((Kind::Prefix, Some(0), same_case(0)));
            }
            if !self.contains {
                return None;
            }
            let mut found = None;
            for i in 1..=nb.len() - qb.len() {
                if nb[i..i + qb.len()].eq_ignore_ascii_case(qb) {
                    if starts_word(nb, i) {
                        return Some((Kind::WordStart, Some(i), same_case(i)));
                    }
                    if found.is_none() {
                        found = Some((Kind::Contains, Some(i), same_case(i)));
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
            Some((kind, None, name == self.needle))
        } else if lower.starts_with(q) {
            Some((Kind::Prefix, None, name.starts_with(self.needle.as_str())))
        } else if self.contains && lower.contains(q) {
            Some((Kind::Contains, None, name.contains(self.needle.as_str())))
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
        let locked = ffz_sub_only && !self.ctx.ffz_subwoofer;
        if self.inert || (locked && !self.include_locked) {
            return;
        }
        // YouTube emoji names are `:shortcut:`; what people type is the inside.
        let (key, lead) = if slot == Slot::YouTube {
            let inner = name.trim_start_matches(':');
            (inner.trim_end_matches(':'), name.len() - inner.len())
        } else {
            (name, 0)
        };
        let Some((kind, at, case_exact)) = self.kind_of(key) else {
            return;
        };
        let favorite = self.ctx.favorites.contains(id);
        self.hits[slot.index()].push(Hit {
            slot,
            name: name.to_string(),
            kind,
            at: at.map(|a| a + lead),
            case_exact,
            favorite,
            locked,
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
                owner_name: e.owner_name.clone(),
                original_name: e.original_name.clone(),
                animated: e.animated,
                width: e.width,
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
        let (rows, total, _) = self.finish_counted();
        (rows, total)
    }

    /// `finish`, plus how many matched per provider before the cap.
    pub fn finish_counted(self) -> (Vec<Row>, usize, ProviderCounts) {
        let match_len = (!self.needle.is_empty()).then(|| self.needle.len());
        let Collector { profile, order, limit, browse, mut hits, .. } = self;
        // In the viewer's own messages a personal emote beats the channel's
        // third-party emote of the same exact name (Twitch's own emotes are
        // tagged by Twitch and still win). Offering the channel's row would
        // preview art that never appears, so the personal row takes the name
        // and says what it replaces.
        let personal: HashMap<String, String> = hits
            .iter()
            .flatten()
            .filter(|h| h.is_personal())
            .map(|h| (h.name.clone(), h.seed.id.clone()))
            .collect();
        // Chat draws a channel's own emote over a global of the same name
        // whatever the provider (the 7TV extension's order), so the menu
        // offers that one too rather than previewing art that never appears.
        let channel_names: HashSet<String> = hits
            .iter()
            .flatten()
            .filter(|h| h.shadowable() && !h.is_personal() && !h.seed.global)
            .map(|h| h.name.clone())
            .collect();
        let mut replaced: HashSet<String> = HashSet::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut merged: Vec<Ranked> = Vec::new();
        let slots = order.slots();
        let tier_of = |slot: Slot| slots.iter().position(|s| *s == slot).unwrap_or(0) as u8;
        // The first provider walked keeps a name two of them define, in the
        // order chat draws them (`PRECEDENCE`), so the menu never previews art
        // that never appears. The ranking order only sorts the rows.
        for slot in PRECEDENCE {
            for hit in std::mem::take(&mut hits[slot.index()]) {
                if hit.shadowable() && !hit.is_personal() {
                    if let Some(mine) = personal.get(&hit.name) {
                        // The same emote in both sets replaces nothing.
                        if *mine != hit.seed.id {
                            replaced.insert(hit.name);
                        }
                        continue;
                    }
                    if hit.seed.global && channel_names.contains(&hit.name) {
                        continue;
                    }
                }
                // Exact name: `LULE` and `lule` are two emotes.
                if seen.insert(hit.name.clone()) {
                    merged.push(Ranked { tier: tier_of(slot), hit });
                }
            }
        }
        let total = merged.len();
        let mut counts = ProviderCounts::new();
        for r in &merged {
            *counts.entry(r.hit.slot.wire()).or_default() += 1;
            // The emote menu's Favorites tab counts its hits before the cap too.
            if r.hit.favorite {
                *counts.entry(FAVORITES_COUNT).or_default() += 1;
            }
        }
        let cmp = |a: &Ranked, b: &Ranked| compare(profile, browse, a, b);
        if browse {
            merged.sort_by(cmp);
            if merged.len() > limit {
                merged = share_between_groups(merged, limit);
            }
        } else {
            if merged.len() > limit {
                merged.select_nth_unstable_by(limit - 1, cmp);
                // A case variant of a name that made the cut comes along, so
                // `LULE` is never cut while `lule` is shown.
                let cut = merged.split_off(limit);
                let kept: HashSet<String> = merged.iter().map(|r| r.hit.name.to_lowercase()).collect();
                merged.extend(cut.into_iter().filter(|r| kept.contains(&r.hit.name.to_lowercase())));
            }
            merged.sort_by(cmp);
            merged = gather_case_variants(merged);
            // The cap never splits a case pair: when it lands inside one, the
            // rest of that pair comes along.
            let mut end = limit.min(merged.len());
            while end > 0
                && end < merged.len()
                && merged[end].hit.name.to_lowercase() == merged[end - 1].hit.name.to_lowercase()
            {
                end += 1;
            }
            merged.truncate(end);
        }
        let rows = merged
            .into_iter()
            .map(|r| {
                let h = r.hit;
                let replaces = h.is_personal() && replaced.contains(&h.name);
                let label = if replaces { PERSONAL_REPLACES_LABEL } else { source_label(h.slot, &h.seed) };
                let detail = if replaces { PERSONAL_REPLACES_DETAIL } else { source_detail(h.slot, &h.seed) };
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
                    emote_type: h.seed.emote_type,
                    owner_name: h.seed.owner_name,
                    original_name: h.seed.original_name,
                    animated: h.seed.animated,
                    width: h.seed.width,
                    favorite: h.favorite,
                    locked: h.locked,
                    login: None,
                }
            })
            .collect();
        (rows, total, counts)
    }
}

/// Pull names that differ only by case together, at the place of the first one
/// ranked, the variant whose case matches what was typed first. Everything else
/// keeps its rank. `LULE` (Twitch) and `lule` (7TV) are two emotes, and someone
/// typing either needs to see both side by side.
fn gather_case_variants(sorted: Vec<Ranked>) -> Vec<Ranked> {
    let folded: Vec<String> = sorted.iter().map(|r| r.hit.name.to_lowercase()).collect();
    let mut members: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, key) in folded.iter().enumerate() {
        members.entry(key.as_str()).or_default().push(i);
    }
    if members.len() == sorted.len() {
        return sorted;
    }
    let mut slots: Vec<Option<Ranked>> = sorted.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(slots.len());
    for key in &folded {
        let Some(mut group) = members.remove(key.as_str()) else {
            continue;
        };
        // Stable, so variants of the same case keep their rank order.
        group.sort_by_key(|&i| !slots[i].as_ref().is_some_and(|r| r.hit.case_exact));
        out.extend(group.into_iter().filter_map(|i| slots[i].take()));
    }
    out
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
            original_name: None,
            owner_ref: None,
            width: None,
            height: None,
            modifier_flags: None,
            ffz_sub_only: None,
            animated: None,
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

    /// The emote fill menu gives a phone's word and the desktop's colon form of
    /// it the same rows, for every word in the fixture the TypeScript and Swift
    /// tests read too.
    #[test]
    fn fill_menu_is_one_answer_for_every_app() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../src/utils/emoteFill.fixture.json")).expect("fixture");
        let req = Request::resolve(Some("fill"), None, None, None);
        assert_eq!((req.profile, req.order, req.limit, req.typing), (Profile::Search, Order::TwitchFirst, FILL_LIMIT, true));
        // What the caller sends besides the profile does not change the question.
        assert_eq!(Request::resolve(Some("fill"), Some("default"), None, Some(false)), req);
        let f = Fixture::new();
        let run = |query: &str| -> Vec<String> {
            let mut c = Collector::new(
                Spec { query, profile: req.profile, order: req.order, contains: false, limit: req.limit },
                Context {
                    channel_id: Some("42"),
                    favorites: &f.favorites,
                    seventv_globals: &f.globals,
                    ffz_subwoofer: false,
                },
            )
            .typing(req.typing);
            c.offer_set(&set());
            c.finish().0.into_iter().map(|r| r.name).collect()
        };
        let words = fixture["words"].as_array().expect("words");
        assert!(!words.is_empty());
        for word in words {
            let word = word.as_str().expect("word");
            assert_eq!(run(word), run(&format!(":{word}")), "{word}");
        }
        assert!(!run("lo").is_empty());
        // One letter gets the full list, not only an emote named exactly that.
        let one = fixture["oneLetter"].as_str().expect("oneLetter");
        assert!(run(one).len() > 1, "{one}");
    }

    #[test]
    fn cycle_keeps_provider_tier_then_name() {
        let f = Fixture::new();
        let (rows, _) = f.run("lo", Profile::Cycle, Order::Default, false);
        // 7TV first (Loading, lol). BTTV's LOADING and FFZ's LoL are different
        // emotes, each beside the 7TV name it differs from only by case; the
        // variant typed in the same case ("lo") leads its pair.
        assert_eq!(names(&rows), vec!["Loading", "LOADING", "lol", "LoL"]);
    }

    #[test]
    fn a_channel_emote_beats_a_global_of_the_same_name() {
        // Chat draws the channel's BetterTTV EZ over 7TV's global EZ (the 7TV
        // extension's order), so the menu offers that one.
        let mut s = EmoteSet::new();
        s.seven_tv = vec![emote(EmoteProvider::SevenTV, "g-EZ", "EZ", Some(GLOBAL_EMOTE_TYPE), None)];
        s.bttv = vec![emote(EmoteProvider::BTTV, "b-EZ", "EZ", None, None)];
        let favorites = HashSet::new();
        let globals: HashSet<String> = ["g-EZ".to_string()].into_iter().collect();
        let mut c = Collector::new(
            Spec { query: "EZ", profile: Profile::Search, order: Order::Default, contains: false, limit: 50 },
            Context { channel_id: Some("42"), favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        );
        c.offer_set(&s);
        let rows = c.finish().0;
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["b-EZ"]);
    }

    #[test]
    fn ffz_keeps_a_name_betterttv_also_has() {
        // Chat draws FFZ over BetterTTV (the 7TV extension's order), in every
        // ranking order, so the menu offers FFZ's emote.
        let mut s = EmoteSet::new();
        s.bttv = vec![emote(EmoteProvider::BTTV, "b-Dup", "Dup", None, None)];
        s.ffz = vec![emote(EmoteProvider::FFZ, "f-Dup", "Dup", None, None)];
        let favorites = HashSet::new();
        let globals = HashSet::new();
        for order in [Order::Default, Order::TwitchFirst] {
            let mut c = Collector::new(
                Spec { query: "Dup", profile: Profile::Search, order, contains: false, limit: 50 },
                Context { channel_id: Some("42"), favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
            );
            c.offer_set(&s);
            let rows = c.finish().0;
            assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["f-Dup"]);
        }
    }

    fn case_pair(extra_7tv: &[&str]) -> EmoteSet {
        let mut s = EmoteSet::new();
        s.twitch = vec![emote(EmoteProvider::Twitch, "t-LULE", "LULE", Some("subscriptions"), Some("99"))];
        s.seven_tv = std::iter::once("lule")
            .chain(extra_7tv.iter().copied())
            .map(|n| emote(EmoteProvider::SevenTV, &format!("s-{n}"), n, None, None))
            .collect();
        s
    }

    fn run_set(set: &EmoteSet, query: &str, profile: Profile, order: Order, limit: usize) -> Vec<Row> {
        let favorites = HashSet::new();
        let globals = HashSet::new();
        let mut c = Collector::new(
            Spec { query, profile, order, contains: false, limit },
            Context { channel_id: Some("42"), favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        );
        c.offer_set(set);
        c.finish().0
    }

    #[test]
    fn names_that_differ_only_by_case_are_both_offered_typed_case_first() {
        let set = case_pair(&["luluDance"]);
        for profile in [Profile::Cycle, Profile::Search] {
            for order in [Order::Default, Order::TwitchFirst] {
                let cases = [("lu", "lule", "LULE"), ("LU", "LULE", "lule"), ("lule", "lule", "LULE"), ("LULE", "LULE", "lule")];
                for (query, first, second) in cases {
                    let rows = run_set(&set, query, profile, order, 50);
                    let n = names(&rows);
                    let at = |x: &str| n.iter().position(|r| *r == x).unwrap_or_else(|| panic!("{x} missing for {query} {profile:?} {order:?}: {n:?}"));
                    assert_eq!(at(second), at(first) + 1, "{query} {profile:?} {order:?}: {n:?}");
                    assert_eq!(rows[at("LULE")].provider, "twitch");
                    assert_eq!(rows[at("lule")].provider, "7tv");
                }
            }
        }
        // The pair leads the 7TV-first cycle as a pair, ahead of luluDance.
        let rows = run_set(&set, "LU", Profile::Cycle, Order::Default, 50);
        assert_eq!(names(&rows), vec!["LULE", "lule", "luluDance"]);
    }

    #[test]
    fn a_case_variant_survives_the_row_cap_beside_its_pair() {
        let set = case_pair(&["lulw", "luna"]);
        let rows = run_set(&set, "lu", Profile::Cycle, Order::Default, 2);
        assert_eq!(names(&rows), vec!["lule", "LULE"]);
    }

    #[test]
    fn a_case_pair_on_the_last_row_keeps_both_variants() {
        // `lubba` ranks first, then the lule/LULE pair: a cap of 2 lands inside
        // the pair, which comes along whole rather than losing LULE.
        let set = case_pair(&["lubba", "lulw"]);
        let rows = run_set(&set, "lu", Profile::Cycle, Order::Default, 50);
        let n = names(&rows);
        let first = n.iter().position(|r| *r == "lule" || *r == "LULE").expect("pair offered");
        assert!(first > 0, "a name ranks ahead of the pair: {n:?}");
        let rows = run_set(&set, "lu", Profile::Cycle, Order::Default, first + 1);
        let capped = names(&rows);
        assert_eq!(capped.len(), first + 2, "{capped:?}");
        assert!(capped.contains(&"lule") && capped.contains(&"LULE"), "{capped:?}");
        // A cap that ends between two unrelated names stays exact.
        let rows = run_set(&set, "lu", Profile::Cycle, Order::Default, first);
        assert_eq!(rows.len(), first);
    }

    #[test]
    fn an_identical_twitch_and_7tv_name_keeps_the_twitch_emote() {
        let mut set = EmoteSet::new();
        set.twitch = vec![emote(EmoteProvider::Twitch, "t1", "xqcL", Some("subscriptions"), Some("71"))];
        set.seven_tv = vec![emote(EmoteProvider::SevenTV, "s1", "xqcL", None, None)];
        for profile in [Profile::Cycle, Profile::Search] {
            for order in [Order::Default, Order::TwitchFirst] {
                let rows = run_set(&set, "xq", profile, order, 50);
                assert_eq!(rows.len(), 1, "{profile:?} {order:?}");
                assert_eq!(rows[0].id, "t1", "{profile:?} {order:?}");
            }
        }
    }

    #[test]
    fn a_personal_emote_only_hides_the_identical_name() {
        let favorites = HashSet::new();
        let globals = HashSet::new();
        for profile in [Profile::Cycle, Profile::Search] {
            let mut c = Collector::new(
                Spec { query: "buh", profile, order: Order::Default, contains: false, limit: 10 },
                Context { channel_id: Some("42"), favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
            );
            c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, "chan", "buh", None, None));
            c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, "chanUpper", "BUH", None, None));
            c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, "mine", "buh", Some(PERSONAL_EMOTE_TYPE), None));
            let (rows, _) = c.finish();
            let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
            assert_eq!(ids, vec!["mine", "chanUpper"], "{profile:?}");
        }
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
    fn picker_mode_counts_before_the_cap_and_shows_locked_ffz() {
        let favorites: HashSet<String> = ["fav".to_string()].into_iter().collect();
        let globals = HashSet::new();
        let mut c = Collector::new(
            Spec { query: "pog", profile: Profile::Search, order: Order::Default, contains: false, limit: 2 },
            Context { channel_id: None, favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        )
        .include_locked(true);
        let mut owned = emote(EmoteProvider::SevenTV, "fav", "PogU", None, None);
        owned.owner_name = Some("Alpha".into());
        owned.original_name = Some("PogUOriginal".into());
        owned.width = Some(64);
        c.offer_emote(Slot::SevenTv, &owned);
        c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, "s2", "Pogpega", None, None));
        c.offer_emote(Slot::Bttv, &emote(EmoteProvider::BTTV, "b1", "PogChamp2", None, None));
        let mut locked = emote(EmoteProvider::FFZ, "9", "PogSub", None, None);
        locked.ffz_sub_only = Some(true);
        c.offer_emote(Slot::Ffz, &locked);
        let (rows, total, counts) = c.finish_counted();
        assert_eq!(total, 4);
        assert_eq!(rows.len(), 2);
        assert_eq!(counts.get("7tv"), Some(&2));
        assert_eq!(counts.get("bttv"), Some(&1));
        assert_eq!(counts.get("ffz"), Some(&1));
        assert_eq!(counts.get(FAVORITES_COUNT), Some(&1), "favorites among the matches, before the cap");
        let pogu = rows.iter().find(|r| r.name == "PogU").unwrap();
        assert!(pogu.favorite);
        assert_eq!(pogu.owner_name.as_deref(), Some("Alpha"));
        assert_eq!(pogu.original_name.as_deref(), Some("PogUOriginal"));
        assert_eq!(pogu.width, Some(64));

        // Completion never offers what cannot be sent.
        let mut c = Collector::new(
            Spec { query: "pogsub", profile: Profile::Search, order: Order::Default, contains: false, limit: 10 },
            Context { channel_id: None, favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        );
        c.offer_emote(Slot::Ffz, &locked);
        assert!(c.finish().0.is_empty());
    }

    #[test]
    fn a_personal_emote_takes_the_name_from_the_channel_emote_it_hides() {
        let favorites = HashSet::new();
        let globals = HashSet::new();
        for profile in [Profile::Cycle, Profile::Search] {
            let mut c = Collector::new(
                Spec { query: "buh", profile, order: Order::Default, contains: false, limit: 10 },
                Context { channel_id: Some("42"), favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
            );
            // The channel's rows are offered first, as they are in chat.
            c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, "pumpkin", "buh", None, None));
            c.offer_emote(Slot::Bttv, &emote(EmoteProvider::BTTV, "b", "buh", None, None));
            c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, "s", "buhShakey", None, None));
            c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, "mine", "buh", Some(PERSONAL_EMOTE_TYPE), None));
            let (rows, _) = c.finish();
            let buh: Vec<&Row> = rows.iter().filter(|r| r.name == "buh").collect();
            assert_eq!(buh.len(), 1, "{profile:?}");
            assert_eq!(buh[0].id, "mine");
            assert_eq!(buh[0].source_detail, "Personal, replaces channel's");
            let shakey = rows.iter().find(|r| r.name == "buhShakey").unwrap();
            assert_eq!(shakey.source_label, "7TV");
        }
    }

    #[test]
    fn a_twitch_emote_is_never_hidden_by_a_personal_one() {
        let favorites = HashSet::new();
        let globals = HashSet::new();
        let mut c = Collector::new(
            Spec { query: "Kappa", profile: Profile::Search, order: Order::TwitchFirst, contains: false, limit: 10 },
            Context { channel_id: None, favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        );
        c.offer_emote(Slot::Twitch, &emote(EmoteProvider::Twitch, "t1", "Kappa", Some("globals"), Some("0")));
        c.offer_emote(Slot::SevenTv, &emote(EmoteProvider::SevenTV, "mine", "Kappa", Some(PERSONAL_EMOTE_TYPE), None));
        let (rows, _) = c.finish();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "t1");
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
            ..Default::default()
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

    fn chatter(login: &str, id: &str, display: &str) -> (String, String, String) {
        (login.to_string(), id.to_string(), display.to_string())
    }

    #[test]
    fn chatters_follow_the_emotes_newest_first_and_capped() {
        let recent = vec![
            chatter("lulu", "1", "Lulu"),
            chatter("luke", "2", "Luke"),
            chatter("bob", "3", "Bob"),
            chatter("lunar", "4", "ルナ"),
        ];
        let rows = chatter_rows("lu", &[], false, &recent);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Lulu", "Luke", "ルナ"]);
        assert!(rows.iter().all(|r| r.provider == "user"));
        // A localized display name inserts the login, which Twitch resolves.
        assert_eq!(rows[2].insert_text.as_deref(), Some("lunar"));
        assert_eq!(rows[0].insert_text.as_deref(), Some("Lulu"));
        assert_eq!(rows[2].login.as_deref(), Some("lunar"));
    }

    /// A channel with single-character emote names on every provider.
    fn short_names() -> EmoteSet {
        let mut s = EmoteSet::new();
        s.seven_tv = vec![
            emote(EmoteProvider::SevenTV, "s0", "0", None, None),
            emote(EmoteProvider::SevenTV, "s00", "00", None, None),
            emote(EmoteProvider::SevenTV, "s0h", "0head", None, None),
            emote(EmoteProvider::SevenTV, "sD", "D", None, None),
        ];
        s.bttv = vec![emote(EmoteProvider::BTTV, "bB", "B", None, None)];
        s.ffz = vec![emote(EmoteProvider::FFZ, "fF", "F", None, None)];
        s.twitch = vec![emote(EmoteProvider::Twitch, "tT", "T", Some("subscriptions"), Some("42"))];
        s
    }

    fn short_run(query: &str, profile: Profile, typing: bool) -> Vec<String> {
        let (favorites, globals) = (HashSet::new(), HashSet::new());
        let mut c = Collector::new(
            Spec { query, profile, order: Order::Default, contains: false, limit: 50 },
            Context { channel_id: Some("42"), favorites: &favorites, seventv_globals: &globals, ffz_subwoofer: false },
        )
        .typing(typing);
        c.offer_set(&short_names());
        c.finish().0.into_iter().map(|r| r.name).collect()
    }

    #[test]
    fn one_typed_character_gets_the_full_list() {
        // One letter typed in passing answers as Tab asked on purpose does.
        for q in ["0", ":0", "D", "d", "B", "F", "T"] {
            assert_eq!(short_run(q, Profile::Cycle, true), short_run(q, Profile::Cycle, false), "{q}");
        }
        assert_eq!(short_run("0", Profile::Cycle, true), vec!["0", "00", "0head"]);
        // The fill ranking still leads with the exact name, then the prefixes.
        assert_eq!(short_run("0", Profile::Search, true)[..3], ["0", "00", "0head"]);
    }

    #[test]
    fn short_words_are_counted_without_their_colon() {
        assert!(!is_short_word("0"));
        assert!(!is_short_word(":0"));
        assert!(!is_short_word(":0:"));
        assert!(is_short_word(":"));
        assert!(is_short_word(""));
        // A mention is a chatter lookup, not an emote word.
        assert!(!is_short_word("@"));
    }

    #[test]
    fn an_at_word_completes_to_a_mention_and_a_colon_word_to_none() {
        let recent = vec![chatter("lulu", "1", "Lulu")];
        let rows = chatter_rows("@lu", &[], false, &recent);
        assert_eq!(rows[0].name, "@Lulu");
        assert_eq!(rows[0].insert_text.as_deref(), Some("@Lulu"));
        assert!(chatter_rows(":lu", &[], false, &recent).is_empty());
        assert_eq!(chatter_rows("@", &[], false, &recent)[0].name, "@Lulu");
        assert!(chatter_rows("", &[], false, &recent).is_empty());
    }

    #[test]
    fn the_streamer_leads_every_chatter_list() {
        let owner = chatter("kaileymae", "9", "KaileyMae");
        let recent = vec![chatter("knutzz", "1", "knutzz"), chatter("koki_v2", "2", "koki_v2")];
        let list = owner_first(Some(owner.clone()), recent.clone());
        let cycle: Vec<_> = chatter_rows("@k", &[], false, &list).into_iter().map(|r| r.name).collect();
        assert_eq!(cycle, vec!["@KaileyMae", "@knutzz", "@koki_v2"]);
        let at: Vec<_> = mention_rows("twitch", "", &list, MENTION_LIMIT).into_iter().map(|r| r.login).collect();
        assert_eq!(at, vec!["kaileymae", "knutzz", "koki_v2"]);
        assert_eq!(mention_rows("twitch", "KAI", &list, MENTION_LIMIT).len(), 1);
        assert!(mention_rows("twitch", "z", &list, MENTION_LIMIT).is_empty());
        // A streamer who chatted is moved to the front, not listed twice.
        let chatted = vec![chatter("knutzz", "1", "knutzz"), owner.clone()];
        let found = find_owner("kaileymae", Some("9"), &chatted);
        assert_eq!(found.as_ref(), Some(&owner));
        let moved: Vec<_> = owner_first(found, chatted).into_iter().map(|c| c.0).collect();
        assert_eq!(moved, vec!["kaileymae", "knutzz"]);
        // A room keyed by id (YouTube) finds its streamer by id.
        let yt = vec![chatter("some one", "UCabc", "Some One")];
        assert!(find_owner("handle", Some("UCabc"), &yt).is_some());
        assert_eq!(owner_first(None, recent.clone()), recent);
    }

    #[test]
    fn each_platform_reads_its_own_ring() {
        assert_eq!(history_room("twitch", "#Forsen"), "Forsen");
        assert_eq!(history_room("kick", "XQC"), "kick:xqc");
        assert_eq!(history_room("youtube", "UCabc"), "youtube:UCabc");
        assert_eq!(history_room("kick", "kick:xqc"), "kick:xqc");
    }

    #[test]
    fn chatters_skip_emote_names_and_respect_the_cap() {
        let recent = vec![chatter("lulw", "1", "LULW"), chatter("lulu", "2", "lulu")];
        let emote = Row {
            slot: Slot::Bttv,
            id: "e".into(),
            name: "LULW".into(),
            insert_text: None,
            url: String::new(),
            local_path: None,
            provider: "bttv",
            source_label: "",
            source_detail: "",
            match_at: None,
            match_len: None,
            group: None,
            is_zero_width: None,
            modifier_flags: None,
            emote_type: None,
            owner_name: None,
            original_name: None,
            animated: None,
            width: None,
            favorite: false,
            locked: false,
            login: None,
        };
        let rows = chatter_rows("lul", std::slice::from_ref(&emote), false, &recent);
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["lulu"]);
        let full = vec![emote; CYCLE_CAP];
        assert!(chatter_rows("lul", &full, false, &recent).is_empty());
        // Includes mode finds a name in the middle.
        let mid = chatter_rows("ulu", &[], true, &recent);
        assert_eq!(mid.len(), 1);
        assert!(chatter_rows("ulu", &[], false, &recent).is_empty());
    }
}
