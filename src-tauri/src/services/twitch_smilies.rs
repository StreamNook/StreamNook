//! Twitch's text-face emotes: `:)`, `;)`, `<3`, `R)` and the rest of the
//! "smilies" sets (the robot faces, the purple set, the Turbo/Prime monkeys).
//!
//! Helix names them by the pattern Twitch's old chat matched them with
//! (`\:-?\)`, `\&lt\;3`, `[oO](_|\.)[oO]`), so no typed word ever equals the
//! name: a message the user sent drew `:)` as plain text, and the emote menu
//! and Tab completion listed the pattern instead of the face. Every Twitch
//! emote list is tidied here as it arrives (`tidy_names`) so the rest of the
//! app sees the face itself, and `word_matches` accepts the other forms the
//! pattern allowed (`:-)` for `:)`, `:p` for `:P`, `o.O` for `O_o`).
//!
//! Which set's art a face shows is the sender's choice: Twitch tags a received
//! message with the id from the set its sender picked, and Helix lists the
//! signed-in user's own set, so a sent message wears that one.

use std::collections::{HashMap, HashSet};
use std::sync::{RwLock, RwLockReadGuard};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::services::emote_service::{Emote, EmoteProvider};
use crate::services::twitch_service::TwitchService;
use crate::services::twitch_smiley_style::{swap_from_view, Record, SmileyStyle};

/// Helix's pattern names for the faces every set shares, and the face each
/// one is shown as.
const KNOWN: &[(&str, &str)] = &[
    (r"\:-?\)", ":)"),
    (r"\:-?\(", ":("),
    (r"\:-?D", ":D"),
    (r"\&gt\;\(", ">("),
    (r"\:-?[z|Z|\|]", ":|"),
    (r"[oO](_|\.)[oO]", "O_o"),
    (r"B-?\)", "B)"),
    (r"\:-?(o|O)", ":O"),
    (r"\&lt\;3", "<3"),
    (r"\:-?[\\/]", ":/"),
    (r"\;-?\)", ";)"),
    (r"\:-?(p|P)", ":P"),
    (r"\;-?(p|P)", ";P"),
    (r"R-?\)", "R)"),
];

/// Each known face with the pattern, HTML entities decoded, matching the whole
/// word.
static PATTERNS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    KNOWN
        .iter()
        .filter_map(|(pattern, face)| {
            let plain = pattern.replace(r"\&lt\;", "<").replace(r"\&gt\;", ">");
            Regex::new(&format!("^(?:{plain})$")).ok().map(|re| (*face, re))
        })
        .collect()
});

/// Whether a Helix emote name is one of these patterns rather than a name.
/// Ordinary emote names are letters and digits; a pattern always carries an
/// escape, an optional mark, a choice or a character class.
fn is_pattern(name: &str) -> bool {
    name.contains(['\\', '?', '|', '['])
}

/// The face a Helix smiley name stands for, or None for an ordinary name.
pub fn display_code(name: &str) -> Option<String> {
    if !is_pattern(name) {
        return None;
    }
    if let Some((_, face)) = KNOWN.iter().find(|(pattern, _)| *pattern == name) {
        return Some((*face).to_string());
    }
    let face = first_match(name);
    let face = face.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
    (!face.is_empty()).then_some(face)
}

/// The simplest text a pattern in Twitch's small smiley grammar matches:
/// optional parts left out, the first choice of a group or class taken,
/// escapes read as the character itself.
fn first_match(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    // Byte length of the last unit written, so a following `?` can drop it.
    let mut last_len = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let before = out.len();
        match c {
            '\\' => {
                if let Some(&next) = chars.get(i + 1) {
                    out.push(next);
                }
                i += 2;
            }
            '?' => {
                out.truncate(out.len() - last_len);
                last_len = 0;
                i += 1;
                continue;
            }
            '[' => {
                let mut j = i + 1;
                let mut members: Vec<char> = Vec::new();
                while j < chars.len() && chars[j] != ']' {
                    if chars[j] == '\\' {
                        j += 1;
                    }
                    if let Some(&ch) = chars.get(j) {
                        members.push(ch);
                    }
                    j += 1;
                }
                // `[\\/]` reads as `/` and `[z|Z|\|]` as `z`: a backslash or a
                // pipe is a face's mouth only when the class offers nothing else.
                let pick = members
                    .iter()
                    .find(|ch| !matches!(ch, '\\' | '|'))
                    .or(members.first());
                if let Some(&ch) = pick {
                    out.push(ch);
                }
                i = j + 1;
            }
            '(' => {
                let mut depth = 1usize;
                let mut j = i + 1;
                let mut first_end: Option<usize> = None;
                while j < chars.len() && depth > 0 {
                    match chars[j] {
                        '\\' => j += 1,
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        '|' if depth == 1 && first_end.is_none() => first_end = Some(j),
                        _ => {}
                    }
                    j += 1;
                }
                let close = j.saturating_sub(1);
                let inner: String = chars[i + 1..first_end.unwrap_or(close).max(i + 1)].iter().collect();
                out.push_str(&first_match(&inner));
                i = j;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
        last_len = out.len() - before;
    }
    out
}

/// Rename every pattern-named Twitch emote in `twitch` to its face, in place.
/// Names that are already faces or ordinary names are left alone, so a list
/// tidied once (or read back from the disk cache) passes through unchanged.
pub fn tidy_names(twitch: &mut [Emote]) {
    for e in twitch.iter_mut() {
        if e.provider != EmoteProvider::Twitch {
            continue;
        }
        if let Some(face) = display_code(&e.name) {
            e.name = face;
        }
    }
}

/// Whether `word` is a typed form of the face `code`: the face itself, or any
/// other form Twitch's pattern for it accepts.
pub fn word_matches(code: &str, word: &str) -> bool {
    word == code
        || PATTERNS
            .iter()
            .any(|(face, re)| *face == code && re.is_match(word))
}

/// The smiley in `twitch` that `word` types, if any. Only Twitch's smilies are
/// considered, and only short words, so an ordinary word costs one length test.
pub fn smiley_for<'a>(word: &str, twitch: &'a [Emote]) -> Option<&'a Emote> {
    if word.is_empty() || word.chars().count() > 4 {
        return None;
    }
    twitch
        .iter()
        .filter(|e| e.emote_type.as_deref() == Some("smilies"))
        .find(|e| word_matches(&e.name, word))
}

/// One of Twitch's smiley sets: the art a set of text faces is drawn in.
pub struct CatalogSet {
    /// The key the apps use for the set.
    pub key: &'static str,
    /// What Twitch calls the set.
    pub name: &'static str,
    /// (emote id, the face it shows), in Twitch's order.
    pub faces: &'static [(&'static str, &'static str)],
}

/// The three sets an account can pick from, in the order the apps list them.
pub const SETS: &[CatalogSet] = &[
    CatalogSet {
        key: "robots",
        name: "Robots",
        faces: &[
            ("1", ":)"),
            ("2", ":("),
            ("3", ":D"),
            ("4", ">("),
            ("5", ":|"),
            ("6", "O_o"),
            ("7", "B)"),
            ("8", ":O"),
            ("9", "<3"),
            ("10", ":/"),
            ("11", ";)"),
            ("12", ":P"),
            ("13", ";P"),
            ("14", "R)"),
        ],
    },
    CatalogSet {
        key: "purple",
        name: "Purple",
        faces: &[
            ("432", ">("),
            ("433", ":/"),
            ("434", ":("),
            ("435", "R)"),
            ("436", ":O"),
            ("437", "O_o"),
            ("438", ":P"),
            ("439", ";)"),
            ("440", ":)"),
            ("441", "B)"),
            ("442", ";P"),
            ("443", ":D"),
            ("444", ":|"),
            ("445", "<3"),
        ],
    },
    CatalogSet {
        key: "monkeys",
        name: "Monkeys",
        faces: &[
            ("483", "<3"),
            ("484", "R)"),
            ("485", "#/"),
            ("486", ":>"),
            ("487", "<]"),
            ("488", ":7"),
            ("489", ":("),
            ("490", ":P"),
            ("491", ";P"),
            ("492", ":O"),
            ("493", ":/"),
            ("494", ":|"),
            ("495", ":s"),
            ("496", ":D"),
            ("497", "O_o"),
            ("498", ">("),
            ("499", ":)"),
            ("500", "B)"),
            ("501", ";)"),
        ],
    },
];

/// The three faces a set's tile shows.
pub const PREVIEW_FACES: [&str; 3] = [":)", ";)", "<3"];

/// Every catalog emote id with its set key and face.
static FACE_INDEX: Lazy<HashMap<&'static str, (&'static str, &'static str)>> = Lazy::new(|| {
    SETS.iter()
        .flat_map(|set| {
            set.faces
                .iter()
                .map(move |(id, face)| (*id, (set.key, *face)))
        })
        .collect()
});

/// The catalog set with this key.
pub fn catalog_set(key: &str) -> Option<&'static CatalogSet> {
    SETS.iter().find(|set| set.key == key)
}

/// The key of the catalog set this emote id belongs to.
pub fn catalog_key_of(id: &str) -> Option<&'static str> {
    FACE_INDEX.get(id).map(|(key, _)| *key)
}

/// The face a catalog emote id shows.
pub fn face_of(id: &str) -> Option<&'static str> {
    FACE_INDEX.get(id).map(|(_, face)| *face)
}

/// Whether two catalog ids are the same face in different sets.
pub fn same_face(a: &str, b: &str) -> bool {
    matches!((face_of(a), face_of(b)), (Some(x), Some(y)) if x == y)
}

/// The image of a smiley.
pub fn emote_url(id: &str) -> String {
    format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0")
}

/// A smiley as the emote lists hold it.
pub fn face_row(id: &str, face: &str) -> Emote {
    Emote {
        id: id.to_string(),
        name: face.to_string(),
        url: emote_url(id),
        provider: EmoteProvider::Twitch,
        is_zero_width: None,
        local_url: None,
        emote_type: Some("smilies".to_string()),
        owner_id: None,
        owner_name: None,
        original_name: None,
        owner_ref: None,
        width: None,
        height: None,
        modifier_flags: None,
        ffz_sub_only: None,
        animated: Some(false),
    }
}

/// The rows of a catalog set.
pub fn catalog_rows(key: &str) -> Option<Vec<Emote>> {
    catalog_set(key).map(|set| {
        set.faces
            .iter()
            .map(|(id, face)| face_row(id, face))
            .collect()
    })
}

/// The key of the catalog set most of these emote ids belong to.
pub fn catalog_key_for_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Option<&'static str> {
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for id in ids {
        if let Some(key) = catalog_key_of(id) {
            *counts.entry(key).or_default() += 1;
        }
    }
    SETS.iter()
        .filter_map(|set| counts.get(set.key).map(|n| (set.key, *n)))
        .max_by_key(|(_, n)| *n)
        .map(|(key, _)| key)
}

/// What to do to an emote list so it holds one set of smilies: drop these
/// ids, make sure these rows are there.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SmileySwap {
    pub remove_ids: Vec<String>,
    pub rows: Vec<Emote>,
}

impl PartialEq for SmileySwap {
    fn eq(&self, other: &Self) -> bool {
        self.remove_ids == other.remove_ids
            && self.rows.len() == other.rows.len()
            && self
                .rows
                .iter()
                .zip(&other.rows)
                .all(|(a, b)| a.id == b.id && a.name == b.name && a.url == b.url)
    }
}

/// The swap that leaves `rows` as the only catalog smilies in a list.
pub fn swap_to(rows: Vec<Emote>) -> SmileySwap {
    let keep: HashSet<&str> = rows.iter().map(|e| e.id.as_str()).collect();
    let remove_ids = SETS
        .iter()
        .flat_map(|set| set.faces.iter())
        .filter(|(id, _)| !keep.contains(id))
        .map(|(id, _)| id.to_string())
        .collect();
    SmileySwap { remove_ids, rows }
}

fn is_smiley(e: &Emote) -> bool {
    e.provider == EmoteProvider::Twitch && e.emote_type.as_deref() == Some("smilies")
}

/// Puts a list of Twitch emotes in the smiley style `swap` names. Only a list
/// that holds Twitch smilies is touched (a Kick or YouTube table, or the
/// fallback list of a signed-out viewer, has none), and only Twitch rows
/// change, so FFZ, BTTV and 7TV rows that share a number are never removed.
/// Applying the same swap twice changes nothing more. True when the list
/// changed.
pub fn apply_swap(list: &mut Vec<Emote>, swap: &SmileySwap) -> bool {
    let holds_smilies = list.iter().any(|e| {
        is_smiley(e) || (e.provider == EmoteProvider::Twitch && catalog_key_of(&e.id).is_some())
    });
    if !holds_smilies {
        return false;
    }
    let keep: HashSet<&str> = swap.rows.iter().map(|e| e.id.as_str()).collect();
    let remove: HashSet<&str> = swap.remove_ids.iter().map(String::as_str).collect();
    let mut changed = false;
    list.retain(|e| {
        if e.provider != EmoteProvider::Twitch {
            return true;
        }
        let drop =
            remove.contains(e.id.as_str()) || (is_smiley(e) && !keep.contains(e.id.as_str()));
        changed |= drop;
        !drop
    });
    let have: HashSet<String> = list
        .iter()
        .filter(|e| e.provider == EmoteProvider::Twitch)
        .map(|e| e.id.clone())
        .collect();
    for row in &swap.rows {
        if !have.contains(&row.id) {
            list.push(row.clone());
            changed = true;
        }
    }
    changed
}

/// Dresses a list in the signed-in account's own smiley style, where a
/// style is known.
pub fn wear_own(list: &mut Vec<Emote>) {
    if let Some(swap) = own_swap() {
        apply_swap(list, &swap);
    }
}

/// The id of the smiley in `rows` that shows the same face as the catalog id
/// `id`; any other id, and a face `rows` lacks, stays as it is.
pub fn resolve_in(rows: &[Emote], id: &str) -> String {
    let Some(face) = face_of(id) else {
        return id.to_string();
    };
    rows.iter()
        .find(|e| is_smiley(e) && e.name == face)
        .map(|e| e.id.clone())
        .unwrap_or_else(|| id.to_string())
}

/// `resolve_in` over a list of ids, each resolved id once, in order.
pub fn resolve_ids(rows: &[Emote], ids: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.iter()
        .map(|id| resolve_in(rows, id))
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// What a Helix answer listed as the account's own smilies.
#[derive(Clone, Debug)]
pub(crate) struct HelixEntry {
    pub rows: Vec<Emote>,
    pub at: i64,
}

/// Everything the app knows about the signed-in account's smiley style, under
/// one lock (`OWN`).
pub(crate) struct Own {
    /// The record read from (or written to) Twitch for the identity's account.
    pub record: Option<Record>,
    /// Moves on every record change; keys the disk writer.
    pub ver: u64,
    /// The smilies Helix listed, per account it was fetched for.
    pub helix: HashMap<String, HelixEntry>,
    /// The write fence: moves at a switch start, a switch commit, an identity
    /// that loaded a record and an account move. A read commits only when it
    /// started under the current value.
    pub gen: u64,
    /// Moves whenever anything a window draws changes.
    pub rev: u64,
    /// (Twitch user id, account generation) the state belongs to.
    pub identity: Option<(String, u64)>,
    /// Reads that failed while Helix and the record disagreed.
    pub disagree_failures: u32,
    /// The (record version, Helix set) a read has already been decided for.
    pub decided: Option<(u64, String)>,
}

impl Own {
    pub(crate) fn new() -> Self {
        Own {
            record: None,
            ver: 0,
            helix: HashMap::new(),
            gen: 0,
            rev: 0,
            identity: None,
            disagree_failures: 0,
            decided: None,
        }
    }
}

pub(crate) static OWN: Lazy<RwLock<Own>> = Lazy::new(|| RwLock::new(Own::new()));

/// The state, for reading. A lock poisoned by a panic elsewhere is still read.
pub(crate) fn read_own() -> RwLockReadGuard<'static, Own> {
    OWN.read().unwrap_or_else(|e| e.into_inner())
}

/// The record that is shown for the identity, if there is one.
pub(crate) fn visible_record(own: &Own, g: u64) -> Option<&Record> {
    let (user_id, account_gen) = own.identity.as_ref()?;
    if *account_gen != g {
        return None;
    }
    own.record.as_ref().filter(|r| &r.user_id == user_id)
}

/// The smilies swap for the signed-in account: the record when it is valid
/// and not yielding, else what Helix listed for the identity's account, else
/// none. `g` is the account generation now.
pub(crate) fn effective(own: &Own, g: u64) -> Option<SmileySwap> {
    let (user_id, account_gen) = own.identity.as_ref()?;
    if *account_gen != g {
        return None;
    }
    if let Some(record) = own
        .record
        .as_ref()
        .filter(|r| &r.user_id == user_id && !r.yield_to_helix)
    {
        if let Some(swap) = swap_from_view(&record.view) {
            return Some(swap);
        }
    }
    let entry = own.helix.get(user_id)?;
    (!entry.rows.is_empty()).then(|| swap_to(entry.rows.clone()))
}

/// Everything a window draws from the state; a change in any part moves `rev`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Observable {
    pub swap: Option<SmileySwap>,
    pub view: Option<SmileyStyle>,
    pub shown: bool,
}

pub(crate) fn observable(own: &Own, g: u64) -> Observable {
    let record = visible_record(own, g);
    Observable {
        swap: effective(own, g),
        view: record.map(|r| r.view.clone()),
        shown: record.is_some(),
    }
}

#[cfg(test)]
thread_local! {
    static TESTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Under test the process-wide state answers only on a thread that asked for it,
/// so no other test ever sees a swap.
#[cfg(test)]
pub(crate) fn set_testing(on: bool) {
    TESTING.with(|t| t.set(on));
}

pub(crate) fn state_visible() -> bool {
    #[cfg(test)]
    {
        TESTING.with(|t| t.get())
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// The smilies swap for the signed-in account, or None when its style is not
/// known. Called inside the lock of every place an emote list is installed.
pub fn own_swap() -> Option<SmileySwap> {
    if !state_visible() {
        return None;
    }
    effective(&read_own(), TwitchService::account_generation())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smiley(id: &str, name: &str) -> Emote {
        Emote {
            id: id.to_string(),
            name: name.to_string(),
            url: String::new(),
            provider: EmoteProvider::Twitch,
            is_zero_width: None,
            local_url: None,
            emote_type: Some("smilies".to_string()),
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

    #[test]
    fn every_known_pattern_compiles() {
        assert_eq!(PATTERNS.len(), KNOWN.len());
    }

    #[test]
    fn helix_pattern_names_read_as_faces() {
        // The exact names Helix sent for the robot set (ids 1-14).
        for (name, face) in [
            (r"\:-?\)", ":)"),
            (r"\;-?\)", ";)"),
            (r"\&lt\;3", "<3"),
            (r"\&gt\;\(", ">("),
            (r"R-?\)", "R)"),
            (r"[oO](_|\.)[oO]", "O_o"),
            (r"\:-?[\\/]", ":/"),
            (r"\;-?(p|P)", ";P"),
        ] {
            assert_eq!(display_code(name).as_deref(), Some(face), "{name}");
        }
    }

    #[test]
    fn unknown_patterns_take_their_simplest_text() {
        // Faces only a set beyond the robot one has (the monkeys' extras).
        assert_eq!(display_code(r"\&lt\;\]").as_deref(), Some("<]"));
        assert_eq!(display_code(r"\#-?[\\/]").as_deref(), Some("#/"));
        assert_eq!(display_code(r"\:-?(s|S)").as_deref(), Some(":s"));
        assert_eq!(display_code(r"\:-?7").as_deref(), Some(":7"));
        assert_eq!(display_code(r"\:-?\&gt\;").as_deref(), Some(":>"));
    }

    #[test]
    fn ordinary_names_are_left_alone() {
        assert_eq!(display_code("Kappa"), None);
        assert_eq!(display_code(":)"), None);
        assert_eq!(display_code("<3"), None);
        let mut list = vec![smiley("1", r"\:-?\)"), smiley("25", "Kappa"), smiley("9", "<3")];
        tidy_names(&mut list);
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [":)", "Kappa", "<3"]);
    }

    #[test]
    fn typed_forms_find_the_face() {
        assert!(word_matches(":)", ":)"));
        assert!(word_matches(":)", ":-)"));
        assert!(word_matches(":P", ":p"));
        assert!(word_matches(";P", ";-p"));
        assert!(word_matches("O_o", "o.O"));
        assert!(word_matches(":/", ":\\"));
        assert!(word_matches(":|", ":z"));
        assert!(word_matches("<3", "<3"));
        assert!(!word_matches(":)", ":("));
        assert!(!word_matches(":)", "x:)"));
        assert!(!word_matches(":D", ":d"));
    }

    #[test]
    fn smiley_for_reads_only_the_smilies() {
        let mut list = vec![smiley("1", r"\:-?\)"), smiley("11", r"\;-?\)")];
        tidy_names(&mut list);
        assert_eq!(smiley_for(";)", &list).map(|e| e.id.as_str()), Some("11"));
        assert_eq!(smiley_for(":-)", &list).map(|e| e.id.as_str()), Some("1"));
        assert!(smiley_for("hello", &list).is_none());
        let mut kappa = smiley("25", ":)");
        kappa.emote_type = Some("globals".into());
        assert!(smiley_for(":)", &[kappa]).is_none());
    }
}

#[cfg(test)]
#[path = "twitch_smilies_own_tests.rs"]
mod own_tests;
