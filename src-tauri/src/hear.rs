//! Hearing a song: when no source has lyrics for a track under its name (a re-upload with another title,
//! "track 3", a name in transliteration), a small speech model listens to 15 seconds of it and the words it
//! hears are looked up in the lyrics search (the same as «Поиск по тексту»).
//!
//! Kept light on purpose:
//! * runs only for a track nobody found lyrics for, once per track (the answer is kept, also "nothing");
//! * one at a time, 2 CPU threads, about a second of work on a 2013+ CPU (AVX2); older CPUs skip it;
//! * the model (whisper base, 57 MB) is downloaded on first use, loaded for the job and dropped right after:
//!   between jobs it takes no memory at all.

use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::{
    api::lyrics::{self, Lyrics, LyricsState, Query},
    error::{AppError, AppResult},
    net::HttpClient,
};

const MODEL_FILE: &str = "ggml-base-q5_1.bin";
const MODEL_SHA256: &str = "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898";
/// the second one is a mirror of the first, for networks where it is blocked
const MODEL_URLS: [&str; 2] = [
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin",
    "https://hf-mirror.com/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin",
];
const RATE: u32 = 16_000;
const CLIP_SECS: u32 = 15;
/// the model's window cut to the clip (1500 = 30 s): half the work
const AUDIO_CTX: i32 = 768;
const THREADS: i32 = 2;

static BUSY: AtomicBool = AtomicBool::new(false);
static MODEL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The engine is built for AVX2 + FMA + F16C: on an older CPU it would crash, so it is not run there.
pub fn supported() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") && is_x86_feature_detected!("f16c")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// The model file, downloaded (and checked) the first time it is needed.
async fn model(http: &HttpClient, dir: &Path) -> AppResult<PathBuf> {
    let _one = MODEL_LOCK.lock().await;
    let path = dir.join(MODEL_FILE);
    if path.exists() {
        return Ok(path);
    }
    tokio::fs::create_dir_all(dir).await?;
    let part = dir.join(format!("{MODEL_FILE}.part"));
    let mut last = AppError::Other("model download failed".into());
    for url in MODEL_URLS {
        tracing::info!(url, "hear: downloading the speech model");
        match http.download_file(url, &part).await {
            Ok(sha) if sha == MODEL_SHA256 => {
                tokio::fs::rename(&part, &path).await?;
                return Ok(path);
            }
            Ok(sha) => last = AppError::Other(format!("model checksum mismatch: {sha}")),
            Err(e) => last = e,
        }
        let _ = tokio::fs::remove_file(&part).await;
    }
    Err(last)
}

/// `Arc<Vec<u8>>` read in place by the decoder (no copy of the whole track).
struct Shared(Arc<Vec<u8>>);
impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// `CLIP_SECS` of the track from `start_s`, mono 16 kHz.
fn clip(audio: Arc<Vec<u8>>, start_s: u32) -> Option<Vec<f32>> {
    use rodio::Source;
    let dec = rodio::Decoder::new(Cursor::new(Shared(audio))).ok()?;
    let (channels, rate) = (dec.channels().max(1) as usize, dec.sample_rate().max(1));
    let skip = start_s as usize * rate as usize * channels;
    let take = CLIP_SECS as usize * rate as usize * channels;
    let mut mono = Vec::with_capacity(CLIP_SECS as usize * rate as usize);
    let (mut sum, mut n) = (0f32, 0);
    for s in dec.convert_samples::<f32>().skip(skip).take(take) {
        sum += s;
        n += 1;
        if n == channels {
            mono.push(sum / channels as f32);
            (sum, n) = (0.0, 0);
        }
    }
    if mono.len() < rate as usize * 5 {
        return None;
    }
    // linear resampling to 16 kHz: enough for a voice
    let step = rate as f64 / RATE as f64;
    let len = (mono.len() as f64 / step) as usize;
    Some(
        (0..len)
            .map(|i| {
                let x = i as f64 * step;
                let (j, f) = (x as usize, (x.fract()) as f32);
                let a = mono[j];
                let b = *mono.get(j + 1).unwrap_or(&a);
                a + (b - a) * f
            })
            .collect(),
    )
}

/// What the model hears in the clip. Blocking: the model is loaded, used and dropped here.
fn transcribe(model: &Path, pcm: &[f32], lang: &str) -> Option<String> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
    static QUIET: std::sync::Once = std::sync::Once::new();
    QUIET.call_once(whisper_rs::install_logging_hooks);
    let ctx = WhisperContext::new_with_params(model.to_str()?, WhisperContextParameters::default()).ok()?;
    let mut st = ctx.create_state().ok()?;
    let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    p.set_n_threads(THREADS);
    p.set_language(Some(lang));
    p.set_audio_ctx(AUDIO_CTX);
    // no second passes at other temperatures: a clip it cannot make out is just skipped
    p.set_temperature_inc(0.0);
    p.set_no_context(true);
    p.set_single_segment(true);
    p.set_no_timestamps(true);
    p.set_suppress_blank(true);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    st.full(p, pcm).ok()?;
    let text: String = (0..st.full_n_segments()).filter_map(|i| st.get_segment(i)?.to_str_lossy().ok().map(|s| s.into_owned())).collect();
    Some(text)
}

/// Words of a text, lower case, letters only ("[Music]" and the like dropped).
fn words(text: &str) -> Vec<String> {
    let mut clean = String::with_capacity(text.len());
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth - 1).max(0),
            _ if depth == 0 => clean.push(c),
            _ => {}
        }
    }
    clean
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase().replace('ё', "е"))
        .collect()
}

/// Up to five short phrases to search by. The search wants every word of a phrase, and the model mishears
/// some: a phrase of 3-5 words is the most likely to come out whole. The model's own pieces (between commas)
/// of that size first, then longer pieces cut in fours.
fn phrases(text: &str) -> Vec<String> {
    if words(text).len() < 5 {
        return vec![];
    }
    let pieces: Vec<Vec<String>> = text.split(['.', ',', '!', '?', ';', '…']).map(words).collect();
    let short = pieces.iter().filter(|p| (3..=5).contains(&p.len())).cloned();
    let cut = pieces.iter().filter(|p| p.len() > 5).flat_map(|p| p.chunks(4).filter(|c| c.len() >= 3).map(|c| c.to_vec()).collect::<Vec<_>>());
    let mut out: Vec<String> = Vec::new();
    for p in short.chain(cut) {
        let s = p.join(" ");
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out.truncate(5);
    out
}

/// How much of what was heard is in the lyrics (0 … 1): the check that a found song is this one.
fn overlap(heard: &str, l: &Lyrics) -> (f32, usize) {
    let text: String = l.lines.iter().map(|x| x.text.as_str()).collect::<Vec<_>>().join(" ");
    let have: std::collections::HashSet<String> = words(&text).into_iter().collect();
    let heard: Vec<String> = words(heard).into_iter().filter(|w| w.chars().count() >= 3).collect();
    if heard.is_empty() {
        return (0.0, 0);
    }
    let hit = heard.iter().filter(|w| have.contains(*w)).count();
    (hit as f32 / heard.len() as f32, hit)
}

/// Lyrics of the song heard in `audio`, if the words lead to one. `None`: nothing clear enough.
pub async fn find(
    http: &HttpClient,
    state: &LyricsState,
    models: &Path,
    audio: Arc<Vec<u8>>,
    duration_ms: u64,
    cyrillic: bool,
) -> AppResult<Option<Lyrics>> {
    if !supported() || BUSY.swap(true, Ordering::AcqRel) {
        return Ok(None);
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            BUSY.store(false, Ordering::Release);
        }
    }
    let _release = Release;
    let model = model(http, models).await?;
    let secs = (duration_ms / 1000) as u32;
    // a verse or a chorus rather than the intro; a second try further in if the first clip has no words
    for start in [(secs * 3 / 10).max(10), secs * 6 / 10] {
        if start + CLIP_SECS > secs {
            continue;
        }
        let (audio, model) = (audio.clone(), model.clone());
        let lang = if cyrillic { "ru" } else { "auto" };
        let heard = tokio::task::spawn_blocking(move || clip(audio, start).and_then(|pcm| transcribe(&model, &pcm, lang)))
            .await
            .map_err(|e| AppError::Other(e.to_string()))?
            .unwrap_or_default();
        tracing::info!(start, heard = %heard.chars().take(160).collect::<String>(), "hear: what the model heard");
        if let Some(l) = by_words(http, state, &heard, duration_ms).await? {
            return Ok(Some(l));
        }
    }
    Ok(None)
}

/// The lyrics search with phrases of `heard`; a song counts only if its lyrics hold enough of the heard words.
/// An error (the search is down): the track is not marked as heard, it is tried again another time.
async fn by_words(http: &HttpClient, state: &LyricsState, heard: &str, duration_ms: u64) -> AppResult<Option<Lyrics>> {
    let mut tried = std::collections::HashSet::new();
    for phrase in phrases(heard) {
        let songs = lyrics::search_by_lyrics(http, state, &phrase).await?;
        tracing::debug!(%phrase, found = songs.len(), "hear: searched");
        // a known length that is far off is another song (or another version: those are found by name already)
        let fits = songs.into_iter().filter(|(_, _, len)| *len == 0 || len.abs_diff(duration_ms) <= 5_000).take(3);
        for (artist, title, _) in fits {
            if !tried.insert(format!("{artist}\u{1}{title}")) {
                continue;
            }
            let q = Query { kind: lyrics::version_kind(&title), artist, title, duration_ms, version_of: None };
            let Ok(Some(l)) = lyrics::find(http, state, &q).await else { continue };
            let (share, hits) = overlap(heard, &l);
            tracing::info!(artist = %q.artist, title = %q.title, share, hits, "hear: candidate");
            if share >= 0.35 && hits >= 4 {
                return Ok(Some(l));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{phrases, words};

    #[test]
    fn phrases_are_the_longest_heard_pieces() {
        let p = phrases(" Равалную изстал с обменомин. Этот цивюду нахал на плевал на всех. Запомни, есть два типа и людей. [Music]");
        assert_eq!(p, ["равалную изстал с обменомин", "есть два типа и людей", "этот цивюду нахал на", "плевал на всех"]);
        assert!(phrases("[Music]").is_empty());
    }

    /// Real model and network, every `*.pcm` (f32 16 kHz mono) of a folder with one Musixmatch token:
    /// `HEAR_MODEL=…bin HEAR_DIR=… cargo test hear_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn hear_live() {
        let _ = tracing_subscriber::fmt().with_env_filter("sc_desk::hear=debug").with_test_writer().try_init();
        let model = std::path::PathBuf::from(std::env::var("HEAR_MODEL").unwrap());
        let http = crate::net::HttpClient::new(&crate::config::AppConfig::default()).unwrap();
        let state = crate::api::lyrics::LyricsState::default();
        for f in std::fs::read_dir(std::env::var("HEAR_DIR").unwrap()).unwrap().flatten() {
            let bytes = std::fs::read(f.path()).unwrap();
            let pcm: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            let t = std::time::Instant::now();
            let heard = super::transcribe(&model, &pcm, "ru").unwrap();
            println!("=== {:?} heard in {:?}: {heard}", f.file_name(), t.elapsed());
            let l = super::by_words(&http, &state, &heard, 0).await.unwrap_or_else(|e| { println!("search failed: {e}"); None });
            println!("found: {:?}", l.map(|l| (l.source, l.lines.into_iter().find(|x| !x.text.trim().is_empty()).map(|x| x.text))));
        }
    }

    #[test]
    fn words_drop_the_marks() {
        assert_eq!(words("Ёлки (музыка) [Music] палки!"), vec!["елки", "палки"]);
    }
}
