//! Measured limits of Twitch's private APIs. Several fail SILENTLY when
//! exceeded, so measure before raising one; don't infer from Twitch's
//! published caps, which describe different things.

/// Topics per PubSub LISTEN *frame*. Over this, Twitch sends no RESPONSE and
/// subscribes to nothing. Cliff measured at 28; 20 leaves byte-size headroom.
pub const PUBSUB_LISTEN_TOPICS_PER_FRAME: usize = 20;

/// Topics per PubSub *connection*, across all frames. Twitch documents this one.
pub const PUBSUB_MAX_TOPICS_PER_CONNECTION: usize = 50;

/// Operations per batched GQL POST. 36 returns HTTP 400 with a valid JSON body,
/// so callers must check the response is an array AND matches the request
/// length before mapping results positionally.
pub const GQL_MAX_BATCHED_OPERATIONS: usize = 35;

/// Root field aliases in one GQL query (`c0: channel(id:) { .. }`). Twitch
/// rejects a query with more than this, for every client id: the answer is
/// an error and empty `data`, so a whole batch fails at once. Root fields
/// without an alias, and aliases nested inside a field, don't count.
pub const GQL_MAX_ALIASES: usize = 15;

/// The root field aliases in a GQL document: names followed by `:` directly
/// inside the outermost selection set, the count Twitch holds to
/// `GQL_MAX_ALIASES`. Arguments, input objects, strings and nested
/// selections are skipped.
#[cfg(test)]
pub fn root_aliases(query: &str) -> usize {
    let b = query.as_bytes();
    let (mut i, mut parens, mut depth, mut count) = (0, 0i32, 0i32, 0);
    let mut after_name = false;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                after_name = false;
            }
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'(' => {
                parens += 1;
                after_name = false;
            }
            b')' => {
                parens -= 1;
                after_name = false;
            }
            b'{' if parens == 0 => {
                depth += 1;
                after_name = false;
            }
            b'}' if parens == 0 => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                after_name = false;
            }
            b':' => {
                if depth == 1 && parens == 0 && after_name {
                    count += 1;
                }
                after_name = false;
            }
            c if c.is_ascii_whitespace() || c == b',' => {}
            c if c.is_ascii_alphanumeric() || c == b'_' => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                after_name = true;
                continue;
            }
            _ => after_name = false,
        }
        i += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::twitch_service::TwitchService;

    /// Ids for a roster far past one batch, so every site has to split it.
    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| (1000 + i).to_string()).collect()
    }

    /// Every chunk's query stays inside the cap, and the aliases across the
    /// chunks cover every id (so the count is real, not zero).
    fn check(site: &str, queries: Vec<String>, total: usize) {
        let mut seen = 0;
        for q in &queries {
            let n = root_aliases(q);
            assert!(n <= GQL_MAX_ALIASES, "{site}: {n} root aliases in one query");
            seen += n;
        }
        assert_eq!(seen, total, "{site}: aliases across the chunks");
    }

    #[test]
    fn the_counter_counts_only_root_aliases() {
        assert_eq!(root_aliases("{ g0: game(id: \"1\") { id } g1: game(id: \"2\") { id } }"), 2);
        assert_eq!(root_aliases("query Q($a: ID!, $b: ID!) { a: user(id: $a) { x: id } user(id: $b) { id } }"), 1);
        assert_eq!(
            root_aliases("query { c0: user(id: \"1\") { clips(criteria: { filter: ALL_TIME }) { edges { node { id } } } } currentUser { id } }"),
            1
        );
        assert_eq!(root_aliases("query { user(login: \"a:b\") { id } }"), 0);
    }

    #[test]
    fn every_aliased_batch_fits_the_cap() {
        let all = ids(100);

        // Stream Together sends its ids in one list argument, so a full batch
        // carries no root aliases at all.
        for c in all.chunks(crate::services::collaboration::BATCH) {
            let body = TwitchService::collaborations_body(c);
            assert_eq!(root_aliases(body["query"].as_str().unwrap()), 0, "collaborations: aliases");
            assert_eq!(body["variables"]["ids"].as_array().unwrap().len(), c.len());
        }

        let faces: Vec<String> = all
            .chunks(crate::services::shared_chat::FACES_BATCH)
            .map(|c| TwitchService::channel_faces_body(c)["query"].as_str().unwrap().to_string())
            .collect();
        check("shared chat faces", faces, all.len());

        let vods: Vec<String> = all
            .chunks(TwitchService::VODS_BATCH)
            .map(|c| TwitchService::videos_by_ids_body(c)["query"].as_str().unwrap().to_string())
            .collect();
        check("videos by ids", vods, all.len());

        let refs: Vec<&str> = all.iter().map(String::as_str).collect();
        let games: Vec<String> = refs
            .chunks(crate::services::favorite_categories::BATCH)
            .map(crate::services::favorite_categories::live_query)
            .collect();
        check("favorite categories", games, all.len());

        let eligible: Vec<String> = all
            .chunks(crate::services::drops_service::ELIGIBILITY_BATCH)
            .map(crate::services::drops_service::eligibility_query)
            .collect();
        check("drop eligibility", eligible, all.len());

        let details: Vec<String> = crate::services::reward_drops::detail_chunks(all.len())
            .into_iter()
            .map(|r| crate::services::reward_drops::details_query(r.len()))
            .collect();
        check("reward drop details", details, all.len());
    }
}
