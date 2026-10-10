//! Shared playlists: several people edit one playlist. Transport-independent
//! core (like rooms.rs), written and tested before the server and the UI.
//!
//! Merging must give the same list on every device whatever order the edits
//! arrive in, and a retransmitted edit must change nothing. So the playlist is
//! a map of entries; each entry has its own last-writer-wins registers (place,
//! deleted) stamped with a Lamport clock + author id. Applying an edit is a
//! per-register "keep the newer stamp"; that is commutative and idempotent.
//! The place is a fractional position (between its neighbours), so a move or
//! an insert is one edit, never a renumbering of the list.
#![allow(dead_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Lamport time, then author: a total order that every device agrees on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Stamp {
    pub time: u64,
    pub author: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// only the owner and the people invited by name
    Closed,
    /// anybody with the link may look, only invited people edit
    Link,
    /// invited people see and edit; the link alone does nothing
    Invite,
    /// anybody may look and add tracks
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub title: String,
    pub access: Access,
    /// SoundCloud user id -> role
    pub members: BTreeMap<u64, Role>,
}

impl Settings {
    pub fn role_of(&self, user: Option<u64>) -> Option<Role> {
        user.and_then(|u| self.members.get(&u).copied())
    }

    /// May this person (or somebody holding only the link) open the playlist?
    pub fn can_view(&self, user: Option<u64>, has_link: bool) -> bool {
        self.role_of(user).is_some()
            || match self.access {
                Access::Closed | Access::Invite => false,
                Access::Link => has_link,
                Access::Open => true,
            }
    }

    pub fn can_edit(&self, user: Option<u64>) -> bool {
        match self.role_of(user) {
            Some(Role::Owner | Role::Editor) => true,
            Some(Role::Viewer) => false,
            // "open": any signed-in person may add
            None => self.access == Access::Open && user.is_some(),
        }
    }

    /// Only the owner changes settings, invites and removes people.
    pub fn can_manage(&self, user: Option<u64>) -> bool {
        self.role_of(user) == Some(Role::Owner)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Edit {
    /// a new entry (`entry` is unique, made by the author)
    Add { entry: String, track_id: u64, pos: f64, stamp: Stamp },
    Move { entry: String, pos: f64, stamp: Stamp },
    Remove { entry: String, stamp: Stamp },
    /// removing again (undo) is a newer "not removed"
    Restore { entry: String, stamp: Stamp },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    track_id: u64,
    pos: f64,
    pos_stamp: Stamp,
    removed: bool,
    removed_stamp: Stamp,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    entries: BTreeMap<String, Entry>,
    /// highest Lamport time seen: our next edit is later than everything we know
    clock: u64,
}

impl Playlist {
    pub fn apply(&mut self, edit: &Edit) {
        let stamp = match edit {
            Edit::Add { stamp, .. } | Edit::Move { stamp, .. } | Edit::Remove { stamp, .. } | Edit::Restore { stamp, .. } => stamp,
        };
        self.clock = self.clock.max(stamp.time);
        match edit {
            Edit::Add { entry, track_id, pos, stamp } => {
                self.entries.entry(entry.clone()).or_insert_with(|| Entry {
                    track_id: *track_id,
                    pos: *pos,
                    pos_stamp: stamp.clone(),
                    removed: false,
                    removed_stamp: Stamp { time: 0, author: String::new() },
                });
            }
            Edit::Move { entry, pos, stamp } => {
                if let Some(e) = self.entries.get_mut(entry) {
                    if *stamp > e.pos_stamp {
                        e.pos = *pos;
                        e.pos_stamp = stamp.clone();
                    }
                }
            }
            Edit::Remove { entry, stamp } | Edit::Restore { entry, stamp } => {
                if let Some(e) = self.entries.get_mut(entry) {
                    if *stamp > e.removed_stamp {
                        e.removed = matches!(edit, Edit::Remove { .. });
                        e.removed_stamp = stamp.clone();
                    }
                }
            }
        }
    }

    /// A stamp for the next local edit.
    pub fn next_stamp(&mut self, author: &str) -> Stamp {
        self.clock += 1;
        Stamp { time: self.clock, author: author.to_owned() }
    }

    /// Visible entries in order: (entry id, track id).
    pub fn list(&self) -> Vec<(&str, u64)> {
        let mut v: Vec<(&String, &Entry)> = self.entries.iter().filter(|(_, e)| !e.removed).collect();
        // equal places (two people inserted at the same spot): older stamp first, same everywhere
        v.sort_by(|a, b| a.1.pos.total_cmp(&b.1.pos).then_with(|| a.1.pos_stamp.cmp(&b.1.pos_stamp)).then_with(|| a.0.cmp(b.0)));
        v.into_iter().map(|(id, e)| (id.as_str(), e.track_id)).collect()
    }

    /// The position for a track to go at index `at` (0 = first) of the visible list.
    pub fn pos_for(&self, at: usize) -> f64 {
        let list = self.list();
        let pos = |i: usize| self.entries[list[i].0].pos;
        match (at.checked_sub(1).map(pos), (at < list.len()).then(|| pos(at))) {
            (None, None) => 0.0,
            (None, Some(next)) => next - 1.0,
            (Some(prev), None) => prev + 1.0,
            (Some(prev), Some(next)) => (prev + next) / 2.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(time: u64, author: &str) -> Stamp {
        Stamp { time, author: author.into() }
    }

    fn add(entry: &str, track: u64, pos: f64, s: Stamp) -> Edit {
        Edit::Add { entry: entry.into(), track_id: track, pos, stamp: s }
    }

    fn ids(p: &Playlist) -> Vec<u64> {
        p.list().into_iter().map(|(_, t)| t).collect()
    }

    #[test]
    fn same_result_in_any_order_and_when_repeated() {
        let edits = vec![
            add("a", 1, 0.0, st(1, "x")),
            add("b", 2, 1.0, st(2, "y")),
            add("c", 3, 2.0, st(3, "x")),
            Edit::Move { entry: "c".into(), pos: -1.0, stamp: st(4, "y") },
            Edit::Remove { entry: "a".into(), stamp: st(5, "x") },
        ];
        let mut forward = Playlist::default();
        edits.iter().for_each(|e| forward.apply(e));
        let mut backward = Playlist::default();
        // a move or remove can arrive before its add: it must not be lost
        let mut pending: Vec<&Edit> = edits.iter().rev().collect();
        for _ in 0..3 {
            let mut still = Vec::new();
            for e in pending {
                let known = match e {
                    Edit::Add { .. } => true,
                    Edit::Move { entry, .. } | Edit::Remove { entry, .. } | Edit::Restore { entry, .. } => backward.entries.contains_key(entry),
                };
                if known { backward.apply(e) } else { still.push(e) }
            }
            pending = still;
        }
        edits.iter().for_each(|e| backward.apply(e)); // everything again: idempotent
        assert_eq!(ids(&forward), vec![3, 2]);
        assert_eq!(ids(&forward), ids(&backward));
    }

    #[test]
    fn concurrent_moves_keep_the_newer_stamp_everywhere() {
        let base = add("a", 1, 0.0, st(1, "x"));
        let m1 = Edit::Move { entry: "a".into(), pos: 5.0, stamp: st(3, "x") };
        let m2 = Edit::Move { entry: "a".into(), pos: 9.0, stamp: st(3, "y") }; // same time, author breaks the tie
        let (mut p, mut q) = (Playlist::default(), Playlist::default());
        for e in [&base, &m1, &m2] { p.apply(e); }
        for e in [&base, &m2, &m1] { q.apply(e); }
        assert_eq!(p, q);
        assert_eq!(p.entries["a"].pos, 9.0);
    }

    #[test]
    fn removing_then_restoring_follows_the_newer_stamp() {
        let mut p = Playlist::default();
        p.apply(&add("a", 1, 0.0, st(1, "x")));
        p.apply(&Edit::Restore { entry: "a".into(), stamp: st(4, "y") });
        p.apply(&Edit::Remove { entry: "a".into(), stamp: st(3, "x") }); // older than the restore: ignored
        assert_eq!(ids(&p), vec![1]);
        p.apply(&Edit::Remove { entry: "a".into(), stamp: st(6, "x") });
        assert!(ids(&p).is_empty());
    }

    #[test]
    fn inserting_between_neighbours_needs_one_edit() {
        let mut p = Playlist::default();
        for (i, t) in [10u64, 20, 30].into_iter().enumerate() {
            let pos = p.pos_for(i);
            let s = p.next_stamp("me");
            p.apply(&add(&format!("e{t}"), t, pos, s));
        }
        assert_eq!(ids(&p), vec![10, 20, 30]);
        let (front, middle, back) = (p.pos_for(0), p.pos_for(1), p.pos_for(3));
        let s = p.next_stamp("me");
        p.apply(&add("mid", 15, middle, s));
        let s = p.next_stamp("me");
        p.apply(&add("front", 5, front, s));
        let s = p.next_stamp("me");
        p.apply(&add("back", 99, back, s));
        assert_eq!(ids(&p), vec![5, 10, 15, 20, 30, 99]);
    }

    #[test]
    fn two_people_inserting_at_the_same_spot_agree_on_the_order() {
        let mut base = Playlist::default();
        base.apply(&add("a", 1, 0.0, st(1, "x")));
        base.apply(&add("b", 2, 10.0, st(2, "x")));
        let (mut anna, mut bob) = (base.clone(), base.clone());
        let ea = add("from-anna", 100, anna.pos_for(1), anna.next_stamp("anna"));
        let eb = add("from-bob", 200, bob.pos_for(1), bob.next_stamp("bob"));
        for e in [&ea, &eb] { anna.apply(e); }
        for e in [&eb, &ea] { bob.apply(e); }
        assert_eq!(ids(&anna), ids(&bob));
        assert_eq!(ids(&anna).len(), 4);
    }

    #[test]
    fn a_clock_that_hears_a_later_edit_stamps_after_it() {
        let mut p = Playlist::default();
        p.apply(&add("a", 1, 0.0, st(50, "far")));
        assert!(p.next_stamp("me").time > 50);
    }

    #[test]
    fn access_modes() {
        let mut s = Settings { title: "t".into(), access: Access::Closed, members: BTreeMap::from([(1, Role::Owner), (2, Role::Editor), (3, Role::Viewer)]) };
        // closed: only the named
        assert!(s.can_view(Some(2), false) && !s.can_view(Some(9), true) && !s.can_view(None, true));
        // link: the link is enough to look, not to edit
        s.access = Access::Link;
        assert!(s.can_view(Some(9), true) && !s.can_view(Some(9), false) && !s.can_edit(Some(9)));
        // invite: the link alone does nothing
        s.access = Access::Invite;
        assert!(!s.can_view(Some(9), true) && s.can_view(Some(3), false));
        // open: anybody signed in looks and adds
        s.access = Access::Open;
        assert!(s.can_view(None, false) && s.can_edit(Some(9)) && !s.can_edit(None));
        // roles
        assert!(s.can_edit(Some(2)) && !s.can_edit(Some(3)));
        assert!(s.can_manage(Some(1)) && !s.can_manage(Some(2)));
    }
}
