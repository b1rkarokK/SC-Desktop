//! Native playback on a dedicated thread (rodio + symphonia).
//!
//! CPU budget:
//! * Playing: the thread wakes at 4 Hz only to publish position / detect end.
//! * Paused or stopped: blocks on the channel (0 wakeups). After 30 s idle the
//!   output device is closed too — an open WASAPI/CoreAudio stream keeps the
//!   mixer callback running even when silent. Resume transparently reopens it
//!   and seeks back to the saved position.

use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc,
    },
    thread,
    time::Duration,
};

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::eq::{EqShared, Equalizer};
use crate::error::{AppError, AppResult};

const POLL_PLAYING: Duration = Duration::from_millis(250);
const RELEASE_DEVICE_AFTER: Duration = Duration::from_secs(30);
/// ticks are 250 ms apart → device check every 2 s while playing
const DEVICE_CHECK_TICKS: u32 = 8;

pub enum AudioCmd {
    Load { data: Arc<Vec<u8>>, generation: u64 },
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
    Error { generation: u64, message: String },
}

pub struct AudioHandle {
    tx: mpsc::Sender<AudioCmd>,
    position_ms: Arc<AtomicU64>,
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
}

/// Spawns the audio thread; fails if no output device can be opened at all.
pub fn spawn(eq: Arc<EqShared>) -> AppResult<(AudioHandle, UnboundedReceiver<AudioEvent>)> {
    let (tx, rx) = mpsc::channel();
    let (ev_tx, ev_rx) = unbounded_channel();
    let (init_tx, init_rx) = mpsc::channel::<Result<(), String>>();
    let position_ms = Arc::new(AtomicU64::new(0));
    let pos = position_ms.clone();

    thread::Builder::new()
        .name("audio".into())
        .spawn(move || {
            let mut engine = Engine::new(ev_tx, pos, eq);
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
        Ok(Ok(())) => Ok((AudioHandle { tx, position_ms }, ev_rx)),
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
    /// name of the device the open stream plays to
    device: Option<String>,
    ticks: u32,
}

impl Engine {
    fn new(events: UnboundedSender<AudioEvent>, position_ms: Arc<AtomicU64>, eq: Arc<EqShared>) -> Self {
        Self {
            eq,
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
            let timeout = if playing {
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
            AudioCmd::Load { data, generation } => {
                self.generation = generation;
                self.current = Some(data);
                self.suspended_at = None;
                self.position_ms.store(0, Ordering::Relaxed);
                self.start(None, false);
            }
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
                if let Some(s) = &self.sink {
                    s.pause();
                }
            }
            AudioCmd::Stop => {
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
                if let Some(s) = &self.sink {
                    s.set_volume(perceptual(self.volume));
                }
            }
        }
    }

    fn tick(&mut self) {
        let Some(sink) = &self.sink else { return };
        let pos = sink.get_pos();
        self.position_ms.store(pos.as_millis() as u64, Ordering::Relaxed);
        if sink.empty() {
            self.sink = None;
            let _ = self.events.send(AudioEvent::Ended { generation: self.generation });
            return;
        }
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
        sink.append(Equalizer::new(decoder.convert_samples::<f32>(), self.eq.clone()));
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
