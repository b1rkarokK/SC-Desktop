//! Spectrum for the full-screen visualizer: Effects copies the left channel
//! into `TAP`; while the visualizer is on, a thread turns the last 1024
//! samples into 32 log-spaced bands ~30 times a second ("viz:bands", 0-255).
//! Off by default: nothing is copied and no thread wakes up.

use std::{
    sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    time::Duration,
};

use rustfft::{num_complex::Complex, FftPlanner};
use tauri::{AppHandle, Emitter};

pub const N: usize = 1024;
pub const BANDS: usize = 32;

pub struct Tap {
    pub on: AtomicBool,
    pos: AtomicUsize,
    buf: [AtomicU32; N],
}

pub static TAP: Tap = Tap { on: AtomicBool::new(false), pos: AtomicUsize::new(0), buf: [const { AtomicU32::new(0) }; N] };

impl Tap {
    #[inline]
    pub fn push(&self, x: f32) {
        let p = self.pos.fetch_add(1, Ordering::Relaxed);
        self.buf[p % N].store(x.to_bits(), Ordering::Relaxed);
    }
}

pub fn set(app: &AppHandle, on: bool) {
    let was = TAP.on.swap(on, Ordering::Relaxed);
    if on && !was {
        let app = app.clone();
        let _ = std::thread::Builder::new().name("viz".into()).spawn(move || run(app));
    }
}

fn run(app: AppHandle) {
    let fft = FftPlanner::<f32>::new().plan_fft_forward(N);
    let window: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
    // band edges in FFT bins: 2 … 400 on a log scale (≈ 85 Hz … 17 kHz)
    let edges: Vec<usize> = (0..=BANDS).map(|b| (2.0 * (200f32).powf(b as f32 / BANDS as f32)).round() as usize).collect();
    let mut last_pos = usize::MAX;
    let mut quiet = 0;
    while TAP.on.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(33));
        let pos = TAP.pos.load(Ordering::Relaxed);
        let bands: Vec<u8> = if pos == last_pos {
            quiet += 1;
            if quiet > 3 {
                continue; // paused: the UI lets the bars fall by itself
            }
            vec![0; BANDS]
        } else {
            quiet = 0;
            let mut c: Vec<Complex<f32>> = (0..N)
                .map(|i| Complex::new(f32::from_bits(TAP.buf[(pos + i) % N].load(Ordering::Relaxed)) * window[i], 0.0))
                .collect();
            fft.process(&mut c);
            (0..BANDS)
                .map(|b| {
                    let (lo, hi) = (edges[b], edges[b + 1].max(edges[b] + 1));
                    let m = c[lo..hi.min(N / 2)].iter().map(|v| v.norm()).fold(0.0f32, f32::max) / (N as f32 / 4.0);
                    let db = 20.0 * m.max(1e-6).log10(); // 0 dB = full scale
                    (((db + 66.0) / 66.0).clamp(0.0, 1.0) * 255.0) as u8
                })
                .collect()
        };
        last_pos = pos;
        let _ = app.emit("viz:bands", bands);
    }
}
