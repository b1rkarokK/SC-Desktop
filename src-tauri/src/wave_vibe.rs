//! «Настроение» of the wave: sad, happy, aggressive … SoundCloud gives no mood or tempo data, so a track's mood is
//! read from the words it carries: its genre, tags and title, in Russian and English. A track with words of the
//! chosen mood is far more likely to be picked, one with the opposite mood's words far less, one with no such words
//! stays in at a lower chance (most uploads have no tags at all).

use crate::api::soundcloud::ScTrack;

/// The moods the user can pick ("any" = no mood chosen).
pub const VIBES: [&str; 10] = ["any", "calm", "energetic", "sad", "happy", "aggressive", "romantic", "party", "focus", "night"];

pub fn is_vibe(v: &str) -> bool {
    VIBES.contains(&v)
}

/// A word of the list: `abc*` matches words that begin with `abc`, `abc` only the whole word.
type Words = &'static [&'static str];

struct Vibe {
    /// the words of this mood
    yes: Words,
    /// the words of a mood that does not go with it
    no: Words,
    /// what to search for to find more of it (Russian, English)
    query: (&'static str, &'static str),
}

fn vibe(v: &str) -> Option<Vibe> {
    Some(match v {
        "calm" => Vibe {
            yes: &["ambient", "chill*", "lofi", "lo-fi", "acoustic", "piano", "downtempo", "classical", "jazz", "sleep*", "relax*", "soft", "slow", "dream*", "folk", "soul", "спокой*", "тих*", "расслаб*", "нежн*", "мягк*", "сон", "колыбел*"],
            no: &["techno", "hardstyle", "metal", "hardcore", "dubstep", "rave", "punk", "drill", "phonk", "агресс*", "жёстк*", "жестк*"],
            query: ("спокойная музыка", "calm chill"),
        },
        "energetic" => Vibe {
            yes: &["techno", "dnb", "drum", "hardstyle", "house", "edm", "trap", "phonk", "rock", "dubstep", "rave", "electro", "bass", "punk", "jungle", "trance", "energy", "energetic", "hype", "pump*", "workout", "gym", "драйв", "бодр*", "энерг*", "качает", "заряд*", "бас"],
            no: &["ambient", "sleep*", "lofi", "lo-fi", "acoustic", "piano", "relax*", "slow", "спокой*", "расслаб*"],
            query: ("энергичная музыка драйв", "energy hype"),
        },
        "sad" => Vibe {
            yes: &["sad*", "depress*", "melanchol*", "heartbreak*", "broken", "cry*", "tears", "lonely", "alone", "pain", "sorrow", "blue", "emo", "grief", "lost", "miss", "goodbye", "breakup", "ballad", "грусть", "грустн*", "печал*", "тоск*", "одинок*", "слёз*", "слез*", "боль", "расстава*", "разбит*", "депресс*", "скуч*", "плач*", "прощай*", "страдан*", "меланхол*"],
            no: &["party", "happy", "dance", "hype", "banger", "fun", "club", "summer", "весел*", "танц*", "вечерин*", "тусов*", "радост*", "счасть*"],
            query: ("грустная песня", "sad"),
        },
        "happy" => Vibe {
            yes: &["happy", "joy*", "sunny", "sunshine", "summer", "smile*", "fun", "upbeat", "feelgood", "good", "cheer*", "party", "dance", "positive", "лето", "летн*", "радост*", "весел*", "солнц*", "счасть*", "улыб*", "тепл*", "позитив*", "танц*", "отпуск*"],
            no: &["sad*", "dark", "depress*", "death", "cry*", "tears", "lonely", "pain", "грусть", "грустн*", "печал*", "тоск*", "слёз*", "слез*", "боль", "депресс*"],
            query: ("весёлая песня", "happy upbeat"),
        },
        "aggressive" => Vibe {
            yes: &["aggressive", "angry", "rage", "hard", "hardcore", "metal", "death", "punk", "drill", "phonk", "scream*", "brutal", "riot", "war", "kill*", "violent", "mosh", "hate", "злой", "злост*", "агресс*", "ярост*", "жёстк*", "жестк*", "бунт", "война", "ненавис*", "рэйдж", "тяжёл*", "тяжел*"],
            no: &["calm", "chill*", "soft", "acoustic", "lofi", "lo-fi", "sleep*", "relax*", "ambient", "спокой*", "нежн*", "тих*"],
            query: ("агрессивный трек", "aggressive hard"),
        },
        "romantic" => Vibe {
            yes: &["love*", "romantic", "romance", "kiss*", "heart*", "valentine*", "passion", "sweet", "lover", "crush", "tender", "любов*", "романт*", "сердц*", "нежн*", "поцел*", "влюб*", "страст*", "обним*", "люблю"],
            no: &["aggressive", "hardcore", "metal", "death", "kill*", "hate", "drill", "агресс*", "ненавис*", "злой"],
            query: ("песня про любовь", "love romantic"),
        },
        "party" => Vibe {
            yes: &["party", "club*", "dance", "edm", "house", "banger", "hype", "festival", "rave", "twerk", "turn", "disco", "remix", "вечерин*", "танц*", "клуб*", "тусов*", "зажига*", "бас", "диско"],
            no: &["sad*", "ambient", "sleep*", "acoustic", "lonely", "depress*", "грусть", "грустн*", "печал*", "тоск*"],
            query: ("танцевальная вечеринка", "party dance"),
        },
        "focus" => Vibe {
            yes: &["focus", "study", "work", "concentration", "instrumental", "lofi", "lo-fi", "ambient", "minimal", "beats", "brain", "reading", "coding", "учеб*", "работ*", "концентр*", "фокус", "инструментал*"],
            no: &["scream*", "metal", "hardcore", "rage", "party", "hype", "banger", "вечерин*"],
            query: ("музыка для работы и учёбы", "lofi focus study"),
        },
        "night" => Vibe {
            yes: &["night*", "midnight", "dark", "drive", "moon*", "noir", "late", "neon", "nocturne", "after", "3am", "city", "ноч*", "полноч*", "тьм*", "тёмн*", "темн*", "неон*", "луна", "драйв", "город*"],
            no: &["morning", "sunny", "sunshine", "summer", "утро", "утренн*", "солнц*", "лето"],
            query: ("ночная музыка", "night drive"),
        },
        _ => return None,
    })
}

fn tokens(t: &ScTrack) -> Vec<String> {
    format!("{} {} {}", t.genre.as_deref().unwrap_or(""), t.tag_list.as_deref().unwrap_or(""), t.title)
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

fn hits(words: Words, toks: &[String]) -> usize {
    words
        .iter()
        .filter(|w| match w.strip_suffix('*') {
            Some(stem) => toks.iter().any(|t| t.starts_with(stem)),
            None => toks.iter().any(|t| t == **w),
        })
        .count()
}

/// How much the chosen mood multiplies a track's chance: 1.0 when no mood is chosen.
pub fn factor(v: &str, t: &ScTrack) -> f64 {
    let Some(vibe) = vibe(v) else { return 1.0 };
    let toks = tokens(t);
    let (yes, no) = (hits(vibe.yes, &toks), hits(vibe.no, &toks));
    match (yes, no) {
        (0, 0) => 0.5,
        (0, _) => 0.12,
        (y, 0) => (2.8 + 0.5 * (y.min(3) - 1) as f64).min(3.8),
        (y, n) if y > n => 1.4,
        _ => 0.3,
    }
}

/// A search that finds more tracks of the mood, in the listener's language.
pub fn query(v: &str, russian: bool) -> Option<String> {
    vibe(v).map(|x| if russian { x.query.0 } else { x.query.1 }.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, genre: &str, tags: &str) -> ScTrack {
        serde_json::from_value(serde_json::json!({
            "id": 1, "title": title, "duration": 180000, "genre": genre, "tag_list": tags,
            "user": { "id": 2, "username": "x" }
        }))
        .unwrap()
    }

    #[test]
    fn a_sad_word_lifts_a_track_and_the_opposite_word_sinks_it() {
        let sad = track("Грустная песня про дождь", "", "");
        let happy = track("Summer party", "dance", "");
        let none = track("Untitled 4", "", "");
        assert!(factor("sad", &sad) > 2.5, "{}", factor("sad", &sad));
        assert!(factor("sad", &happy) < 0.2);
        assert_eq!(factor("sad", &none), 0.5);
        assert!(factor("happy", &happy) > 2.5);
    }

    #[test]
    fn a_word_is_matched_whole_unless_it_is_a_stem() {
        // "blue" is a sad word, but "blueprint" is not
        assert_eq!(factor("sad", &track("Blueprint", "", "")), 0.5);
        // a stem catches the forms of a Russian word
        assert!(factor("sad", &track("Тоскую по тебе", "", "")) > 2.5);
    }

    #[test]
    fn no_mood_changes_nothing() {
        assert_eq!(factor("any", &track("Грустная песня", "", "")), 1.0);
        assert_eq!(factor("normal", &track("Грустная песня", "", "")), 1.0);
    }

    #[test]
    fn every_listed_mood_is_known_and_has_a_search() {
        for v in VIBES.iter().filter(|v| **v != "any") {
            assert!(query(v, true).is_some() && query(v, false).is_some(), "{v}");
        }
        assert!(is_vibe("sad") && !is_vibe("whatever"));
    }
}
