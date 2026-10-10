//! Effects after the equalizer: reverb (Freeverb: 8 combs + 4 allpasses per
//! channel), the loudness-normalization gain of the track, and a soft limiter
//! so neither ever clips. Settings change live like the EQ's.
//!
//! Speed (slowed / sped up) is not here: the sink plays faster or slower,
//! pitch included, the way those edits sound.
//!
//! Loudness: `measure_gain` decodes the track once (every 4th frame, a few
//! tens of ms) and gives the gain that brings it to a common level.

use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};

use rodio::{source::SeekError, Decoder, Source};

/// Live effect settings shared with the audio thread.
pub struct FxShared {
    /// 0.0 … 1.0
    reverb: AtomicU32,
    /// playback rate 0.7 … 1.3
    speed: AtomicU32,
    normalize: AtomicU32,
}

impl FxShared {
    pub fn new(reverb: f32, speed: f32, normalize: bool) -> Arc<Self> {
        let s = Arc::new(Self { reverb: AtomicU32::new(0), speed: AtomicU32::new(0), normalize: AtomicU32::new(0) });
        s.set(reverb, speed, normalize);
        s
    }

    pub fn set(&self, reverb: f32, speed: f32, normalize: bool) {
        self.reverb.store(reverb.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        self.speed.store(speed.clamp(0.7, 1.3).to_bits(), Ordering::Relaxed);
        self.normalize.store(normalize as u32, Ordering::Relaxed);
    }

    pub fn reverb(&self) -> f32 {
        f32::from_bits(self.reverb.load(Ordering::Relaxed))
    }

    pub fn speed(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Relaxed))
    }

    pub fn normalize(&self) -> bool {
        self.normalize.load(Ordering::Relaxed) != 0
    }
}

// ------------------------------------------------------------- loudness

/// Level the tracks are brought to (RMS, about -14 LUFS for typical music).
const TARGET_RMS: f32 = 0.20;
const MIN_GAIN: f32 = 0.35; // -9 dB
const MAX_GAIN: f32 = 2.0; // +6 dB

/// Gain that brings the track to the common level; 1.0 if it can't be decoded.
pub fn measure_gain(data: &[u8]) -> f32 {
    let Ok(dec) = Decoder::new(Cursor::new(data.to_vec())) else { return 1.0 };
    let channels = dec.channels().max(1) as usize;
    let (mut sum, mut n) = (0f64, 0u64);
    // every 4th frame is plenty for an average and 4× faster
    for (i, s) in dec.convert_samples::<f32>().enumerate() {
        if (i / channels) % 4 == 0 {
            sum += (s as f64) * (s as f64);
            n += 1;
        }
    }
    if n < 1000 {
        return 1.0;
    }
    let rms = (sum / n as f64).sqrt() as f32;
    if rms < 1e-4 {
        return 1.0;
    }
    (TARGET_RMS / rms).clamp(MIN_GAIN, MAX_GAIN)
}

// --------------------------------------------------------------- reverb

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
/// right channel lines are a little longer (stereo width)
const SPREAD: usize = 23;
const FEEDBACK: f32 = 0.84;
const DAMP: f32 = 0.2;
const INPUT_GAIN: f32 = 0.015;

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], pos: 0, store: 0.0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        let out = self.buf[self.pos];
        self.store = out * (1.0 - DAMP) + self.store * DAMP;
        self.buf[self.pos] = x + self.store * FEEDBACK;
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], pos: 0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.pos];
        self.buf[self.pos] = x + b * 0.5;
        self.pos = (self.pos + 1) % self.buf.len();
        b - x
    }
}

struct Lane {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

impl Lane {
    fn new(rate: u32, spread: usize) -> Self {
        let scale = rate as f32 / 44_100.0;
        let len = |l: usize| ((l + spread) as f32 * scale) as usize;
        Self {
            combs: COMBS.iter().map(|&l| Comb::new(len(l))).collect(),
            allpasses: ALLPASSES.iter().map(|&l| Allpass::new(len(l))).collect(),
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let input = x * INPUT_GAIN;
        let mut out: f32 = self.combs.iter_mut().map(|c| c.process(input)).sum();
        for a in &mut self.allpasses {
            out = a.process(out);
        }
        out
    }

    fn clear(&mut self) {
        for c in &mut self.combs {
            c.buf.iter_mut().for_each(|v| *v = 0.0);
            c.store = 0.0;
        }
        for a in &mut self.allpasses {
            a.buf.iter_mut().for_each(|v| *v = 0.0);
        }
    }
}

/// Soft knee above 0.9: loud peaks bend instead of clipping.
fn limit(x: f32) -> f32 {
    const KNEE: f32 = 0.9;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let over = a - KNEE;
        x.signum() * (KNEE + (1.0 - KNEE) * (over / (1.0 - KNEE)).tanh())
    }
}

pub struct Effects<S> {
    inner: S,
    shared: Arc<FxShared>,
    /// loudness gain of this track (1.0 = as is)
    gain: f32,
    channels: u16,
    channel: u16,
    lanes: Vec<Lane>,
    /// reverb wet level, read every 1024 samples
    wet: f32,
    norm: bool,
    counter: u32,
}

impl<S: Source<Item = f32>> Effects<S> {
    pub fn new(inner: S, shared: Arc<FxShared>, gain: f32) -> Self {
        let channels = inner.channels().max(1);
        // the reverb lanes are allocated only once reverb is turned on
        Self { inner, wet: shared.reverb(), norm: shared.normalize(), shared, gain, channels, channel: 0, lanes: Vec::new(), counter: 0 }
    }
}

impl<S: Source<Item = f32>> Iterator for Effects<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let x = self.inner.next()?;
        if self.counter == 0 && self.channel == 0 {
            self.wet = self.shared.reverb();
            self.norm = self.shared.normalize();
            if self.wet > 0.0 && self.lanes.is_empty() {
                let rate = self.inner.sample_rate();
                self.lanes = (0..self.channels).map(|c| Lane::new(rate, c as usize * SPREAD)).collect();
            }
        }
        self.counter = (self.counter + 1) % 1024;
        let ch = self.channel as usize;
        self.channel = (self.channel + 1) % self.channels;

        let mut y = if self.norm { x * self.gain } else { x };
        if self.wet > 0.0 {
            if let Some(lane) = self.lanes.get_mut(ch) {
                let r = lane.process(y);
                // the dry part dips a little as the room grows, so the sum stays even
                y = y * (1.0 - 0.35 * self.wet) + r * self.wet * 3.0;
            }
        }
        Some(if self.norm || self.wet > 0.0 { limit(y) } else { y })
    }
}

impl<S: Source<Item = f32>> Source for Effects<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)?;
        self.lanes.iter_mut().for_each(Lane::clear);
        self.channel = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_never_exceeds_one() {
        for x in [-5.0f32, -1.2, -0.95, 0.0, 0.5, 0.9, 0.95, 1.5, 10.0] {
            let y = limit(x);
            assert!(y.abs() <= 1.0, "{x} → {y}");
            assert_eq!(y.signum(), if x == 0.0 { y.signum() } else { x.signum() });
        }
        assert_eq!(limit(0.5), 0.5);
    }

    #[test]
    fn reverb_tail_decays() {
        let mut lane = Lane::new(44_100, 0);
        let first = lane.process(1.0);
        let mut energy_late = 0.0;
        for i in 0..200_000 {
            let y = lane.process(0.0);
            if i > 190_000 {
                energy_late += y * y;
            }
        }
        assert!(first.abs() < 1.0);
        assert!(energy_late < 1e-6, "{energy_late}");
    }
}
