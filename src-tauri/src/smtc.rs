//! Windows media panel (System Media Transport Controls): the "now playing"
//! of the volume flyout, the lock screen and Windows 11 quick settings shows
//! the cover, title, artist, progress and ⏮ ⏯ ⏭ that control SC Desk.
//!
//! The controls come from a `MediaPlayer` whose own command manager is off —
//! that needs no window, so they keep working in the tray. Everything runs on
//! one thread fed by a channel; buttons pressed in Windows go to the player.

use std::sync::{mpsc, Arc};

use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, PartialEq)]
pub struct NowPlaying {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub artwork: Option<String>,
    pub duration_ms: u64,
}

pub enum Update {
    Track(Option<NowPlaying>),
    State { playing: bool, position_ms: u64 },
}

#[derive(Clone)]
pub struct Smtc {
    tx: Option<mpsc::Sender<Update>>,
}

impl Smtc {
    pub fn start(app: AppHandle) -> Self {
        #[cfg(windows)]
        {
            let (tx, rx) = mpsc::channel();
            let spawned = std::thread::Builder::new().name("smtc".into()).spawn(move || {
                if let Err(e) = imp::run(app, rx) {
                    tracing::warn!(error = %e, "media panel unavailable");
                }
            });
            if spawned.is_ok() {
                return Self { tx: Some(tx) };
            }
        }
        let _ = app;
        Self { tx: None }
    }

    pub fn send(&self, u: Update) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(u);
        }
    }
}

/// Buttons pressed in the Windows media panel.
fn on_button(app: &AppHandle, button: &str) {
    let Some(player) = app.try_state::<Arc<crate::player::Player>>().map(|p| p.inner().clone()) else { return };
    match button {
        "play" => player.resume(),
        "pause" | "stop" => player.pause(),
        "toggle" => player.toggle(),
        "next" => player.next(),
        "prev" => player.prev(),
        _ => {}
    }
}

#[cfg(windows)]
mod imp {
    use std::{sync::mpsc, time::Duration};

    use tauri::AppHandle;
    use windows::{
        core::HSTRING,
        Foundation::{TimeSpan, TypedEventHandler, Uri},
        Media::{
            MediaPlaybackStatus, MediaPlaybackType, Playback::MediaPlayer, SystemMediaTransportControls,
            SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
            SystemMediaTransportControlsTimelineProperties,
        },
        Storage::Streams::RandomAccessStreamReference,
    };

    use super::{NowPlaying, Update};

    /// 100-ns ticks, the unit of WinRT time spans.
    fn span(ms: u64) -> TimeSpan {
        TimeSpan { Duration: ms as i64 * 10_000 }
    }

    pub fn run(app: AppHandle, rx: mpsc::Receiver<Update>) -> windows::core::Result<()> {
        let player = MediaPlayer::new()?;
        // its own play/pause logic would fight ours: only its media controls are used
        player.CommandManager()?.SetIsEnabled(false)?;
        let smtc: SystemMediaTransportControls = player.SystemMediaTransportControls()?;
        smtc.SetIsEnabled(true)?;
        smtc.SetIsPlayEnabled(true)?;
        smtc.SetIsPauseEnabled(true)?;
        smtc.SetIsStopEnabled(true)?;
        smtc.SetIsNextEnabled(true)?;
        smtc.SetIsPreviousEnabled(true)?;
        let handle = app.clone();
        smtc.ButtonPressed(&TypedEventHandler::<SystemMediaTransportControls, SystemMediaTransportControlsButtonPressedEventArgs>::new(
            move |_, args| {
                if let Some(args) = args.as_ref() {
                    let b = args.Button()?;
                    let name = match b {
                        SystemMediaTransportControlsButton::Play => "play",
                        SystemMediaTransportControlsButton::Pause => "pause",
                        SystemMediaTransportControlsButton::Stop => "stop",
                        SystemMediaTransportControlsButton::Next => "next",
                        SystemMediaTransportControlsButton::Previous => "prev",
                        _ => "",
                    };
                    super::on_button(&handle, name);
                }
                Ok(())
            },
        ))?;
        smtc.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;

        let mut current: Option<NowPlaying> = None;
        let mut last_state: Option<(bool, u64)> = None;
        // the queue may carry many state updates: only the latest matters
        while let Ok(first) = rx.recv() {
            let mut batch = vec![first];
            while let Ok(more) = rx.recv_timeout(Duration::from_millis(30)) {
                batch.push(more);
            }
            let track = batch.iter().rev().find_map(|u| if let Update::Track(t) = u { Some(t.clone()) } else { None });
            let state = batch.iter().rev().find_map(|u| if let Update::State { playing, position_ms } = u { Some((*playing, *position_ms)) } else { None });
            if let Some(t) = track {
                if t != current {
                    show_track(&smtc, t.as_ref())?;
                    current = t;
                    last_state = None;
                }
            }
            if let (Some((playing, pos)), Some(t)) = (state, current.as_ref()) {
                if last_state != Some((playing, pos)) {
                    smtc.SetPlaybackStatus(if playing { MediaPlaybackStatus::Playing } else { MediaPlaybackStatus::Paused })?;
                    let tl = SystemMediaTransportControlsTimelineProperties::new()?;
                    tl.SetStartTime(span(0))?;
                    tl.SetMinSeekTime(span(0))?;
                    tl.SetEndTime(span(t.duration_ms))?;
                    tl.SetMaxSeekTime(span(t.duration_ms))?;
                    tl.SetPosition(span(pos.min(t.duration_ms)))?;
                    smtc.UpdateTimelineProperties(&tl)?;
                    last_state = Some((playing, pos));
                }
            }
        }
        Ok(())
    }

    fn show_track(smtc: &SystemMediaTransportControls, t: Option<&NowPlaying>) -> windows::core::Result<()> {
        let updater = smtc.DisplayUpdater()?;
        let Some(t) = t else {
            updater.ClearAll()?;
            updater.Update()?;
            smtc.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;
            return Ok(());
        };
        updater.SetType(MediaPlaybackType::Music)?;
        updater.SetAppMediaId(&HSTRING::from("SC Desk"))?;
        let music = updater.MusicProperties()?;
        music.SetTitle(&HSTRING::from(t.title.as_str()))?;
        music.SetArtist(&HSTRING::from(t.artist.as_str()))?;
        if let Some(url) = t.artwork.as_deref().filter(|u| u.starts_with("https://")) {
            if let Ok(uri) = Uri::CreateUri(&HSTRING::from(url)) {
                if let Ok(thumb) = RandomAccessStreamReference::CreateFromUri(&uri) {
                    updater.SetThumbnail(&thumb)?;
                }
            }
        }
        updater.Update()
    }
}
