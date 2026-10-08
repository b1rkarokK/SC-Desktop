//! "Same song, slightly different title" detection for the wave.
//!
//! «Showdown» by Shadowraze and «Showdoun shadowraze» are the same song;
//! «Showdown (speed up)» or «Showdown hardtekk» are different versions and
//! must stay. A title is reduced to a *core* (lowercase letters/digits, no
//! artist names, no brackets, no feat.) plus a set of *version markers*.
//! Two tracks are duplicates when markers match and cores are near-equal.

use std::collections::BTreeSet;

use crate::api::soundcloud::ScTrack;

/// (variant spellings, canonical marker)
const VERSIONS: &[(&[&str], &str)] = &[
    (&["sped up", "speed up", "speedup", "spedup", "speed-up", "sped-up", "ускорен"], "speedup"),
    (&["slowed", "slow version", "замедлен"], "slowed"),
    (&["reverb"], "reverb"),
    (&["hardtekk", "hardtek", "hard tekk"], "hardtekk"),
    (&["hardstyle"], "hardstyle"),
    (&["nightcore"], "nightcore"),
    (&["phonk"], "phonk"),
    (&["bass boosted", "bassboosted", "bass boost"], "bassboost"),
    (&["8d"], "8d"),
    (&["remix", "rmx", "ремикс"], "remix"),
    (&["extended"], "extended"),
    (&["instrumental", "инструментал"], "instrumental"),
    (&["acapella", "a cappella", "acappella"], "acapella"),
    (&["cover", "кавер"], "cover"),
    (&["live"], "live"),
    (&["vip"], "vip"),
    (&["flip"], "flip"),
    (&["bootleg"], "bootleg"),
    (&["mashup", "mash up"], "mashup"),
    (&["rework"], "rework"),
    (&["edit"], "edit"),
    (&["jersey club", "jerseyclub"], "jersey"),
    (&["drill"], "drill"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleKey {
    /// title words without brackets / feat. / version markers
    words: Vec<String>,
    /// words of the uploader and credited artist names
    artist_words: Vec<String>,
    versions: BTreeSet<&'static str>,
}

impl TitleKey {
    /// Title core with the artist names of *both* tracks removed — a re-upload
    /// often puts the real artist into the title ("Showdoun shadowraze").
    fn core_without(&self, other: &TitleKey) -> String {
        self.words
            .iter()
            .filter(|w| !self.artist_words.contains(w) && !other.artist_words.contains(w))
            .flat_map(|w| w.chars())
            .collect()
    }
}

fn contains_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' | '【' => depth += 1,
            ')' | ']' | '}' | '】' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

pub fn key(t: &ScTrack) -> TitleKey {
    let title = t.title.to_lowercase();
    let mut versions = BTreeSet::new();
    for (variants, canon) in VERSIONS {
        if variants.iter().any(|v| contains_word(&title, v)) {
            versions.insert(*canon);
        }
    }

    let mut core = strip_brackets(&title);
    // version markers out first, so "showdown - speed up" keeps "showdown"
    for (variants, _) in VERSIONS {
        for v in *variants {
            if contains_word(&core, v) {
                core = core.replace(v, " ");
            }
        }
    }
    // "artist - title": keep the part after the dash (if anything is left there)
    for sep in [" - ", " – ", " — "] {
        if let Some((left, rest)) = core.split_once(sep) {
            let rest_has_text = rest.chars().any(char::is_alphanumeric);
            core = if rest_has_text { rest.to_owned() } else { left.to_owned() };
            break;
        }
    }
    // feat. and everything after it
    for m in [" feat.", " feat ", " ft.", " ft ", " prod.", " prod "] {
        if let Some(i) = core.find(m) {
            core.truncate(i);
        }
    }
    let split = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
    };
    let artist_words = [t.uploader().to_lowercase(), t.artist_name().to_lowercase()]
        .iter()
        .flat_map(|n| split(n))
        .filter(|w| w.chars().count() >= 3)
        .collect();
    TitleKey { words: split(&core), artist_words, versions }
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + usize::from(ca != cb));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

pub fn same_song(a: &TitleKey, b: &TitleKey) -> bool {
    if a.versions != b.versions {
        return false;
    }
    let (ka, kb) = (a.core_without(b), b.core_without(a));
    if ka.is_empty() || kb.is_empty() {
        return false;
    }
    if ka == kb {
        return true;
    }
    let (ca, cb): (Vec<char>, Vec<char>) = (ka.chars().collect(), kb.chars().collect());
    let (short, long) = if ca.len() <= cb.len() { (&ca, &cb) } else { (&cb, &ca) };
    if short.len() < 4 {
        return false;
    }
    // one title fully inside the other ("showdown" / "showdownoriginal")
    let diff = long.len() - short.len();
    if diff <= 8 && (ka.contains(&kb) || kb.contains(&ka)) {
        return true;
    }
    let dist = levenshtein(&ca, &cb);
    let ratio = 1.0 - dist as f64 / long.len() as f64;
    ratio >= 0.82
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str) -> ScTrack {
        serde_json::from_value(serde_json::json!({
            "id": 1, "title": title, "duration": 1000,
            "user": { "id": 2, "username": artist }
        }))
        .unwrap()
    }

    #[test]
    fn typo_and_artist_in_title_is_same_song() {
        let a = key(&track("Showdown", "Shadowraze"));
        let b = key(&track("Showdoun shadowraze", "some reupload"));
        assert!(same_song(&a, &b));
    }

    #[test]
    fn versions_are_different_songs() {
        let base = key(&track("Showdown", "Shadowraze"));
        for v in ["Showdown (speed up)", "Showdown hardtekk", "SHOWDOWN [slowed + reverb]", "Showdown remix"] {
            assert!(!same_song(&base, &key(&track(v, "Shadowraze"))), "{v}");
        }
        // same version on both sides → same song
        assert!(same_song(&key(&track("Showdown (sped up)", "x")), &key(&track("showdown - speed up", "x"))));
    }

    #[test]
    fn different_songs_stay_different() {
        assert!(!same_song(&key(&track("Midnight Drive", "a")), &key(&track("Midnight Rain", "a"))));
        assert!(!same_song(&key(&track("Run", "a")), &key(&track("Ran", "a"))));
    }
}
