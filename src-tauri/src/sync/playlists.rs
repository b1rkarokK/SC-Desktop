//! Shared playlists: a playlist that several people edit. Each one is a channel of the relay; its
//! content is the CRDT of `shared_playlist.rs`, so the copies on all devices end up the same
//! whatever order the edits arrive in and a repeated edit changes nothing.
//!
//! * Every edit is a message that is also kept by the relay (`keep`), so somebody who was away asks
//!   for what is newer than the last message they saw. An edit made without a connection waits in
//!   an outbox (saved to disk) and is sent when the connection is back.
//! * Now and then somebody posts a snapshot of the whole state, because the relay keeps only the
//!   newest messages: a newcomer starts from a snapshot and replays what follows.
//! * The key is in the invitation link (`scdesk://playlist/<channel>#<key>`). The roles (owner,
//!   editor, viewer) and the access mode are checked by every client for every message; the first
//!   settings message founds the playlist and its owner. Whoever holds the link can write
//!   ciphertext, so these checks protect against honest mistakes and casual meddling, not against
//!   somebody who reads the source: an "open" or "link" playlist is as private as its link.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::{
    crypto::{self, Key},
    engine::Transport,
    hub::HubEvent,
};
use crate::shared_playlist::{Access, Edit, Playlist, Role, Settings, Stamp};

pub const LINK_PREFIX: &str = "scdesk://playlist/";
/// a snapshot after this many edits seen
const SNAPSHOT_EVERY: u32 = 100;

// externally tagged on purpose: serde cannot read numeric map keys (the members) out of an internally tagged enum
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
enum PlMsg {
    Edit { edit: Edit },
    /// from the owner: title, access, members
    Settings { settings: Settings, stamp: Stamp },
    Snapshot { playlist: Playlist, settings: Settings, stamp: Stamp },
    /// "this id is called so": for the nicks next to the tracks
    Name { id: u64, name: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Stored {
    pub key: String,
    pub mine: bool,
    pub playlist: Playlist,
    pub settings: Option<Settings>,
    pub settings_stamp: Option<Stamp>,
    pub names: BTreeMap<u64, String>,
    pub last_seq: u64,
    pub outbox: Vec<Edit>,
    #[serde(default)]
    pub seen_since_snapshot: u32,
}

pub trait Store: Send + Sync + 'static {
    fn save(&self, channel: &str, s: &Stored);
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Member {
    pub id: u64,
    pub name: String,
    pub role: Role,
}

#[derive(Clone, Serialize, Debug)]
pub struct Row {
    pub entry: String,
    pub track_id: u64,
    pub added_by: u64,
    pub added_by_name: String,
}

pub struct Pl {
    pub channel: String,
    key: Key,
    me: u64,
    my_name: String,
    st: Mutex<Stored>,
    transport: Arc<dyn Transport>,
    store: Arc<dyn Store>,
    on_change: Arc<dyn Fn(&str) + Send + Sync>,
}

fn author_id(s: &Stamp) -> Option<u64> {
    s.author.parse().ok()
}

pub fn new_key() -> String {
    B64.encode(crypto::random_key())
}

pub fn make_link(channel: &str, key_b64: &str) -> String {
    format!("{LINK_PREFIX}{channel}#{key_b64}")
}

/// `scdesk://playlist/<channel>#<key>` -> (channel, key).
pub fn parse_link(link: &str) -> Option<(String, String)> {
    let t = link.trim();
    let rest = t.strip_prefix(LINK_PREFIX)?;
    let (channel, key) = rest.split_once('#')?;
    let key_ok = B64.decode(key).ok().is_some_and(|k| k.len() == 32);
    (channel.len() >= 16 && channel.len() <= 64 && channel.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && key_ok).then(|| (channel.to_owned(), key.to_owned()))
}

impl Pl {
    pub fn new(channel: String, stored: Stored, me: u64, my_name: String, transport: Arc<dyn Transport>, store: Arc<dyn Store>, on_change: Arc<dyn Fn(&str) + Send + Sync>) -> Arc<Pl> {
        let key: Key = B64.decode(&stored.key).ok().and_then(|k| k.try_into().ok()).unwrap_or([0u8; 32]);
        Arc::new(Pl { channel, key, me, my_name, st: Mutex::new(stored), transport, store, on_change })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Stored> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save_and_tell(&self, s: &Stored) {
        self.store.save(&self.channel, s);
        (self.on_change)(&self.channel);
    }

    fn send(&self, m: &PlMsg, id: String) {
        let blob = crypto::seal(&self.key, &self.channel, &serde_json::to_vec(m).expect("messages serialise"));
        self.transport.send(blob, true, Some(id));
    }

    fn edit_id(e: &Edit) -> String {
        let st = match e {
            Edit::Add { stamp, .. } | Edit::Move { stamp, .. } | Edit::Remove { stamp, .. } | Edit::Restore { stamp, .. } => stamp,
        };
        format!("ob-{}-{}", st.time, st.author)
    }

    // ------------------------------------------------------------- reading

    pub fn settings(&self) -> Option<Settings> {
        self.lock().settings.clone()
    }

    pub fn role(&self) -> Option<Role> {
        self.lock().settings.as_ref().and_then(|s| s.role_of(Some(self.me)))
    }

    pub fn can_edit(&self) -> bool {
        self.lock().settings.as_ref().is_some_and(|s| s.can_edit(Some(self.me)))
    }

    pub fn can_view(&self) -> bool {
        // the link is what got us here
        self.lock().settings.as_ref().is_none_or(|s| s.can_view(Some(self.me), true))
    }

    pub fn title(&self) -> String {
        self.lock().settings.as_ref().map(|s| s.title.clone()).unwrap_or_default()
    }

    pub fn rows(&self) -> Vec<Row> {
        let st = self.lock();
        st.playlist
            .rows()
            .into_iter()
            .map(|(entry, track_id, by)| {
                let id = by.parse().unwrap_or(0);
                Row { entry, track_id, added_by: id, added_by_name: st.names.get(&id).cloned().unwrap_or_default() }
            })
            .collect()
    }

    /// The last `limit` changes: (removed?, who, track id), newest first.
    pub fn history(&self, limit: usize) -> Vec<(bool, String, u64)> {
        let st = self.lock();
        st.playlist
            .history(limit)
            .into_iter()
            .map(|e| {
                let id: u64 = e.author.parse().unwrap_or(0);
                (e.removed, st.names.get(&id).cloned().unwrap_or_default(), e.track_id)
            })
            .collect()
    }

    pub fn members(&self) -> Vec<Member> {
        let st = self.lock();
        let Some(s) = &st.settings else { return Vec::new() };
        s.members.iter().map(|(id, role)| Member { id: *id, name: st.names.get(id).cloned().unwrap_or_default(), role: *role }).collect()
    }

    pub fn stored(&self) -> Stored {
        self.lock().clone()
    }

    // ------------------------------------------------------------- writing

    fn local_edit(&self, make: impl FnOnce(&mut Playlist, Stamp) -> Option<Edit>) -> Result<(), String> {
        let mut st = self.lock();
        if !st.settings.as_ref().is_some_and(|s| s.can_edit(Some(self.me))) {
            return Err("нет права менять этот плейлист".into());
        }
        let stamp = st.playlist.next_stamp(&self.me.to_string());
        let Some(edit) = make(&mut st.playlist, stamp) else { return Ok(()) };
        st.playlist.apply(&edit);
        st.outbox.push(edit.clone());
        st.seen_since_snapshot += 1;
        let snapshot = st.seen_since_snapshot >= SNAPSHOT_EVERY;
        if snapshot {
            st.seen_since_snapshot = 0;
        }
        let s = st.clone();
        drop(st);
        self.send(&PlMsg::Edit { edit: edit.clone() }, Self::edit_id(&edit));
        if snapshot {
            self.send_snapshot();
        }
        self.save_and_tell(&s);
        Ok(())
    }

    pub fn add(&self, track_id: u64) -> Result<(), String> {
        self.local_edit(|p, stamp| {
            let pos = p.pos_for(p.len());
            let entry = format!("{:x}{:x}", stamp.time, rand::random::<u32>());
            Some(Edit::Add { entry, track_id, pos, stamp })
        })
    }

    pub fn remove(&self, entry: &str) -> Result<(), String> {
        self.local_edit(|_, stamp| Some(Edit::Remove { entry: entry.to_owned(), stamp }))
    }

    /// Put the entry at `index` of the visible list (0 = first).
    pub fn move_to(&self, entry: &str, index: usize) -> Result<(), String> {
        self.local_edit(|p, stamp| {
            // the place is computed without the entry itself, or it would be its own neighbour
            let rows: Vec<String> = p.rows().into_iter().map(|r| r.0).filter(|e| e != entry).collect();
            let at = index.min(rows.len());
            let pos_of = |i: usize| p.position_of(&rows[i]);
            let pos = match (at.checked_sub(1).and_then(pos_of), (at < rows.len()).then(|| pos_of(at)).flatten()) {
                (None, None) => 0.0,
                (None, Some(next)) => next - 1.0,
                (Some(prev), None) => prev + 1.0,
                (Some(prev), Some(next)) => (prev + next) / 2.0,
            };
            Some(Edit::Move { entry: entry.to_owned(), pos, stamp })
        })
    }

    fn change_settings(&self, f: impl FnOnce(&mut Settings)) -> Result<(), String> {
        let mut st = self.lock();
        let Some(mut s) = st.settings.clone() else { return Err("плейлист ещё не загружен".into()) };
        if !s.can_manage(Some(self.me)) {
            return Err("менять доступ может только владелец".into());
        }
        f(&mut s);
        let stamp = st.playlist.next_stamp(&self.me.to_string());
        st.settings = Some(s.clone());
        st.settings_stamp = Some(stamp.clone());
        let copy = st.clone();
        drop(st);
        self.send(&PlMsg::Settings { settings: s, stamp: stamp.clone() }, format!("st-{}-{}", stamp.time, stamp.author));
        self.save_and_tell(&copy);
        Ok(())
    }

    pub fn set_access(&self, access: Access) -> Result<(), String> {
        self.change_settings(|s| s.access = access)
    }

    pub fn set_title(&self, title: String) -> Result<(), String> {
        self.change_settings(|s| s.title = title)
    }

    pub fn invite(&self, id: u64, name: String, role: Role) -> Result<(), String> {
        let mut names = BTreeMap::new();
        names.insert(id, name.clone());
        self.change_settings(|s| {
            s.members.insert(id, role);
        })?;
        self.lock().names.insert(id, name.clone());
        self.send(&PlMsg::Name { id, name }, format!("nm-{id}-{}", self.me));
        let _ = names;
        Ok(())
    }

    pub fn remove_member(&self, id: u64) -> Result<(), String> {
        self.change_settings(|s| {
            if s.members.get(&id) != Some(&Role::Owner) {
                s.members.remove(&id);
            }
        })
    }

    /// The first settings of a playlist we found.
    pub fn found(&self, title: String, access: Access) {
        let mut st = self.lock();
        let mut s = Settings { title, access, members: BTreeMap::new() };
        s.members.insert(self.me, Role::Owner);
        let stamp = st.playlist.next_stamp(&self.me.to_string());
        st.settings = Some(s.clone());
        st.settings_stamp = Some(stamp.clone());
        st.names.insert(self.me, self.my_name.clone());
        let copy = st.clone();
        drop(st);
        self.send(&PlMsg::Settings { settings: s, stamp: stamp.clone() }, format!("st-{}-{}", stamp.time, stamp.author));
        self.send(&PlMsg::Name { id: self.me, name: self.my_name.clone() }, format!("nm-{}-{}", self.me, self.me));
        self.save_and_tell(&copy);
    }

    fn send_snapshot(&self) {
        let st = self.lock();
        let (Some(settings), Some(stamp)) = (st.settings.clone(), st.settings_stamp.clone()) else { return };
        let m = PlMsg::Snapshot { playlist: st.playlist.clone(), settings, stamp: stamp.clone() };
        drop(st);
        self.send(&m, format!("sn-{}-{}", stamp.time, rand::random::<u16>()));
    }

    // ------------------------------------------------------------ incoming

    /// Takes the new settings if they come from the owner (or found the playlist).
    fn take_settings(st: &mut Stored, settings: Settings, stamp: Stamp) {
        let author = author_id(&stamp);
        let ok = match &st.settings {
            None => true, // the first one founds the playlist (and the owner)
            Some(cur) => author.is_some_and(|a| cur.can_manage(Some(a))),
        };
        if ok && st.settings_stamp.as_ref().is_none_or(|old| stamp > *old) {
            st.settings = Some(settings);
            st.settings_stamp = Some(stamp);
        }
    }

    fn handle(&self, seq: u64, keep: bool, id: Option<String>, m: PlMsg) {
        let mut st = self.lock();
        if keep && seq > st.last_seq {
            st.last_seq = seq;
        }
        // our own message coming back: it reached the relay, leave the outbox
        if let Some(i) = id.as_deref() {
            st.outbox.retain(|e| Self::edit_id(e) != i);
        }
        match m {
            PlMsg::Edit { edit } => {
                let stamp = match &edit {
                    Edit::Add { stamp, .. } | Edit::Move { stamp, .. } | Edit::Remove { stamp, .. } | Edit::Restore { stamp, .. } => stamp.clone(),
                };
                let allowed = st.settings.as_ref().is_some_and(|s| s.can_edit(author_id(&stamp)));
                if allowed {
                    st.playlist.apply(&edit);
                    st.seen_since_snapshot += 1;
                }
            }
            PlMsg::Settings { settings, stamp } => Self::take_settings(&mut st, settings, stamp),
            PlMsg::Snapshot { playlist, settings, stamp } => {
                Self::take_settings(&mut st, settings, stamp);
                // a snapshot is taken in if its sender could edit; the entries are merged one by one
                st.playlist.merge(&playlist);
            }
            PlMsg::Name { id, name } => {
                st.names.insert(id, name.chars().take(60).collect());
            }
        }
        let s = st.clone();
        drop(st);
        self.save_and_tell(&s);
    }

    /// What to do when the relay says hello: send what waited in the outbox, tell who we are.
    fn on_connected(&self) {
        let (outbox, known_me) = {
            let st = self.lock();
            (st.outbox.clone(), st.names.get(&self.me).is_some_and(|n| *n == self.my_name))
        };
        for e in outbox {
            self.send(&PlMsg::Edit { edit: e.clone() }, Self::edit_id(&e));
        }
        if !known_me && !self.my_name.is_empty() {
            self.lock().names.insert(self.me, self.my_name.clone());
            self.send(&PlMsg::Name { id: self.me, name: self.my_name.clone() }, format!("nm-{}-{}", self.me, self.me));
        }
    }

    pub fn last_seq(&self) -> u64 {
        self.lock().last_seq
    }

    /// Runs until the event stream ends.
    pub fn spawn(self: &Arc<Self>, mut events: mpsc::UnboundedReceiver<HubEvent>) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(ev) = events.recv().await {
                match ev {
                    HubEvent::Connected { .. } => me.on_connected(),
                    HubEvent::Msg { seq, blob, keep, id } => {
                        let Some(plain) = crypto::open(&me.key, &me.channel, &blob) else { continue };
                        let Ok(m) = serde_json::from_slice::<PlMsg>(&plain) else { continue };
                        me.handle(seq, keep, id, m);
                    }
                    HubEvent::Disconnected => {}
                }
            }
        });
    }
}

pub fn empty_stored(key_b64: String, mine: bool) -> Stored {
    Stored { key: key_b64, mine, playlist: Playlist::default(), settings: None, settings_stamp: None, names: BTreeMap::new(), last_seq: 0, outbox: Vec::new(), seen_since_snapshot: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A relay in memory that keeps what is marked `keep` and replays it from a sequence number.
    #[derive(Default)]
    struct Relay {
        log: Mutex<Vec<(u64, String)>>,
        peers: Mutex<Vec<mpsc::UnboundedSender<HubEvent>>>,
    }
    struct Line(Arc<Relay>);
    impl Transport for Line {
        fn send(&self, blob: String, keep: bool, id: Option<String>) {
            let mut seq = 0;
            if keep {
                let mut log = self.0.log.lock().unwrap();
                seq = log.len() as u64 + 1;
                log.push((seq, blob.clone()));
            }
            for p in self.0.peers.lock().unwrap().iter() {
                let _ = p.send(HubEvent::Msg { seq, blob: blob.clone(), keep, id: id.clone() });
            }
        }
    }
    impl Relay {
        fn connect(self: &Arc<Self>, since: u64) -> (Arc<dyn Transport>, mpsc::UnboundedReceiver<HubEvent>, mpsc::UnboundedSender<HubEvent>) {
            let (tx, rx) = mpsc::unbounded_channel();
            let _ = tx.send(HubEvent::Connected { head: 0, peers: 0 });
            for (seq, blob) in self.log.lock().unwrap().iter().filter(|(s, _)| *s > since) {
                let _ = tx.send(HubEvent::Msg { seq: *seq, blob: blob.clone(), keep: true, id: None });
            }
            self.peers.lock().unwrap().push(tx.clone());
            (Arc::new(Line(self.clone())), rx, tx)
        }
    }

    #[derive(Default)]
    struct Mem(Mutex<BTreeMap<String, Stored>>);
    impl Store for Mem {
        fn save(&self, channel: &str, s: &Stored) {
            self.0.lock().unwrap().insert(channel.into(), s.clone());
        }
    }

    fn pl(relay: &Arc<Relay>, channel: &str, key: &str, me: u64, name: &str, mine: bool) -> Arc<Pl> {
        let (tr, rx, _tx) = relay.connect(0);
        let p = Pl::new(channel.into(), empty_stored(key.into(), mine), me, name.into(), tr, Arc::new(Mem::default()), Arc::new(|_| {}));
        p.spawn(rx);
        p
    }

    async fn settle() {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    }

    fn tracks(p: &Pl) -> Vec<u64> {
        p.rows().into_iter().map(|r| r.track_id).collect()
    }

    #[test]
    fn links_carry_the_channel_and_the_key() {
        let key = new_key();
        let link = make_link("abcdefghijklmnopqrstuvwx", &key);
        assert_eq!(parse_link(&link), Some(("abcdefghijklmnopqrstuvwx".into(), key)));
        assert!(parse_link("scdesk://playlist/short#x").is_none());
        assert!(parse_link("https://example.com").is_none());
    }

    #[test]
    fn friends_edit_one_playlist_and_see_each_other_and_who_added_what() {
        tauri::async_runtime::block_on(async {
            let relay = Arc::new(Relay::default());
            let key = new_key();
            let anna = pl(&relay, "chan-a-playlist-01", &key, 1, "anna", true);
            anna.found("Дорога домой".into(), Access::Link);
            anna.invite(2, "bob".into(), Role::Editor).unwrap();
            settle().await;
            let bob = pl(&relay, "chan-a-playlist-01", &key, 2, "bob", false);
            settle().await;
            assert_eq!(bob.title(), "Дорога домой");
            assert_eq!(bob.role(), Some(Role::Editor));

            anna.add(10).unwrap();
            anna.add(20).unwrap();
            settle().await;
            bob.add(30).unwrap();
            settle().await;
            assert_eq!(tracks(&anna), vec![10, 20, 30]);
            assert_eq!(tracks(&anna), tracks(&bob));
            let by: Vec<(u64, String)> = bob.rows().into_iter().map(|r| (r.track_id, r.added_by_name)).collect();
            assert_eq!(by, vec![(10, "anna".into()), (20, "anna".into()), (30, "bob".into())]);

            bob.move_to(&bob.rows()[2].entry.clone(), 0).unwrap();
            settle().await;
            anna.remove(&anna.rows()[2].entry.clone()).unwrap();
            settle().await;
            assert_eq!(tracks(&anna), vec![30, 10]);
            assert_eq!(tracks(&bob), vec![30, 10]);
        });
    }

    #[test]
    fn a_viewer_cannot_edit_and_a_forged_edit_is_ignored() {
        tauri::async_runtime::block_on(async {
            let relay = Arc::new(Relay::default());
            let key = new_key();
            let anna = pl(&relay, "chan-b-playlist-02", &key, 1, "anna", true);
            anna.found("t".into(), Access::Link);
            anna.invite(3, "carol".into(), Role::Viewer).unwrap();
            settle().await;
            let carol = pl(&relay, "chan-b-playlist-02", &key, 3, "carol", false);
            settle().await;
            assert!(carol.add(1).is_err(), "a viewer has no edit button");
            // somebody with the link writes an Add in Carol's name: the copies refuse it
            let forged = PlMsg::Edit { edit: Edit::Add { entry: "x".into(), track_id: 66, pos: 0.0, stamp: Stamp { time: 99, author: "3".into() } } };
            let blob = crypto::seal(&carol.key, "chan-b-playlist-02", &serde_json::to_vec(&forged).unwrap());
            Line(relay.clone()).send(blob, true, None);
            settle().await;
            assert!(tracks(&anna).is_empty() && tracks(&carol).is_empty());
            // and only the owner changes the access
            assert!(carol.set_access(Access::Open).is_err());
            anna.set_access(Access::Open).unwrap();
            settle().await;
            assert!(!carol.can_edit(), "a viewer named by the owner stays a viewer");
            let dave = pl(&relay, "chan-b-playlist-02", &key, 4, "dave", false);
            settle().await;
            assert!(dave.can_edit(), "in an open playlist anybody else signed in may add");
            dave.add(5).unwrap();
            settle().await;
            assert_eq!(tracks(&anna), vec![5]);
        });
    }

    #[test]
    fn somebody_who_was_away_catches_up_and_sends_what_he_wrote_offline() {
        tauri::async_runtime::block_on(async {
            let relay = Arc::new(Relay::default());
            let key = new_key();
            let anna = pl(&relay, "chan-c-playlist-03", &key, 1, "anna", true);
            anna.found("t".into(), Access::Link);
            anna.invite(2, "bob".into(), Role::Editor).unwrap();
            anna.add(1).unwrap();
            settle().await;
            // bob is offline: his events are not delivered, his sends go nowhere
            let offline = Arc::new(Relay::default());
            let (tr, rx, _tx) = offline.connect(0);
            let store = Arc::new(Mem::default());
            let first_seen = {
                let (tr1, rx1, _t) = relay.connect(0);
                let p = Pl::new("chan-c-playlist-03".into(), empty_stored(key.clone(), false), 2, "bob".into(), tr1, store.clone(), Arc::new(|_| {}));
                p.spawn(rx1);
                settle().await;
                p.stored()
            };
            drop((tr, rx));
            assert_eq!(first_seen.playlist.len(), 1);
            // anna keeps working while bob is away
            anna.add(2).unwrap();
            anna.add(3).unwrap();
            settle().await;
            // bob comes back from his saved state with an edit he made offline sitting in the outbox
            let mut saved = first_seen;
            let (dead, _r, _t) = offline.connect(0);
            let bob_off = Pl::new("chan-c-playlist-03".into(), saved.clone(), 2, "bob".into(), dead, store.clone(), Arc::new(|_| {}));
            bob_off.add(9).unwrap(); // goes to the dead line: stays in the outbox
            saved = bob_off.stored();
            assert_eq!(saved.outbox.len(), 1);
            let (tr2, rx2, _t2) = relay.connect(saved.last_seq);
            let bob = Pl::new("chan-c-playlist-03".into(), saved, 2, "bob".into(), tr2, store, Arc::new(|_| {}));
            bob.spawn(rx2);
            settle().await;
            settle().await;
            let mut all = tracks(&bob);
            all.sort();
            assert_eq!(all, vec![1, 2, 3, 9], "caught up and the offline edit is in");
            assert_eq!(tracks(&anna), tracks(&bob), "and anna got it, in the same order");
            assert!(bob.stored().outbox.is_empty(), "the outbox empties when the echo comes");
        });
    }

    #[test]
    fn a_newcomer_starts_from_a_snapshot_when_the_relay_forgot_the_early_edits() {
        tauri::async_runtime::block_on(async {
            let relay = Arc::new(Relay::default());
            let key = new_key();
            let anna = pl(&relay, "chan-d-playlist-04", &key, 1, "anna", true);
            anna.found("t".into(), Access::Open);
            for t in 1..=130u64 {
                anna.add(t).unwrap();
            }
            settle().await;
            // the relay keeps only the tail: everything before the snapshot is gone
            let snap_at = {
                let log = relay.log.lock().unwrap();
                let blobs: Vec<usize> = log
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, b))| matches!(crypto::open(&anna.key, "chan-d-playlist-04", b).and_then(|p| serde_json::from_slice::<PlMsg>(&p).ok()), Some(PlMsg::Snapshot { .. })))
                    .map(|(i, _)| i)
                    .collect();
                *blobs.first().expect("a snapshot was posted after 100 edits")
            };
            relay.log.lock().unwrap().drain(..snap_at);
            let dave = pl(&relay, "chan-d-playlist-04", &key, 4, "dave", false);
            settle().await;
            settle().await;
            assert_eq!(tracks(&dave).len(), 130, "everything is there although the early edits were lost");
        });
    }
}
