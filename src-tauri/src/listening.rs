//! What the listening itself says about a track: how far it was heard, where it was skipped, how often it came
//! back. Every play is logged when it ends (player/mod.rs → `listens`); here those rows are turned into what the
//! taste model learns from. A plain "played once" used to count as a weak yes, whatever came of it; now a track left
//! after three seconds is a no, one heard to the end a yes, one played again and again a strong yes.

/// One play of a track.
#[derive(Debug, Clone, Copy)]
pub struct Listen {
    /// how far into the track it got when it ended or the user moved on
    pub listened_ms: u64,
    pub duration_ms: u64,
    /// played to the end (not left)
    pub ended: bool,
}

/// 0 … 1: how much of the track was heard.
pub fn completion(l: &Listen) -> f32 {
    if l.ended {
        1.0
    } else if l.duration_ms == 0 {
        0.5
    } else {
        (l.listened_ms as f32 / l.duration_ms as f32).min(1.0)
    }
}

/// What one play says: > 0 liked, < 0 turned down. A skip in the first seconds says the most.
fn value(l: &Listen) -> f32 {
    let c = completion(l);
    if c >= 0.9 {
        1.0
    } else if c >= 0.5 {
        0.3
    } else if c >= 0.2 {
        if l.listened_ms < 10_000 { -0.6 } else { -0.2 }
    } else if l.listened_ms < 5_000 {
        -1.2
    } else {
        -0.8
    }
}

/// All the plays of one track together.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Agg {
    pub plays: u32,
    /// heard to the end (or nearly)
    pub full: u32,
    /// the plays and the replays together: > 0 liked, < 0 turned down
    pub score: f32,
}

pub fn aggregate(ls: &[Listen]) -> Agg {
    let full = ls.iter().filter(|l| completion(l) >= 0.9).count() as u32;
    let sum: f32 = ls.iter().map(value).sum();
    // coming back to a song is the strongest sign there is: every full play after the first adds to it
    let replay = if full >= 2 { (0.5 * (full - 1) as f32).min(2.0) } else { 0.0 };
    Agg { plays: ls.len() as u32, full, score: sum + replay }
}

/// For the model: (a yes or a no, its weight). A track with no clear verdict teaches nothing.
pub fn training_sample(a: &Agg) -> Option<(bool, f32)> {
    if a.score >= 0.5 {
        Some((true, (a.score * 0.8).min(3.0)))
    } else if a.score <= -0.6 {
        Some((false, (-a.score).min(2.0)))
    } else {
        None
    }
}

/// How much more a track that comes back is worth as a seed and as a "familiar" pick: 1 for a single play.
pub fn replay_weight(a: &Agg) -> f64 {
    (1.0 + 0.6 * a.full.saturating_sub(1) as f64).min(4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(listened_s: u64, duration_s: u64, ended: bool) -> Listen {
        Listen { listened_ms: listened_s * 1000, duration_ms: duration_s * 1000, ended }
    }

    #[test]
    fn heard_to_the_end_is_a_yes_and_a_skip_in_seconds_is_a_no() {
        let yes = aggregate(&[l(180, 180, true)]);
        let no = aggregate(&[l(3, 180, false)]);
        assert!(training_sample(&yes).is_some_and(|(p, _)| p));
        assert!(training_sample(&no).is_some_and(|(p, _)| !p));
        // a skip in the first seconds weighs more than leaving after half a minute
        let early = aggregate(&[l(2, 200, false)]).score;
        let later = aggregate(&[l(40, 200, false)]).score;
        assert!(early < later, "{early} vs {later}");
    }

    #[test]
    fn leaving_near_the_end_counts_as_listened() {
        assert!(aggregate(&[l(170, 180, false)]).score > 0.5);
        // the middle says nothing either way
        assert!(training_sample(&aggregate(&[l(100, 180, false)])).is_none());
    }

    #[test]
    fn a_track_played_again_and_again_is_a_strong_yes() {
        let once = aggregate(&[l(180, 180, true)]);
        let often = aggregate(&[l(180, 180, true), l(180, 180, true), l(180, 180, true), l(175, 180, false)]);
        assert!(often.score > once.score + 1.0);
        assert!(training_sample(&often).unwrap().1 > training_sample(&once).unwrap().1);
        assert!(replay_weight(&often) > replay_weight(&once));
        assert_eq!(replay_weight(&once), 1.0);
    }

    #[test]
    fn one_skip_among_many_listens_does_not_turn_it_into_a_no() {
        let a = aggregate(&[l(180, 180, true), l(180, 180, true), l(4, 180, false)]);
        assert!(training_sample(&a).is_some_and(|(p, _)| p), "{a:?}");
    }

    #[test]
    fn an_unknown_length_is_neutral() {
        let a = aggregate(&[Listen { listened_ms: 30_000, duration_ms: 0, ended: false }]);
        assert!(training_sample(&a).is_none() || a.score.abs() < 1.0);
    }
}
