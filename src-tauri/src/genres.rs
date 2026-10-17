//! Genre names for «Итоги». SoundCloud has one free-text `genre` per track plus free tags, and people write the
//! same thing many ways ("Hip-hop & Rap", "Hip Hop/rap"; "phonk", "фонк"). A track counts under its genre and its
//! tags, with the spellings that mean one thing folded into one name.

/// Letters and digits only, lower-case, `&` read as "and".
fn key(s: &str) -> String {
    s.to_lowercase().replace('&', " and ").chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Spellings that mean one genre → its name.
fn alias(k: &str) -> Option<&'static str> {
    Some(match k {
        "hiphopandrap" | "hiphoprap" | "hiphop" | "rap" | "рэп" | "хипхоп" => "hip-hop & rap",
        "danceandedm" | "danceedm" | "edm" => "dance & edm",
        "rnb" | "randb" | "rnbandsoul" | "rnbsoul" => "r&b & soul",
        "phonk" | "фонк" | "brazilianphonk" | "drillphonk" | "phonkhouse" => "phonk",
        "funk" | "фанк" | "funky" => "funk",
        "drumandbass" | "dnb" | "drumnbass" => "drum & bass",
        "lofi" | "lofihiphop" | "lofibeats" => "lo-fi",
        "altrock" | "alternativerock" | "alternativeandrock" => "alternative rock",
        "альтернатива" | "alternative" | "alt" => "alternative",
        "electronic" | "electronica" | "electro" => "electronic",
        _ => return None,
    })
}

/// Words that say nothing about the sound.
fn is_noise(k: &str) -> bool {
    let n = k.chars().count();
    n < 3
        || n > 30
        || k.chars().all(|c| c.is_ascii_digit())
        || matches!(
            k,
            "music" | "new" | "original" | "soundcloud" | "free" | "official" | "song" | "songs" | "track" | "mix" | "unreleased" | "музыка" | "новое" | "freedownload"
        )
}

/// The genre names of one track: its genre first, then up to eight tags, each once.
pub fn labels(genre: Option<&str>, tag_list: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |raw: &str| {
        let raw = raw.trim();
        let k = key(raw);
        if k.is_empty() {
            return;
        }
        let name = match alias(&k) {
            Some(a) => a.to_owned(),
            None if is_noise(&k) => return,
            None => raw.to_lowercase(),
        };
        if !out.contains(&name) {
            out.push(name);
        }
    };
    if let Some(g) = genre {
        add(g);
    }
    for t in crate::wave::parse_tags(tag_list).iter().take(8) {
        add(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_genre_written_two_ways_is_one() {
        assert_eq!(labels(Some("Hip-hop & Rap"), ""), labels(Some("Hip Hop/rap"), ""));
    }

    #[test]
    fn tags_count_and_noise_does_not() {
        let l = labels(Some("Hip-hop & Rap"), r#"фонк phonk "memphis rap" 2023 music"#);
        assert!(l.contains(&"phonk".to_owned()));
        assert!(l.contains(&"memphis rap".to_owned()));
        assert!(!l.iter().any(|g| g == "2023" || g == "music"));
        assert_eq!(l.iter().filter(|g| *g == "phonk").count(), 1);
    }
}
