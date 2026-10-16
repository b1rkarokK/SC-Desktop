//! The room engine: one task per room we are in, as the host or as a member. It speaks `rooms::Msg`
//! (sealed with `crypto`) over a `Transport`, keeps the room core (`rooms::Room` on the host),
//! and drives the local player through a `Port`.
//!
//! The stabilizer, in short: a track is loaded everywhere first and starts only when everybody is
//! ready, at a moment of the host's clock (`Go`), which every member converts with its own clock
//! offset (`ClockSync`, trusting the shortest round trip). While it plays the host repeats
//! where it is every few seconds (`Play`); a member that is audibly off seeks, one that is a
//! little off speeds up or slows down by 2 % for a while (`Port::trim`) instead of a jump.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::sync::mpsc;

use super::{
    crypto::{self, Identity, Key},
    hub::HubEvent,
};
use crate::rooms::{
    drift_fix, ClockSync, Member, Ms, Msg, PlayState, QueueItem, Room, RoomSnapshot, Round, Settings, Status, Barrier, DRIFT_SEEK_MS,
};

/// How the engine reaches the relay.
pub trait Transport: Send + Sync + 'static {
    fn send(&self, blob: String, keep: bool, id: Option<String>);
}

impl Transport for super::hub::HubClient {
    fn send(&self, blob: String, keep: bool, id: Option<String>) {
        super::hub::HubClient::send(self, blob, keep, id)
    }
}

/// How the engine drives the local player.
pub trait Port: Send + Sync + 'static {
    /// Load the track and hold it at 0:00, paused. The engine learns that it is loaded from `Cmd::Loaded`.
    fn prepare(&self, item: &QueueItem);
    /// Start the held track after this delay.
    fn start_after(&self, delay: Duration);
    /// Pause or play, at this position.
    fn set_playing(&self, playing: bool, pos_ms: i64);
    fn position_ms(&self) -> Option<i64>;
    /// Run 1 ± a little faster / slower for a while (1.0 = normal).
    fn trim(&self, factor: f32);
    fn stop(&self);
    /// «Мои файлы»: this device has that track (its own, or one received in a room)
    fn has_local(&self, _track_id: u64) -> bool {
        true
    }
    /// «Мои файлы»: the audio of a track this device has, handed to `done` (None: not here)
    fn local_audio(&self, _track_id: u64, done: Box<dyn FnOnce(Option<Vec<u8>>) + Send>) {
        done(None)
    }
    /// «Мои файлы»: a track received in the room, kept for this session
    fn local_received(&self, _item: &QueueItem, _audio: Vec<u8>) {}
}

/// «Мои файлы» go through the room in pieces this big: the relay takes a message of up to 400 K characters,
/// and a piece is base64 twice over (in the message, then sealed), about 230 K
const FILE_CHUNK: usize = 128 * 1024;
/// a file that did not come is asked for again after this long, three times, then the track counts as failed
const FILE_WAIT: Duration = Duration::from_secs(20);

/// A track of «Мои файлы» arriving piece by piece.
struct Incoming {
    total: u32,
    parts: Vec<Option<Vec<u8>>>,
}

pub enum Cmd {
    Enqueue(QueueItem),
    /// the host plays this track right now (a track the host clicked)
    PlayNow(QueueItem),
    /// the host skips to the next track
    Next,
    /// restart the track, or go back to the one before it
    Prev,
    Play(bool),
    Seek(i64),
    Kick(String),
    /// the host lets a removed member back in
    Unban(String),
    /// the host takes a track out of the queue ( guards against a queue that moved meanwhile)
    Remove { index: usize, track_id: u64 },
    Settings(Settings),
    /// the local player has the prepared track (or could not get it)
    Loaded { ok: bool },
    /// the local track played to its end
    TrackEnded,
    Leave,
}

#[derive(Clone, Serialize, Default, Debug)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    /// "connecting" | "loading" | "ready" | "playing" | "problem"
    pub status: String,
    pub problem: Option<String>,
    pub host: bool,
    pub you: bool,
}

#[derive(Clone, Serialize, Default, Debug)]
pub struct RoomView {
    pub active: bool,
    pub host: bool,
    pub host_id: String,
    pub channel: String,
    pub name: String,
    pub has_password: bool,
    pub members: Vec<MemberView>,
    pub queue: Vec<QueueItem>,
    pub current: Option<QueueItem>,
    pub play: Option<PlayState>,
    pub who_can_queue_all: bool,
    /// ids of the members the host removed (host only)
    pub banned: Vec<String>,
    pub connected: bool,
    /// set when the room is over for us: "kicked", "closed", "no answer"
    pub ended: Option<String>,
}

pub struct Params {
    pub channel: String,
    pub password: String,
    pub me: Member,
    pub host: bool,
    pub settings: Settings,
}

pub struct Handle {
    pub tx: mpsc::UnboundedSender<Cmd>,
    pub view: Arc<Mutex<RoomView>>,
}

const TICK: Duration = Duration::from_millis(500);
const PLAY_EVERY: Duration = Duration::from_secs(5);
const HELLO_EVERY: Duration = Duration::from_secs(2);
#[cfg(not(test))]
mod timing {
    use std::time::Duration;
    pub const HOST_SILENT: Duration = Duration::from_secs(12);
    pub const ELECT_STEP: Duration = Duration::from_secs(5);
    pub const BEAT_EVERY: Duration = Duration::from_secs(3);
    pub const ENTER_WAIT: Duration = Duration::from_millis(2500);
    pub const STATE_EVERY: Duration = Duration::from_secs(6);
}
#[cfg(test)]
mod timing {
    use std::time::Duration;
    pub const HOST_SILENT: Duration = Duration::from_millis(2500);
    pub const ELECT_STEP: Duration = Duration::from_millis(1500);
    pub const BEAT_EVERY: Duration = Duration::from_millis(500);
    pub const ENTER_WAIT: Duration = Duration::from_millis(1200);
    pub const STATE_EVERY: Duration = Duration::from_millis(800);
}
use timing::*;
/// how many played tracks "previous" can go back through
const HISTORY: usize = 30;
/// "previous" within this long of the start goes to the earlier track, later it restarts this one
const PREV_RESTART_MS: i64 = 3_000;
#[cfg(not(test))]
const PING_EVERY: Duration = Duration::from_secs(4);
#[cfg(test)]
const PING_EVERY: Duration = Duration::from_secs(1);
/// below this the picture is already "together"
const TRIM_FROM_MS: i64 = 120;
const TRIM: f32 = 0.02;

fn status_kind(s: &Status) -> (&'static str, Option<String>) {
    match s {
        Status::Connecting => ("connecting", None),
        Status::Loading => ("loading", None),
        Status::Ready => ("ready", None),
        Status::Playing => ("playing", None),
        Status::Problem(t) => ("problem", Some(t.clone())),
    }
}

struct Engine {
    p: Params,
    transport: Arc<dyn Transport>,
    port: Arc<dyn Port>,
    /// «Мои файлы»: the track waiting for its file before it is loaded, and the pieces coming in
    pending_file: Option<QueueItem>,
    /// when the file was last asked for, and how many times
    file_asked: Option<(Instant, u32)>,
    incoming: std::collections::HashMap<u64, Incoming>,
    /// audio read for somebody who asked: (to, track id, bytes), sent from the loop
    file_tx: mpsc::UnboundedSender<(String, u64, Vec<u8>)>,
    view: Arc<Mutex<RoomView>>,
    on_view: Arc<dyn Fn(&RoomView) + Send + Sync>,
    key: Key,
    epoch: u32,
    /// room keys we used before the last change of key (newest first): a newcomer still comes in with the password's
    old_keys: Vec<Key>,
    /// set while the host handles a Hello that arrived under an old key: the answer goes under the same one
    reply_key: Option<Key>,
    identity: Identity,
    t0: Instant,
    host_pub: Option<String>,
    // host side
    room: Option<Room>,
    round: Option<Round>,
    // member side
    snap: Option<RoomSnapshot>,
    clock: ClockSync,
    welcomed: bool,
    /// the track the room is on is still being loaded here: the host's play state waits for it
    awaiting_load: bool,
    ping_sent: u32,
    trim_until: Option<Instant>,
    seq_loading: Option<u64>,
    last_play: Option<PlayState>,
    msg_n: u64,
    /// who the room's host is now, and the term of that host's rule
    host_id: String,
    term: u32,
    /// the last time anything came from the host
    host_heard: Option<Instant>,
    /// when we connected to the relay (the wait before taking over an empty room counts from here)
    connected_at: Option<Instant>,
    /// the room as the relay has it: for whoever enters an empty room
    last_state: Option<RoomSnapshot>,
    decrypted: u32,
    undecrypted: u32,
    state_dirty: bool,
    last_state_sent: Instant,
    last_beat: Instant,
    /// played tracks, newest last, for "previous"
    history: Vec<QueueItem>,
    skip_history: bool,
    /// a restored room starts its track from here
    resume_ms: Option<i64>,
}

impl Engine {
    fn now(&self) -> Ms {
        self.t0.elapsed().as_millis() as Ms
    }

    fn host_now(&self) -> Ms {
        if self.p.host {
            self.now()
        } else {
            self.clock.to_host(self.now())
        }
    }

    /// Loads a track; one of «Мои файлы» this device lacks is asked for first and loaded once it is here.
    fn prepare(&mut self, item: &QueueItem) {
        if crate::local::is_local(item.track_id) && !self.port.has_local(item.track_id) {
            self.pending_file = Some(item.clone());
            self.file_asked = Some((Instant::now(), 1));
            let m = Msg::FileWant { from: self.p.me.id.clone(), owner: item.added_by.clone(), track_id: item.track_id };
            self.send(&m, false);
            return;
        }
        self.port.prepare(item);
    }

    /// The file messages, the same for the host and the members. True: handled.
    fn file_msg(&mut self, m: &Msg) -> bool {
        match m {
            Msg::FileWant { from, owner, track_id } => {
                // the one who put it on answers; the host too when it has it (the owner may have left)
                let me = &self.p.me.id;
                if (owner == me || self.p.host) && from != me && self.port.has_local(*track_id) {
                    let (tx, to, id) = (self.file_tx.clone(), from.clone(), *track_id);
                    self.port.local_audio(id, Box::new(move |a| {
                        if let Some(a) = a {
                            let _ = tx.send((to, id, a));
                        }
                    }));
                }
                true
            }
            Msg::FileChunk { to, track_id, index, total, data } => {
                if to != &self.p.me.id {
                    return true;
                }
                use base64::Engine as _;
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else { return true };
                let inc = self.incoming.entry(*track_id).or_insert_with(|| Incoming { total: *total, parts: vec![None; *total as usize] });
                if let Some(slot) = inc.parts.get_mut(*index as usize) {
                    *slot = Some(bytes);
                }
                if inc.parts.iter().all(Option::is_some) && inc.total as usize == inc.parts.len() {
                    let inc = self.incoming.remove(track_id).expect("just checked");
                    let audio: Vec<u8> = inc.parts.into_iter().flatten().flatten().collect();
                    if let Some(item) = self.pending_file.take().filter(|i| i.track_id == *track_id) {
                        self.file_asked = None;
                        tracing::info!(track = track_id, bytes = audio.len(), "room: a track of «Мои файлы» received");
                        self.port.local_received(&item, audio);
                        self.port.prepare(&item);
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// A file that does not come: asked again, and after three tries the track counts as failed (the room goes on).
    fn file_tick(&mut self) {
        let Some((at, tries)) = self.file_asked else { return };
        if at.elapsed() < FILE_WAIT {
            return;
        }
        let Some(item) = self.pending_file.clone() else {
            self.file_asked = None;
            return;
        };
        if tries < 3 {
            tracing::info!(track = item.track_id, tries, "room: a track of «Мои файлы» did not come, asking again");
            self.incoming.remove(&item.track_id);
            self.file_asked = Some((Instant::now(), tries + 1));
            let m = Msg::FileWant { from: self.p.me.id.clone(), owner: item.added_by.clone(), track_id: item.track_id };
            self.send(&m, false);
        } else {
            tracing::warn!(track = item.track_id, "room: a track of «Мои файлы» never came");
            self.pending_file = None;
            self.file_asked = None;
            let _ = if self.p.host { self.host_cmd(Cmd::Loaded { ok: false }) } else { self.member_cmd(Cmd::Loaded { ok: false }) };
        }
    }

    /// Sends a track of «Мои файлы» to the one who asked, piece by piece.
    fn send_file(&mut self, to: String, track_id: u64, audio: Vec<u8>) {
        use base64::Engine as _;
        let total = audio.len().div_ceil(FILE_CHUNK) as u32;
        for (index, part) in audio.chunks(FILE_CHUNK).enumerate() {
            let data = base64::engine::general_purpose::STANDARD.encode(part);
            self.send(&Msg::FileChunk { to: to.clone(), track_id, index: index as u32, total, data }, false);
        }
        tracing::info!(track = track_id, bytes = audio.len(), pieces = total, "room: a track of «Мои файлы» sent");
    }

    fn send(&mut self, m: &Msg, keep: bool) {
        self.msg_n += 1;
        let blob = crypto::seal(&self.key, &self.p.channel, &serde_json::to_vec(m).expect("messages serialise"));
        self.transport.send(blob, keep, Some(format!("{}-{}", self.p.me.id, self.msg_n)));
    }

    fn send_under(&mut self, m: &Msg, key: &Key) {
        self.msg_n += 1;
        let blob = crypto::seal(key, &self.p.channel, &serde_json::to_vec(m).expect("messages serialise"));
        self.transport.send(blob, false, Some(format!("{}-{}", self.p.me.id, self.msg_n)));
    }

    fn publish(&self) {
        let mut v = RoomView { active: true, host: self.p.host, host_id: self.host_id.clone(), channel: self.p.channel.clone(), connected: true, ..Default::default() };
        let (settings, members, queue, current, play, host_id) = if let Some(r) = &self.room {
            (r.settings.clone(), r.members.clone(), r.queue.iter().cloned().collect(), r.current.clone().map(|c| c.1), r.play, r.host.clone())
        } else if let Some(s) = &self.snap {
            (s.settings.clone(), s.members.clone(), s.queue.clone(), s.current.clone().map(|c| c.1), s.play, self.host_id.clone())
        } else {
            (self.p.settings.clone(), vec![self.p.me.clone()], vec![], None, None, String::new())
        };
        v.name = settings.name.clone();
        v.has_password = settings.has_password;
        v.who_can_queue_all = settings.who_can_queue == crate::rooms::Who::Everyone;
        v.members = members
            .iter()
            .map(|m| {
                let (kind, problem) = status_kind(&m.status);
                MemberView { id: m.id.clone(), name: m.name.clone(), avatar_url: m.avatar_url.clone(), status: kind.into(), problem, host: m.id == host_id, you: m.id == self.p.me.id }
            })
            .collect();
        v.queue = queue;
        v.current = current;
        v.play = play;
        // only the host manages the list of the removed
        v.banned = self.room.as_ref().map(|r| r.banned().to_vec()).unwrap_or_default();
        {
            let mut g = self.view.lock().unwrap_or_else(|e| e.into_inner());
            v.ended = g.ended.clone();
            *g = v.clone();
        }
        (self.on_view)(&v);
    }

    fn end(&mut self, why: &str) {
        self.port.stop();
        let mut g = self.view.lock().unwrap_or_else(|e| e.into_inner());
        g.ended = Some(why.into());
        g.connected = false;
        let v = g.clone();
        drop(g);
        (self.on_view)(&v);
    }

    // ------------------------------------------------------------------ host

    fn roster(&mut self) {
        let Some(r) = &self.room else { return };
        let m = Msg::Roster { members: r.members.clone(), settings: r.settings.clone() };
        let q = Msg::Queue { items: r.queue.iter().cloned().collect() };
        self.send(&m, false);
        self.send(&q, false);
        self.state_dirty = true;
        self.publish();
    }

    fn start_next(&mut self) {
        let now = self.now();
        let Some(room) = self.room.as_mut() else { return };
        if !self.skip_history {
            if let Some((_, played)) = room.current.clone() {
                self.history.push(played);
                if self.history.len() > HISTORY {
                    self.history.remove(0);
                }
            }
        }
        self.skip_history = false;
        let Some((round, item)) = room.next_round(now) else {
            room.current = None;
            room.play = None;
            self.round = None;
            self.roster();
            return;
        };
        let seq = round.seq;
        self.round = Some(round);
        self.prepare(&item);
        self.send(&Msg::Load { seq, item }, false);
        self.roster();
    }

    fn go(&mut self) {
        let host_now = self.now();
        let at = Round::start_at(host_now);
        let Some(room) = self.room.as_mut() else { return };
        let seq = room.current.as_ref().map(|c| c.0).unwrap_or(0);
        let mut st = room.start(at);
        self.round = None;
        match self.resume_ms.take() {
            Some(ms) if ms > 2_000 => {
                // a restored room: from where it stopped (the others catch up through the play state)
                st.pos_ms = ms;
                room.play = Some(st);
                self.port.set_playing(true, ms);
            }
            _ => self.port.start_after(Duration::from_millis((at - host_now).max(0) as u64)),
        }
        self.send(&Msg::Go { seq, at }, false);
        self.send(&Msg::Play { state: st }, false);
        self.publish();
    }

    fn host_state_now(&self, playing: bool, pos: Option<i64>) -> PlayState {
        PlayState { at: self.now(), pos_ms: pos.unwrap_or_else(|| self.port.position_ms().unwrap_or(0)), playing }
    }

    fn host_msg(&mut self, from_member: &str, m: Msg) {
        let now = self.now();
        match m {
            Msg::Hello { member } => {
                let reply_key = self.reply_key;
                let Some(room) = self.room.as_mut() else { return };
                let m_id = member.id.clone();
                let pubkey = member.pubkey.clone();
                match room.join(member, true, now) {
                    Ok(()) => {
                        let snap = room.snapshot();
                        let host_pub = self.identity.public_b64();
                        // came in with an old key (after a kick the room moved on): hand it the current one
                        let rekey = reply_key.and_then(|_| pubkey.and_then(|pk| self.identity.wrap_for(&pk, &self.p.channel, &self.key))).map(|w| (self.epoch, w));
                        let w = Msg::Welcome { to: m_id, room: snap, host_now: now, host_pub, host: self.p.me.id.clone(), rekey };
                        match reply_key {
                            Some(k) => self.send_under(&w, &k),
                            None => self.send(&w, false),
                        }
                        self.roster();
                    }
                    Err(reason) => {
                        let d = Msg::Denied { to: m_id, reason };
                        match reply_key {
                            Some(k) => self.send_under(&d, &k),
                            None => self.send(&d, false),
                        }
                    }
                }
            }
            Msg::Status { id, status } => {
                if let Some(r) = self.room.as_mut() {
                    r.set_status(&id, status, now);
                }
                self.roster();
            }
            Msg::Loaded { seq, id, ok } => {
                if let Some(round) = self.round.as_mut() {
                    round.answer(seq, &id, ok);
                }
                if let Some(r) = self.room.as_mut() {
                    r.set_status(&id, if ok { Status::Ready } else { Status::Problem("не загрузился трек".into()) }, now);
                }
                self.roster();
            }
            Msg::Enqueue { item } => {
                let ok = self.room.as_mut().map(|r| r.enqueue(from_member, item)).unwrap_or(false);
                if ok {
                    self.roster();
                    if self.room.as_ref().is_some_and(|r| r.current.is_none()) {
                        self.start_next();
                    }
                }
            }
            Msg::Ping { id, t0 } => {
                // the ping is a member's heartbeat: without it a listener whose status does not change
                // (steadily playing) would look silent and be dropped
                let mut back = false;
                if let Some(r) = self.room.as_mut() {
                    r.heard_from(&id, now);
                    if r.members.iter().any(|m| m.id == id && matches!(&m.status, Status::Problem(p) if p == "проблема с соединением")) {
                        r.set_status(&id, Status::Ready, now);
                        back = true;
                    }
                }
                self.send(&Msg::Pong { id, t0, host_t: now }, false);
                if back {
                    self.roster();
                }
            }
            Msg::HostBeat { id, term, pubkey } => {
                // two hosts (both took an empty room at once, or the old one came back): the higher term wins,
                // at equal terms the smaller id
                if id != self.p.me.id && (term, std::cmp::Reverse(id.clone())) > (self.term, std::cmp::Reverse(self.p.me.id.clone())) {
                    self.demote(&id, term, pubkey);
                }
            }
            Msg::Leave { id } => {
                if let Some(r) = self.room.as_mut() {
                    r.leave(&id);
                }
                if let Some(round) = self.round.as_mut() {
                    round.drop_member(&id);
                }
                self.roster();
            }
            _ => {}
        }
    }

    fn host_cmd(&mut self, c: Cmd) -> bool {
        let now = self.now();
        match c {
            Cmd::Enqueue(item) => {
                let me = self.p.me.id.clone();
                let ok = self.room.as_mut().map(|r| r.enqueue(&me, item)).unwrap_or(false);
                if ok {
                    self.roster();
                    if self.room.as_ref().is_some_and(|r| r.current.is_none()) {
                        self.start_next();
                    }
                }
            }
            Cmd::PlayNow(item) => {
                if let Some(r) = self.room.as_mut() {
                    r.queue.push_front(item);
                }
                self.start_next();
            }
            Cmd::Remove { index, track_id } => {
                let done = self.room.as_mut().is_some_and(|r| {
                    if r.queue.get(index).is_some_and(|q| q.track_id == track_id) {
                        r.queue.remove(index);
                        true
                    } else {
                        false
                    }
                });
                if done {
                    self.roster();
                }
            }
            Cmd::Next | Cmd::TrackEnded => self.start_next(),
            Cmd::Prev => {
                let pos = self.port.position_ms().unwrap_or(0);
                let earlier = self.history.pop();
                match (pos <= PREV_RESTART_MS, earlier) {
                    (true, Some(prev)) => {
                        // the track before this one, then this one again
                        if let Some(r) = self.room.as_mut() {
                            if let Some((_, cur)) = r.current.clone() {
                                r.queue.push_front(cur);
                            }
                            r.queue.push_front(prev);
                        }
                        self.skip_history = true;
                        self.start_next();
                    }
                    (_, earlier) => {
                        if let Some(e) = earlier {
                            self.history.push(e);
                        }
                        let playing = self.room.as_ref().and_then(|r| r.play).is_none_or(|p| p.playing);
                        let st = PlayState { at: now, pos_ms: 0, playing };
                        self.port.set_playing(playing, 0);
                        if let Some(r) = self.room.as_mut() {
                            r.play = Some(st);
                        }
                        self.send(&Msg::Play { state: st }, false);
                        self.publish();
                    }
                }
            }
            Cmd::Play(true) if self.room.as_ref().is_some_and(|r| r.current.is_none() && !r.queue.is_empty()) => self.start_next(),
            Cmd::Play(playing) => {
                let st = self.host_state_now(playing, None);
                self.port.set_playing(playing, st.pos_ms);
                if let Some(r) = self.room.as_mut() {
                    r.play = Some(st);
                }
                self.send(&Msg::Play { state: st }, false);
                self.publish();
            }
            Cmd::Seek(ms) => {
                let playing = self.room.as_ref().and_then(|r| r.play).is_none_or(|p| p.playing);
                let st = PlayState { at: now, pos_ms: ms, playing };
                self.port.set_playing(playing, ms);
                if let Some(r) = self.room.as_mut() {
                    r.play = Some(st);
                }
                self.send(&Msg::Play { state: st }, false);
                self.publish();
            }
            Cmd::Kick(id) => self.kick(&id),
            Cmd::Unban(id) => {
                let me = self.p.me.id.clone();
                if self.room.as_mut().is_some_and(|r| r.unban(&me, &id)) {
                    self.roster();
                }
            }
            Cmd::Settings(s) => {
                if let Some(r) = self.room.as_mut() {
                    r.settings = s;
                }
                self.roster();
            }
            Cmd::Loaded { ok } => {
                // the host's own player has the track
                let me = self.p.me.id.clone();
                let seq = self.round.as_ref().map(|r| r.seq).unwrap_or(0);
                if let Some(round) = self.round.as_mut() {
                    round.answer(seq, &me, ok);
                }
                if let Some(r) = self.room.as_mut() {
                    r.set_status(&me, if ok { Status::Ready } else { Status::Problem("не загрузился трек".into()) }, now);
                }
                self.roster();
            }
            Cmd::Leave => {
                // the room stays: its state is kept for the next one, the next in the list takes over
                if let Some(pos) = self.port.position_ms() {
                    let st = self.host_state_now(self.room.as_ref().and_then(|r| r.play).is_some_and(|p| p.playing), Some(pos));
                    if let Some(r) = self.room.as_mut() {
                        if r.current.is_some() {
                            r.play = Some(st);
                        }
                    }
                }
                self.send_state();
                let me = self.p.me.id.clone();
                self.send(&Msg::HostLeft { id: me.clone() }, false);
                self.send(&Msg::Leave { id: me }, false);
                self.end("closed");
                return false;
            }
        }
        true
    }

    /// Out of the room, with a new key the kicked one does not get.
    fn kick(&mut self, id: &str) {
        let me = self.p.me.id.clone();
        let Some(room) = self.room.as_mut() else { return };
        let pubkeys: Vec<(String, Option<String>)> = room.members.iter().filter(|m| m.id != id && m.id != me).map(|m| (m.id.clone(), m.pubkey.clone())).collect();
        if !room.kick(&me, id) {
            return;
        }
        if let Some(round) = self.round.as_mut() {
            round.drop_member(id);
        }
        self.send(&Msg::Kick { id: id.to_owned() }, false);
        let new_key = crypto::random_key();
        let mut wraps = BTreeMap::new();
        for (mid, pk) in pubkeys {
            if let Some(w) = pk.and_then(|pk| self.identity.wrap_for(&pk, &self.p.channel, &new_key)) {
                wraps.insert(mid, w);
            }
        }
        self.epoch += 1;
        let epoch = self.epoch;
        self.send(&Msg::Rekey { epoch, wraps }, false);
        self.old_keys.insert(0, self.key);
        self.key = new_key; // everything after this is under the new key
        self.roster();
    }

    fn host_tick(&mut self) {
        let now = self.now();
        let gone = self.room.as_mut().map(|r| r.check_silence(now)).unwrap_or_default();
        for g in &gone {
            if let Some(round) = self.round.as_mut() {
                round.drop_member(g);
            }
        }
        let mut changed = !gone.is_empty();
        if self.last_beat.elapsed() >= BEAT_EVERY {
            self.beat();
        }
        if self.state_dirty && self.last_state_sent.elapsed() >= STATE_EVERY {
            self.send_state();
        }
        if let Some(round) = &self.round {
            match round.state(now) {
                Barrier::Go | Barrier::TimedOut => {
                    self.go();
                    changed = false;
                }
                Barrier::Waiting { .. } => {}
            }
        }
        if changed {
            self.roster();
        }
    }

    fn host_repeat_state(&mut self) {
        let playing = self.room.as_ref().and_then(|r| r.play).is_some_and(|p| p.playing);
        if !playing {
            return;
        }
        if let Some(pos) = self.port.position_ms() {
            let st = self.host_state_now(true, Some(pos));
            if let Some(r) = self.room.as_mut() {
                r.play = Some(st);
            }
            self.send(&Msg::Play { state: st }, false);
        }
    }

    fn beat(&mut self) {
        self.last_beat = Instant::now();
        let m = Msg::HostBeat { id: self.p.me.id.clone(), term: self.term, pubkey: self.identity.public_b64() };
        self.send(&m, false);
    }

    /// The room as it is, kept on the relay: whoever enters the empty room next continues from it.
    fn send_state(&mut self) {
        self.state_dirty = false;
        self.last_state_sent = Instant::now();
        let Some(r) = &self.room else { return };
        let mut snapshot = r.snapshot();
        // who is in the room is not part of what is kept: names stay, state does not
        for m in &mut snapshot.members {
            m.status = Status::Connecting;
        }
        self.send(&Msg::State { snapshot }, true);
    }

    /// Become the host: of an empty room (`restored` from the relay) or in place of one that went quiet.
    fn promote(&mut self, restored: Option<RoomSnapshot>) {
        let now = self.now();
        let old_host = self.host_id.clone();
        let me = self.p.me.clone();
        let mut host = me.clone();
        host.status = Status::Ready;
        host.pubkey = Some(self.identity.public_b64());
        host.last_seen = now;
        let from_election = self.snap.is_some();
        let source = self.snap.take().or(restored);
        let mut settings = source.as_ref().map(|s| s.settings.clone()).unwrap_or_else(|| self.p.settings.clone());
        settings.has_password = !self.p.password.is_empty();
        let mut room = Room::new(host, settings);
        if let Some(src) = &source {
            for m in src.members.iter().filter(|m| m.id != me.id && m.id != old_host) {
                let mut m = m.clone();
                m.status = Status::Connecting;
                m.last_seen = now;
                room.members.push(m);
            }
            room.queue = src.queue.iter().cloned().collect();
            room.set_banned(src.banned.clone());
            if let Some((_, item)) = &src.current {
                let carry_on = from_election && !self.awaiting_load && self.last_play.is_some_and(|p| p.playing);
                if carry_on {
                    // the music is playing here already: just go on, now as the host
                    room.current = src.current.clone();
                    room.play = Some(PlayState { at: now, pos_ms: self.port.position_ms().unwrap_or(0), playing: true });
                } else {
                    // continue from where it stopped, when somebody presses play
                    room.queue.push_front(item.clone());
                    self.resume_ms = src.play.map(|p| p.pos_ms);
                    self.port.stop();
                }
            }
        }
        self.term = self.term.max(1) + u32::from(from_election);
        self.round = None;
        self.room = Some(room);
        self.p.host = true;
        self.host_id = me.id.clone();
        self.welcomed = true;
        self.host_heard = None;
        self.awaiting_load = false;
        self.beat();
        self.roster();
    }

    /// Another host has the room (it won the tie, or it is the newer one): back to being a member of it.
    fn demote(&mut self, host: &str, term: u32, pubkey: String) {
        self.p.host = false;
        self.room = None;
        self.round = None;
        self.snap = None;
        self.welcomed = false;
        self.term = term;
        self.host_id = host.to_owned();
        self.host_pub = Some(pubkey);
        self.clock = ClockSync::default();
        self.ping_sent = 0;
        self.host_heard = Some(Instant::now());
        self.say_hello();
        self.publish();
    }

    // ---------------------------------------------------------------- member

    fn say_hello(&mut self) {
        let mut me = self.p.me.clone();
        me.pubkey = Some(self.identity.public_b64());
        self.send(&Msg::Hello { member: me }, false);
    }

    fn apply_play(&mut self, st: PlayState) {
        self.last_play = Some(st);
        if self.awaiting_load {
            return;
        }
        let host_now = self.host_now();
        let want = st.position_at(host_now);
        let Some(mine) = self.port.position_ms() else {
            self.port.set_playing(st.playing, want);
            return;
        };
        if !st.playing {
            self.port.set_playing(false, st.pos_ms);
            return;
        }
        if let Some(target) = drift_fix(&st, host_now, mine) {
            tracing::debug!(off = mine - want, "room: audibly off, seeking");
            self.port.set_playing(true, target);
            self.port.trim(1.0);
            self.trim_until = None;
            return;
        }
        let off = mine - want;
        if off.abs() > TRIM_FROM_MS && off.abs() <= DRIFT_SEEK_MS {
            // behind: a little faster; ahead: a little slower; for a few seconds
            self.port.trim(if off < 0 { 1.0 + TRIM } else { 1.0 - TRIM });
            self.trim_until = Some(Instant::now() + Duration::from_millis((off.unsigned_abs() * 50).clamp(1500, 8000)));
        } else if self.trim_until.is_none() {
            self.port.trim(1.0);
        }
    }

    fn member_msg(&mut self, m: Msg) -> bool {
        let me = self.p.me.id.clone();
        match m {
            Msg::Welcome { to, room, host_now, host_pub, host, rekey } if to == me => {
                if let Some((epoch, wrap)) = rekey {
                    if let Some(k) = self.identity.unwrap_from(&host_pub, &self.p.channel, &wrap) {
                        self.old_keys.insert(0, self.key);
                        self.key = k;
                        self.epoch = epoch;
                    }
                }
                self.welcomed = true;
                self.host_heard = Some(Instant::now());
                if !host.is_empty() {
                    self.host_id = host;
                }
                self.clock.set_rough(host_now - self.now());
                self.host_pub = Some(host_pub);
                // joined in the middle of a track: load it like any other, then catch up with the host
                let current = room.current.clone();
                self.last_play = room.play;
                self.snap = Some(room);
                self.ping_sent = 0;
                if let Some((seq, item)) = current {
                    self.seq_loading = Some(seq);
                    self.awaiting_load = true;
                    self.prepare(&item);
                    self.send(&Msg::Status { id: me, status: Status::Loading }, false);
                } else {
                    // nothing is on: we are simply here, ready (not "connecting" for ever)
                    self.send(&Msg::Status { id: me, status: Status::Ready }, false);
                }
                self.publish();
            }
            Msg::Denied { to, reason } if to == me => {
                self.end(if reason == crate::rooms::BANNED { "kicked" } else { &reason });
                return false;
            }
            Msg::Roster { members, settings } => {
                self.host_heard = Some(Instant::now());
                let listed = members.iter().any(|m| m.id == me);
                if let Some(s) = self.snap.as_mut() {
                    s.members = members;
                    s.settings = settings;
                }
                self.publish();
                if !listed && self.welcomed {
                    // a new host that does not know us yet
                    self.say_hello();
                }
            }
            Msg::HostBeat { id, term, pubkey } if id != me => {
                let better = (term, std::cmp::Reverse(id.clone())) > (self.term, std::cmp::Reverse(self.host_id.clone()));
                if id == self.host_id || better || self.host_id.is_empty() {
                    if id != self.host_id {
                        // a new host: its clock is its own, sync again; it may not know us yet
                        self.host_id = id;
                        self.host_pub = Some(pubkey);
                        self.clock = ClockSync::default();
                        self.ping_sent = 0;
                        self.welcomed = false;
                        self.say_hello();
                    }
                    self.term = term;
                    self.host_heard = Some(Instant::now());
                }
            }
            Msg::HostLeft { id } if id == self.host_id => {
                // the host said goodbye: no waiting for the silence to run out
                self.host_heard = Instant::now().checked_sub(HOST_SILENT);
            }
            Msg::State { snapshot } => {
                self.last_state = Some(snapshot);
            }
            Msg::Queue { items } => {
                self.host_heard = Some(Instant::now());
                if let Some(s) = self.snap.as_mut() {
                    s.queue = items;
                }
                self.publish();
            }
            Msg::Load { seq, item } => {
                self.host_heard = Some(Instant::now());
                self.seq_loading = Some(seq);
                self.awaiting_load = true;
                self.last_play = None;
                if let Some(s) = self.snap.as_mut() {
                    s.current = Some((seq, item.clone()));
                    s.play = None;
                }
                self.prepare(&item);
                self.send(&Msg::Status { id: me, status: Status::Loading }, false);
                self.publish();
            }
            Msg::Go { seq: _, at } => {
                self.host_heard = Some(Instant::now());
                let delay = (self.clock.to_local(at) - self.now()).max(0) as u64;
                self.port.start_after(Duration::from_millis(delay));
                self.send(&Msg::Status { id: me, status: Status::Playing }, false);
            }
            Msg::Play { state } => {
                self.host_heard = Some(Instant::now());
                if let Some(s) = self.snap.as_mut() {
                    s.play = Some(state);
                }
                self.apply_play(state);
                self.publish();
            }
            Msg::Pong { id, t0, host_t } if id == me => {
                self.host_heard = Some(Instant::now());
                self.clock.sample(t0, host_t, self.now());
            }
            Msg::Kick { id } if id == me => {
                self.end("kicked");
                return false;
            }
            Msg::Rekey { epoch, wraps } => {
                if let (Some(w), Some(hp)) = (wraps.get(&me), self.host_pub.clone()) {
                    if let Some(k) = self.identity.unwrap_from(&hp, &self.p.channel, w) {
                        self.old_keys.insert(0, self.key);
                        self.key = k;
                        self.epoch = epoch;
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn member_cmd(&mut self, c: Cmd) -> bool {
        let me = self.p.me.id.clone();
        match c {
            Cmd::Enqueue(item) => self.send(&Msg::Enqueue { item }, false),
            Cmd::Loaded { ok } => {
                let seq = self.seq_loading.unwrap_or(0);
                self.send(&Msg::Loaded { seq, id: me, ok }, false);
                self.awaiting_load = false;
                // a track that was already playing when we came: catch up with the host now
                if let Some(st) = self.last_play {
                    self.apply_play(st);
                    if st.playing {
                        self.send(&Msg::Status { id: self.p.me.id.clone(), status: Status::Playing }, false);
                    }
                }
            }
            Cmd::Leave => {
                self.send(&Msg::Leave { id: me }, false);
                self.end("closed");
                return false;
            }
            _ => {}
        }
        true
    }

    fn member_tick(&mut self, since_hello: Duration) -> bool {
        if let Some(t) = self.trim_until {
            if Instant::now() >= t {
                self.trim_until = None;
                self.port.trim(1.0);
            }
        }
        let _ = since_hello;
        // nobody answered: the room is empty (or the key is wrong)
        if !self.welcomed && self.host_heard.is_none() && self.connected_at.is_some_and(|t| t.elapsed() >= ENTER_WAIT) {
            if self.decrypted == 0 && self.undecrypted > 0 {
                // the relay has messages we cannot read: not our password
                self.end("no answer");
                return false;
            }
            let restored = self.last_state.clone();
            if restored.as_ref().is_some_and(|s| s.banned.contains(&self.p.me.id)) {
                self.end("kicked");
                return false;
            }
            self.promote(restored);
            return true;
        }
        // the host went quiet: the next in the list takes over, one after another
        if self.welcomed {
            if let (Some(heard), Some(snap)) = (self.host_heard, self.snap.as_ref()) {
                let me = self.p.me.id.clone();
                let order: Vec<&Member> = snap.members.iter().filter(|m| m.id != self.host_id && !matches!(m.status, Status::Problem(_))).collect();
                if let Some(rank) = order.iter().position(|m| m.id == me) {
                    if heard.elapsed() >= HOST_SILENT + ELECT_STEP * rank as u32 {
                        tracing::info!(rank, "room: the host is gone, taking over");
                        self.promote(None);
                        return true;
                    }
                }
            }
        }
        true
    }

    fn ping(&mut self) {
        let t0 = self.now();
        let me = self.p.me.id.clone();
        self.ping_sent += 1;
        self.send(&Msg::Ping { id: me, t0 }, false);
    }
}

/// Runs the room until it ends. `transport` + `events` are the relay, `port` the local player.
pub fn start(
    p: Params,
    transport: Arc<dyn Transport>,
    mut events: mpsc::UnboundedReceiver<HubEvent>,
    port: Arc<dyn Port>,
    on_view: Arc<dyn Fn(&RoomView) + Send + Sync>,
) -> Handle {
    let (tx, mut cmds) = mpsc::unbounded_channel::<Cmd>();
    let (file_tx, mut file_rx) = mpsc::unbounded_channel::<(String, u64, Vec<u8>)>();
    let view = Arc::new(Mutex::new(RoomView { active: true, host: p.host, channel: p.channel.clone(), ..Default::default() }));
    let handle = Handle { tx, view: view.clone() };
    let key = crypto::room_key(&p.channel, &p.password);
    let identity = Identity::new();
    let is_host = p.host;
    let host_id0 = if is_host { p.me.id.clone() } else { String::new() };
    let room = is_host.then(|| {
        let mut host = p.me.clone();
        host.status = Status::Ready;
        host.pubkey = Some(identity.public_b64());
        let mut settings = p.settings.clone();
        settings.has_password = !p.password.is_empty();
        Room::new(host, settings)
    });
    let mut eng = Engine {
        p,
        transport,
        port,
        pending_file: None,
        file_asked: None,
        incoming: std::collections::HashMap::new(),
        file_tx,
        view,
        on_view,
        key,
        epoch: 0,
        old_keys: Vec::new(),
        reply_key: None,
        identity,
        t0: Instant::now(),
        host_pub: None,
        room,
        round: None,
        snap: None,
        clock: ClockSync::default(),
        welcomed: false,
        awaiting_load: false,
        ping_sent: 0,
        trim_until: None,
        seq_loading: None,
        last_play: None,
        msg_n: 0,
        host_id: host_id0,
        term: u32::from(is_host),
        host_heard: None,
        connected_at: None,
        last_state: None,
        decrypted: 0,
        undecrypted: 0,
        state_dirty: false,
        last_state_sent: Instant::now(),
        last_beat: Instant::now(),
        history: Vec::new(),
        skip_history: false,
        resume_ms: None,
    };
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        let started = Instant::now();
        let mut last_hello = Instant::now() - HELLO_EVERY;
        let mut last_play = Instant::now();
        let mut last_ping = Instant::now() - PING_EVERY;
        let mut ping_burst = Instant::now();
        eng.publish();
        loop {
            tokio::select! {
                ev = events.recv() => {
                    let Some(ev) = ev else { break };
                    match ev {
                        HubEvent::Connected { .. } => {
                            eng.connected_at = Some(Instant::now());
                            if !eng.p.host { eng.say_hello(); last_hello = Instant::now(); }
                            else { eng.roster(); eng.beat(); }
                        }
                        HubEvent::Disconnected => {
                            let mut g = eng.view.lock().unwrap_or_else(|e| e.into_inner());
                            g.connected = false;
                            let v = g.clone();
                            drop(g);
                            (eng.on_view)(&v);
                        }
                        HubEvent::Msg { blob, id, .. } => {
                            // our own message coming back is an acknowledgement, not news
                            if id.as_deref().is_some_and(|i| i.starts_with(&format!("{}-", eng.p.me.id))) { continue; }
                            // the current key; the host also reads a Hello under an older one (a newcomer after a kick)
                            let opened = crypto::open(&eng.key, &eng.p.channel, &blob).map(|p| (p, None)).or_else(|| {
                                if !eng.p.host { return None; }
                                eng.old_keys.iter().find_map(|k| crypto::open(k, &eng.p.channel, &blob).map(|p| (p, Some(*k))))
                            });
                            let Some((plain, old)) = opened else { eng.undecrypted += 1; continue };
                            let Ok(m) = serde_json::from_slice::<Msg>(&plain) else { continue };
                            // under an old key only a Hello counts: whoever was removed still knows that key
                            if old.is_some() && !matches!(m, Msg::Hello { .. }) { continue; }
                            eng.reply_key = old;
                            eng.decrypted += 1;
                            if eng.file_msg(&m) { eng.reply_key = None; continue; }
                            // who is it from? the host needs it for the queue permission
                            let from = match &m { Msg::Enqueue { item } => item.added_by.clone(), _ => String::new() };
                            if eng.p.host { eng.host_msg(&from, m); eng.reply_key = None; } else if !eng.member_msg(m) { break; }
                        }
                    }
                }
                f = file_rx.recv() => {
                    if let Some((to, id, audio)) = f { eng.send_file(to, id, audio); }
                }
                c = cmds.recv() => {
                    let Some(c) = c else { break };
                    let go_on = if eng.p.host { eng.host_cmd(c) } else { eng.member_cmd(c) };
                    if !go_on { break; }
                }
                _ = tick.tick() => {
                    eng.file_tick();
                    if eng.p.host {
                        eng.host_tick();
                        if last_play.elapsed() >= PLAY_EVERY { eng.host_repeat_state(); last_play = Instant::now(); }
                    } else {
                        if !eng.member_tick(started.elapsed()) { break; }
                        if !eng.welcomed && last_hello.elapsed() >= HELLO_EVERY { eng.say_hello(); last_hello = Instant::now(); }
                        // a quick burst of pings to learn the clock, then now and then
                        let due = if eng.ping_sent < 6 { ping_burst.elapsed() >= Duration::from_millis(400) } else { last_ping.elapsed() >= PING_EVERY };
                        if eng.welcomed && due { eng.ping(); last_ping = Instant::now(); ping_burst = Instant::now(); }
                    }
                }
            }
        }
    });
    handle
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

    /// A relay in memory: every message goes to everybody, the sender included.
    struct Loopback {
        peers: Mutex<Vec<mpsc::UnboundedSender<HubEvent>>>,
        /// what the relay keeps (messages marked `keep`), replayed to whoever comes later
        log: Mutex<Vec<String>>,
    }
    struct Line {
        net: Arc<Loopback>,
    }
    impl Transport for Line {
        fn send(&self, blob: String, keep: bool, id: Option<String>) {
            if keep {
                self.net.log.lock().unwrap().push(blob.clone());
            }
            for p in self.net.peers.lock().unwrap().iter() {
                let _ = p.send(HubEvent::Msg { seq: 0, blob: blob.clone(), keep, id: id.clone() });
            }
        }
    }
    impl Loopback {
        fn new() -> Arc<Self> {
            Arc::new(Loopback { peers: Mutex::new(Vec::new()), log: Mutex::new(Vec::new()) })
        }
        fn join(self: &Arc<Self>) -> (Arc<dyn Transport>, mpsc::UnboundedReceiver<HubEvent>) {
            let (tx, rx) = mpsc::unbounded_channel();
            self.peers.lock().unwrap().push(tx.clone());
            let _ = tx.send(HubEvent::Connected { head: 0, peers: 0 });
            for (i, blob) in self.log.lock().unwrap().iter().enumerate() {
                let _ = tx.send(HubEvent::Msg { seq: i as u64 + 1, blob: blob.clone(), keep: true, id: None });
            }
            (Arc::new(Line { net: self.clone() }), rx)
        }
    }

    /// A fake player that records what it was asked, with a position that runs with real time.
    #[derive(Default)]
    pub struct FakePlayer {
        pub prepared: Mutex<Vec<String>>,
        pub started: Mutex<Option<Instant>>,
        pub position: AtomicI64,
        pub playing: Mutex<bool>,
        pub seeks: AtomicU32,
        pub trims: Mutex<Vec<f32>>,
        pub stopped: AtomicU32,
        pub offset_ms: AtomicI64,
    }
    impl Port for FakePlayer {
        fn prepare(&self, item: &QueueItem) {
            self.prepared.lock().unwrap().push(item.title.clone());
            *self.started.lock().unwrap() = None;
        }
        fn start_after(&self, delay: Duration) {
            *self.started.lock().unwrap() = Some(Instant::now() + delay);
            *self.playing.lock().unwrap() = true;
        }
        fn set_playing(&self, playing: bool, pos_ms: i64) {
            *self.playing.lock().unwrap() = playing;
            self.seeks.fetch_add(1, Ordering::SeqCst);
            // as if the player now stood at pos_ms
            *self.started.lock().unwrap() = Some(Instant::now() - Duration::from_millis(pos_ms.max(0) as u64));
            self.position.store(pos_ms, Ordering::SeqCst);
        }
        fn position_ms(&self) -> Option<i64> {
            let s = (*self.started.lock().unwrap())?;
            let ms = if Instant::now() >= s { s.elapsed().as_millis() as i64 } else { 0 };
            Some(ms + self.offset_ms.load(Ordering::SeqCst))
        }
        fn trim(&self, f: f32) {
            self.trims.lock().unwrap().push(f);
        }
        fn stop(&self) {
            self.stopped.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn member(id: &str) -> Member {
        Member { id: id.into(), name: id.into(), avatar_url: None, status: Status::Connecting, pubkey: None, last_seen: 0 }
    }

    pub fn item(n: u64) -> QueueItem {
        QueueItem { track_id: n, title: format!("track{n}"), artist: "a".into(), added_by: "host".into(), duration_ms: 0 }
    }

    fn quiet() -> Arc<dyn Fn(&RoomView) + Send + Sync> {
        Arc::new(|_| {})
    }

    pub async fn wait(cond: impl Fn() -> bool, ms: u64) -> bool {
        for _ in 0..(ms / 20) {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        cond()
    }

    fn room(channel: &str, password: &str, net: &Arc<Loopback>, id: &str, host: bool) -> (Handle, Arc<FakePlayer>) {
        let (tr, rx) = net.join();
        let port = Arc::new(FakePlayer::default());
        let h = start(
            Params { channel: channel.into(), password: password.into(), me: member(id), host, settings: Settings { name: "Ночной фонк".into(), ..Settings::default() } },
            tr,
            rx,
            port.clone(),
            quiet(),
        );
        (h, port)
    }

    /// A player with «Мои файлы»: what it has, and what came to it through the room.
    #[derive(Default)]
    struct FilePlayer {
        inner: FakePlayer,
        have: Mutex<std::collections::HashMap<u64, Vec<u8>>>,
        received: Mutex<Vec<(u64, usize)>>,
    }
    impl Port for FilePlayer {
        fn prepare(&self, item: &QueueItem) { self.inner.prepare(item) }
        fn start_after(&self, d: Duration) { self.inner.start_after(d) }
        fn set_playing(&self, p: bool, pos: i64) { self.inner.set_playing(p, pos) }
        fn position_ms(&self) -> Option<i64> { self.inner.position_ms() }
        fn trim(&self, f: f32) { self.inner.trim(f) }
        fn stop(&self) { self.inner.stop() }
        fn has_local(&self, id: u64) -> bool { !crate::local::is_local(id) || self.have.lock().unwrap().contains_key(&id) }
        fn local_audio(&self, id: u64, done: Box<dyn FnOnce(Option<Vec<u8>>) + Send>) { done(self.have.lock().unwrap().get(&id).cloned()) }
        fn local_received(&self, item: &QueueItem, audio: Vec<u8>) {
            self.received.lock().unwrap().push((item.track_id, audio.len()));
            self.have.lock().unwrap().insert(item.track_id, audio);
        }
    }

    #[test]
    fn a_track_of_my_files_reaches_a_member_who_lacks_it_and_then_loads() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let mk = |id: &str, host: bool| {
                let (tr, rx) = net.join();
                let port = Arc::new(FilePlayer::default());
                let h = start(Params { channel: "chan-files".into(), password: String::new(), me: member(id), host, settings: Settings::default() }, tr, rx, port.clone(), quiet());
                (h, port)
            };
            let (host, hp) = mk("host", true);
            let (anna, ap) = mk("anna", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 2, 4000).await, "anna joined");
            let id = crate::local::BASE + 7;
            // 600 KB: three pieces
            let audio: Vec<u8> = (0..600_000u32).map(|i| (i % 251) as u8).collect();
            hp.have.lock().unwrap().insert(id, audio.clone());
            host.tx.send(Cmd::Enqueue(QueueItem { track_id: id, title: "мой трек".into(), artist: "я".into(), added_by: "host".into(), duration_ms: 1000 })).unwrap();
            assert!(wait(|| !ap.received.lock().unwrap().is_empty(), 5000).await, "the file reached anna");
            assert_eq!(ap.have.lock().unwrap().get(&id), Some(&audio), "whole and the same");
            assert!(wait(|| ap.inner.prepared.lock().unwrap().len() == 1, 2000).await, "then she loads it");
            drop(anna);
        });
    }

    #[test]
    fn everybody_loads_then_starts_together_and_a_late_one_is_waited_for() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, hp) = room("chan-1", "", &net, "host", true);
            let (anna, ap) = room("chan-1", "", &net, "anna", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 2, 4000).await, "anna joined");
            assert!(wait(|| anna.view.lock().unwrap().members.len() == 2, 2000).await, "anna got the roster");

            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            assert!(wait(|| hp.prepared.lock().unwrap().len() == 1 && ap.prepared.lock().unwrap().len() == 1, 3000).await, "both asked to load");
            // the host has its track, anna is slow: nobody starts yet
            host.tx.send(Cmd::Loaded { ok: true }).unwrap();
            tokio::time::sleep(Duration::from_millis(1200)).await;
            assert!(hp.started.lock().unwrap().is_none() && ap.started.lock().unwrap().is_none(), "the barrier holds while anna loads");
            let st = |v: &Handle| v.view.lock().unwrap().members.iter().find(|m| m.id == "anna").map(|m| m.status.clone());
            assert_eq!(st(&host).as_deref(), Some("loading"), "her nick shows «грузится»");

            anna.tx.send(Cmd::Loaded { ok: true }).unwrap();
            assert!(wait(|| hp.started.lock().unwrap().is_some() && ap.started.lock().unwrap().is_some(), 4000).await, "all start once everybody is ready");
            let (h0, a0) = (hp.started.lock().unwrap().unwrap(), ap.started.lock().unwrap().unwrap());
            let gap = if h0 > a0 { h0 - a0 } else { a0 - h0 };
            assert!(gap < Duration::from_millis(60), "start moments differ by {gap:?}");
            assert_eq!(st(&host).as_deref(), Some("playing"));
        });
    }

    #[test]
    fn an_empty_room_is_taken_by_whoever_enters_first_and_continues_from_its_saved_state() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, _hp) = room("chan-keep", "", &net, "host", true);
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            host.tx.send(Cmd::Enqueue(item(2))).unwrap();
            assert!(wait(|| host.view.lock().unwrap().queue.len() == 1, 3000).await);
            // the state is saved on the relay a moment later; then the host goes and the room is empty
            tokio::time::sleep(Duration::from_millis(1200)).await;
            host.tx.send(Cmd::Leave).unwrap();
            assert!(wait(|| host.view.lock().unwrap().ended.is_some(), 2000).await);

            let (anna, ap) = room("chan-keep", "", &net, "anna", false);
            assert!(wait(|| anna.view.lock().unwrap().host, 4000).await, "anna became the host of the empty room");
            let v = anna.view.lock().unwrap().clone();
            assert_eq!(v.queue.len(), 2, "the queue was kept: {:?}", v.queue);
            assert_eq!(v.name, "Ночной фонк");
            // she presses play: the first track is asked for
            anna.tx.send(Cmd::Play(true)).unwrap();
            assert!(wait(|| ap.prepared.lock().unwrap().len() == 1, 2000).await, "play starts the queue");
        });
    }

    #[test]
    fn a_member_who_cannot_load_the_track_does_not_hold_the_others_up() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, hp) = room("chan-fail", "", &net, "host", true);
            let (anna, ap) = room("chan-fail", "", &net, "anna", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 2, 4000).await);
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            assert!(wait(|| ap.prepared.lock().unwrap().len() == 1, 3000).await);
            host.tx.send(Cmd::Loaded { ok: true }).unwrap();
            anna.tx.send(Cmd::Loaded { ok: false }).unwrap();
            assert!(wait(|| hp.started.lock().unwrap().is_some(), 4000).await, "the host's music starts without waiting for anna");
            // the round is not started again by itself
            tokio::time::sleep(Duration::from_millis(1500)).await;
            assert_eq!(hp.prepared.lock().unwrap().len(), 1, "no second round");
        });
    }

    #[test]
    fn when_the_host_leaves_the_next_in_the_list_takes_over_and_the_music_goes_on() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, hp) = room("chan-hand", "", &net, "host", true);
            let (anna, ap) = room("chan-hand", "", &net, "anna", false);
            let (bob, _bp) = room("chan-hand", "", &net, "bob", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 3, 4000).await);
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            assert!(wait(|| ap.prepared.lock().unwrap().len() == 1, 3000).await);
            host.tx.send(Cmd::Loaded { ok: true }).unwrap();
            anna.tx.send(Cmd::Loaded { ok: true }).unwrap();
            bob.tx.send(Cmd::Loaded { ok: true }).unwrap();
            assert!(wait(|| hp.started.lock().unwrap().is_some() && ap.started.lock().unwrap().is_some(), 4000).await);

            host.tx.send(Cmd::Leave).unwrap();
            assert!(wait(|| anna.view.lock().unwrap().host, 4000).await, "anna, first after the host, took over");
            assert!(!bob.view.lock().unwrap().host, "bob stays a member");
            assert!(wait(|| bob.view.lock().unwrap().host_id == "anna", 3000).await, "bob follows the new host");
            assert!(*ap.playing.lock().unwrap(), "the music goes on for the new host");
            assert_eq!(anna.view.lock().unwrap().current.as_ref().map(|c| c.title.clone()), Some("track1".into()));
        });
    }

    #[test]
    fn two_who_enter_an_empty_room_at_once_end_up_with_one_host() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (a, _ap) = room("chan-race", "", &net, "aa", false);
            let (b, _bp) = room("chan-race", "", &net, "bb", false);
            assert!(wait(|| a.view.lock().unwrap().host || b.view.lock().unwrap().host, 4000).await);
            tokio::time::sleep(Duration::from_millis(2500)).await;
            let (ha, hb) = (a.view.lock().unwrap().host, b.view.lock().unwrap().host);
            assert!(ha ^ hb, "exactly one host, got aa={ha} bb={hb}");
            assert!(ha, "the smaller id wins the tie");
            assert!(wait(|| b.view.lock().unwrap().host_id == "aa", 3000).await);
            assert!(wait(|| a.view.lock().unwrap().members.len() == 2, 3000).await, "the loser is in the winner's room");
        });
    }

    #[test]
    fn a_track_the_host_picks_plays_at_once_and_the_queue_keeps_its_order() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, hp) = room("chan-now", "", &net, "host", true);
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            host.tx.send(Cmd::Enqueue(item(2))).unwrap();
            assert!(wait(|| hp.prepared.lock().unwrap().len() == 1, 2000).await);
            host.tx.send(Cmd::PlayNow(item(9))).unwrap();
            assert!(wait(|| hp.prepared.lock().unwrap().len() == 2, 2000).await, "the picked track is loaded now");
            assert_eq!(hp.prepared.lock().unwrap().last().cloned(), Some("track9".into()));
            assert_eq!(host.view.lock().unwrap().queue.first().map(|q| q.title.clone()), Some("track2".into()), "the queue goes on after it");
        });
    }

    #[test]
    fn the_host_takes_a_track_out_of_the_queue_and_a_stale_index_changes_nothing() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, hp) = room("chan-remove", "", &net, "host", true);
            for n in 1..=4 {
                host.tx.send(Cmd::Enqueue(item(n))).unwrap();
            }
            // track 1 starts, tracks 2, 3, 4 wait
            assert!(wait(|| hp.prepared.lock().unwrap().len() == 1 && host.view.lock().unwrap().queue.len() == 3, 2000).await);
            host.tx.send(Cmd::Remove { index: 1, track_id: 3 }).unwrap();
            assert!(wait(|| host.view.lock().unwrap().queue.len() == 2, 2000).await, "track 3 is gone");
            let titles: Vec<String> = host.view.lock().unwrap().queue.iter().map(|q| q.title.clone()).collect();
            assert_eq!(titles, vec!["track2".to_string(), "track4".to_string()]);
            // the queue moved under the click: the id does not match, so nothing is removed
            host.tx.send(Cmd::Remove { index: 0, track_id: 4 }).unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            assert_eq!(host.view.lock().unwrap().queue.len(), 2);
        });
    }

    #[test]
    fn previous_goes_back_a_track_and_restarts_when_pressed_late() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, hp) = room("chan-prev", "", &net, "host", true);
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            host.tx.send(Cmd::Enqueue(item(2))).unwrap();
            assert!(wait(|| hp.prepared.lock().unwrap().len() == 1, 2000).await);
            host.tx.send(Cmd::Next).unwrap();
            assert!(wait(|| hp.prepared.lock().unwrap().len() == 2, 2000).await, "second track asked for");
            host.tx.send(Cmd::Prev).unwrap();
            assert!(wait(|| hp.prepared.lock().unwrap().len() == 3, 2000).await, "previous loads the first track again");
            assert_eq!(hp.prepared.lock().unwrap().last().cloned(), Some("track1".into()));
            // and the track that was playing waits in the queue right after it
            assert_eq!(host.view.lock().unwrap().queue.first().map(|q| q.title.clone()), Some("track2".into()));
        });
    }

    #[test]
    fn joining_in_the_middle_of_a_track_loads_it_and_catches_up() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, hp) = room("chan-mid", "", &net, "host", true);
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            assert!(wait(|| hp.prepared.lock().unwrap().len() == 1, 3000).await);
            host.tx.send(Cmd::Loaded { ok: true }).unwrap();
            assert!(wait(|| hp.started.lock().unwrap().is_some(), 3000).await, "the host plays alone");
            tokio::time::sleep(Duration::from_millis(1500)).await;

            let (anna, ap) = room("chan-mid", "", &net, "anna", false);
            assert!(wait(|| ap.prepared.lock().unwrap().len() == 1, 4000).await, "anna is asked to load the track in progress");
            assert!(!*ap.playing.lock().unwrap(), "not before it is loaded");
            anna.tx.send(Cmd::Loaded { ok: true }).unwrap();
            assert!(wait(|| *ap.playing.lock().unwrap(), 6000).await, "anna plays after loading");
            let (h, a) = (hp.position.load(Ordering::SeqCst).max(hp.position_ms().unwrap_or(0)), ap.position_ms().unwrap_or(-1));
            assert!((a - hp.position_ms().unwrap_or(0)).abs() < 400, "anna stands at {a} ms, host at {h} ms");
        });
    }

    #[test]
    fn after_a_kick_newcomers_still_get_in_and_the_kicked_one_cannot_come_back() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, _hp) = room("chan-kick", "pw", &net, "host", true);
            let (anna, _ap) = room("chan-kick", "pw", &net, "anna", false);
            let (bob, _bp) = room("chan-kick", "pw", &net, "bob", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 3, 5000).await);
            host.tx.send(Cmd::Kick("anna".into())).unwrap();
            assert!(wait(|| anna.view.lock().unwrap().ended.as_deref() == Some("kicked"), 4000).await, "anna is told");
            assert!(wait(|| host.view.lock().unwrap().members.len() == 2, 3000).await);

            // a newcomer knows only the password's key: the host still reads her hello and hands her the new key
            let (carol, _cp) = room("chan-kick", "pw", &net, "carol", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 3, 5000).await, "carol came in after the kick");
            assert!(!carol.view.lock().unwrap().host, "and she did not take the room for herself");
            assert!(wait(|| carol.view.lock().unwrap().members.len() == 3, 3000).await, "she hears the room under the new key");
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            assert!(wait(|| carol.view.lock().unwrap().current.is_some() || !carol.view.lock().unwrap().queue.is_empty(), 4000).await, "carol follows what the host does");
            assert!(wait(|| bob.view.lock().unwrap().current.is_some() || !bob.view.lock().unwrap().queue.is_empty(), 4000).await, "bob too");

            // anna comes back with the password: refused, she does not start a room of her own
            let (anna2, _ap2) = room("chan-kick", "pw", &net, "anna", false);
            assert!(wait(|| anna2.view.lock().unwrap().ended.as_deref() == Some("kicked"), 4000).await, "anna is refused again");
            assert!(!anna2.view.lock().unwrap().host);

            // the host lets her back: the list shows her, and she comes in
            assert_eq!(host.view.lock().unwrap().banned, vec!["anna".to_string()]);
            host.tx.send(Cmd::Unban("anna".into())).unwrap();
            assert!(wait(|| host.view.lock().unwrap().banned.is_empty(), 3000).await, "the list is empty again");
            let (anna3, _ap3) = room("chan-kick", "pw", &net, "anna", false);
            assert!(wait(|| anna3.view.lock().unwrap().members.len() == 4, 6000).await, "anna is back in the room");
            assert!(!anna3.view.lock().unwrap().host);
            assert_eq!(anna3.view.lock().unwrap().ended, None);
        });
    }

    #[test]
    fn a_quiet_listener_who_keeps_pinging_stays_in_the_room() {
        // test build: silent for 3 s is a problem, 5 s is gone; a steadily playing member sends nothing else
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, _hp) = room("chan-quiet", "", &net, "host", true);
            let (anna, _ap) = room("chan-quiet", "", &net, "anna", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 2, 4000).await);
            tokio::time::sleep(Duration::from_millis(7500)).await;
            let st = |v: &Handle| v.view.lock().unwrap().members.iter().find(|m| m.id == "anna").map(|m| m.status.clone());
            assert_eq!(host.view.lock().unwrap().members.len(), 2, "anna was dropped for being quiet");
            assert_ne!(st(&host).as_deref(), Some("problem"), "anna shows a connection problem");
            assert_eq!(anna.view.lock().unwrap().members.len(), 2);
        });
    }

    #[test]
    fn a_member_who_joined_long_after_the_host_started_still_starts_together() {
        // the clocks of the two engines begin 900 ms apart: without a clock estimate the start would be 900 ms off
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, hp) = room("chan-5", "", &net, "host", true);
            tokio::time::sleep(Duration::from_millis(900)).await;
            let (anna, ap) = room("chan-5", "", &net, "anna", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 2, 4000).await);
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            assert!(wait(|| ap.prepared.lock().unwrap().len() == 1, 3000).await);
            host.tx.send(Cmd::Loaded { ok: true }).unwrap();
            anna.tx.send(Cmd::Loaded { ok: true }).unwrap();
            assert!(wait(|| hp.started.lock().unwrap().is_some() && ap.started.lock().unwrap().is_some(), 4000).await);
            let (h0, a0) = (hp.started.lock().unwrap().unwrap(), ap.started.lock().unwrap().unwrap());
            let gap = if h0 > a0 { h0 - a0 } else { a0 - h0 };
            assert!(gap < Duration::from_millis(60), "start moments differ by {gap:?}");
        });
    }

    #[test]
    fn a_wrong_password_reads_nothing_and_gives_up_with_no_answer() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, _hp) = room("chan-2", "secret", &net, "host", true);
            let (eve, _ep) = room("chan-2", "guess", &net, "eve", false);
            tokio::time::sleep(Duration::from_millis(1500)).await;
            assert_eq!(host.view.lock().unwrap().members.len(), 1, "the host never saw eve's hello");
            assert!(eve.view.lock().unwrap().members.len() <= 1, "and eve was never let in");
            let (ok, _p) = room("chan-2", "secret", &net, "anna", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 2, 4000).await, "the right password gets in");
            drop(ok);
        });
    }

    #[test]
    fn a_member_who_drifts_is_trimmed_a_little_or_seeks_when_far_off() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, hp) = room("chan-3", "", &net, "host", true);
            let (anna, ap) = room("chan-3", "", &net, "anna", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 2, 4000).await);
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            assert!(wait(|| ap.prepared.lock().unwrap().len() == 1, 3000).await);
            host.tx.send(Cmd::Loaded { ok: true }).unwrap();
            anna.tx.send(Cmd::Loaded { ok: true }).unwrap();
            assert!(wait(|| ap.started.lock().unwrap().is_some(), 4000).await);
            tokio::time::sleep(Duration::from_millis(2200)).await; // after the start moment

            // anna is 500 ms behind: not a jump, a little faster
            ap.offset_ms.store(-500, Ordering::SeqCst);
            hp.offset_ms.store(0, Ordering::SeqCst);
            host.tx.send(Cmd::Play(true)).unwrap(); // the host repeats where it is now
            assert!(wait(|| ap.trims.lock().unwrap().iter().any(|f| *f > 1.0), 3000).await, "a gentle speed-up, not a seek: {:?}", ap.trims.lock().unwrap());
            let seeks_before = ap.seeks.load(Ordering::SeqCst);

            // 4 s behind: that is audible, seek
            ap.offset_ms.store(-4000, Ordering::SeqCst);
            host.tx.send(Cmd::Play(true)).unwrap();
            assert!(wait(|| ap.seeks.load(Ordering::SeqCst) > seeks_before, 3000).await, "a big gap is fixed by seeking");
        });
    }

    #[test]
    fn a_kicked_member_is_out_and_cannot_read_what_comes_next() {
        tauri::async_runtime::block_on(async {
            let net = Loopback::new();
            let (host, _hp) = room("chan-4", "", &net, "host", true);
            let (anna, ap) = room("chan-4", "", &net, "anna", false);
            let (bob, bp) = room("chan-4", "", &net, "bob", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 3, 5000).await);
            host.tx.send(Cmd::Kick("bob".into())).unwrap();
            assert!(wait(|| bob.view.lock().unwrap().ended.as_deref() == Some("kicked"), 3000).await, "bob learns he is out");
            // after the new key, what the host enqueues reaches anna and not bob
            host.tx.send(Cmd::Enqueue(item(9))).unwrap();
            assert!(wait(|| ap.prepared.lock().unwrap().contains(&"track9".to_string()), 4000).await, "anna still follows the room");
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(bp.prepared.lock().unwrap().is_empty(), "bob got no track");
            assert_eq!(host.view.lock().unwrap().members.len(), 2);
            drop(anna);
        });
    }
}

#[cfg(test)]
mod live {
    use super::tests::{item, member, wait, FakePlayer};
    use super::*;

    /// The same story as the in-memory tests, over the real relay and the real WebSocket client.
    /// `cd server && npm run dev`, then `SYNC_URL=http://127.0.0.1:8787 cargo test sync::engine::live -- --ignored`
    #[test]
    #[ignore]
    fn host_and_member_through_the_real_relay() {
        let Ok(base) = std::env::var("SYNC_URL") else { return };
        tauri::async_runtime::block_on(async move {
            let channel = format!("live{}", crate::sync::crypto::new_channel_id());
            let mk = |id: &str, host: bool| {
                let (hub, rx) = crate::sync::hub::connect(base.clone(), channel.clone(), 0);
                let port = Arc::new(FakePlayer::default());
                let h = start(
                    Params { channel: channel.clone(), password: "pw".into(), me: member(id), host, settings: Settings { name: "live".into(), ..Settings::default() } },
                    Arc::new(hub),
                    rx,
                    port.clone(),
                    Arc::new(|_| {}),
                );
                (h, port)
            };
            let (host, hp) = mk("host", true);
            tokio::time::sleep(Duration::from_millis(600)).await;
            let (anna, ap) = mk("anna", false);
            assert!(wait(|| host.view.lock().unwrap().members.len() == 2 && anna.view.lock().unwrap().members.len() == 2, 8000).await, "joined over the relay");
            host.tx.send(Cmd::Enqueue(item(1))).unwrap();
            assert!(wait(|| ap.prepared.lock().unwrap().len() == 1, 4000).await);
            host.tx.send(Cmd::Loaded { ok: true }).unwrap();
            anna.tx.send(Cmd::Loaded { ok: true }).unwrap();
            assert!(wait(|| hp.started.lock().unwrap().is_some() && ap.started.lock().unwrap().is_some(), 6000).await, "started together");
            let (h0, a0) = (hp.started.lock().unwrap().unwrap(), ap.started.lock().unwrap().unwrap());
            let gap = if h0 > a0 { h0 - a0 } else { a0 - h0 };
            println!("start gap over the relay: {gap:?}");
            assert!(gap < Duration::from_millis(150), "{gap:?}");
            host.tx.send(Cmd::Leave).unwrap();
            anna.tx.send(Cmd::Leave).unwrap();
        });
    }
}
