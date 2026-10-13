//! Turning downloaded audio into samples: MP3 / AAC through rodio, Ogg Opus (what SoundCloud serves more and more,
//! and the smallest download) through libopus, packet by packet as it plays: the whole track is never unpacked
//! into memory, a seek jumps to the packet and decodes a little before it so the sound settles.

use std::{io::Cursor, sync::Arc, time::Duration};

use rodio::{source::SeekError, Decoder, Source};

/// `Arc<Vec<u8>>` read in place.
#[derive(Clone)]
pub struct Shared(pub Arc<Vec<u8>>);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

pub type Samples = Box<dyn Source<Item = f32> + Send>;

/// Ogg with an Opus stream inside.
pub fn is_opus(data: &[u8]) -> bool {
    data.starts_with(b"OggS") && data.windows(8).take(200).any(|w| w == b"OpusHead")
}

/// Samples of any format the app plays.
pub fn open(data: Arc<Vec<u8>>) -> Result<Samples, String> {
    if is_opus(&data) {
        return Opus::new(data).map(|o| Box::new(o) as Samples);
    }
    Decoder::new(Cursor::new(Shared(data))).map(|d| Box::new(d.convert_samples::<f32>()) as Samples).map_err(|e| e.to_string())
}

const RATE: u32 = 48_000;
/// decoded before a seek target, then thrown away: the decoder needs it to settle (80 ms, as the Opus spec says)
const PREROLL: u64 = 3_840;

struct Packet {
    /// where its first sample is, counted from the stream start (pre-skip included)
    start: u64,
    data: Vec<u8>,
}

pub struct Opus {
    packets: Vec<Packet>,
    channels: u16,
    pre_skip: u64,
    total: u64,
    decoder: audiopus::coder::Decoder,
    next: usize,
    buf: Vec<f32>,
    at: usize,
    /// samples (per channel) still to drop: the pre-skip and the pre-roll before a seek target
    drop: u64,
}

impl Opus {
    fn new(data: Arc<Vec<u8>>) -> Result<Self, String> {
        let mut reader = ogg::PacketReader::new(Cursor::new(Shared(data)));
        let mut packets = Vec::new();
        let (mut channels, mut pre_skip) = (2u16, 0u64);
        let (mut pos, mut index) = (0u64, 0usize);
        while let Some(p) = reader.read_packet().map_err(|e| e.to_string())? {
            match index {
                0 => {
                    if p.data.len() < 19 || &p.data[..8] != b"OpusHead" {
                        return Err("not an Opus stream".into());
                    }
                    channels = u16::from(p.data[9]).clamp(1, 2);
                    pre_skip = u64::from(u16::from_le_bytes([p.data[10], p.data[11]]));
                }
                1 => {} // OpusTags
                _ => {
                    let n = audiopus::packet::nb_samples(&p.data[..], audiopus::SampleRate::Hz48000).map_err(|e| e.to_string())? as u64;
                    packets.push(Packet { start: pos, data: p.data });
                    pos += n;
                }
            }
            index += 1;
        }
        if packets.is_empty() {
            return Err("Opus: no audio".into());
        }
        let decoder = new_decoder(channels)?;
        Ok(Self { packets, channels, pre_skip, total: pos, decoder, next: 0, buf: Vec::new(), at: 0, drop: pre_skip })
    }
}

fn new_decoder(channels: u16) -> Result<audiopus::coder::Decoder, String> {
    let ch = if channels == 1 { audiopus::Channels::Mono } else { audiopus::Channels::Stereo };
    audiopus::coder::Decoder::new(audiopus::SampleRate::Hz48000, ch).map_err(|e| e.to_string())
}

impl Opus {
    /// Decodes the next packet into `buf`; false at the end.
    fn fill(&mut self) -> bool {
        let Some(p) = self.packets.get(self.next) else { return false };
        self.next += 1;
        let mut out = vec![0f32; 5760 * usize::from(self.channels)];
        let packet = &p.data[..];
        let n = self.decoder.decode_float(Some(packet), &mut out[..], false).unwrap_or(0);
        out.truncate(n * usize::from(self.channels));
        let drop = (self.drop as usize).min(n);
        self.drop -= drop as u64;
        self.buf = out;
        self.at = drop * usize::from(self.channels);
        true
    }
}

impl Iterator for Opus {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        while self.at >= self.buf.len() {
            if !self.fill() {
                return None;
            }
        }
        let x = self.buf[self.at];
        self.at += 1;
        Some(x)
    }
}

impl Source for Opus {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(self.total.saturating_sub(self.pre_skip) as f64 / f64::from(RATE)))
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let target = (pos.as_secs_f64() * f64::from(RATE)) as u64 + self.pre_skip;
        let from = target.saturating_sub(PREROLL);
        let i = self.packets.partition_point(|p| p.start <= from).saturating_sub(1);
        self.next = i;
        self.drop = target.saturating_sub(self.packets[i].start);
        self.buf.clear();
        self.at = 0;
        if let Ok(d) = new_decoder(self.channels) {
            self.decoder = d;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rodio::Source;

    /// `OPUS_FILE=…opus cargo test opus_live -- --ignored --nocapture` (ffmpeg makes one: a 6 s sine)
    #[test]
    #[ignore]
    fn opus_live() {
        let data = std::sync::Arc::new(std::fs::read(std::env::var("OPUS_FILE").unwrap()).unwrap());
        assert!(super::is_opus(&data));
        let src = super::open(data.clone()).unwrap();
        let (ch, rate, dur) = (src.channels(), src.sample_rate(), src.total_duration());
        let n = src.count();
        println!("channels {ch} rate {rate} duration {dur:?} samples {n}");
        assert!((n as f64 / f64::from(ch) / f64::from(rate) - 6.0).abs() < 0.1);
        let mut s = super::open(data).unwrap();
        s.try_seek(std::time::Duration::from_secs(4)).unwrap();
        let left = s.count() as f64 / f64::from(ch) / f64::from(rate);
        println!("after a seek to 4 s: {left:.3} s left");
        assert!((left - 2.0).abs() < 0.05);
    }
}
