//! Friends of the app (not SoundCloud followers): requests, a list, invitations that arrive.
//!
//! * Everybody has an inbox on the relay (`fi-<hash of the SoundCloud id>`), a channel like any other
//!   that keeps what was sent while they were away. A request, its answer and every later message go
//!   there as ciphertext; the relay sees only that somebody wrote to that inbox.
//! * Each install has a key pair (`crypto::Identity`, kept between runs). A request carries the sender's
//!   public key, the answer carries ours: from then on the two share a key (`pair_key`) and everything
//!   between them (an invitation, "I removed you") is sealed with it, so a third party can neither read
//!   nor forge it. The first exchange is "trust on first use": whoever answers first with a key is the
//!   friend, as with any app without accounts of its own.
//! * A friend whose key changes (they reinstalled) asks again and waits for a yes; their old key keeps
//!   working until then.
//! * `Book` is the plain logic (no network), tested on its own; `Friends` ties it to the relay.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use super::{
    crypto::{self, Identity},
    hub,
};
use crate::{
    error::{AppError, AppResult},
    lang::pick,
    state::AppState,
};

const BOOK: &str = "friends:book";
const KEY: &str = "friends:key";
const SINCE: &str = "friends:since";
const PKEY: &str = "friends:pkey";
const OWNER: &str = "friends:owner";
/// what we listen to is shown for this long without a refresh
const NOW_FRESH_MS: i64 = 4 * 60 * 1000;
const MAX_INVITES: usize = 50;
const MAX_PEOPLE: usize = 500;

pub fn inbox_channel(user_id: u64) -> String {
    format!("fi-{}", hex::encode(&Sha256::digest(format!("scdesk-inbox|{user_id}").as_bytes())[..16]))
}

/// The inbox key is derived from the public channel name: it only keeps the relay's log unreadable at a
/// glance, the real secrecy is in the pair keys.
fn inbox_key(channel: &str) -> crypto::Key {
    crypto::room_key(channel, "")
}

/// Where "listening now" of a person sits in the relay's directory (readable only with their presence key).
pub fn now_hash(user_id: u64) -> String {
    hex::encode(&Sha256::digest(format!("scdesk-now|{user_id}").as_bytes())[..16])
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct NowPlaying {
    track_id: u64,
    title: String,
    artist: String,
    #[serde(default)]
    artwork_url: Option<String>,
    /// ms since epoch when it was published
    at: i64,
}

/// A friend who is listening to something right now.
#[derive(Serialize, Clone, Debug)]
pub struct Listening {
    pub id: u64,
    pub track_id: u64,
    pub title: String,
    pub artist: String,
    pub artwork_url: Option<String>,
}

fn direct_aad(from: u64, to: u64) -> String {
    format!("friends|{from}>{to}")
}

// ------------------------------------------------------------------- data

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Friend,
    /// we asked, no answer yet
    Outgoing,
    /// they asked, we have not answered
    Incoming,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Friend {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub pubkey: String,
    pub state: State,
    /// when the request was made / the friendship began, ms
    #[serde(default)]
    pub since: i64,
    /// a friend asks again with another key (reinstalled?): waits for a yes
    #[serde(default)]
    pub new_key: Option<String>,
    /// the key this friend reads our "listening now" with is theirs: the one we read theirs with
    #[serde(default)]
    pub pkey: String,
    /// our own key has been handed to this friend (again after every change of it)
    #[serde(default)]
    pub key_sent: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Invite {
    pub id: String,
    pub from: u64,
    pub from_name: String,
    /// "room" | "playlist"
    pub kind: String,
    pub link: String,
    pub title: String,
    #[serde(default)]
    pub password: String,
    pub at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Wire {
    Req {
        from: u64,
        name: String,
        #[serde(default)]
        avatar: Option<String>,
        pubkey: String,
    },
    Accept {
        from: u64,
        name: String,
        #[serde(default)]
        avatar: Option<String>,
        pubkey: String,
    },
    /// sealed with the pair key
    Direct { from: u64, blob: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Direct {
    Remove,
    Invite {
        kind: String,
        link: String,
        title: String,
        #[serde(default)]
        password: String,
        name: String,
        at: i64,
    },
    /// the key to read what we are listening to (sent again when it changes)
    PresenceKey { key: String },
}

#[derive(Debug, PartialEq)]
pub enum Effect {
    /// answer a request: we are friends
    Accept(u64),
    Notify { title: String, body: String },
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Book {
    pub people: Vec<Friend>,
    pub invites: Vec<Invite>,
}

impl Book {
    fn find(&self, id: u64) -> Option<&Friend> {
        self.people.iter().find(|p| p.id == id)
    }

    fn find_mut(&mut self, id: u64) -> Option<&mut Friend> {
        self.people.iter_mut().find(|p| p.id == id)
    }

    pub fn friend_ids(&self) -> Vec<u64> {
        self.people.iter().filter(|p| p.state == State::Friend).map(|p| p.id).collect()
    }

    /// What arrived in our inbox. Returns whether the book changed, and what to do about it.
    pub fn handle(&mut self, ident: &Identity, me: u64, wire: Wire, now: i64) -> (bool, Vec<Effect>) {
        match wire {
            Wire::Req { from, name, avatar, pubkey } => {
                if from == me || ident.pair_key(&pubkey).is_none() {
                    return (false, vec![]);
                }
                let title = name.clone();
                let notify = |text: &str| Effect::Notify { title: title.clone(), body: text.to_owned() };
                match self.find_mut(from).map(|f| (f.state, f.pubkey == pubkey)) {
                    None => {
                        if self.people.len() >= MAX_PEOPLE {
                            return (false, vec![]);
                        }
                        self.people.push(Friend { id: from, name: name.clone(), avatar_url: avatar, pubkey, state: State::Incoming, since: now, new_key: None, pkey: String::new(), key_sent: false });
                        (true, vec![notify(pick("хочет дружить с вами", "wants to be friends"))])
                    }
                    Some((State::Incoming, _)) => {
                        let f = self.find_mut(from).expect("found above");
                        let changed = f.pubkey != pubkey || f.name != name;
                        f.name = name;
                        f.avatar_url = avatar;
                        f.pubkey = pubkey;
                        (changed, vec![])
                    }
                    Some((State::Outgoing, _)) => {
                        // both asked: that is a yes
                        let f = self.find_mut(from).expect("found above");
                        f.state = State::Friend;
                        f.pubkey = pubkey;
                        f.since = now;
                        f.name = name;
                        f.avatar_url = avatar;
                        (true, vec![Effect::Accept(from), notify(pick("теперь ваш друг", "is now your friend"))])
                    }
                    Some((State::Friend, true)) => (false, vec![Effect::Accept(from)]),
                    Some((State::Friend, false)) => {
                        let f = self.find_mut(from).expect("found above");
                        let changed = f.new_key.as_deref() != Some(pubkey.as_str());
                        f.new_key = Some(pubkey);
                        (changed, if changed { vec![notify(pick("просит добавить заново (новый ключ)", "asks to be added again (new key)"))] } else { vec![] })
                    }
                }
            }
            Wire::Accept { from, name, avatar, pubkey } => {
                if from == me || ident.pair_key(&pubkey).is_none() {
                    return (false, vec![]);
                }
                match self.find_mut(from) {
                    Some(f) if f.state == State::Outgoing => {
                        f.state = State::Friend;
                        f.pubkey = pubkey;
                        f.since = now;
                        f.name = name.clone();
                        f.avatar_url = avatar;
                        (true, vec![Effect::Notify { title: name, body: pick("принял(а) ваш запрос в друзья", "accepted your friend request").into() }])
                    }
                    _ => (false, vec![]),
                }
            }
            Wire::Direct { from, blob } => {
                let Some(f) = self.find(from).filter(|f| f.state == State::Friend) else { return (false, vec![]) };
                let Some(key) = ident.pair_key(&f.pubkey) else { return (false, vec![]) };
                let Some(plain) = crypto::open(&key, &direct_aad(from, me), &blob) else { return (false, vec![]) };
                let Ok(d) = serde_json::from_slice::<Direct>(&plain) else { return (false, vec![]) };
                let name = f.name.clone();
                match d {
                    Direct::PresenceKey { key } => {
                        if crypto::key_from_b64(&key).is_none() {
                            return (false, vec![]);
                        }
                        if let Some(f) = self.find_mut(from) {
                            f.pkey = key;
                        }
                        (true, vec![])
                    }
                    Direct::Remove => {
                        self.people.retain(|p| p.id != from);
                        self.invites.retain(|i| i.from != from);
                        (true, vec![])
                    }
                    Direct::Invite { kind, link, title, password, name: claimed, at } => {
                        if kind != "room" && kind != "playlist" {
                            return (false, vec![]);
                        }
                        let _ = claimed;
                        if self.invites.iter().any(|i| i.from == from && i.link == link && i.at == at) {
                            return (false, vec![]);
                        }
                        let id = format!("{from}-{at}-{}", self.invites.len());
                        let what = if kind == "room" { pick("зовёт в комнату", "invites you to the room") } else { pick("зовёт в общий плейлист", "invites you to the shared playlist") };
                        self.invites.push(Invite { id, from, from_name: name.clone(), kind, link, title: title.clone(), password, at });
                        if self.invites.len() > MAX_INVITES {
                            self.invites.remove(0);
                        }
                        (true, vec![Effect::Notify { title: name, body: format!("{what} «{title}»") }])
                    }
                }
            }
        }
    }

    /// Our request to somebody. `Ok(true)`: the request has to be sent; `Ok(false)`: nothing to send.
    pub fn request(&mut self, id: u64, name: String, avatar: Option<String>, me: u64, now: i64) -> Result<bool, String> {
        if id == me {
            return Err(pick("Это вы", "That is you").into());
        }
        if self.find(id).is_some_and(|f| f.state == State::Incoming) {
            return Err(pick("Этот человек уже отправил вам запрос: примите его", "This person already asked you: accept the request").into());
        }
        match self.find(id).map(|f| f.state) {
            Some(State::Friend) => Err(pick("Вы уже друзья", "You are already friends").into()),
            Some(State::Outgoing) => Ok(true), // sent again (it may not have arrived)
            _ => {
                if self.people.len() >= MAX_PEOPLE {
                    return Err(pick("Слишком много друзей и запросов", "Too many friends and requests").into());
                }
                self.people.push(Friend { id, name, avatar_url: avatar, pubkey: String::new(), state: State::Outgoing, since: now, new_key: None, pkey: String::new(), key_sent: false });
                Ok(true)
            }
        }
    }

    /// We say yes to a request (or to a friend's new key). `true`: an answer has to be sent.
    pub fn accept(&mut self, id: u64, now: i64) -> bool {
        let Some(f) = self.find_mut(id) else { return false };
        match (f.state, f.new_key.take()) {
            (State::Incoming, _) => {
                f.state = State::Friend;
                f.since = now;
                true
            }
            (State::Friend, Some(key)) => {
                f.pubkey = key;
                f.pkey.clear();
                f.key_sent = false;
                true
            }
            _ => false,
        }
    }

    /// No to a request, or to a friend's new key. Nothing is sent: they just stay unanswered.
    pub fn decline(&mut self, id: u64) -> bool {
        match self.find(id).map(|f| (f.state, f.new_key.is_some())) {
            Some((State::Incoming, _)) => {
                self.people.retain(|p| p.id != id);
                true
            }
            Some((State::Friend, true)) => {
                if let Some(f) = self.find_mut(id) {
                    f.new_key = None;
                }
                true
            }
            _ => false,
        }
    }

    /// Take back our request.
    pub fn cancel(&mut self, id: u64) -> bool {
        let before = self.people.len();
        self.people.retain(|p| !(p.id == id && p.state == State::Outgoing));
        self.people.len() != before
    }

    /// Remove a friend. `true`: they have to be told.
    pub fn remove(&mut self, id: u64) -> bool {
        let was = self.find(id).is_some_and(|f| f.state == State::Friend);
        self.people.retain(|p| p.id != id);
        self.invites.retain(|i| i.from != id);
        was
    }

    pub fn dismiss(&mut self, invite_id: &str) -> bool {
        let before = self.invites.len();
        self.invites.retain(|i| i.id != invite_id);
        self.invites.len() != before
    }
}

// ------------------------------------------------------------------ service

#[derive(Serialize, Clone, Debug)]
pub struct FriendDto {
    pub id: u64,
    pub name: String,
    pub avatar_url: Option<String>,
    pub since: i64,
    /// a friend who asks again with a new key (shown among the requests)
    pub rekey: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct FriendsView {
    pub friends: Vec<FriendDto>,
    pub incoming: Vec<FriendDto>,
    pub outgoing: Vec<FriendDto>,
    pub invites: Vec<Invite>,
}

impl Book {
    pub fn view(&self) -> FriendsView {
        let dto = |p: &Friend| FriendDto { id: p.id, name: p.name.clone(), avatar_url: p.avatar_url.clone(), since: p.since, rekey: p.new_key.is_some() };
        let mut v = FriendsView { invites: self.invites.iter().rev().cloned().collect(), ..Default::default() };
        for p in &self.people {
            match p.state {
                State::Friend => {
                    v.friends.push(dto(p));
                    if p.new_key.is_some() {
                        v.incoming.push(dto(p));
                    }
                }
                State::Incoming => v.incoming.push(dto(p)),
                State::Outgoing => v.outgoing.push(dto(p)),
            }
        }
        v.friends.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        v
    }
}

#[derive(Default)]
struct Inner {
    book: Book,
    ident: Option<Arc<Identity>>,
    hub: Option<hub::HubClient>,
    since: u64,
    me: u64,
    my_name: String,
    my_avatar: Option<String>,
    /// the key friends read what we are listening to with
    my_pkey: String,
    /// what was last published (track id, playing) and when
    last_pub: Option<(u64, bool, std::time::Instant)>,
}

#[derive(Default)]
pub struct Friends {
    inner: Mutex<Inner>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Friends {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn view(&self) -> FriendsView {
        self.lock().book.view()
    }

    pub fn ids(&self) -> Vec<u64> {
        self.lock().book.friend_ids()
    }

    fn save(&self, app: &AppHandle) {
        let (book, since, pkey) = {
            let g = self.lock();
            (g.book.clone(), g.since, g.my_pkey.clone())
        };
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            let db = &app2.state::<AppState>().db;
            let _ = db.kv_put(BOOK, &book).await;
            let _ = db.kv_put(SINCE, &since).await;
            if !pkey.is_empty() {
                let _ = db.kv_put(PKEY, &pkey).await;
            }
        });
        let _ = app.emit("friends:changed", ());
    }

    /// At start (after the sign-in is known): load the list, open our inbox.
    pub async fn start(&self, app: &AppHandle) {
        let state = app.state::<AppState>();
        let Some(me) = state.me() else { return };
        let Ok(base) = super::service::server_url(&state) else { return };
        // the list, the keys and the cursor belong to one account: another one starts from nothing
        let owner = state.db.kv_get::<u64>(OWNER).await.ok().flatten().map(|(o, _)| o);
        // (no owner recorded yet = a list made before this was kept: it is this account's)
        if owner.is_some_and(|o| o != me.id) {
            tracing::info!("another account signed in: the friend list starts empty");
            for key in [KEY, PKEY, BOOK, SINCE] {
                let _ = state.db.kv_delete(key).await;
            }
        }
        if owner != Some(me.id) {
            let _ = state.db.kv_put(OWNER, &me.id).await;
        }
        let ident = match state.db.kv_get::<String>(KEY).await.ok().flatten().and_then(|(k, _)| Identity::from_secret_b64(&k)) {
            Some(i) => i,
            None => {
                let i = Identity::new();
                let _ = state.db.kv_put(KEY, &i.secret_b64()).await;
                i
            }
        };
        let book = state.db.kv_get::<Book>(BOOK).await.ok().flatten().map(|(b, _)| b).unwrap_or_default();
        let since = state.db.kv_get::<u64>(SINCE).await.ok().flatten().map(|(s, _)| s).unwrap_or(0);
        let pkey = match state.db.kv_get::<String>(PKEY).await.ok().flatten().map(|(k, _)| k).filter(|k| crypto::key_from_b64(k).is_some()) {
            Some(k) => k,
            None => crypto::key_to_b64(&crypto::random_key()),
        };
        let channel = inbox_channel(me.id);
        let (client, events) = hub::connect(base, channel.clone(), since);
        {
            let mut g = self.lock();
            if let Some(old) = g.hub.take() {
                old.close();
            }
            g.book = book;
            g.ident = Some(Arc::new(ident));
            g.hub = Some(client);
            g.since = since;
            g.me = me.id;
            g.my_name = me.username.clone();
            g.my_avatar = me.avatar_url.clone();
            g.my_pkey = pkey;
        }
        self.save(app);
        // "listening now": our key to the friends, our line to the relay, every few seconds
        static LOOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !LOOP.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let app4 = app.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    app4.state::<Friends>().presence_tick(&app4).await;
                }
            });
        }
        let _ = app.emit("friends:changed", ());
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            pump(app2, channel, events).await;
        });
    }

    fn identity(&self) -> AppResult<(Arc<Identity>, u64, String, Option<String>)> {
        let g = self.lock();
        let ident = g.ident.clone().ok_or_else(|| AppError::Other(pick("Друзья недоступны: войдите в SoundCloud и проверьте сеть", "Friends are unavailable: sign in to SoundCloud and check the network").into()))?;
        Ok((ident, g.me, g.my_name.clone(), g.my_avatar.clone()))
    }

    fn my_wire(&self, accept: bool) -> AppResult<Wire> {
        let (ident, me, name, avatar) = self.identity()?;
        let pubkey = ident.public_b64();
        Ok(if accept { Wire::Accept { from: me, name, avatar, pubkey } } else { Wire::Req { from: me, name, avatar, pubkey } })
    }

    pub async fn request(&self, app: &AppHandle, id: u64, name: String, avatar: Option<String>) -> AppResult<()> {
        let (_, me, _, _) = self.identity()?;
        let send = self.lock().book.request(id, name, avatar, me, now_ms()).map_err(AppError::Other)?;
        if send {
            let wire = self.my_wire(false)?;
            if let Err(e) = deliver(app, id, &wire).await {
                // nothing left behind: a request that never left is not "sent"
                let mut g = self.lock();
                if g.book.find(id).is_some_and(|f| f.state == State::Outgoing && f.pubkey.is_empty()) {
                    g.book.cancel(id);
                }
                drop(g);
                self.save(app);
                return Err(e);
            }
        }
        self.save(app);
        Ok(())
    }

    pub async fn accept(&self, app: &AppHandle, id: u64) -> AppResult<()> {
        let send = self.lock().book.accept(id, now_ms());
        self.save(app);
        if send {
            deliver(app, id, &self.my_wire(true)?).await?;
        }
        Ok(())
    }

    pub fn decline(&self, app: &AppHandle, id: u64) {
        if self.lock().book.decline(id) {
            self.save(app);
        }
    }

    pub fn cancel(&self, app: &AppHandle, id: u64) {
        if self.lock().book.cancel(id) {
            self.save(app);
        }
    }

    pub async fn remove(&self, app: &AppHandle, id: u64) -> AppResult<()> {
        // the key is taken before the record goes: the message to them is sealed with it
        let key = self.lock().book.find(id).filter(|f| f.state == State::Friend).map(|f| f.pubkey.clone());
        {
            let mut g = self.lock();
            g.book.remove(id);
            // the one removed must not keep reading what we listen to: a new key for everybody who stays
            g.my_pkey = crypto::key_to_b64(&crypto::random_key());
            for p in g.book.people.iter_mut() {
                p.key_sent = false;
            }
            g.last_pub = None;
        }
        self.save(app);
        if let Some(pubkey) = key {
            // best effort: they learn it the next time they are online
            let _ = self.send_direct(app, id, &pubkey, Direct::Remove).await;
        }
        Ok(())
    }

    pub fn dismiss(&self, app: &AppHandle, invite_id: &str) {
        if self.lock().book.dismiss(invite_id) {
            self.save(app);
        }
    }

    /// An invitation for a friend: a room or a shared playlist.
    pub async fn invite(&self, app: &AppHandle, id: u64, kind: &str, link: String, title: String, password: String) -> AppResult<()> {
        let (_, _, name, _) = self.identity()?;
        self.send_direct_to(app, id, Direct::Invite { kind: kind.to_owned(), link, title, password, name, at: now_ms() }).await
    }

    async fn send_direct_to(&self, app: &AppHandle, to: u64, d: Direct) -> AppResult<()> {
        self.identity()?;
        let pubkey = {
            let g = self.lock();
            g.book.find(to).filter(|f| f.state == State::Friend).map(|f| f.pubkey.clone())
        };
        let Some(pubkey) = pubkey else {
            return Err(AppError::Other(pick("Этого человека нет в друзьях", "This person is not your friend").into()));
        };
        self.send_direct(app, to, &pubkey, d).await
    }

    async fn send_direct(&self, app: &AppHandle, to: u64, pubkey: &str, d: Direct) -> AppResult<()> {
        let (ident, me, _, _) = self.identity()?;
        let key = ident.pair_key(pubkey).ok_or_else(|| AppError::Other("bad key".into()))?;
        let blob = crypto::seal(&key, &direct_aad(me, to), &serde_json::to_vec(&d)?);
        deliver(app, to, &Wire::Direct { from: me, blob }).await
    }

    /// From the pump: one message of our inbox.
    fn incoming(&self, app: &AppHandle, wire: Wire) {
        let (ident, me) = {
            let g = self.lock();
            match &g.ident {
                Some(i) => (i.clone(), g.me),
                None => return,
            }
        };
        let from = match &wire {
            Wire::Req { from, .. } | Wire::Accept { from, .. } | Wire::Direct { from, .. } => *from,
        };
        let (changed, effects) = self.lock().book.handle(&ident, me, wire, now_ms());
        if changed {
            self.save(app);
            self.refresh_profile(app, from);
        }
        for e in effects {
            match e {
                Effect::Accept(to) => {
                    let (app, wire) = (app.clone(), self.my_wire(true));
                    tauri::async_runtime::spawn(async move {
                        if let Ok(w) = wire {
                            let _ = deliver(&app, to, &w).await;
                        }
                    });
                }
                Effect::Notify { title, body } => {
                    let _ = app.notification().builder().title(title).body(body).show();
                }
            }
        }
    }

    /// What the message claims about a name is not trusted for display: the SoundCloud profile is.
    fn refresh_profile(&self, app: &AppHandle, id: u64) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let Ok(sc) = state.sc() else { return };
            let Ok(u) = sc.user(id).await else { return };
            let friends = app.state::<Friends>();
            let changed = {
                let mut g = friends.lock();
                match g.book.find_mut(id) {
                    Some(f) if f.name != u.username || f.avatar_url != u.avatar_url => {
                        f.name = u.username.clone();
                        f.avatar_url = u.avatar_url.clone();
                        true
                    }
                    _ => false,
                }
            };
            if changed {
                friends.save(&app);
            }
        });
    }

    /// Every few seconds: hand our key to friends who do not have it, publish what we listen to.
    async fn presence_tick(&self, app: &AppHandle) {
        let todo: Vec<(u64, String)> = {
            let g = self.lock();
            g.book.people.iter().filter(|p| p.state == State::Friend && !p.key_sent && !p.pubkey.is_empty()).map(|p| (p.id, p.pubkey.clone())).collect()
        };
        for (id, pubkey) in todo {
            let key = self.lock().my_pkey.clone();
            if key.is_empty() {
                break;
            }
            if self.send_direct(app, id, &pubkey, Direct::PresenceKey { key }).await.is_ok() {
                if let Some(f) = self.lock().book.find_mut(id) {
                    f.key_sent = true;
                }
                self.save(app);
            }
        }
        self.publish_now(app).await;
    }

    /// Puts what plays now into the relay's directory (sealed with our presence key) or takes it out.
    pub async fn publish_now(&self, app: &AppHandle) {
        let state = app.state::<AppState>();
        let Some(me) = state.me() else { return };
        let Ok(base) = super::service::server_url(&state) else { return };
        let (key, has_friends, last) = {
            let g = self.lock();
            (g.my_pkey.clone(), !g.book.friend_ids().is_empty(), g.last_pub)
        };
        let Some(key) = crypto::key_from_b64(&key) else { return };
        let snap = app.state::<Arc<crate::player::Player>>().snapshot();
        let want = state.config.get().share_listening && has_friends && snap.playing && snap.track.is_some();
        let hash = now_hash(me.id);
        let Ok(client) = super::service::http() else { return };
        if want {
            let t = snap.track.expect("checked above");
            if matches!(last, Some((id, true, at)) if id == t.id && at.elapsed() < Duration::from_secs(50)) {
                return;
            }
            let body = NowPlaying { track_id: t.id, title: t.title, artist: t.artist, artwork_url: t.artwork_url, at: now_ms() };
            let blob = crypto::seal(&key, &hash, &serde_json::to_vec(&body).unwrap_or_default());
            if client.put(format!("{base}/dir/{hash}")).body(blob).send().await.is_ok() {
                self.lock().last_pub = Some((t.id, true, std::time::Instant::now()));
            }
        } else if matches!(last, Some((_, true, _))) {
            let _ = client.delete(format!("{base}/dir/{hash}")).send().await;
            self.lock().last_pub = None;
        }
    }

    /// What the friends who share it are listening to right now.
    pub async fn listening(&self, app: &AppHandle) -> AppResult<Vec<Listening>> {
        let state = app.state::<AppState>();
        let base = super::service::server_url(&state)?;
        let who: Vec<(String, u64, crypto::Key)> = {
            let g = self.lock();
            g.book
                .people
                .iter()
                .filter(|p| p.state == State::Friend)
                .filter_map(|p| Some((now_hash(p.id), p.id, crypto::key_from_b64(&p.pkey)?)))
                .collect()
        };
        if who.is_empty() {
            return Ok(Vec::new());
        }
        let hosts = who.iter().map(|(h, ..)| h.as_str()).collect::<Vec<_>>().join(",");
        let text = super::service::http()?
            .get(format!("{base}/dir?hosts={hosts}"))
            .send()
            .await
            .map_err(|e| AppError::Network(e.to_string()))?
            .text()
            .await
            .map_err(|e| AppError::Network(e.to_string()))?;
        let list: Vec<serde_json::Value> = serde_json::from_str(&text).map_err(|e| AppError::Other(e.to_string()))?;
        let mut out = Vec::new();
        for e in list {
            let (Some(host), Some(blob)) = (e["host"].as_str(), e["blob"].as_str()) else { continue };
            let Some((_, id, key)) = who.iter().find(|(h, ..)| h == host) else { continue };
            let Some(plain) = crypto::open(key, host, blob) else { continue };
            let Ok(n) = serde_json::from_slice::<NowPlaying>(&plain) else { continue };
            if now_ms() - n.at <= NOW_FRESH_MS {
                out.push(Listening { id: *id, track_id: n.track_id, title: n.title, artist: n.artist, artwork_url: n.artwork_url });
            }
        }
        Ok(out)
    }

    fn advance(&self, seq: u64) {
        let mut g = self.lock();
        if seq > g.since {
            g.since = seq;
        }
    }

    fn since(&self) -> u64 {
        self.lock().since
    }

    fn reset_since(&self, head: u64) {
        let mut g = self.lock();
        if head < g.since {
            g.since = head;
        }
    }
}

async fn pump(app: AppHandle, channel: String, mut events: tokio::sync::mpsc::UnboundedReceiver<hub::HubEvent>) {
    let key = inbox_key(&channel);
    while let Some(e) = events.recv().await {
        let friends = app.state::<Friends>();
        match e {
            hub::HubEvent::Connected { head, .. } => friends.reset_since(head),
            hub::HubEvent::Msg { seq, blob, keep, .. } => {
                if !keep || seq <= friends.since() {
                    continue;
                }
                friends.advance(seq);
                let wire = crypto::open(&key, &channel, &blob).and_then(|p| serde_json::from_slice::<Wire>(&p).ok());
                if let Some(w) = wire {
                    friends.incoming(&app, w);
                } else {
                    // nothing to change, but the cursor moved
                    friends.save(&app);
                }
            }
            hub::HubEvent::Disconnected => {}
        }
    }
}

/// Puts a message into somebody's inbox: a short connection that waits for the relay's echo.
async fn deliver(app: &AppHandle, to: u64, wire: &Wire) -> AppResult<()> {
    let state = app.state::<AppState>();
    let base = super::service::server_url(&state)?;
    let channel = inbox_channel(to);
    let blob = crypto::seal(&inbox_key(&channel), &channel, &serde_json::to_vec(wire)?);
    let id = format!("d{}", now_ms());
    let (client, mut events) = hub::connect(base, channel, u64::MAX);
    client.send(blob, true, Some(id.clone()));
    let sent = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(e) = events.recv().await {
            if let hub::HubEvent::Msg { id: Some(got), .. } = e {
                if got == id {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    client.close();
    if sent {
        Ok(())
    } else {
        Err(AppError::Network(pick("Не удалось связаться с сервером. Проверьте интернет и попробуйте ещё раз.", "Could not reach the server. Check the connection and try again.").into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Identity, Identity) {
        (Identity::new(), Identity::new())
    }

    fn req(from: u64, name: &str, id: &Identity) -> Wire {
        Wire::Req { from, name: name.into(), avatar: None, pubkey: id.public_b64() }
    }

    fn accept_wire(from: u64, name: &str, id: &Identity) -> Wire {
        Wire::Accept { from, name: name.into(), avatar: None, pubkey: id.public_b64() }
    }

    #[test]
    fn a_request_and_its_answer_make_two_friends() {
        let (ia, ib) = pair();
        let (mut a, mut b) = (Book::default(), Book::default());
        // anna asks bob
        assert_eq!(a.request(2, "bob".into(), None, 1, 10), Ok(true));
        let (changed, fx) = b.handle(&ib, 2, req(1, "anna", &ia), 11);
        assert!(changed);
        assert!(matches!(fx.as_slice(), [Effect::Notify { .. }]));
        assert_eq!(b.view().incoming.len(), 1);
        // bob says yes
        assert!(b.accept(1, 12));
        let (changed, _) = a.handle(&ia, 1, accept_wire(2, "bob", &ib), 13);
        assert!(changed);
        assert_eq!(a.friend_ids(), vec![2]);
        assert_eq!(b.friend_ids(), vec![1]);
        assert!(a.view().outgoing.is_empty());
    }

    #[test]
    fn when_both_ask_each_other_it_is_a_yes() {
        let (ia, ib) = pair();
        let (mut a, mut b) = (Book::default(), Book::default());
        a.request(2, "bob".into(), None, 1, 1).unwrap();
        b.request(1, "anna".into(), None, 2, 1).unwrap();
        let (_, fx) = a.handle(&ia, 1, req(2, "bob", &ib), 2);
        assert!(fx.contains(&Effect::Accept(2)));
        assert_eq!(a.friend_ids(), vec![2]);
        let (_, _) = b.handle(&ib, 2, accept_wire(1, "anna", &ia), 3);
        assert_eq!(b.friend_ids(), vec![1]);
    }

    #[test]
    fn an_unasked_answer_does_nothing() {
        let (ia, ib) = pair();
        let mut a = Book::default();
        let (changed, _) = a.handle(&ia, 1, accept_wire(2, "bob", &ib), 1);
        assert!(!changed);
        assert!(a.people.is_empty());
    }

    fn befriend() -> (Identity, Identity, Book, Book) {
        let (ia, ib) = pair();
        let (mut a, mut b) = (Book::default(), Book::default());
        a.request(2, "bob".into(), None, 1, 1).unwrap();
        b.handle(&ib, 2, req(1, "anna", &ia), 2);
        b.accept(1, 3);
        a.handle(&ia, 1, accept_wire(2, "bob", &ib), 4);
        (ia, ib, a, b)
    }

    fn direct(from_id: &Identity, from: u64, to_pub: &str, to: u64, d: &Direct) -> Wire {
        let key = from_id.pair_key(to_pub).unwrap();
        Wire::Direct { from, blob: crypto::seal(&key, &direct_aad(from, to), &serde_json::to_vec(d).unwrap()) }
    }

    #[test]
    fn an_invitation_from_a_friend_arrives_and_is_not_repeated() {
        let (ia, ib, _a, mut b) = befriend();
        let inv = Direct::Invite { kind: "room".into(), link: "scdesk://room/abc".into(), title: "Friday".into(), password: "pw".into(), name: "anna".into(), at: 100 };
        let w = direct(&ia, 1, &ib.public_b64(), 2, &inv);
        let (changed, fx) = b.handle(&ib, 2, w.clone(), 101);
        assert!(changed);
        assert!(matches!(fx.as_slice(), [Effect::Notify { .. }]));
        assert_eq!(b.view().invites.len(), 1);
        assert_eq!(b.view().invites[0].password, "pw");
        let (again, _) = b.handle(&ib, 2, w, 102);
        assert!(!again, "the same invitation twice");
        assert!(b.dismiss(&b.view().invites[0].id.clone()));
        assert!(b.view().invites.is_empty());
    }

    #[test]
    fn a_stranger_cannot_forge_an_invitation_or_a_removal() {
        let (_ia, ib, _a, mut b) = befriend();
        let eve = Identity::new();
        let inv = Direct::Invite { kind: "room".into(), link: "scdesk://room/evil".into(), title: "x".into(), password: String::new(), name: "anna".into(), at: 1 };
        // eve claims to be anna (id 1) but seals with her own key
        let w = direct(&eve, 1, &ib.public_b64(), 2, &inv);
        let (changed, _) = b.handle(&ib, 2, w, 5);
        assert!(!changed);
        let w = direct(&eve, 1, &ib.public_b64(), 2, &Direct::Remove);
        let (changed, _) = b.handle(&ib, 2, w, 6);
        assert!(!changed);
        assert_eq!(b.friend_ids(), vec![1], "still friends");
        assert!(b.view().invites.is_empty());
    }

    #[test]
    fn removing_a_friend_is_told_and_drops_their_invitations() {
        let (ia, ib, mut a, mut b) = befriend();
        let inv = Direct::Invite { kind: "playlist".into(), link: "scdesk://playlist/x".into(), title: "mix".into(), password: String::new(), name: "anna".into(), at: 9 };
        b.handle(&ib, 2, direct(&ia, 1, &ib.public_b64(), 2, &inv), 10);
        assert_eq!(b.view().invites.len(), 1);
        // anna removes bob and tells him
        assert!(a.remove(2));
        let w = direct(&ia, 1, &ib.public_b64(), 2, &Direct::Remove);
        let (changed, _) = b.handle(&ib, 2, w, 11);
        assert!(changed);
        assert!(b.friend_ids().is_empty());
        assert!(b.view().invites.is_empty());
    }

    #[test]
    fn a_friend_with_a_new_key_waits_for_a_yes_and_the_old_key_keeps_working_meanwhile() {
        let (ia, ib, _a, mut b) = befriend();
        let new_anna = Identity::new();
        let (changed, _) = b.handle(&ib, 2, req(1, "anna", &new_anna), 20);
        assert!(changed);
        assert_eq!(b.view().incoming.len(), 1, "shown among the requests");
        assert_eq!(b.view().friends.len(), 1, "and still a friend");
        // the old key still opens her messages
        let inv = Direct::Invite { kind: "room".into(), link: "scdesk://room/r".into(), title: "t".into(), password: String::new(), name: "anna".into(), at: 21 };
        let (ok, _) = b.handle(&ib, 2, direct(&ia, 1, &ib.public_b64(), 2, &inv), 22);
        assert!(ok);
        // a yes switches to the new key
        assert!(b.accept(1, 23));
        assert!(b.view().incoming.is_empty());
        let inv2 = Direct::Invite { kind: "room".into(), link: "scdesk://room/r2".into(), title: "t".into(), password: String::new(), name: "anna".into(), at: 24 };
        let (ok, _) = b.handle(&ib, 2, direct(&new_anna, 1, &ib.public_b64(), 2, &inv2), 25);
        assert!(ok, "the new key works after the yes");
    }

    #[test]
    fn declining_forgets_the_request_and_cancelling_takes_ours_back() {
        let (ia, ib) = pair();
        let (mut a, mut b) = (Book::default(), Book::default());
        a.request(2, "bob".into(), None, 1, 1).unwrap();
        b.handle(&ib, 2, req(1, "anna", &ia), 2);
        assert!(b.decline(1));
        assert!(b.people.is_empty());
        assert!(a.cancel(2));
        assert!(a.people.is_empty());
        assert!(!a.cancel(2));
    }

    #[test]
    fn you_cannot_ask_yourself_or_a_friend_twice() {
        let (_ia, _ib, mut a, _b) = befriend();
        assert!(a.request(1, "me".into(), None, 1, 1).is_err());
        assert!(a.request(2, "bob".into(), None, 1, 1).is_err());
    }

    #[test]
    fn a_friend_hands_over_the_key_to_read_what_they_listen_to_and_a_stranger_cannot() {
        let (ia, ib, _a, mut b) = befriend();
        let key = crypto::key_to_b64(&crypto::random_key());
        let w = direct(&ia, 1, &ib.public_b64(), 2, &Direct::PresenceKey { key: key.clone() });
        let (changed, _) = b.handle(&ib, 2, w, 30);
        assert!(changed);
        assert_eq!(b.find(1).unwrap().pkey, key);
        // not a key at all: ignored
        let bad = direct(&ia, 1, &ib.public_b64(), 2, &Direct::PresenceKey { key: "nope".into() });
        assert!(!b.handle(&ib, 2, bad, 31).0);
        assert_eq!(b.find(1).unwrap().pkey, key);
        // a stranger's key does not replace it
        let eve = Identity::new();
        let forged = direct(&eve, 1, &ib.public_b64(), 2, &Direct::PresenceKey { key: crypto::key_to_b64(&crypto::random_key()) });
        assert!(!b.handle(&ib, 2, forged, 32).0);
        assert_eq!(b.find(1).unwrap().pkey, key);
    }

    #[test]
    fn a_new_identity_of_a_friend_clears_their_old_presence_key_and_asks_for_ours_again() {
        let (ia, ib, _a, mut b) = befriend();
        let key = crypto::key_to_b64(&crypto::random_key());
        b.handle(&ib, 2, direct(&ia, 1, &ib.public_b64(), 2, &Direct::PresenceKey { key }), 30);
        b.find_mut(1).unwrap().key_sent = true;
        let new_anna = Identity::new();
        b.handle(&ib, 2, req(1, "anna", &new_anna), 40);
        assert!(b.accept(1, 41));
        let f = b.find(1).unwrap();
        assert!(f.pkey.is_empty() && !f.key_sent, "the old key is gone, ours goes out again");
    }

    #[test]
    fn the_inbox_channel_is_a_valid_relay_id_and_the_same_for_everybody() {
        let c = inbox_channel(12345);
        assert_eq!(c, inbox_channel(12345));
        assert_ne!(c, inbox_channel(12346));
        assert!(c.len() >= 16 && c.len() <= 64 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
    }
}
