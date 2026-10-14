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
        let mut s = super::open(data.clone()).unwrap();
        s.try_seek(std::time::Duration::from_secs(4)).unwrap();
        let left = s.count() as f64 / f64::from(ch) / f64::from(rate);
        println!("after a seek to 4 s: {left:.3} s left");
        assert!((left - 2.0).abs() < 0.05);
        // packed again (as «Мои файлы» does): still 6 s of sound, and small
        let packed = super::pack_opus(data.clone()).unwrap();
        let back = super::open(std::sync::Arc::new(packed.clone())).unwrap();
        let secs = back.count() as f64 / 2.0 / 48_000.0;
        println!("packed: {} bytes, {secs:.3} s", packed.len());
        assert!((secs - 6.0).abs() < 0.05);
    }
}

/// Bits per second of «Мои файлы» once packed: 96 kbit/s stereo Opus is where listeners stop telling it from the
/// original (a song of 3 minutes ≈ 2.2 MB).
const PACK_BITRATE: i32 = 96_000;

/// Any audio the player reads, re-encoded as Ogg Opus at 96 kbit/s: one pass, frame by frame (the song is never
/// unpacked whole into memory). Resampled to 48 kHz by cubic interpolation (keeps the highs a straight line would
/// dull), the level untouched, more than two channels folded to stereo.
pub fn pack_opus(data: Arc<Vec<u8>>) -> Option<Vec<u8>> {
    use audiopus::{coder::Encoder, Application, Bitrate, Channels, SampleRate};
    let src = open(data).ok()?;
    let in_ch = usize::from(src.channels().max(1));
    let rate = f64::from(src.sample_rate().max(1));
    let ch = in_ch.min(2);
    let mut enc = Encoder::new(SampleRate::Hz48000, if ch == 1 { Channels::Mono } else { Channels::Stereo }, Application::Audio).ok()?;
    enc.set_bitrate(Bitrate::BitsPerSecond(PACK_BITRATE)).ok()?;
    let pre_skip = enc.lookahead().unwrap_or(312) as u64;

    let mut out = Vec::new();
    {
        let mut w = ogg::PacketWriter::new(&mut out);
        let serial = 0x5C_DE5C;
        let mut head = b"OpusHead".to_vec();
        head.push(1);
        head.push(ch as u8);
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        w.write_packet(head, serial, ogg::PacketWriteEndInfo::EndPage, 0).ok()?;
        let vendor = b"SC Desk";
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        w.write_packet(tags, serial, ogg::PacketWriteEndInfo::EndPage, 0).ok()?;

        // cubic (Catmull-Rom) resampling over the last four input frames; 20 ms (960 samples) per Opus frame
        const FRAME: usize = 960;
        let step = rate / 48_000.0;
        let mut hist = [vec![0f32; ch], vec![0f32; ch], vec![0f32; ch], vec![0f32; ch]];
        let (mut have, mut t) = (0u64, 0f64);
        let mut frame_buf: Vec<f32> = Vec::with_capacity(FRAME * ch);
        let mut packet = vec![0u8; 4000];
        let mut granule = pre_skip;
        let mut written = 0usize;
        let mut src = src;
        let mut flush = |frame: &[f32], last: bool, granule: u64, w: &mut ogg::PacketWriter<&mut Vec<u8>>| -> Option<()> {
            let n = enc.encode_float(frame, &mut packet).ok()?;
            written += 1;
            let end = if last {
                ogg::PacketWriteEndInfo::EndStream
            } else if written % 50 == 0 {
                ogg::PacketWriteEndInfo::EndPage
            } else {
                ogg::PacketWriteEndInfo::NormalPacket
            };
            w.write_packet(packet[..n].to_vec(), serial, end, granule).ok()
        };
        let mut frame_in = vec![0f32; in_ch];
        'read: loop {
            // the next input frame
            for c in 0..in_ch {
                match src.next() {
                    Some(s) => frame_in[c] = s,
                    None => break 'read,
                }
            }
            hist.rotate_left(1);
            for c in 0..ch {
                hist[3][c] = if in_ch > 2 && c == 1 { frame_in[1..].iter().sum::<f32>() / (in_ch - 1) as f32 } else { frame_in[c] };
            }
            have += 1;
            // output samples between input frames have-3 and have-2 (hist[1] .. hist[2]), with one either side
            while have >= 3 && t <= (have - 2) as f64 {
                let f = (t - (have - 3) as f64) as f32;
                for c in 0..ch {
                    let (p0, p1, p2, p3) = (hist[0][c], hist[1][c], hist[2][c], hist[3][c]);
                    let v = p1 + 0.5 * f * (p2 - p0 + f * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + f * (3.0 * (p1 - p2) + p3 - p0)));
                    frame_buf.push(v.clamp(-1.0, 1.0));
                }
                t += step;
                if frame_buf.len() == FRAME * ch {
                    granule += FRAME as u64;
                    flush(&frame_buf, false, granule, &mut w)?;
                    frame_buf.clear();
                }
            }
        }
        // the last frame, padded with silence
        let real = (frame_buf.len() / ch) as u64;
        frame_buf.resize(FRAME * ch, 0.0);
        granule += real;
        flush(&frame_buf, true, granule, &mut w)?;
    }
    Some(out)
}

#[cfg(test)]
mod pack_tests {
    use rodio::Source;

    /// A 1 kHz tone at 44.1 kHz packed: same pitch (zero crossings), same level, same length.
    /// `PACK_FILE=…mp3 cargo test pack_quality -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn pack_quality() {
        let data = std::sync::Arc::new(std::fs::read(std::env::var("PACK_FILE").unwrap()).unwrap());
        let measure = |d: std::sync::Arc<Vec<u8>>| {
            let s = super::open(d).unwrap();
            let (ch, rate) = (s.channels() as usize, s.sample_rate() as f64);
            let left: Vec<f32> = s.step_by(ch).collect();
            let mid = &left[left.len() / 4..left.len() * 3 / 4];
            let crossings = mid.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f64;
            let hz = crossings / (mid.len() as f64 / rate);
            let rms = (mid.iter().map(|x| x * x).sum::<f32>() / mid.len() as f32).sqrt();
            (left.len() as f64 / rate, hz, rms)
        };
        let packed = super::pack_opus(data.clone()).unwrap();
        let (a, b) = (measure(data.clone()), measure(std::sync::Arc::new(packed.clone())));
        println!("original {} bytes: {:.3} s, {:.1} Hz, rms {:.4}", data.len(), a.0, a.1, a.2);
        println!("packed   {} bytes: {:.3} s, {:.1} Hz, rms {:.4}", packed.len(), b.0, b.1, b.2);
        assert!((a.0 - b.0).abs() < 0.1 && (a.1 - b.1).abs() < 2.0 && (a.2 - b.2).abs() / a.2 < 0.03);
    }
}
