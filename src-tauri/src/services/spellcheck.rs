//! English spell checking for the chat composers, shared by every window.
//!
//! One parsed dictionary serves the main window and every MultiChat popout.
//! It loads on first use (never while spell checking is off, since nothing
//! calls in), and is dropped again after five idle minutes so a composer
//! focused once does not pin the word map for the rest of the session.
//!
//! This module only knows English. Emote names, chatter logins and the user's
//! own dictionary are chat vocabulary, filtered out by the caller before words
//! arrive here. Nothing touches the network: the dictionary is compiled in.

use spellbook::Dictionary;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const AFF: &str = include_str!("../../dictionaries/en_US.aff");
const DIC: &str = include_str!("../../dictionaries/en_US.dic");

/// Past this length, suggestion spends a long time generating candidates that
/// are never right anyway. Keyboard-mash gets no suggestions, instantly.
const MAX_SUGGEST_LENGTH: usize = 20;

/// Ranked suggestions are long; the menu only has room for a handful.
const MAX_SUGGESTIONS: usize = 5;

const IDLE_TEARDOWN: Duration = Duration::from_secs(5 * 60);
const IDLE_SWEEP: Duration = Duration::from_secs(60);

struct Loaded {
    dict: Arc<Dictionary>,
    last_used: Instant,
}

static SPELLER: Mutex<Option<Loaded>> = Mutex::new(None);

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct SpellVerdict {
    /// False only when the word is definitely misspelled.
    pub correct: bool,
    /// Corrections, best first. Empty for a correct word, and also possible for
    /// a misspelled one the dictionary cannot get close to.
    pub suggestions: Vec<String>,
}

fn parse() -> Result<Dictionary, String> {
    Dictionary::new(AFF, DIC).map_err(|e| format!("spellcheck dictionary failed to parse: {e}"))
}

/// The shared dictionary, loading it if needed. The first load also starts the
/// idle sweep, which ends itself once it has dropped the dictionary, so no task
/// outlives the thing it looks after.
fn dictionary() -> Result<Arc<Dictionary>, String> {
    let mut slot = SPELLER.lock().map_err(|e| e.to_string())?;
    if let Some(loaded) = slot.as_mut() {
        loaded.last_used = Instant::now();
        return Ok(loaded.dict.clone());
    }
    let dict = Arc::new(parse()?);
    *slot = Some(Loaded { dict: dict.clone(), last_used: Instant::now() });
    drop(slot);
    tauri::async_runtime::spawn(idle_sweep());
    Ok(dict)
}

async fn idle_sweep() {
    loop {
        tokio::time::sleep(IDLE_SWEEP).await;
        let Ok(mut slot) = SPELLER.lock() else { return };
        match slot.as_ref() {
            None => return,
            Some(loaded) if loaded.last_used.elapsed() >= IDLE_TEARDOWN => {
                *slot = None;
                return;
            }
            Some(_) => {}
        }
    }
}

/// Load the dictionary without asking anything, so the first right-click in a
/// freshly focused composer already has a warm engine.
pub fn warm() -> Result<(), String> {
    dictionary().map(|_| ())
}

/// The words in `words` the dictionary does not know, in input order.
pub fn check(words: &[String]) -> Result<Vec<String>, String> {
    let dict = dictionary()?;
    Ok(misspelled(&dict, words))
}

/// Whether one word is spelled correctly, and what it should be if not.
pub fn suggest(word: &str) -> Result<SpellVerdict, String> {
    let dict = dictionary()?;
    Ok(verdict(&dict, word))
}

fn misspelled(dict: &Dictionary, words: &[String]) -> Vec<String> {
    words.iter().filter(|w| !dict.check(w)).cloned().collect()
}

fn verdict(dict: &Dictionary, word: &str) -> SpellVerdict {
    let correct = dict.check(word);
    let suggestions = if correct || word.chars().count() > MAX_SUGGEST_LENGTH {
        Vec::new()
    } else {
        ranked_suggestions(dict, word)
    };
    SpellVerdict { correct, suggestions }
}

/// Corrections for one misspelled word, best first.
///
/// Transpositions go ahead of the dictionary's own ranking. Swapping two
/// letters typed in the wrong order is both the most common typo and the most
/// confident fix: if "the" is a candidate for "teh", it is almost certainly the
/// word that was meant, and burying it under "ten, eh, meh" is the wrong answer.
/// Hunspell-style suggestion builds candidates from the replacement table,
/// keyboard-adjacent substitutions and doubled letters, none of which produce
/// a transposition.
fn ranked_suggestions(dict: &Dictionary, word: &str) -> Vec<String> {
    let mut from_dictionary = Vec::new();
    dict.suggest(word, &mut from_dictionary);

    let swapped = adjacent_transpositions(word)
        .into_iter()
        .filter(|candidate| dict.check(candidate));

    let mut seen = HashSet::new();
    let mut ranked = Vec::with_capacity(MAX_SUGGESTIONS);
    for suggestion in swapped.chain(from_dictionary) {
        if !seen.insert(suggestion.to_lowercase()) {
            continue;
        }
        ranked.push(suggestion);
        if ranked.len() == MAX_SUGGESTIONS {
            break;
        }
    }
    ranked
}

/// Every version of `word` with one adjacent pair of characters swapped.
/// Case rides along, so "Teh" yields "The".
fn adjacent_transpositions(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    (0..chars.len().saturating_sub(1))
        .map(|i| {
            let mut swapped = chars.clone();
            swapped.swap(i, i + 1);
            swapped.into_iter().collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> Dictionary {
        parse().expect("bundled dictionary parses")
    }

    fn owned(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn generates_every_adjacent_transposition() {
        assert_eq!(adjacent_transpositions("teh"), owned(&["eth", "the"]));
        assert_eq!(adjacent_transpositions("adn"), owned(&["dan", "and"]));
        assert!(adjacent_transpositions("Teh").contains(&"The".to_string()));
        assert!(adjacent_transpositions("a").is_empty());
        assert!(adjacent_transpositions("").is_empty());
    }

    #[test]
    fn reports_only_the_misspelled_words_in_order() {
        let d = dict();
        let words = owned(&["hello", "recieve", "world", "teh"]);
        assert_eq!(misspelled(&d, &words), owned(&["recieve", "teh"]));
    }

    #[test]
    fn a_transposition_fix_ranks_first() {
        let v = verdict(&dict(), "teh");
        assert!(!v.correct);
        assert_eq!(v.suggestions.first().map(String::as_str), Some("the"));
        assert!(v.suggestions.len() <= MAX_SUGGESTIONS);
    }

    #[test]
    fn a_capitalised_typo_gets_a_capitalised_fix() {
        let v = verdict(&dict(), "Teh");
        assert_eq!(v.suggestions.first().map(String::as_str), Some("The"));
    }

    #[test]
    fn a_correct_word_that_is_one_swap_from_another_gets_no_fix() {
        // form/from: without the correctness guard the transposition pass
        // would offer to "fix" a perfectly good word.
        let v = verdict(&dict(), "form");
        assert!(v.correct);
        assert!(v.suggestions.is_empty());
    }

    #[test]
    fn keyboard_mash_gets_no_suggestions() {
        let v = verdict(&dict(), "asdfghjklqwertyuiopzxcv");
        assert!(!v.correct);
        assert!(v.suggestions.is_empty());
    }

    #[test]
    fn suggestions_are_deduped_case_insensitively() {
        let v = verdict(&dict(), "recieve");
        let lowered: HashSet<String> = v.suggestions.iter().map(|s| s.to_lowercase()).collect();
        assert_eq!(lowered.len(), v.suggestions.len());
        assert!(v.suggestions.iter().any(|s| s == "receive"));
    }
}
