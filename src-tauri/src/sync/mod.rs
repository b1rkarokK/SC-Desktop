//! Rooms and shared playlists talk through a small relay server (see /server): everything that
//! travels is encrypted on the devices (`crypto`), the hub keeps channels (`hub`).
pub mod crypto;
pub mod engine;
pub mod hub;
pub mod playlists;
pub mod shared;
pub mod watch;
pub mod service;
