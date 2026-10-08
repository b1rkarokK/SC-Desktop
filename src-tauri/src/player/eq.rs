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

#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone, Copy, Default)]
struct BiquadState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

fn peaking(fs: f32, f0: f32, gain_db: f32) -> Biquad {
    let a = 10f32.powf(gain_db / 40.0);
    let w0 = 2.0 * std::f32::consts::PI * f0 / fs;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * Q);
    let a0 = 1.0 + alpha / a;
    Biquad {
        b0: (1.0 + alpha * a) / a0,
        b1: (-2.0 * cos) / a0,
        b2: (1.0 - alpha * a) / a0,
        a1: (-2.0 * cos) / a0,
        a2: (1.0 - alpha / a) / a0,
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
        self.preamp = 10f32.powf(cfg.preamp / 20.0);
        let fs = rate as f32;
        self.filters = EQ_BANDS
            .iter()
            .zip(cfg.gains.iter())
            .filter(|(f, g)| g.abs() >= 0.05 && **f < fs * 0.45)
            .map(|(f, g)| peaking(fs, *f, g.clamp(-12.0, 12.0)))
            .collect();
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
            let mut x = sample * self.preamp;
            let ch = self.channel as usize;
            let stride = self.channels as usize;
            for (i, f) in self.filters.iter().enumerate() {
                let st = &mut self.states[i * stride + ch];
                let y = f.b0 * x + f.b1 * st.x1 + f.b2 * st.x2 - f.a1 * st.y1 - f.a2 * st.y2;
                st.x2 = st.x1;
                st.x1 = x;
                st.y2 = st.y1;
                st.y1 = y;
                x = y;
            }
            x.clamp(-1.0, 1.0)
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
        assert!((f.b0 - 1.0).abs() < 1e-5 && (f.b1 - f.a1).abs() < 1e-5 && (f.b2 - f.a2).abs() < 1e-5);
    }
}
