//! Text as search compares it: lowercase, with Latin accents folded, so
//! "pokemon" typed on any keyboard matches "Pokémon" and "beyonce" matches
//! "Beyoncé". Category search and Your songs both read it.

/// `s` in lowercase with Latin accents folded to their plain letters.
pub fn fold_search(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(fold_accent)
        .collect()
}

fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
        'ğ' => 'g',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => 'i',
        'ł' => 'l',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
        'ř' => 'r',
        'ś' | 'š' | 'ş' => 's',
        'ť' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        c => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_and_latin_accents_fold_away() {
        assert_eq!(fold_search("Pokémon"), "pokemon");
        assert_eq!(fold_search("Mötley Crüe"), "motley crue");
        assert_eq!(fold_search("SIGUR RÓS"), "sigur ros");
        assert_eq!(fold_search("Tití Me Preguntó"), "titi me pregunto");
        assert_eq!(fold_search("plain"), "plain");
    }
}
