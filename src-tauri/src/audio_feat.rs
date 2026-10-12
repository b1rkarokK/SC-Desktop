//! What a track sounds like, in a handful of coarse numbers, for the taste model.
//!
//! The audio is downloaded for playing anyway; from its first two minutes six values are taken
//! (tempo, brightness, loudness, dynamics, bass share, busyness), each cut into eight bins. They
//! feed the wave model as extra features next to genre and tags, so two tracks with the same
//! genre but a different feel (calm / hard) no longer look the same. Only tracks that were played
//! have them. A couple of hundred milliseconds of work once per track, in the background.

use std::{
    collections::HashMap,
    io::Cursor,
    sync::{OnceLock, RwLock},
};

use rodio::{Decoder, Source};
use rustfft::{num_complex::Complex, FftPlanner};

/// bins per value
pub const BINS: u8 = 8;
pub const N: usize = 6;
const RATE: u32 = 11_025;
const FRAME: usize = 1024;
const HOP: usize = 512;
const SECS: u32 = 120;

pub type Feat = [u8; N];

fn store() -> &'static RwLock<HashMap<u64, Feat>> {
    static S: OnceLock<RwLock<HashMap<u64, Feat>>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn get(id: u64) -> Option<Feat> {
    store().read().unwrap_or_else(|e| e.into_inner()).get(&id).copied()
}

pub fn put(id: u64, f: Feat) {
    store().write().unwrap_or_else(|e| e.into_inner()).insert(id, f);
}

pub fn loaded() -> bool {
    !store().read().unwrap_or_else(|e| e.into_inner()).is_empty()
}

pub fn load(all: HashMap<u64, Feat>) {
    *store().write().unwrap_or_else(|e| e.into_inner()) = all;
}

fn bin(x: f32, lo: f32, hi: f32) -> u8 {
    (((x - lo) / (hi - lo)).clamp(0.0, 0.999) * f32::from(BINS)) as u8
}

/// Six bins, or None if the audio cannot be decoded / is too short.
pub fn compute(bytes: &[u8]) -> Option<Feat> {
    let dec = Decoder::new(Cursor::new(bytes.to_vec())).ok()?;
    let channels = dec.channels().max(1) as usize;
    let rate = dec.sample_rate().max(1);
    let samples = crate::fingerprint::mono_resampled(dec.convert_samples::<f32>(), channels, rate, SECS);
    of_samples(&samples)
}

/// The same from mono samples at `RATE`.
fn of_samples(samples: &[f32]) -> Option<Feat> {
    if samples.len() < FRAME * 40 {
        return None;
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FRAME);
    let window: Vec<f32> = (0..FRAME).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FRAME as f32).cos()).collect();
    let hz = |k: usize| k as f32 * RATE as f32 / FRAME as f32;

    let mut buf = vec![Complex::new(0.0f32, 0.0); FRAME];
    let (mut rms, mut centroid, mut bass, mut flux) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut prev = vec![0f32; FRAME / 2];
    let mut start = 0;
    while start + FRAME <= samples.len() {
        let mut sq = 0f32;
        for (i, c) in buf.iter_mut().enumerate() {
            let s = samples[start + i];
            sq += s * s;
            *c = Complex::new(s * window[i], 0.0);
        }
        rms.push((sq / FRAME as f32).sqrt());
        fft.process(&mut buf);
        let (mut tot, mut weighted, mut low, mut fl) = (1e-9f32, 0f32, 0f32, 0f32);
        for k in 1..FRAME / 2 {
            let m = buf[k].norm();
            tot += m;
            weighted += m * hz(k);
            if hz(k) < 250.0 {
                low += m;
            }
            let d = m - prev[k];
            if d > 0.0 {
                fl += d;
            }
            prev[k] = m;
        }
        centroid.push(weighted / tot);
        bass.push(low / tot);
        flux.push(fl / tot);
        start += HOP;
    }

    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let loud = mean(&rms);
    // dynamics: how much the loudness moves (loud and flat vs. quiet and breathing)
    let var = rms.iter().map(|x| (x - loud).powi(2)).sum::<f32>() / rms.len() as f32;
    let dynamics = var.sqrt() / (loud + 1e-6);

    // tempo: the autocorrelation of the onset curve, 70 to 180 beats per minute
    let frames_per_sec = RATE as f32 / HOP as f32;
    let m = mean(&flux);
    let on: Vec<f32> = flux.iter().map(|x| x - m).collect();
    let (lo, hi) = ((frames_per_sec * 60.0 / 180.0) as usize, (frames_per_sec * 60.0 / 70.0) as usize);
    let mut best = (0f32, lo);
    for lag in lo..=hi.min(on.len() / 2) {
        let r: f32 = on.iter().zip(&on[lag..]).map(|(a, b)| a * b).sum::<f32>() / (on.len() - lag) as f32;
        if r > best.0 {
            best = (r, lag);
        }
    }
    let mut bpm = 60.0 * frames_per_sec / best.1 as f32;
    // a half or double of the true tempo is the same feel: fold into 80..160
    while bpm < 80.0 {
        bpm *= 2.0;
    }
    while bpm > 160.0 {
        bpm /= 2.0;
    }

    Some([
        bin(bpm, 80.0, 160.0),
        bin(mean(&centroid), 600.0, 2600.0),
        bin(loud, 0.02, 0.25),
        bin(dynamics, 0.2, 1.0),
        bin(mean(&bass), 0.15, 0.7),
        bin(mean(&flux), 0.05, 0.5),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, secs: u32, loud: f32) -> Vec<f32> {
        (0..RATE * secs).map(|i| (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin() * loud).collect()
    }

    #[test]
    fn a_dull_quiet_tone_and_a_bright_loud_one_land_in_different_bins() {
        let dull = of_samples(&tone(120.0, 30, 0.1)).unwrap();
        let bright = of_samples(&tone(3000.0, 30, 0.6)).unwrap();
        assert!(bright[1] > dull[1], "brightness {:?} vs {:?}", bright, dull);
        assert!(bright[2] > dull[2], "loudness {:?} vs {:?}", bright, dull);
        assert!(dull[4] > bright[4], "bass share {:?} vs {:?}", dull, bright);
    }

    #[test]
    fn too_short_audio_gives_nothing() {
        assert!(of_samples(&tone(440.0, 1, 0.3)).is_none());
    }

    #[test]
    fn garbage_bytes_give_nothing() {
        assert!(compute(&[1, 2, 3, 4]).is_none());
    }
}
