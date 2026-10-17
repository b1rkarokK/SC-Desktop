//! «Слух» of the wave: what a candidate sounds like, learned BEFORE it is played. The taste model already takes six
//! coarse numbers about a track's sound (audio_feat.rs), but only for tracks that were played, so a brand-new song
//! was judged by its name and tags alone. Here the runners-up of a batch (good candidates that were not chosen this
//! time) are listened to in the background: the first ~30 seconds, a few hundred KB, decoded in a fraction of a
//! second. The numbers are kept, and the next batches judge those songs by ear as well.
//!
//! It is meant to cost almost nothing: a few clips per batch, at most a dozen an hour, one at a time, on a thread
//! of the lowest priority, never while a game is in front or the microphone is in use, a track that failed once is
//! not tried again, and nothing is asked of the user.

use std::{
    collections::{HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use crate::{api::soundcloud::{ScTrack, SoundCloud}, db::cache::Db};

/// HLS segments are about ten seconds: three of them are the first half-minute
const SEGMENTS: usize = 3;
/// clips in one run, and in any hour
const PER_RUN: usize = 8;
const PER_HOUR: usize = 12;
/// a clip that takes longer than this (a slow link) is dropped
const CLIP_TIMEOUT: Duration = Duration::from_secs(25);
/// the pause between two clips: the machine is never kept busy
const BREATH: Duration = Duration::from_millis(1500);

static RUNNING: AtomicBool = AtomicBool::new(false);
static DONE: Mutex<VecDeque<Instant>> = Mutex::new(VecDeque::new());
static FAILED: Mutex<Option<HashSet<u64>>> = Mutex::new(None);

/// Whether another clip fits in the hour; counts it when it does.
fn allow(window: &mut VecDeque<Instant>, now: Instant, per_hour: usize) -> bool {
    while window.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(3600)) {
        window.pop_front();
    }
    if window.len() >= per_hour {
        return false;
    }
    window.push_back(now);
    true
}

fn failed_before(id: u64) -> bool {
    FAILED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).contains(&id)
}

fn remember_failure(id: u64) {
    let mut f = FAILED.lock().unwrap_or_else(|e| e.into_inner());
    let set = f.get_or_insert_with(HashSet::new);
    if set.len() > 5000 {
        set.clear();
    }
    set.insert(id);
}

/// Runs `f` on this (blocking-pool) thread at the lowest priority, then puts the priority back.
#[cfg(windows)]
fn at_low_priority<T>(f: impl FnOnce() -> T) -> T {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST, THREAD_PRIORITY_NORMAL};
    // SAFETY: the pseudo-handle of the current thread is always valid.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST) };
    let r = f();
    // SAFETY: as above.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_NORMAL) };
    r
}

#[cfg(not(windows))]
fn at_low_priority<T>(f: impl FnOnce() -> T) -> T {
    f()
}

/// Listens to up to `PER_RUN` of `candidates` (best first) in the background. Returns at once.
pub fn warm(sc: SoundCloud, db: Db, candidates: Vec<ScTrack>) {
    let todo: Vec<ScTrack> = candidates
        .into_iter()
        .filter(|t| crate::audio_feat::get(t.id).is_none() && !failed_before(t.id) && t.is_fully_playable())
        .take(PER_RUN)
        .collect();
    if todo.is_empty() || RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut got = 0;
        for t in todo {
            // never in the way of a game or a call
            if crate::autopause::busy() {
                break;
            }
            if !allow(&mut DONE.lock().unwrap_or_else(|e| e.into_inner()), Instant::now(), PER_HOUR) {
                break;
            }
            match tokio::time::timeout(CLIP_TIMEOUT, sc.download_clip(&t, SEGMENTS)).await {
                Ok(Ok(bytes)) => {
                    let feat = tokio::task::spawn_blocking(move || at_low_priority(|| crate::audio_feat::compute(&bytes))).await.ok().flatten();
                    match feat {
                        Some(f) => {
                            crate::audio_feat::put(t.id, f);
                            let _ = db.audio_feat_put(t.id, f).await;
                            got += 1;
                        }
                        None => remember_failure(t.id),
                    }
                }
                _ => remember_failure(t.id),
            }
            tokio::time::sleep(BREATH).await;
        }
        if got > 0 {
            tracing::debug!(got, "wave ear: candidates listened to");
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_more_than_the_allowed_clips_in_an_hour() {
        let mut w = VecDeque::new();
        let t0 = Instant::now();
        for _ in 0..PER_HOUR {
            assert!(allow(&mut w, t0, PER_HOUR));
        }
        assert!(!allow(&mut w, t0, PER_HOUR), "the thirteenth in the same hour");
        // an hour and a minute later the old ones have expired
        assert!(allow(&mut w, t0 + Duration::from_secs(3660), PER_HOUR));
    }

    #[test]
    fn a_track_that_failed_is_not_tried_again() {
        assert!(!failed_before(987_654_321));
        remember_failure(987_654_321);
        assert!(failed_before(987_654_321));
    }

    /// The real chain on a real clip (init + three segments of an AAC HLS stream, saved by hand):
    /// `CLIP=<file> cargo test real_clip -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_clip() {
        let Ok(path) = std::env::var("CLIP") else { return };
        let raw = std::fs::read(path).unwrap();
        let adts = crate::api::fmp4::to_adts(&raw).expect("the partial fMP4 unpacks");
        let started = Instant::now();
        let feat = at_low_priority(|| crate::audio_feat::compute(&adts));
        println!("raw {} bytes, adts {} bytes, features {:?} in {} ms", raw.len(), adts.len(), feat, started.elapsed().as_millis());
        assert!(feat.is_some(), "the clip is decoded");
    }

    #[test]
    fn the_work_runs_and_returns_its_result_at_low_priority() {
        assert_eq!(at_low_priority(|| 2 + 2), 4);
    }
}
