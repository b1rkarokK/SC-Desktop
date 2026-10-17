//! «Что ещё добавляют рядом»: a collaborative signal for the wave that needs no accounts of other people and no
//! server of ours. People put songs they like together in playlists. The playlists that hold a seed track are
//! SoundCloud's own «в плейлистах» list of that track; the tracks that sit next to it there, in several playlists
//! and close to it, are what its listeners also keep. That is how "people who like this like that" is built, with
//! playlists as the baskets. The result is added to the candidates next to «похожие» of SoundCloud.

use crate::{
    api::soundcloud::SoundCloud,
    error::AppResult,
    state::AppState,
};

/// a playlist shorter than this is a stub, longer than this a dump of everything: neither says much
const MIN_LEN: usize = 6;
const MAX_LEN: usize = 200;
/// tracks this close to the seed in a playlist count double
const NEAR: usize = 5;
/// how many neighbours of one seed are kept
const KEEP: usize = 60;
/// the answer for a seed is kept this long
const TTL_SECS: i64 = 24 * 3600;

/// The tracks that sit next to `seed` in `playlists`, strongest first: the more playlists, the closer to the seed
/// and the shorter the playlist (a small themed one says more than a huge one), the higher.
pub fn neighbours(seed: u64, playlists: &[Vec<u64>]) -> Vec<(u64, f32)> {
    let mut acc: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
    for list in playlists {
        if !(MIN_LEN..=MAX_LEN).contains(&list.len()) {
            continue;
        }
        let Some(at) = list.iter().position(|&id| id == seed) else { continue };
        let w = 1.0 / ((list.len() + 2) as f32).ln();
        // a track listed twice in one playlist counts once
        let mut seen = std::collections::HashSet::new();
        for (i, &id) in list.iter().enumerate() {
            if id == seed || !seen.insert(id) {
                continue;
            }
            let near = if i.abs_diff(at) <= NEAR { 2.0 } else { 1.0 };
            *acc.entry(id).or_default() += w * near;
        }
    }
    let mut out: Vec<(u64, f32)> = acc.into_iter().collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out.truncate(KEEP);
    out
}

/// The neighbours of a seed, from SoundCloud once a day and then from the local cache.
pub async fn neighbours_cached(state: &AppState, sc: &SoundCloud, seed: u64) -> AppResult<Vec<(u64, f32)>> {
    let key = format!("wave:cop:{seed}");
    if let Ok(Some((hit, at))) = state.db.kv_get::<Vec<(u64, f32)>>(&key).await {
        if crate::db::cache::now() - at < TTL_SECS {
            return Ok(hit);
        }
    }
    let lists = sc.track_in_playlists(seed).await?;
    let n = neighbours(seed, &lists);
    state.db.kv_put(&key, &n).await?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(seed: u64, others: &[u64], pad: u64) -> Vec<u64> {
        // `pad` unrelated tracks make it long enough
        let mut v: Vec<u64> = (1000..1000 + pad).collect();
        v.insert(0, seed);
        for (i, o) in others.iter().enumerate() {
            v.insert(1 + i, *o);
        }
        v
    }

    #[test]
    fn a_track_in_several_playlists_next_to_the_seed_wins() {
        let lists = vec![list(1, &[7, 8], 12), list(1, &[7, 9], 12), list(1, &[7], 12)];
        let n = neighbours(1, &lists);
        assert_eq!(n[0].0, 7, "{n:?}");
        assert!(n[0].1 > n.iter().find(|x| x.0 == 8).unwrap().1);
    }

    #[test]
    fn stubs_and_dumps_say_nothing() {
        let tiny = vec![1, 2, 3];
        let dump: Vec<u64> = std::iter::once(1).chain(5000..5300).collect();
        assert!(neighbours(1, &[tiny, dump]).is_empty());
    }

    #[test]
    fn the_seed_itself_and_a_playlist_without_it_do_not_count() {
        let with = list(1, &[7], 10);
        let without: Vec<u64> = (2000..2012).collect();
        let n = neighbours(1, &[with, without]);
        assert!(n.iter().all(|x| x.0 != 1));
        assert!(!n.iter().any(|x| x.0 == 2000));
    }

    #[test]
    fn close_to_the_seed_is_worth_more_than_far() {
        let mut l: Vec<u64> = vec![1, 50];
        l.extend(100..130);
        l.push(60);
        let n = neighbours(1, &[l]);
        let near = n.iter().find(|x| x.0 == 50).unwrap().1;
        let far = n.iter().find(|x| x.0 == 60).unwrap().1;
        assert!(near > far);
    }
}
