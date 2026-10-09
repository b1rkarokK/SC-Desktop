//! Acoustic fingerprints: is this upload the very same recording?
//!
//! Shazam-style, no neural network needed: the audio is decoded to mono
//! 11 kHz, cut into 186 ms frames every 46 ms, and for each frame 16 bits
//! record whether the energy difference between neighbouring frequency bands
//! (300 Hz to 3 kHz) grew or shrank since the previous frame. The same
//! recording gives (almost) the same bits even through another encoder or
//! bitrate; a remix, a sped-up or slowed version, a cover or another beat
//! does not.
//!
//! A 30 s Go+ preview is searched for inside a candidate upload: the best
//! alignment's bit error rate tells whether it's the same recording.

use std::io::Cursor;

use rodio::{Decoder, Source};
use rustfft::{num_complex::Complex, FftPlanner};

const RATE: u32 = 11_025;
const FRAME: usize = 2048;
const HOP: usize = 512;
const BANDS: usize = 17;
const LOW_HZ: f32 = 300.0;
const HIGH_HZ: f32 = 3_000.0;
/// at most this much audio is fingerprinted (a whole song, not a 1-hour mix)
const MAX_SECS: u32 = 15 * 60;

/// Same recording if at most this share of bits differ at the best alignment.
/// Re-encodes of one recording stay well below; other recordings sit near 0.5.
pub const SAME_MAX_BER: f32 = 0.30;

/// 16-bit sub-fingerprints, one per 46 ms.
pub fn compute(bytes: &[u8]) -> Option<Vec<u16>> {
    compute_head(bytes, MAX_SECS)
}

/// Like `compute`, for the first `secs` seconds only (a preview sits near the
/// start of an upload: no need to decode the whole song).
pub fn compute_head(bytes: &[u8], secs: u32) -> Option<Vec<u16>> {
    let dec = Decoder::new(Cursor::new(bytes.to_vec())).ok()?;
    let channels = dec.channels().max(1) as usize;
    let rate = dec.sample_rate().max(1);
    let samples = mono_resampled(dec.convert_samples::<f32>(), channels, rate, secs.min(MAX_SECS));
    if samples.len() < FRAME * 4 {
        return None;
    }

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FRAME);
    let window: Vec<f32> = (0..FRAME).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FRAME as f32).cos()).collect();
    let edges: Vec<usize> = (0..=BANDS)
        .map(|b| {
            let hz = LOW_HZ * (HIGH_HZ / LOW_HZ).powf(b as f32 / BANDS as f32);
            ((hz / RATE as f32) * FRAME as f32).round() as usize
        })
        .collect();

    let mut prev: Option<[f32; BANDS]> = None;
    let mut out = Vec::with_capacity(samples.len() / HOP);
    let mut buf = vec![Complex::new(0.0f32, 0.0); FRAME];
    let mut start = 0;
    while start + FRAME <= samples.len() {
        for (i, c) in buf.iter_mut().enumerate() {
            *c = Complex::new(samples[start + i] * window[i], 0.0);
        }
        fft.process(&mut buf);
        let mut energy = [0f32; BANDS];
        for (b, e) in energy.iter_mut().enumerate() {
            *e = buf[edges[b]..edges[b + 1].max(edges[b] + 1)].iter().map(|c| c.norm_sqr()).sum();
        }
        if let Some(p) = prev {
            let mut bits = 0u16;
            for b in 0..16 {
                let d = (energy[b] - energy[b + 1]) - (p[b] - p[b + 1]);
                if d > 0.0 {
                    bits |= 1 << b;
                }
            }
            out.push(bits);
        }
        prev = Some(energy);
        start += HOP;
    }
    Some(out)
}

/// Downmix to mono and resample to `RATE` by averaging (cheap low-pass).
fn mono_resampled(src: impl Iterator<Item = f32>, channels: usize, rate: u32, secs: u32) -> Vec<f32> {
    let step = rate as f64 / RATE as f64;
    let max = (secs * RATE) as usize;
    let mut out = Vec::with_capacity(max.min(4 * 60 * RATE as usize));
    let (mut acc, mut n, mut pos, mut next) = (0f64, 0u32, 0f64, step);
    let mut frame_sum = 0f32;
    let mut ch = 0;
    for s in src {
        frame_sum += s;
        ch += 1;
        if ch < channels {
            continue;
        }
        let mono = frame_sum / channels as f32;
        frame_sum = 0.0;
        ch = 0;
        acc += f64::from(mono);
        n += 1;
        pos += 1.0;
        if pos >= next {
            out.push((acc / f64::from(n.max(1))) as f32);
            acc = 0.0;
            n = 0;
            next += step;
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

/// Seconds per sub-fingerprint.
pub const SECS_PER_FRAME: f32 = HOP as f32 / RATE as f32;

/// (bit error rate, position of `a` inside `b` in seconds; negative if `b` sits inside `a`).
pub fn best_match(a: &[u16], b: &[u16]) -> (f32, f32) {
    let swapped = a.len() > b.len();
    let (short, long) = if swapped { (b, a) } else { (a, b) };
    if short.len() < 64 {
        return (1.0, 0.0);
    }
    // ignore the edges of the snippet: fades and encoder warm-up
    let short = &short[8..short.len() - 8];
    let total = (short.len() * 16) as f32;
    let mut best = u32::MAX;
    let mut best_off = 0usize;
    for off in 0..=(long.len() - short.len()) {
        let mut errs = 0u32;
        for (x, y) in short.iter().zip(&long[off..]) {
            errs += (x ^ y).count_ones();
            if errs >= best {
                break;
            }
        }
        if errs < best {
            best = errs;
            best_off = off;
        }
    }
    // +8: the edge frames trimmed from the snippet above
    let secs = best_off as f32 * SECS_PER_FRAME - 8.0 * SECS_PER_FRAME;
    (best as f32 / total, if swapped { -secs } else { secs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_snippet_and_rejects_noise() {
        let long: Vec<u16> = (0..3000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u16).collect();
        let snippet: Vec<u16> = long[1200..1800].to_vec();
        assert!(best_match(&snippet, &long).0 < 0.01);
        let other: Vec<u16> = (0..600u32).map(|i| (i.wrapping_mul(40_503) ^ 0x5a5a) as u16).collect();
        assert!(best_match(&other, &long).0 > 0.3);
    }
}
