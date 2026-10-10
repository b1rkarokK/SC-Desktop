//! Native playback on a dedicated thread (rodio + symphonia).
//!
//! CPU budget:
//! * Playing: the thread wakes at 4 Hz only to publish position / detect end.
//! * Paused or stopped: blocks on the channel (0 wakeups). After 30 s idle the
//!   output device is closed too — an open WASAPI/CoreAudio stream keeps the
//!   mixer callback running even when silent. Resume transparently reopens it
//!   and seeks back to the saved position.
//! * Crossfade: for the few seconds two tracks overlap, 20 Hz volume steps.

use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::{
    eq::{EqShared, Equalizer},
    fx::{Effects, FxShared},
};
use crate::error::{AppError, AppResult};

const POLL_PLAYING: Duration = Duration::from_millis(250);
const POLL_FADING: Duration = Duration::from_millis(50);
const RELEASE_DEVICE_AFTER: Duration = Duration::from_secs(30);
/// ticks are 250 ms apart → device check every 2 s while playing
const DEVICE_CHECK_TICKS: u32 = 8;

pub enum AudioCmd {
    /// `gain`: loudness normalization of this track; `fade`: crossfade from
    /// the playing track over that long
    Load { data: Arc<Vec<u8>>, generation: u64, gain: f32, duration_ms: u64, fade: Option<Duration> },
    /// crossfade length (0 = off): `NearEnd` is reported that long before the end
    Crossfade(Duration),
    Replay { generation: u64 },
    Play,
    Pause,
    Stop,
    Seek(Duration),
    Volume(f32),
}

#[derive(Debug)]
pub enum AudioEvent {
    Ended { generation: u64 },
    /// the crossfade into the next track should start now
    NearEnd { generation: u64 },
    /// sound actually started (widget engine: after buffering / DRM licence)
    Started { generation: u64 },
    Error { generation: u64, message: String },
}

pub struct AudioHandle {
    tx: mpsc::Sender<AudioCmd>,
    position_ms: Arc<AtomicU64>,
    events: UnboundedSender<AudioEvent>,
}

impl AudioHandle {
    pub fn send(&self, cmd: AudioCmd) {
        if self.tx.send(cmd).is_err() {
            tracing::error!("audio thread is gone");
        }
    }

    pub fn position_ms(&self) -> u64 {
        self.position_ms.load(Ordering::Relaxed)
    }

    /// The event channel, shared with the widget engine.
    pub fn events(&self) -> UnboundedSender<AudioEvent> {
        self.events.clone()
    }
}

/// Spawns the audio thread; fails if no output device can be opened at all.
pub fn spawn(eq: Arc<EqShared>, fx: Arc<FxShared>) -> AppResult<(AudioHandle, UnboundedReceiver<AudioEvent>)> {
    let (tx, rx) = mpsc::channel();
    let (ev_tx, ev_rx) = unbounded_channel();
    let (init_tx, init_rx) = mpsc::channel::<Result<(), String>>();
    let position_ms = Arc::new(AtomicU64::new(0));
    let pos = position_ms.clone();
    let events = ev_tx.clone();

    thread::Builder::new()
        .name("audio".into())
        .spawn(move || {
            let mut engine = Engine::new(ev_tx, pos, eq, fx);
            match engine.output() {
                Ok(_) => {
                    let _ = init_tx.send(Ok(()));
                }
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return;
                }
            }
            engine.run(rx);
        })?;

    match init_rx.recv() {
        Ok(Ok(())) => Ok((AudioHandle { tx, position_ms, events }, ev_rx)),
        Ok(Err(e)) => Err(AppError::Audio(format!("нет устройства вывода: {e}"))),
        Err(_) => Err(AppError::Audio("audio thread exited during init".into())),
    }
}

/// `Cursor` over shared bytes so Repeat-One / device reopen never re-download.
#[derive(Clone)]
struct SharedBytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

struct Engine {
    stream: Option<(OutputStream, OutputStreamHandle)>,
    sink: Option<Sink>,
    current: Option<Arc<Vec<u8>>>,
    generation: u64,
    volume: f32,
    /// Position to resume from after the device was released while paused.
    suspended_at: Option<Duration>,
    events: UnboundedSender<AudioEvent>,
    position_ms: Arc<AtomicU64>,
    eq: Arc<EqShared>,
    fx: Arc<FxShared>,
    /// name of the device the open stream plays to
    device: Option<String>,
    ticks: u32,
    /// loudness gain and length of the current track
    gain: f32,
    duration_ms: u64,
    crossfade: Duration,
    near_end_sent: bool,
    /// the previous track fading out under the new one
    fading_out: Option<Sink>,
    /// a crossfade in progress: (started, length)
    fade: Option<(Instant, Duration)>,
}

impl Engine {
    fn new(events: UnboundedSender<AudioEvent>, position_ms: Arc<AtomicU64>, eq: Arc<EqShared>, fx: Arc<FxShared>) -> Self {
        Self {
            eq,
            fx,
            gain: 1.0,
            duration_ms: 0,
            crossfade: Duration::ZERO,
            near_end_sent: false,
            fading_out: None,
            fade: None,
            device: None,
            ticks: 0,
            stream: None,
            sink: None,
            current: None,
            generation: 0,
            volume: 0.8,
            suspended_at: None,
            events,
            position_ms,
        }
    }

    fn run(&mut self, rx: mpsc::Receiver<AudioCmd>) {
        loop {
            let playing = self.sink.as_ref().is_some_and(|s| !s.is_paused() && !s.empty());
            let timeout = if self.fade.is_some() {
                Some(POLL_FADING)
            } else if playing {
                Some(POLL_PLAYING)
            } else if self.stream.is_some() {
                Some(RELEASE_DEVICE_AFTER)
            } else {
                None
            };

            let cmd = match timeout {
                Some(t) => match rx.recv_timeout(t) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => {
                        if !playing {
                            self.release_device();
                        }
                        None
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(c) => Some(c),
                    Err(_) => break,
                },
            };

            if let Some(cmd) = cmd {
                self.apply(cmd);
            }
            self.tick();
        }
        tracing::info!("audio thread finished");
    }

    fn apply(&mut self, cmd: AudioCmd) {
        match cmd {
            AudioCmd::Load { data, generation, gain, duration_ms, fade } => {
                self.generation = generation;
                self.current = Some(data);
                self.gain = gain;
                self.duration_ms = duration_ms;
                self.near_end_sent = false;
                self.suspended_at = None;
                self.position_ms.store(0, Ordering::Relaxed);
                // crossfade: the playing sink stays and fades out under the new one
                match self.sink.take() {
                    Some(old) if fade.is_some() && !old.is_paused() && !old.empty() => {
                        if let Some(prev) = self.fading_out.replace(old) {
                            prev.stop();
                        }
                    }
                    Some(old) => old.stop(),
                    None => {}
                }
                self.start(None, false);
                match fade {
                    Some(len) if self.fading_out.is_some() && self.sink.is_some() => {
                        self.fade = Some((Instant::now(), len));
                        self.apply_volume();
                    }
                    _ => self.end_fade(),
                }
            }
            AudioCmd::Crossfade(len) => self.crossfade = len,
            AudioCmd::Replay { generation } => {
                if generation == self.generation {
                    self.position_ms.store(0, Ordering::Relaxed);
                    self.start(None, false);
                }
            }
            AudioCmd::Play => {
                // device changed while paused: reopen on the new one at the same spot
                let paused_at = self.sink.as_ref().map(|s| s.get_pos());
                match (paused_at, self.suspended_at) {
                    (Some(at), _) if self.device_changed() => self.start(Some(at), false),
                    (Some(_), _) => {
                        if let Some(s) = &self.sink {
                            s.play();
                        }
                    }
                    (None, Some(at)) => self.start(Some(at), false),
                    (None, None) => {}
                }
            }
            AudioCmd::Pause => {
                self.end_fade();
                if let Some(s) = &self.sink {
                    s.pause();
                }
            }
            AudioCmd::Stop => {
                self.end_fade();
                if let Some(s) = self.sink.take() {
                    s.stop();
                }
                self.current = None;
                self.suspended_at = None;
                self.position_ms.store(0, Ordering::Relaxed);
            }
            AudioCmd::Seek(to) => {
                if let Some(s) = &self.sink {
                    match s.try_seek(to) {
                        Ok(()) => self.position_ms.store(to.as_millis() as u64, Ordering::Relaxed),
                        Err(e) => tracing::warn!(error = %e, "seek failed"),
                    }
                } else if self.suspended_at.is_some() {
                    self.suspended_at = Some(to);
                    self.position_ms.store(to.as_millis() as u64, Ordering::Relaxed);
                }
            }
            AudioCmd::Volume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                self.apply_volume();
            }
        }
    }

    /// Base volume, or the two crossfade curves (equal power: no dip in the middle).
    fn apply_volume(&mut self) {
        let base = perceptual(self.volume);
        let Some((started, len)) = self.fade else {
            if let Some(s) = &self.sink {
                s.set_volume(base);
            }
            return;
        };
        let t = (started.elapsed().as_secs_f32() / len.as_secs_f32().max(0.01)).min(1.0);
        let half_pi = std::f32::consts::FRAC_PI_2;
        if let Some(s) = &self.sink {
            s.set_volume(base * (t * half_pi).sin());
        }
        if let Some(s) = &self.fading_out {
            s.set_volume(base * (t * half_pi).cos());
        }
        if t >= 1.0 {
            self.end_fade();
        }
    }

    fn end_fade(&mut self) {
        if let Some(old) = self.fading_out.take() {
            old.stop();
        }
        if self.fade.take().is_some() {
            if let Some(s) = &self.sink {
                s.set_volume(perceptual(self.volume));
            }
        }
    }

    fn tick(&mut self) {
        if self.fade.is_some() {
            self.apply_volume();
        }
        let Some(sink) = &self.sink else { return };
        let pos = sink.get_pos();
        self.position_ms.store(pos.as_millis() as u64, Ordering::Relaxed);
        if sink.empty() {
            self.sink = None;
            let _ = self.events.send(AudioEvent::Ended { generation: self.generation });
            return;
        }
        // slowed / sped up changed in the effects
        let speed = self.fx.speed();
        if (sink.speed() - speed).abs() > 0.001 {
            sink.set_speed(speed);
        }
        // the crossfade into the next track: that long before the end (in real time)
        if !self.near_end_sent && !self.crossfade.is_zero() && self.duration_ms > 0 && !sink.is_paused() {
            let left_ms = self.duration_ms.saturating_sub(pos.as_millis() as u64) as f32 / speed.max(0.1);
            if left_ms <= self.crossfade.as_millis() as f32 && pos.as_millis() as u64 > 5_000 {
                self.near_end_sent = true;
                let _ = self.events.send(AudioEvent::NearEnd { generation: self.generation });
            }
        }
        let Some(sink) = &self.sink else { return };
        // while playing, look for a new default output device every ~2 s
        // (headphones unplugged, switched to a wireless dongle, …)
        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks % DEVICE_CHECK_TICKS == 0 && !sink.is_paused() && self.device_changed() {
            tracing::info!(from = ?self.device, to = ?default_device_name(), "output device changed, switching");
            self.stream = None;
            self.start(Some(pos), false);
        }
    }

    fn device_changed(&self) -> bool {
        self.stream.is_some() && default_device_name() != self.device
    }

    fn output(&mut self) -> Result<OutputStreamHandle, String> {
        if self.device_changed() {
            self.sink = None;
            self.stream = None;
        }
        if self.stream.is_none() {
            let pair = OutputStream::try_default().map_err(|e| e.to_string())?;
            self.device = default_device_name();
            tracing::debug!(device = ?self.device, "audio device opened");
            self.stream = Some(pair);
        }
        Ok(self.stream.as_ref().map(|(_, h)| h.clone()).expect("stream just set"))
    }

    fn start(&mut self, resume_at: Option<Duration>, paused: bool) {
        self.sink = None;
        let Some(data) = self.current.clone() else { return };
        let handle = match self.output() {
            Ok(h) => h,
            Err(e) => return self.fail(format!("устройство вывода: {e}")),
        };
        let decoder = match Decoder::new(Cursor::new(SharedBytes(data))) {
            Ok(d) => d,
            Err(e) => return self.fail(format!("не удалось декодировать: {e}")),
        };
        let sink = match Sink::try_new(&handle) {
            Ok(s) => s,
            Err(e) => return self.fail(format!("sink: {e}")),
        };
        sink.set_volume(perceptual(self.volume));
        sink.set_speed(self.fx.speed());
        let eq = Equalizer::new(decoder.convert_samples::<f32>(), self.eq.clone());
        sink.append(Effects::new(eq, self.fx.clone(), self.gain));
        if let Some(at) = resume_at {
            if let Err(e) = sink.try_seek(at) {
                tracing::warn!(error = %e, "resume seek failed");
            }
        }
        if paused {
            sink.pause();
        }
        self.sink = Some(sink);
        self.suspended_at = None;
    }

    /// Close the device when idle. A paused track is remembered and resumed later.
    fn release_device(&mut self) {
        if let Some(s) = &self.sink {
            if !s.is_paused() {
                return;
            }
            self.suspended_at = Some(s.get_pos());
        }
        self.sink = None;
        if self.stream.take().is_some() {
            tracing::debug!("audio device released (idle)");
        }
    }

    fn fail(&mut self, message: String) {
        tracing::error!(%message, "playback error");
        let _ = self.events.send(AudioEvent::Error { generation: self.generation, message });
    }
}

fn default_device_name() -> Option<String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    rodio::cpal::default_host().default_output_device()?.name().ok()
}

/// Slider position → amplitude. Quadratic is close enough to perceived loudness.
fn perceptual(v: f32) -> f32 {
    v * v
}
