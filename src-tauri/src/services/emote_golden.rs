//! Golden chat: real IRC lines through the real parser against a room table
//! built the way a fetch builds it, asserting exactly which emote each word
//! becomes. Every rule a viewer can see is pinned here: precedence across
//! providers and scopes, Twitch's own tags, zero-width, modifiers, personal
//! emotes, and the repair of rows parsed before their table landed.

use crate::models::chat_layout::MessageSegment;
use crate::services::emote_match::GLOBAL_EMOTE_TYPE;
use crate::services::emote_service::{Emote, EmoteProvider, EmoteSet};
use crate::services::irc_service::IrcService;

/// The room every case parses in; no other test uses it.
const ROOM: &str = "goldenroom";

fn emote(provider: EmoteProvider, id: &str, name: &str) -> Emote {
    let url = match provider {
        EmoteProvider::BTTV => format!("https://cdn.betterttv.net/emote/{id}/1x"),
        EmoteProvider::FFZ => format!("https://cdn.frankerfacez.com/emote/{id}/1"),
        _ => format!("https://cdn.7tv.app/emote/{id}/1x.avif"),
    };
    Emote {
        id: id.into(),
        name: name.into(),
        url,
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

fn global(mut e: Emote) -> Emote {
    e.emote_type = Some(GLOBAL_EMOTE_TYPE.into());
    e
}

/// A room table shaped like a fetched one: each provider's globals and the
/// channel's own rows, with every collision a real room has.
fn room_set() -> EmoteSet {
    let mut zero_width = global(emote(EmoteProvider::SevenTV, "7g-petpet", "PETPET"));
    zero_width.is_zero_width = Some(true);
    let mut ffz_flip = global(emote(EmoteProvider::FFZ, "ffz-flipx", "ffzX"));
    ffz_flip.is_zero_width = Some(true);
    ffz_flip.modifier_flags = Some(2);

    let mut set = EmoteSet::new();
    set.seven_tv = vec![
        // 7TV globals named like common channel emotes.
        global(emote(EmoteProvider::SevenTV, "7g-ez", "EZ")),
        global(emote(EmoteProvider::SevenTV, "7g-clap", "Clap")),
        zero_width,
        // The channel's own 7TV rows.
        emote(EmoteProvider::SevenTV, "7c-kekw", "KEKW"),
        emote(EmoteProvider::SevenTV, "7c-catjam", "catJAM"),
        emote(EmoteProvider::SevenTV, "7c-kappa", "Kappa"),
    ];
    set.bttv = vec![
        global(emote(EmoteProvider::BTTV, "bg-lul", "LULW")),
        // The channel's BTTV EZ must beat 7TV's global EZ.
        emote(EmoteProvider::BTTV, "bc-ez", "EZ"),
        // And loses to the channel's own 7TV catJAM.
        emote(EmoteProvider::BTTV, "bc-catjam", "catJAM"),
        // FFZ beats BetterTTV, among the channel's rows and among globals.
        emote(EmoteProvider::BTTV, "bc-dup", "Dup"),
        global(emote(EmoteProvider::BTTV, "bg-gdup", "GDup")),
    ];
    set.ffz = vec![
        ffz_flip,
        // A channel FFZ Clap beats 7TV's global Clap.
        emote(EmoteProvider::FFZ, "fc-clap", "Clap"),
        emote(EmoteProvider::FFZ, "fc-dup", "Dup"),
        global(emote(EmoteProvider::FFZ, "fg-gdup", "GDup")),
    ];
    set
}

fn line(id: &str, user_id: &str, emotes_tag: &str, body: &str) -> String {
    format!(
        "@display-name=Gold;emotes={emotes_tag};id={id};room-id=4242;tmi-sent-ts=1700000000000;user-id={user_id} :gold!gold@gold.tmi.twitch.tv PRIVMSG #{ROOM} :{body}"
    )
}

/// `(word, emote id)` for every emote segment, in order.
fn emotes_of(segments: &[MessageSegment]) -> Vec<(String, String)> {
    segments
        .iter()
        .filter_map(|s| match s {
            MessageSegment::Emote { content, emote_id, .. } => {
                Some((content.clone(), emote_id.clone().unwrap_or_default()))
            }
            _ => None,
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(w, id)| (w.to_string(), id.to_string())).collect()
}

fn parse(raw: &str) -> Vec<MessageSegment> {
    IrcService::parse_live_line(raw).expect("a chat line").segments
}

#[test]
fn every_word_becomes_the_emote_the_7tv_extension_draws() {
    IrcService::install_parse_lookup_for_test(ROOM, &room_set());
    let got = emotes_of(&parse(&line("g1", "1", "", "EZ Clap catJAM KEKW LULW notAnEmote")));
    assert_eq!(
        got,
        pairs(&[
            ("EZ", "bc-ez"),         // channel BTTV over 7TV global
            ("Clap", "fc-clap"),     // channel FFZ over 7TV global
            ("catJAM", "7c-catjam"), // channel 7TV over channel BTTV
            ("KEKW", "7c-kekw"),
            ("LULW", "bg-lul"),      // a global nobody shadows
        ])
    );
}

#[test]
fn ffz_beats_betterttv_of_the_same_name() {
    // The 7TV extension's order, kept on purpose (Brandon, 2026-10-21).
    IrcService::install_parse_lookup_for_test(ROOM, &room_set());
    let got = emotes_of(&parse(&line("g6", "1", "", "Dup GDup")));
    assert_eq!(got, pairs(&[("Dup", "fc-dup"), ("GDup", "fg-gdup")]));
}

#[test]
fn a_word_twitch_tagged_is_twitchs_emote() {
    IrcService::install_parse_lookup_for_test(ROOM, &room_set());
    // Twitch tags Kappa at 0-4; the channel's 7TV Kappa must not replace it.
    let segs = parse(&line("g2", "1", "25:0-4", "Kappa KEKW"));
    let got = emotes_of(&segs);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].0, "Kappa");
    assert_eq!(got[0].1, "25");
    assert_eq!(got[1], ("KEKW".to_string(), "7c-kekw".to_string()));
}

#[test]
fn zero_width_and_modifier_emotes_keep_their_flags() {
    IrcService::install_parse_lookup_for_test(ROOM, &room_set());
    let segs = parse(&line("g3", "1", "", "KEKW PETPET KEKW ffzX"));
    let flags: Vec<(String, Option<bool>, Option<u32>)> = segs
        .iter()
        .filter_map(|s| match s {
            MessageSegment::Emote { content, is_zero_width, modifier_flags, .. } => {
                Some((content.clone(), *is_zero_width, *modifier_flags))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        flags,
        vec![
            ("KEKW".to_string(), None, None),
            ("PETPET".to_string(), Some(true), None),
            ("KEKW".to_string(), None, None),
            ("ffzX".to_string(), Some(true), Some(2)),
        ]
    );
}

#[tokio::test]
async fn a_senders_personal_emote_beats_the_rooms_and_only_for_them() {
    IrcService::install_parse_lookup_for_test(ROOM, &room_set());
    let mine = emote(EmoteProvider::SevenTV, "p-ez", "EZ");
    IrcService::set_personal_emotes("golden-sender".into(), "golden-set".into(), vec![mine]).await;
    let theirs = emotes_of(&parse(&line("g4", "golden-sender", "", "EZ")));
    assert_eq!(theirs, pairs(&[("EZ", "p-ez")]));
    let others = emotes_of(&parse(&line("g5", "someone-else", "", "EZ")));
    assert_eq!(others, pairs(&[("EZ", "bc-ez")]));
}

#[test]
fn a_row_parsed_before_its_table_is_named_for_repair_when_it_lands() {
    use crate::services::emote_restamp::gains_emote;
    use crate::services::irc_service::EmoteLookup;
    // Parsed with no table: the word stayed text.
    let early = EmoteLookup::build(&EmoteSet::new());
    let full = EmoteLookup::build(&room_set());
    assert!(gains_emote("look EZ", &full, Some(&*early)));
    // And a table that only re-confirms what was drawn names nothing.
    assert!(!gains_emote("look EZ", &full, Some(&*full)));
}
