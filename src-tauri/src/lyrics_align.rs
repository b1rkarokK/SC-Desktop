//! Timed lyrics against the recording itself: by how much are they early or late?
//!
//! The lyrics come from a database that timed them for some recording of the song; the upload that plays may
//! start a moment earlier or later (a different intro, a cut, another master). Start times of the lines are
//! the moments somebody starts to sing, and in the audio those are the moments new energy appears in the
//! voice band. So the audio is turned into an "onset strength" curve (how much the 200-4000 Hz spectrum grew
//! at each 10 ms), and the line starts are slid over it: the shift at which they land on the loudest onsets
//! is the offset of the lyrics. Nothing is learned and nothing leaves the computer: a few tens of
//! milliseconds of work once per track.
//!
//! The answer is only used when it is clear: a distinct peak (not one of many equal ones the beat makes), well
//! above the noise and clearly better than no shift at all. Otherwise the lyrics stay as they are.

use rustfft::{num_complex::Complex, FftPlanner};

pub const RATE: u32 = 11_025;
const FRAME: usize = 512;
const HOP: usize = 110;
const LOW_HZ: f32 = 200.0;
const HIGH_HZ: f32 = 4_000.0;
/// how far each way the lyrics may be off
pub const MAX_LAG_MS: f64 = 8_000.0;
/// fewer lines say too little
const MIN_LINES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    /// add this to the timestamps of the lyrics (positive: the lyrics are early)
    pub offset_ms: i64,
    /// how far above the noise the peak is (a z-score)
    pub confidence: f32,
}

/// Milliseconds one envelope step stands for (110 samples at 11025 Hz).
fn hop_ms() -> f64 {
    HOP as f64 * 1000.0 / f64::from(RATE)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// how much the log spectrum grew in the band, per frame
    Flux,
    /// the same per narrow band, each band counted as much as the others (a beat cannot drown a voice)
    BandFlux,
    /// how much the loudness of the band rose over the last 60 ms
    EnergyRise,
}

#[derive(Clone, Copy, Debug)]
pub struct Cfg {
    pub low: f32,
    pub high: f32,
    pub mode: Mode,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { low: LOW_HZ, high: HIGH_HZ, mode: Mode::Flux }
    }
}

/// Log magnitudes, frame after frame (FRAME / 2 bins each), and the number of frames.
fn spectrogram(samples: &[f32]) -> (Vec<f32>, usize) {
    let bins = FRAME / 2;
    if samples.len() < FRAME * 4 {
        return (Vec::new(), 0);
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FRAME);
    let window: Vec<f32> = (0..FRAME).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FRAME as f32).cos()).collect();
    let mut buf = vec![Complex::new(0.0f32, 0.0); FRAME];
    let mut out = Vec::with_capacity(samples.len() / HOP * bins);
    let mut start = 0;
    while start + FRAME <= samples.len() {
        for (i, c) in buf.iter_mut().enumerate() {
            *c = Complex::new(samples[start + i] * window[i], 0.0);
        }
        fft.process(&mut buf);
        // a log scale: a quiet voice over a loud beat still counts
        out.extend(buf[..bins].iter().map(|c| (1.0 + 10.0 * c.norm()).ln()));
        start += HOP;
    }
    let frames = out.len() / bins;
    (out, frames)
}

/// Onset strength per 10 ms of mono audio at RATE.
pub fn envelope(samples: &[f32]) -> Vec<f32> {
    envelope_with(samples, Cfg::default())
}

pub fn envelope_with(samples: &[f32], cfg: Cfg) -> Vec<f32> {
    let (spec, n) = spectrogram(samples);
    if n == 0 {
        return Vec::new();
    }
    let bins = FRAME / 2;
    let bin = |hz: f32| ((hz / RATE as f32) * FRAME as f32).round() as usize;
    let (lo, hi) = (bin(cfg.low).max(1), bin(cfg.high).min(bins));
    let at = |t: usize, k: usize| spec[t * bins + k];
    let flux_in = |a: usize, b: usize| -> Vec<f32> {
        (0..n).map(|t| if t == 0 { 0.0 } else { (a..b).map(|k| (at(t, k) - at(t - 1, k)).max(0.0)).sum() }).collect()
    };
    let curve: Vec<f32> = match cfg.mode {
        Mode::Flux => flux_in(lo, hi),
        Mode::BandFlux => {
            // eight bands, equally wide on a log scale
            let edges: Vec<usize> = (0..=8).map(|j| ((lo as f32) * ((hi as f32) / (lo as f32)).powf(j as f32 / 8.0)).round() as usize).collect();
            let mut sum = vec![0f32; n];
            for j in 0..8 {
                let (a, b) = (edges[j], edges[j + 1].max(edges[j] + 1).min(hi));
                if a >= b {
                    continue;
                }
                let c = flux_in(a, b);
                let mean = c.iter().sum::<f32>() / n as f32 + 1e-6;
                for (s, x) in sum.iter_mut().zip(&c) {
                    *s += x / mean;
                }
            }
            sum
        }
        Mode::EnergyRise => {
            let e: Vec<f32> = (0..n).map(|t| (lo..hi).map(|k| at(t, k)).sum::<f32>() / (hi - lo).max(1) as f32).collect();
            (0..n).map(|t| if t < 6 { 0.0 } else { (e[t] - e[t - 6]).max(0.0) }).collect()
        }
    };
    postprocess(&curve)
}

/// What stands out from its surroundings (the last second), in standard deviations, smoothed a little.
fn postprocess(flux: &[f32]) -> Vec<f32> {
    let n = flux.len();
    let half = 50usize;
    let mut csum = vec![0f64; n + 1];
    for i in 0..n {
        csum[i + 1] = csum[i] + f64::from(flux[i]);
    }
    let local: Vec<f32> = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(half), (i + half + 1).min(n));
            let mean = (csum[b] - csum[a]) / (b - a) as f64;
            (f64::from(flux[i]) - mean).max(0.0) as f32
        })
        .collect();
    let sd = {
        let m = local.iter().sum::<f32>() / n.max(1) as f32;
        (local.iter().map(|x| (x - m).powi(2)).sum::<f32>() / n.max(1) as f32).sqrt().max(1e-6)
    };
    // a gaussian of 30 ms: line starts and onsets are never exactly on the same frame
    let kernel: Vec<f32> = (-9i32..=9).map(|i| (-(i * i) as f32 / (2.0 * 9.0)).exp()).collect();
    (0..n)
        .map(|i| {
            let mut acc = 0f32;
            for (j, w) in kernel.iter().enumerate() {
                let idx = i as i64 + j as i64 - 9;
                if idx >= 0 && (idx as usize) < n {
                    acc += local[idx as usize] * w;
                }
            }
            acc / sd
        })
        .collect()
}

/// The raw readings before any decision (for tuning the thresholds on real tracks).
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Reading {
    pub lag_ms: f64,
    pub z: f32,
    pub peak: f32,
    pub second: f32,
    pub at_zero: f32,
}

/// The offset of the lyrics against the audio, if it can be told with confidence.
pub fn estimate(env: &[f32], starts_ms: &[u64]) -> Option<Estimate> {
    let r = reading(env, starts_ms)?;
    let clear = r.z >= 5.0 && r.peak >= 1.15 * r.second && r.peak >= 1.3 * r.at_zero.max(0.0) + 0.05;
    (clear && r.lag_ms.abs() >= 250.0).then_some(Estimate { offset_ms: r.lag_ms.round() as i64, confidence: r.z })
}

pub fn reading(env: &[f32], starts_ms: &[u64]) -> Option<Reading> {
    let mut starts: Vec<f64> = starts_ms.iter().map(|s| *s as f64 / hop_ms()).collect();
    starts.dedup_by(|a, b| (*a - *b).abs() < 30.0);
    if starts.len() < MIN_LINES || env.len() < 2_000 {
        return None;
    }
    let max_lag = (MAX_LAG_MS / hop_ms()) as i64;
    // the mean onset strength where the lines would start for every shift
    let lags: Vec<i64> = (-max_lag..=max_lag).collect();
    let r: Vec<f32> = lags
        .iter()
        .map(|lag| {
            let (mut sum, mut n) = (0f32, 0u32);
            for s in &starts {
                let idx = s.round() as i64 + lag;
                if idx >= 0 && (idx as usize) < env.len() {
                    sum += env[idx as usize];
                    n += 1;
                }
            }
            if n as usize >= starts.len() * 3 / 4 {
                sum / n as f32
            } else {
                0.0
            }
        })
        .collect();
    let mean = r.iter().sum::<f32>() / r.len() as f32;
    let sd = (r.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / r.len() as f32).sqrt().max(1e-6);
    let (best, &peak) = r.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    let z = (peak - mean) / sd;
    // the best of the others, away from the peak (the beat makes copies of a peak every half second)
    let apart = (400.0 / hop_ms()) as usize;
    let second = r.iter().enumerate().filter(|(i, _)| i.abs_diff(best) > apart).map(|(_, v)| *v).fold(0.0f32, f32::max);
    // no shift at all: the best reading within 40 ms of zero
    let zero = (max_lag as usize).saturating_sub(4)..=(max_lag as usize + 4);
    let at_zero = zero.map(|i| r[i]).fold(0.0f32, f32::max);

    // parabola through the peak and its neighbours: finer than the 10 ms step
    let frac = if best > 0 && best + 1 < r.len() {
        let (a, b, c) = (r[best - 1], r[best], r[best + 1]);
        let d = a - 2.0 * b + c;
        if d.abs() > 1e-9 { (0.5 * (a - c) / d).clamp(-0.5, 0.5) } else { 0.0 }
    } else {
        0.0
    };
    let lag_ms = ((best as f64 - max_lag as f64) + f64::from(frac)) * hop_ms();

    Some(Reading { lag_ms, z, peak, second, at_zero })
}

/// The whole step: audio samples (mono, `RATE`) and the timed lines.
pub fn align(samples: &[f32], starts_ms: &[u64]) -> Option<Estimate> {
    estimate(&envelope(samples), starts_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, Rng, SeedableRng};

    /// A fake song of `secs` seconds: noise, a steady beat, and a burst of "voice" at every line start.
    fn song(starts_ms: &[u64], shift_ms: i64, secs: usize) -> Vec<f32> {
        let mut rng = StdRng::seed_from_u64(3);
        let n = secs * RATE as usize;
        let mut x: Vec<f32> = (0..n).map(|_| rng.gen_range(-0.02..0.02)).collect();
        let burst = |x: &mut Vec<f32>, at_ms: i64, amp: f32, hz: f32, len_ms: i64| {
            let a = (at_ms.max(0) as f64 / 1000.0 * f64::from(RATE)) as usize;
            let len = (len_ms as f64 / 1000.0 * f64::from(RATE)) as usize;
            for i in 0..len {
                if a + i < x.len() {
                    let env = (1.0 - i as f32 / len as f32).powi(2);
                    x[a + i] += amp * env * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin();
                }
            }
        };
        // the beat: every 460 ms, a low thump
        let mut t = 0;
        while t < secs as i64 * 1000 {
            burst(&mut x, t, 0.25, 90.0, 120);
            t += 460;
        }
        // the voice: starts of the lines (as they are in the audio), a mid-band burst each
        for s in starts_ms {
            burst(&mut x, *s as i64 + shift_ms, 0.35, 700.0, 400);
        }
        x
    }

    fn lines() -> Vec<u64> {
        let mut rng = StdRng::seed_from_u64(11);
        let mut t = 9_000u64;
        let mut v = Vec::new();
        while t < 160_000 {
            v.push(t);
            t += rng.gen_range(2_200..5_800);
        }
        v
    }

    #[test]
    fn it_finds_how_far_the_lyrics_are_off_in_either_direction() {
        let starts = lines();
        // the audio sings 1.7 s after the stamps: the lyrics are early, add 1700
        let est = align(&song(&starts, 1_700, 175), &starts).expect("a clear peak");
        assert!((est.offset_ms - 1_700).abs() <= 40, "{est:?}");
        // and 2.4 s before them: subtract
        let est = align(&song(&starts, -2_400, 175), &starts).expect("a clear peak");
        assert!((est.offset_ms + 2_400).abs() <= 40, "{est:?}");
    }

    #[test]
    fn lyrics_that_are_right_are_left_alone() {
        let starts = lines();
        assert_eq!(align(&song(&starts, 0, 175), &starts), None);
        // a tenth of a second is below what is worth moving
        assert_eq!(align(&song(&starts, 100, 175), &starts), None);
    }

    #[test]
    fn noise_and_a_beat_alone_say_nothing() {
        let starts = lines();
        let none: Vec<u64> = Vec::new();
        assert_eq!(align(&song(&none, 0, 175), &starts), None);
    }

    /// Real tracks: ALIGN_DUMP=<dir with N.f32 + N.json> cargo test eval_real_dump -- --ignored --nocapture.
    /// The words of karaoke lyrics are exact, so they say where the voice really is: a feature that finds
    /// its onsets at no shift sees the voice; then the (rougher) line starts are tried on it.
    #[test]
    #[ignore]
    fn eval_real_dump() {
        let Ok(dir) = std::env::var("ALIGN_DUMP") else { return };
        let mut tracks: Vec<(String, Vec<f32>, Vec<u64>, Vec<u64>)> = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("f32") {
                continue;
            }
            let bytes = std::fs::read(&p).unwrap();
            let samples: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p.with_extension("json")).unwrap()).unwrap();
            let list = |k: &str| -> Vec<u64> { meta[k].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).collect()).unwrap_or_default() };
            tracks.push((p.file_stem().unwrap().to_string_lossy().into_owned(), samples, list("starts"), list("words")));
        }
        tracks.sort_by(|a, b| a.0.cmp(&b.0));
        println!("{} tracks, {} with word timings", tracks.len(), tracks.iter().filter(|t| t.3.len() >= 40).count());
        let cfgs: Vec<(&str, Cfg)> = vec![
            ("flux 200-4000", Cfg { low: 200.0, high: 4000.0, mode: Mode::Flux }),
            ("flux 500-2500", Cfg { low: 500.0, high: 2500.0, mode: Mode::Flux }),
            ("flux 300-3400", Cfg { low: 300.0, high: 3400.0, mode: Mode::Flux }),
            ("bandflux 150-5000", Cfg { low: 150.0, high: 5000.0, mode: Mode::BandFlux }),
            ("bandflux 300-3400", Cfg { low: 300.0, high: 3400.0, mode: Mode::BandFlux }),
            ("rise 300-3400", Cfg { low: 300.0, high: 3400.0, mode: Mode::EnergyRise }),
            ("rise 150-1500", Cfg { low: 150.0, high: 1500.0, mode: Mode::EnergyRise }),
        ];
        for (name, cfg) in cfgs {
            let (mut w_hit, mut w_n, mut l_hit, mut l_n) = (0, 0, 0, 0);
            let (mut w_z, mut l_z) = (0.0f32, 0.0f32);
            let mut detail = Vec::new();
            for (id, samples, starts, words) in &tracks {
                let env = envelope_with(samples, cfg);
                let tag = &id[id.len().saturating_sub(4)..];
                if words.len() >= 40 {
                    if let Some(r) = reading(&env, words) {
                        w_n += 1;
                        w_z += r.z;
                        if r.lag_ms.abs() <= 250.0 && r.z >= 5.0 {
                            w_hit += 1;
                        }
                        detail.push(format!("{tag}:w{:+.0}/{:.1}", r.lag_ms, r.z));
                    }
                }
                if let Some(r) = reading(&env, starts) {
                    l_n += 1;
                    l_z += r.z;
                    if r.lag_ms.abs() <= 250.0 && r.z >= 5.0 {
                        l_hit += 1;
                    }
                    detail.push(format!("{tag}:l{:+.0}/{:.1}", r.lag_ms, r.z));
                }
            }
            println!("{name:20} words: {w_hit}/{w_n} at zero, mean z {:.1} | lines: {l_hit}/{l_n} at zero, mean z {:.1}", w_z / w_n.max(1) as f32, l_z / l_n.max(1) as f32);
            println!("    {}", detail.join(" "));
        }
    }

    #[test]
    fn too_few_lines_or_too_short_audio_say_nothing() {
        let few: Vec<u64> = vec![1_000, 4_000, 8_000];
        assert_eq!(align(&song(&few, 1_500, 30), &few), None);
        assert_eq!(align(&[0.0; 100], &lines()), None);
    }
}
