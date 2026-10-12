//! Rooms («Комнаты»): listening together, without voice. This file is the
//! transport-independent core, written and tested before any UI or server:
//!
//! * the messages members exchange (`Msg`) and the room they build (`Room`);
//! * clock sync (`ClockSync`): everybody's clock differs, so a member learns
//!   its offset to the host's clock from ping/pong, trusting the sample with
//!   the shortest round trip;
//! * the start barrier (`Barrier`): a track starts for everyone at the same
//!   host-clock moment, only after every member reported "ready"; one who is
//!   slow shows «грузится», one who is silent too long shows «проблема с
//!   соединением» and does not hold the others back forever;
//! * playback projection (`PlayState`): where the track is at a given moment,
//!   so a late joiner or a member who stalled catches up by seeking.
//!
//! The host is the authority (queue, start, kick); members only ask. Not wired
//! into the app yet: transport, encryption, and UI wait for the design to be
//! chosen (see HANDOFF «Комнаты»).
#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

/// Milliseconds on some clock (host clock or local clock, never mixed).
pub type Ms = i64;

/// A member silent for this long is shown as having a connection problem.
#[cfg(not(test))]
pub const SILENT_PROBLEM_MS: Ms = 8_000;
#[cfg(test)]
pub const SILENT_PROBLEM_MS: Ms = 3_000;
/// A member silent for this long is dropped from the barrier and the room.
#[cfg(not(test))]
pub const SILENT_GONE_MS: Ms = 45_000;
#[cfg(test)]
pub const SILENT_GONE_MS: Ms = 5_000;
/// How long the host waits for everybody to load a track before starting anyway.
pub const BARRIER_TIMEOUT_MS: Ms = 20_000;
/// Start is announced this far ahead so every member can still receive it.
pub const START_LEAD_MS: Ms = 1_200;
/// A member further than this from the right position is told to seek.
pub const DRIFT_SEEK_MS: Ms = 1_500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "text", rename_all = "snake_case")]
pub enum Status {
    /// just joined, not synchronised yet
    Connecting,
    /// loading the current track
    Loading,
    /// has the track, waiting for the others
    Ready,
    /// playing in sync
    Playing,
    /// no news for a while, or the track failed to load
    Problem(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub status: Status,
    /// X25519 public key (base64): the host wraps a new room key for it after a kick
    #[serde(default)]
    pub pubkey: Option<String>,
    /// local time of the last message from them
    #[serde(skip)]
    pub last_seen: Ms,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    Everyone,
    HostOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// anybody with the link or the code
    Link,
    /// only SoundCloud accounts the host follows / follows back
    Friends,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub name: String,
    pub has_password: bool,
    pub access: Access,
    pub who_can_queue: Who,
    pub who_can_control: Who,
    pub max_members: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: String::new(),
            has_password: false,
            access: Access::Link,
            who_can_queue: Who::Everyone,
            who_can_control: Who::HostOnly,
            max_members: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItem {
    pub track_id: u64,
    pub title: String,
    pub artist: String,
    pub added_by: String,
}

/// Where the current track is, on the host's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayState {
    /// host-clock moment the track was at `pos_ms`
    pub at: Ms,
    pub pos_ms: Ms,
    pub playing: bool,
}

impl PlayState {
    pub fn position_at(&self, host_now: Ms) -> Ms {
        if self.playing {
            (self.pos_ms + (host_now - self.at)).max(0)
        } else {
            self.pos_ms
        }
    }
}

/// Everything a member sends. The host's copy of the room is the truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Msg {
    /// member -> host: let me in
    Hello { member: Member },
    /// host -> member: you are in; the whole room as it is now
    Welcome { to: String, room: RoomSnapshot, host_now: Ms, host_pub: String },
    /// host -> member: not let in (room full, kicked)
    Denied { to: String, reason: String },
    /// host -> all (under the old key): the room key changed because somebody was kicked
    Rekey { epoch: u32, wraps: std::collections::BTreeMap<String, String> },
    /// member -> host: my status changed
    Status { id: String, status: Status },
    /// member -> host: put this track in the queue (or the host's own add)
    Enqueue { item: QueueItem },
    /// host -> all: the queue changed
    Queue { items: Vec<QueueItem> },
    /// host -> all: load this track now, `seq` ties the answers to it
    Load { seq: u64, item: QueueItem },
    /// member -> host: loaded track `seq` ("ready") or failed to
    Loaded { seq: u64, id: String, ok: bool },
    /// host -> all: start track `seq` at host-clock moment `at`
    Go { seq: u64, at: Ms },
    /// host -> all: pause / resume / seek, host-clock stamped
    Play { state: PlayState },
    /// host -> all: roster or settings changed
    Roster { members: Vec<Member>, settings: Settings },
    /// host -> all: this member is out
    Kick { id: String },
    /// clock sync, either direction pair
    Ping { id: String, t0: Ms },
    Pong { id: String, t0: Ms, host_t: Ms },
    Leave { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSnapshot {
    pub settings: Settings,
    pub members: Vec<Member>,
    pub queue: Vec<QueueItem>,
    pub current: Option<(u64, QueueItem)>,
    pub play: Option<PlayState>,
}

// ------------------------------------------------------------ clock sync

/// Offset of the host clock from the local one: `host = local + offset`.
#[derive(Debug, Default)]
pub struct ClockSync {
    best: Option<(Ms, Ms)>, // (round trip, offset)
    /// a first guess (the host's clock in the Welcome), good to about one network delay, until a real sample comes
    rough: Option<Ms>,
}

impl ClockSync {
    /// One ping/pong: sent at local `t0`, host answered with its clock `host_t`,
    /// the answer arrived at local `t1`. The host stamped it somewhere inside
    /// the round trip; the middle is the best guess, and the sample with the
    /// shortest round trip is the least wrong.
    pub fn sample(&mut self, t0: Ms, host_t: Ms, t1: Ms) {
        let rtt = t1 - t0;
        if rtt < 0 {
            return;
        }
        let offset = host_t - (t0 + rtt / 2);
        if self.best.is_none_or(|(best_rtt, _)| rtt <= best_rtt) {
            self.best = Some((rtt, offset));
        }
    }

    pub fn ready(&self) -> bool {
        self.best.is_some()
    }

    /// `host_now - local_now` taken from a single message: not exact, but far better than nothing.
    pub fn set_rough(&mut self, offset: Ms) {
        self.rough = Some(offset);
    }

    pub fn offset(&self) -> Ms {
        self.best.map(|(_, o)| o).or(self.rough).unwrap_or(0)
    }

    pub fn rtt(&self) -> Option<Ms> {
        self.best.map(|(r, _)| r)
    }

    pub fn to_local(&self, host_time: Ms) -> Ms {
        host_time - self.offset()
    }

    pub fn to_host(&self, local_time: Ms) -> Ms {
        local_time + self.offset()
    }
}

// --------------------------------------------------------- start barrier

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Barrier {
    /// still waiting for these many members
    Waiting { missing: usize },
    /// everybody (still reachable) is ready: start
    Go,
    /// the wait ran out: start without the missing ones
    TimedOut,
}

/// The host's view of one "load, then start together" round.
#[derive(Debug)]
pub struct Round {
    pub seq: u64,
    started: Ms,
    /// member id -> Some(ok) once answered
    answers: BTreeMap<String, Option<bool>>,
}

impl Round {
    pub fn new(seq: u64, now: Ms, members: impl IntoIterator<Item = String>) -> Self {
        Round { seq, started: now, answers: members.into_iter().map(|m| (m, None)).collect() }
    }

    pub fn answer(&mut self, seq: u64, id: &str, ok: bool) {
        if seq == self.seq {
            if let Some(a) = self.answers.get_mut(id) {
                *a = Some(ok);
            }
        }
    }

    /// Somebody left or was kicked mid-round: they do not hold anyone up.
    pub fn drop_member(&mut self, id: &str) {
        self.answers.remove(id);
    }

    /// Who is still loading (for «грузится» next to the nick).
    pub fn waiting_for(&self) -> Vec<&str> {
        self.answers.iter().filter(|(_, a)| a.is_none()).map(|(id, _)| id.as_str()).collect()
    }

    /// Members whose load failed (for «проблема»).
    pub fn failed(&self) -> Vec<&str> {
        self.answers.iter().filter(|(_, a)| **a == Some(false)).map(|(id, _)| id.as_str()).collect()
    }

    pub fn state(&self, now: Ms) -> Barrier {
        let missing = self.waiting_for().len();
        if missing == 0 {
            Barrier::Go
        } else if now - self.started >= BARRIER_TIMEOUT_MS {
            Barrier::TimedOut
        } else {
            Barrier::Waiting { missing }
        }
    }

    /// Host-clock moment to start at, given when the decision is made.
    pub fn start_at(host_now: Ms) -> Ms {
        host_now + START_LEAD_MS
    }
}

// ------------------------------------------------------------ the room

/// The host's room: roster, queue, what plays. Pure bookkeeping; the caller
/// sends the messages these methods return.
#[derive(Debug)]
pub struct Room {
    pub settings: Settings,
    pub host: String,
    pub members: Vec<Member>,
    pub queue: VecDeque<QueueItem>,
    pub current: Option<(u64, QueueItem)>,
    pub play: Option<PlayState>,
    next_seq: u64,
    banned: Vec<String>,
}

impl Room {
    pub fn new(host: Member, settings: Settings) -> Self {
        Room { settings, host: host.id.clone(), members: vec![host], queue: VecDeque::new(), current: None, play: None, next_seq: 1, banned: Vec::new() }
    }

    pub fn snapshot(&self) -> RoomSnapshot {
        RoomSnapshot {
            settings: self.settings.clone(),
            members: self.members.clone(),
            queue: self.queue.iter().cloned().collect(),
            current: self.current.clone(),
            play: self.play,
        }
    }

    /// Hello: let a member in or say why not. `password_ok`: checked by the caller.
    pub fn join(&mut self, mut member: Member, password_ok: bool, now: Ms) -> Result<(), String> {
        if self.banned.contains(&member.id) {
            return Err("вас убрали из комнаты".into());
        }
        if self.settings.has_password && !password_ok {
            return Err("неверный пароль".into());
        }
        if self.members.iter().any(|m| m.id == member.id) {
            return Ok(()); // a repeated hello after a reconnect
        }
        if self.members.len() >= self.settings.max_members as usize {
            return Err("в комнате нет мест".into());
        }
        member.last_seen = now;
        member.status = Status::Connecting;
        self.members.push(member);
        Ok(())
    }

    pub fn leave(&mut self, id: &str) {
        self.members.retain(|m| m.id != id || m.id == self.host);
    }

    /// Only the host may kick, and never themselves; the one kicked cannot rejoin.
    pub fn kick(&mut self, by: &str, id: &str) -> bool {
        if by != self.host || id == self.host || !self.members.iter().any(|m| m.id == id) {
            return false;
        }
        self.members.retain(|m| m.id != id);
        self.banned.push(id.to_owned());
        true
    }

    pub fn set_status(&mut self, id: &str, status: Status, now: Ms) {
        if let Some(m) = self.members.iter_mut().find(|m| m.id == id) {
            m.status = status;
            m.last_seen = now;
        }
    }

    pub fn heard_from(&mut self, id: &str, now: Ms) {
        if let Some(m) = self.members.iter_mut().find(|m| m.id == id) {
            m.last_seen = now;
        }
    }

    /// Mark the quiet ones as having a problem, drop the long gone. Returns
    /// the ids dropped.
    pub fn check_silence(&mut self, now: Ms) -> Vec<String> {
        let host = self.host.clone();
        let mut gone = Vec::new();
        for m in &mut self.members {
            if m.id == host {
                continue;
            }
            let quiet = now - m.last_seen;
            if quiet >= SILENT_GONE_MS {
                gone.push(m.id.clone());
            } else if quiet >= SILENT_PROBLEM_MS && !matches!(m.status, Status::Problem(_)) {
                m.status = Status::Problem("проблема с соединением".into());
            }
        }
        self.members.retain(|m| !gone.contains(&m.id));
        gone
    }

    /// A member may add to the queue if the settings allow it.
    pub fn can_enqueue(&self, id: &str) -> bool {
        self.members.iter().any(|m| m.id == id) && (self.settings.who_can_queue == Who::Everyone || id == self.host)
    }

    pub fn can_control(&self, id: &str) -> bool {
        self.members.iter().any(|m| m.id == id) && (self.settings.who_can_control == Who::Everyone || id == self.host)
    }

    pub fn enqueue(&mut self, by: &str, item: QueueItem) -> bool {
        if !self.can_enqueue(by) {
            return false;
        }
        self.queue.push_back(item);
        true
    }

    /// Take the next track and open a round for it.
    pub fn next_round(&mut self, now: Ms) -> Option<(Round, QueueItem)> {
        let item = self.queue.pop_front()?;
        let seq = self.next_seq;
        self.next_seq += 1;
        self.current = Some((seq, item.clone()));
        self.play = None;
        for m in &mut self.members {
            m.status = Status::Loading;
        }
        let round = Round::new(seq, now, self.members.iter().map(|m| m.id.clone()));
        Some((round, item))
    }

    /// The barrier opened: everybody plays from the start at `at`.
    pub fn start(&mut self, at: Ms) -> PlayState {
        let st = PlayState { at, pos_ms: 0, playing: true };
        self.play = Some(st);
        for m in &mut self.members {
            if m.status != Status::Connecting && !matches!(m.status, Status::Problem(_)) {
                m.status = Status::Playing;
            }
        }
        st
    }
}

/// What a member does with a `Play` state: how far its player is from where
/// it should be, and whether to seek (small drift is left alone, a seek is audible).
pub fn drift_fix(state: &PlayState, host_now: Ms, my_pos_ms: Ms) -> Option<Ms> {
    let want = state.position_at(host_now);
    ((my_pos_ms - want).abs() > DRIFT_SEEK_MS).then_some(want)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str) -> Member {
        Member { id: id.into(), name: id.into(), avatar_url: None, status: Status::Connecting, pubkey: None, last_seen: 0 }
    }

    fn item(n: u64) -> QueueItem {
        QueueItem { track_id: n, title: format!("t{n}"), artist: "a".into(), added_by: "x".into() }
    }

    #[test]
    fn clock_sync_trusts_the_shortest_round_trip() {
        let mut c = ClockSync::default();
        // the host clock is 5000 ms ahead; a slow answer first, a fast one next
        c.sample(1000, 6500, 1600); // rtt 600, lopsided -> offset 5200
        let slow = c.offset();
        c.sample(2000, 7010, 2020); // rtt 20 -> offset 5000
        assert!(c.rtt() == Some(20));
        assert_eq!(c.offset(), 5000);
        assert_ne!(slow, 5000);
        c.sample(3000, 8500, 3900); // slow again: ignored
        assert_eq!(c.offset(), 5000);
        assert_eq!(c.to_local(8000), 3000);
        assert_eq!(c.to_host(3000), 8000);
    }

    #[test]
    fn play_state_projects_the_position() {
        let p = PlayState { at: 10_000, pos_ms: 4_000, playing: true };
        assert_eq!(p.position_at(12_500), 6_500);
        let paused = PlayState { playing: false, ..p };
        assert_eq!(paused.position_at(99_999), 4_000);
    }

    #[test]
    fn barrier_waits_for_everybody_then_goes() {
        let mut r = Round::new(1, 0, ["a".into(), "b".into(), "c".into()]);
        assert_eq!(r.state(100), Barrier::Waiting { missing: 3 });
        r.answer(1, "a", true);
        r.answer(1, "b", true);
        assert_eq!(r.waiting_for(), vec!["c"]);
        r.answer(0, "c", true); // an answer to an old round does nothing
        assert_eq!(r.state(200), Barrier::Waiting { missing: 1 });
        r.answer(1, "c", true);
        assert_eq!(r.state(300), Barrier::Go);
    }

    #[test]
    fn barrier_does_not_wait_forever_nor_for_the_gone() {
        let mut r = Round::new(1, 0, ["a".into(), "b".into()]);
        r.answer(1, "a", true);
        assert_eq!(r.state(BARRIER_TIMEOUT_MS - 1), Barrier::Waiting { missing: 1 });
        assert_eq!(r.state(BARRIER_TIMEOUT_MS), Barrier::TimedOut);
        r.drop_member("b");
        assert_eq!(r.state(1), Barrier::Go);
    }

    #[test]
    fn a_failed_load_is_visible_and_still_lets_the_round_finish() {
        let mut r = Round::new(1, 0, ["a".into(), "b".into()]);
        r.answer(1, "a", true);
        r.answer(1, "b", false);
        assert_eq!(r.failed(), vec!["b"]);
        assert_eq!(r.state(5), Barrier::Go);
    }

    #[test]
    fn password_capacity_and_kick() {
        let mut room = Room::new(member("host"), Settings { has_password: true, max_members: 3, ..Settings::default() });
        assert_eq!(room.join(member("a"), false, 0), Err("неверный пароль".into()));
        assert!(room.join(member("a"), true, 0).is_ok());
        assert!(room.join(member("a"), true, 0).is_ok(), "a repeated hello is fine");
        assert!(room.join(member("b"), true, 0).is_ok());
        assert_eq!(room.join(member("c"), true, 0), Err("в комнате нет мест".into()));
        assert!(!room.kick("a", "b"), "only the host kicks");
        assert!(!room.kick("host", "host"), "the host cannot kick themselves");
        assert!(room.kick("host", "b"));
        assert_eq!(room.join(member("b"), true, 0), Err("вас убрали из комнаты".into()));
        room.leave("host");
        assert!(room.members.iter().any(|m| m.id == "host"), "the host does not just leave");
    }

    #[test]
    fn queue_permissions_follow_the_settings() {
        let mut room = Room::new(member("host"), Settings { who_can_queue: Who::HostOnly, ..Settings::default() });
        room.join(member("a"), true, 0).unwrap();
        assert!(!room.enqueue("a", item(1)));
        assert!(room.enqueue("host", item(1)));
        room.settings.who_can_queue = Who::Everyone;
        assert!(room.enqueue("a", item(2)));
        assert!(!room.enqueue("stranger", item(3)));
        assert!(room.can_control("host") && !room.can_control("a"));
    }

    #[test]
    fn silent_members_show_a_problem_then_are_dropped() {
        let mut room = Room::new(member("host"), Settings::default());
        room.join(member("a"), true, 0).unwrap();
        room.join(member("b"), true, 0).unwrap();
        room.heard_from("b", 7_000);
        assert!(room.check_silence(SILENT_PROBLEM_MS).is_empty());
        let status = |room: &Room, id: &str| room.members.iter().find(|m| m.id == id).unwrap().status.clone();
        assert!(matches!(status(&room, "a"), Status::Problem(_)));
        assert_eq!(status(&room, "b"), Status::Connecting);
        let gone = room.check_silence(SILENT_GONE_MS);
        assert_eq!(gone, vec!["a".to_string()]);
        assert!(room.members.iter().all(|m| m.id != "a"));
        assert!(room.members.iter().any(|m| m.id == "host"), "the host is never timed out");
    }

    #[test]
    fn a_round_from_queue_to_everybody_playing() {
        let mut room = Room::new(member("host"), Settings::default());
        room.join(member("a"), true, 0).unwrap();
        room.enqueue("host", item(7));
        let (mut round, it) = room.next_round(1_000).unwrap();
        assert_eq!(it.track_id, 7);
        assert!(room.members.iter().all(|m| m.status == Status::Loading));
        round.answer(round.seq, "host", true);
        round.answer(round.seq, "a", true);
        assert_eq!(round.state(1_500), Barrier::Go);
        let st = room.start(Round::start_at(60_000));
        assert_eq!(st.at, 61_200);
        assert!(room.members.iter().all(|m| m.status == Status::Playing));
        assert_eq!(st.position_at(61_200 + 3_000), 3_000);
        assert!(room.next_round(2_000).is_none(), "queue is empty");
    }

    #[test]
    fn drift_is_fixed_only_when_audible() {
        let st = PlayState { at: 0, pos_ms: 0, playing: true };
        assert_eq!(drift_fix(&st, 10_000, 9_400), None);
        assert_eq!(drift_fix(&st, 10_000, 7_000), Some(10_000));
        assert_eq!(drift_fix(&st, 10_000, 12_000), Some(10_000));
    }

    #[test]
    fn messages_roundtrip_as_json() {
        let m = Msg::Go { seq: 3, at: 12345 };
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(j, r#"{"t":"go","seq":3,"at":12345}"#);
        assert_eq!(serde_json::from_str::<Msg>(&j).unwrap(), m);
        let s = Msg::Status { id: "a".into(), status: Status::Problem("x".into()) };
        assert_eq!(serde_json::from_str::<Msg>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
    }
}
