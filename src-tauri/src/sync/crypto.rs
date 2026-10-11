//! Encryption for rooms and shared playlists. The server only relays ciphertext, so what it could
//! read is: who connected to which channel, when, and how big the messages are.
//!
//! * The key of a room is derived from its id and, if the room has one, its password
//!   (`room_key`): whoever has the link (and the password) can read; a wrong password simply
//!   decrypts nothing.
//! * A message is XChaCha20-Poly1305 with a random 24-byte nonce, the channel id as associated
//!   data (a message can not be replayed into another channel). On the wire it is base64.
//! * Kicking someone changes the key (`rekey`): the host makes a new random key and wraps it
//!   separately for each remaining member with an X25519 shared secret (their public key came
//!   with their hello). The kicked one sees the new epoch but can not unwrap anything.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

pub type Key = [u8; 32];

/// The key everybody in the room derives from the room id (and password).
pub fn room_key(room_id: &str, password: &str) -> Key {
    // a slow-ish stretch for the password: guessing it offline costs 50 000 hashes a try
    let mut h = Sha256::digest(format!("scdesk-pw|{room_id}|{password}").as_bytes());
    if !password.is_empty() {
        for _ in 0..50_000 {
            h = Sha256::digest(h);
        }
    }
    let hk = Hkdf::<Sha256>::new(Some(b"scdesk-room-v1"), &h);
    let mut key = [0u8; 32];
    hk.expand(room_id.as_bytes(), &mut key).expect("32 bytes is a valid length");
    key
}

pub fn random_key() -> Key {
    let mut k = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut k);
    k
}

pub fn seal(key: &Key, channel: &str, plain: &[u8]) -> String {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: channel.as_bytes() })
        .expect("encryption does not fail");
    let mut out = nonce.to_vec();
    out.extend(ct);
    B64.encode(out)
}

/// `None` for anything that was not made with this key for this channel, or was changed.
pub fn open(key: &Key, channel: &str, blob: &str) -> Option<Vec<u8>> {
    let raw = B64.decode(blob).ok()?;
    if raw.len() < 24 + 16 {
        return None;
    }
    let (nonce, ct) = raw.split_at(24);
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: channel.as_bytes() })
        .ok()
}

/// A member's (or the host's) key pair for wrapping a new room key.
pub struct Identity {
    secret: StaticSecret,
    pub public: [u8; 32],
}

impl Identity {
    pub fn new() -> Self {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let public = PublicKey::from(&secret).to_bytes();
        Identity { secret, public }
    }

    pub fn public_b64(&self) -> String {
        B64.encode(self.public)
    }

    fn shared(&self, other_public_b64: &str) -> Option<Key> {
        let raw: [u8; 32] = B64.decode(other_public_b64).ok()?.try_into().ok()?;
        let secret = self.secret.diffie_hellman(&PublicKey::from(raw));
        let hk = Hkdf::<Sha256>::new(Some(b"scdesk-wrap-v1"), secret.as_bytes());
        let mut key = [0u8; 32];
        hk.expand(b"room key", &mut key).ok()?;
        Some(key)
    }

    /// The new room key, readable only by the holder of `member_public_b64`.
    pub fn wrap_for(&self, member_public_b64: &str, channel: &str, new_key: &Key) -> Option<String> {
        Some(seal(&self.shared(member_public_b64)?, channel, new_key))
    }

    /// A key wrapped for me by `host_public_b64`.
    pub fn unwrap_from(&self, host_public_b64: &str, channel: &str, wrapped: &str) -> Option<Key> {
        open(&self.shared(host_public_b64)?, channel, wrapped)?.try_into().ok()
    }
}

impl Default for Identity {
    fn default() -> Self {
        Self::new()
    }
}

/// A short, readable room code from a channel id ("KX-4821" style is a display name only).
pub fn new_channel_id() -> String {
    let mut raw = [0u8; 18];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_opens_only_with_the_right_key_and_channel() {
        let key = room_key("room-1", "");
        let blob = seal(&key, "room-1", b"play");
        assert_eq!(open(&key, "room-1", &blob).as_deref(), Some(&b"play"[..]));
        assert!(open(&room_key("room-2", ""), "room-1", &blob).is_none(), "another room's key");
        assert!(open(&key, "room-2", &blob).is_none(), "replayed into another channel");
        assert!(open(&room_key("room-1", "secret"), "room-1", &blob).is_none(), "a password changes the key");
        let mut bad = blob.clone().into_bytes();
        let last = bad.len() - 6;
        bad[last] = if bad[last] == b'A' { b'B' } else { b'A' };
        assert!(open(&key, "room-1", &String::from_utf8(bad).unwrap()).is_none(), "a changed message");
        assert!(open(&key, "room-1", "not base64 !").is_none());
    }

    #[test]
    fn the_same_text_looks_different_each_time() {
        let key = room_key("r", "");
        assert_ne!(seal(&key, "r", b"x"), seal(&key, "r", b"x"));
    }

    #[test]
    fn the_password_is_part_of_the_key_and_the_same_everywhere() {
        assert_eq!(room_key("r", "pw"), room_key("r", "pw"));
        assert_ne!(room_key("r", "pw"), room_key("r", "pw2"));
        assert_ne!(room_key("r", ""), room_key("r", "pw"));
    }

    #[test]
    fn a_new_key_reaches_the_ones_who_stay_and_not_the_one_who_is_kicked() {
        let host = Identity::new();
        let (anna, bob) = (Identity::new(), Identity::new());
        let new_key = random_key();
        let for_anna = host.wrap_for(&anna.public_b64(), "ch", &new_key).unwrap();
        assert_eq!(anna.unwrap_from(&host.public_b64(), "ch", &for_anna), Some(new_key));
        // bob was kicked: he may read the wrapped blob (it travelled in the old room) but it is Anna's
        assert_eq!(bob.unwrap_from(&host.public_b64(), "ch", &for_anna), None);
        // and with the new key the old one reads nothing
        let blob = seal(&new_key, "ch", b"after the kick");
        assert!(open(&room_key("ch", ""), "ch", &blob).is_none());
        assert_eq!(open(&new_key, "ch", &blob).as_deref(), Some(&b"after the kick"[..]));
    }

    #[test]
    fn channel_ids_are_long_and_url_safe() {
        let a = new_channel_id();
        assert_eq!(a.len(), 24);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(a, new_channel_id());
    }
}
