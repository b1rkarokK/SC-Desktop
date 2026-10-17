//! 10-band peaking equalizer as a rodio `Source` wrapper (RBJ biquads).
//! Settings change live: the audio thread picks them up every 2048 samples.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use rodio::{source::SeekError, Source};

use crate::config::{EqConfig, EQ_BANDS};

const Q: f32 = 1.0;
const REFRESH_EVERY: u32 = 2048;

pub struct EqShared {
    generation: AtomicU64,
    params: Mutex<EqConfig>,
}

impl EqShared {
    pub fn new(cfg: EqConfig) -> Arc<Self> {
        Arc::new(Self { generation: AtomicU64::new(1), params: Mutex::new(cfg) })
    }

    pub fn set(&self, cfg: EqConfig) {
        *self.params.lock().unwrap_or_else(|p| p.into_inner()) = cfg;
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn snapshot(&self) -> (u64, EqConfig) {
        let g = self.generation.load(Ordering::Acquire);
        (g, self.params.lock().unwrap_or_else(|p| p.into_inner()).clone())
    }
}

/// Filters run in f64: a low band (32 Hz at 44.1 kHz) has poles very close to 1, and f32 rounding there is heard
/// as a rough, crackling bass.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

/// Transposed direct form II state.
#[derive(Clone, Copy, Default)]
struct BiquadState {
    s1: f64,
    s2: f64,
}

fn peaking(fs: f32, f0: f32, gain_db: f32) -> Biquad {
    let a = 10f64.powf(f64::from(gain_db) / 40.0);
    let w0 = 2.0 * std::f64::consts::PI * f64::from(f0) / f64::from(fs);
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * f64::from(Q));
    let a0 = 1.0 + alpha / a;
    Biquad {
        b0: (1.0 + alpha * a) / a0,
        b1: (-2.0 * cos) / a0,
        b2: (1.0 - alpha * a) / a0,
        a1: (-2.0 * cos) / a0,
        a2: (1.0 - alpha / a) / a0,
    }
}

/// Gain of all the filters together at `hz`, in dB.
fn response_db(filters: &[Biquad], fs: f32, hz: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * hz / f64::from(fs);
    let (s1, c1) = w.sin_cos();
    let (s2, c2) = (2.0 * w).sin_cos();
    let mut db = 0.0;
    for f in filters {
        let (nr, ni) = (f.b0 + f.b1 * c1 + f.b2 * c2, -(f.b1 * s1 + f.b2 * s2));
        let (dr, di) = (1.0 + f.a1 * c1 + f.a2 * c2, -(f.a1 * s1 + f.a2 * s2));
        db += 10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10();
    }
    db
}

/// How much to turn the whole signal down so that the boosted bands do not push a loud track over full scale.
/// The loudest point of the combined curve (neighbouring bands add up) plus the preamp, less a small margin
/// that the soft limiter catches. Other equalizers (Equalizer APO and the like) do the same by hand.
fn headroom_db(filters: &[Biquad], fs: f32, preamp_db: f32) -> f64 {
    const MARGIN_DB: f64 = 2.0;
    if filters.is_empty() {
        return 0.0;
    }
    let top = (f64::from(fs) * 0.45).min(20_000.0);
    let peak = (0..96)
        .map(|i| response_db(filters, fs, 20.0 * (top / 20.0).powf(f64::from(i) / 95.0)))
        .fold(f64::MIN, f64::max);
    (peak + f64::from(preamp_db) - MARGIN_DB).max(0.0)
}

/// Transparent below 0.85, then bends towards 1.0 instead of cutting the wave off flat (that flat cut is the crackle).
fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.85;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}

pub struct Equalizer<S> {
    inner: S,
    shared: Arc<EqShared>,
    seen: u64,
    rate: u32,
    channels: u16,
    channel: u16,
    counter: u32,
    enabled: bool,
    preamp: f32,
    filters: Vec<Biquad>,
    /// filters.len() × channels
    states: Vec<BiquadState>,
}

impl<S: Source<Item = f32>> Equalizer<S> {
    pub fn new(inner: S, shared: Arc<EqShared>) -> Self {
        let channels = inner.channels().max(1);
        let rate = inner.sample_rate();
        let mut eq = Self {
            inner,
            shared,
            seen: 0,
            rate,
            channels,
            channel: 0,
            counter: 0,
            enabled: false,
            preamp: 1.0,
            filters: Vec::new(),
            states: Vec::new(),
        };
        eq.refresh(true);
        eq
    }

    fn refresh(&mut self, force: bool) {
        let rate = self.inner.sample_rate();
        let (generation, cfg) = self.shared.snapshot();
        if !force && generation == self.seen && rate == self.rate {
            return;
        }
        self.seen = generation;
        self.rate = rate;
        self.enabled = cfg.enabled;
        let fs = rate as f32;
        self.filters = EQ_BANDS
            .iter()
            .zip(cfg.gains.iter())
            .filter(|(f, g)| g.abs() >= 0.05 && **f < fs * 0.45)
            .map(|(f, g)| peaking(fs, *f, g.clamp(-12.0, 12.0)))
            .collect();
        let down = headroom_db(&self.filters, fs, cfg.preamp);
        self.preamp = 10f32.powf((cfg.preamp - down as f32) / 20.0);
        let need = self.filters.len() * self.channels as usize;
        if self.states.len() != need {
            self.states = vec![BiquadState::default(); need];
        }
    }
}

impl<S: Source<Item = f32>> Iterator for Equalizer<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;
        if self.counter == 0 && self.channel == 0 {
            self.refresh(false);
        }
        self.counter = (self.counter + 1) % REFRESH_EVERY;

        let out = if self.enabled {
            let mut x = f64::from(sample * self.preamp);
            let ch = self.channel as usize;
            let stride = self.channels as usize;
            for (i, f) in self.filters.iter().enumerate() {
                let st = &mut self.states[i * stride + ch];
                let y = f.b0 * x + st.s1;
                st.s1 = f.b1 * x - f.a1 * y + st.s2;
                st.s2 = f.b2 * x - f.a2 * y;
                x = y;
            }
            soft_limit(x as f32).clamp(-1.0, 1.0)
        } else {
            sample
        };
        self.channel = (self.channel + 1) % self.channels;
        Some(out)
    }
}

impl<S: Source<Item = f32>> Source for Equalizer<S> {
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
        self.states.iter_mut().for_each(|s| *s = BiquadState::default());
        self.channel = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_gain_band_is_identity() {
        let f = peaking(44_100.0, 1000.0, 0.0);
        assert!((f.b0 - 1.0).abs() < 1e-9 && (f.b1 - f.a1).abs() < 1e-9 && (f.b2 - f.a2).abs() < 1e-9);
    }

    use rodio::buffer::SamplesBuffer;

    fn cfg(enabled: bool, band: usize, gain: f32, preamp: f32) -> EqConfig {
        let mut gains = [0.0; 10];
        gains[band] = gain;
        EqConfig { enabled, preamp, gains, preset: "custom".into() }
    }

    /// A 1 kHz sine at 0.1 amplitude, `n` mono samples at 44.1 kHz.
    fn sine(n: usize, hz: f32) -> Vec<f32> {
        (0..n).map(|i| 0.1 * (2.0 * std::f32::consts::PI * hz * i as f32 / 44_100.0).sin()).collect()
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    /// RMS of the settled part of the stream through an equalizer.
    fn through(shared: &Arc<EqShared>, input: Vec<f32>) -> f32 {
        let out: Vec<f32> = Equalizer::new(SamplesBuffer::new(1, 44_100, input), shared.clone()).collect();
        rms(&out[out.len() / 2..])
    }

    #[test]
    fn a_boosted_band_makes_that_frequency_louder() {
        let flat = through(&EqShared::new(cfg(true, 5, 0.0, 0.0)), sine(44_100, 1000.0));
        let boosted = through(&EqShared::new(cfg(true, 5, 12.0, 0.0)), sine(44_100, 1000.0));
        // +12 dB at the band's centre (x3.98), less the 10 dB the whole signal is turned down for headroom: about x1.26
        assert!((boosted / flat - 1.26).abs() < 0.1, "{}", boosted / flat);
    }

    #[test]
    fn a_bass_boost_on_a_loud_track_is_not_cut_off_flat() {
        // a loud 60 Hz tone: +12 dB on the bass bands would be far over full scale without the headroom
        let mut gains = [0.0; 10];
        gains[0] = 12.0;
        gains[1] = 12.0;
        gains[2] = 6.0;
        let shared = EqShared::new(EqConfig { enabled: true, preamp: 0.0, gains, preset: "custom".into() });
        let loud: Vec<f32> = (0..44_100).map(|i| 0.95 * (2.0 * std::f32::consts::PI * 60.0 * i as f32 / 44_100.0).sin()).collect();
        let out: Vec<f32> = Equalizer::new(SamplesBuffer::new(1, 44_100, loud), shared).collect();
        let settled = &out[out.len() / 2..];
        let flat_topped = settled.iter().filter(|x| x.abs() >= 0.999).count();
        assert_eq!(flat_topped, 0, "{flat_topped} samples hit the ceiling");
        assert!(settled.iter().any(|x| x.abs() > 0.5), "the bass is still there");
    }

    #[test]
    fn the_soft_limit_is_transparent_below_the_knee_and_never_passes_one() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert_eq!(soft_limit(-0.8), -0.8);
        assert!(soft_limit(3.0) <= 1.0 && soft_limit(-3.0) >= -1.0);
        assert!(soft_limit(1.2) < 1.0, "a moderate overshoot still bends below the ceiling");
        assert!(soft_limit(0.9) > 0.85 && soft_limit(0.9) < soft_limit(1.2));
    }

    #[test]
    fn a_cut_band_makes_it_quieter_and_other_frequencies_stay() {
        let cut = through(&EqShared::new(cfg(true, 5, -12.0, 0.0)), sine(44_100, 1000.0));
        assert!((cut / 0.0707 - 0.251).abs() < 0.02, "{}", cut / 0.0707);
        // 32 Hz is far from the 1 kHz band: almost untouched
        // (it is only turned down with the whole signal for headroom: -10 dB, x0.316)
        let far = through(&EqShared::new(cfg(true, 5, 12.0, 0.0)), sine(44_100, 32.0));
        assert!((far / 0.0707 - 0.316).abs() < 0.04, "{}", far / 0.0707);
    }

    #[test]
    fn switched_off_it_changes_nothing_and_preamp_is_plain_gain() {
        let off = through(&EqShared::new(cfg(false, 5, 12.0, 6.0)), sine(44_100, 1000.0));
        assert!((off / 0.0707 - 1.0).abs() < 0.01, "{}", off / 0.0707);
        let pre = through(&EqShared::new(cfg(true, 5, 0.0, -6.0)), sine(44_100, 1000.0));
        assert!((pre / 0.0707 - 0.501).abs() < 0.02, "{}", pre / 0.0707);
    }

    #[test]
    fn new_settings_are_picked_up_while_the_track_plays() {
        let shared = EqShared::new(cfg(true, 5, 0.0, 0.0));
        let mut eq = Equalizer::new(SamplesBuffer::new(1, 44_100, sine(44_100 * 2, 1000.0)), shared.clone());
        let first: Vec<f32> = (&mut eq).take(20_000).collect();
        shared.set(cfg(true, 5, 12.0, 0.0));
        let _skip: Vec<f32> = (&mut eq).take(8_000).collect();
        let later: Vec<f32> = (&mut eq).take(20_000).collect();
        assert!(rms(&later) > rms(&first[10_000..]) * 1.1, "{} vs {}", rms(&later), rms(&first[10_000..]));
    }
}
