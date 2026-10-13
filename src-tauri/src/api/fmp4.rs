//! Fragmented MP4 (what an HLS AAC stream is: an init segment, then moof+mdat pieces) → a plain ADTS AAC stream.
//!
//! The decoder in use reads ADTS and whole MP4 files but not the fragmented kind, so the pieces are unpacked
//! here: the sample sizes come from the `trun` boxes, the codec setup from the `esds` of the init segment.
//! Nothing is decoded or changed: the very same AAC frames, each with a 7-byte header in front.

/// One box: (type, payload start, end) at `at`, or None past the end / on a broken size.
fn read_box(d: &[u8], at: usize, end: usize) -> Option<([u8; 4], usize, usize)> {
    let head = d.get(at..at.checked_add(8)?)?;
    let size = u32::from_be_bytes(head[0..4].try_into().ok()?) as usize;
    let kind: [u8; 4] = head[4..8].try_into().ok()?;
    let (payload, total) = match size {
        0 => (at + 8, end - at),
        1 => {
            let big = u64::from_be_bytes(d.get(at + 8..at + 16)?.try_into().ok()?) as usize;
            (at + 16, big)
        }
        n => (at + 8, n),
    };
    let stop = at.checked_add(total)?;
    (total >= 8 && stop <= end).then_some((kind, payload, stop))
}

/// All the boxes of `kind` directly inside `d[from..to]`.
fn children(d: &[u8], from: usize, to: usize) -> impl Iterator<Item = ([u8; 4], usize, usize)> + '_ {
    let mut at = from;
    std::iter::from_fn(move || {
        let b = read_box(d, at, to)?;
        at = b.2;
        Some(b)
    })
}

fn find(d: &[u8], from: usize, to: usize, kind: &[u8; 4]) -> Option<(usize, usize)> {
    children(d, from, to).find(|b| &b.0 == kind).map(|b| (b.1, b.2))
}

/// The AudioSpecificConfig bytes inside an `esds` payload (descriptor 0x05).
fn audio_config(esds: &[u8]) -> Option<&[u8]> {
    // 4 bytes version/flags, then descriptors: tag, length (1-4 bytes of 7 bits), body
    let mut i = 4;
    let read_len = |i: &mut usize| -> Option<usize> {
        let mut n = 0usize;
        for _ in 0..4 {
            let b = *esds.get(*i)?;
            *i += 1;
            n = (n << 7) | (b & 0x7F) as usize;
            if b & 0x80 == 0 {
                return Some(n);
            }
        }
        None
    };
    while i < esds.len() {
        let tag = *esds.get(i)?;
        i += 1;
        let len = read_len(&mut i)?;
        match tag {
            0x03 => i += 3,  // ES_Descr: id (2) + flags (1), the next descriptor follows inside
            0x04 => i += 13, // DecoderConfigDescr: fixed part, the next descriptor follows inside
            0x05 => return esds.get(i..i + len),
            _ => i += len,
        }
    }
    None
}

/// (object type, sampling frequency index, channels) from an AudioSpecificConfig.
fn parse_config(c: &[u8]) -> Option<(u8, u8, u8)> {
    let (b0, b1) = (*c.first()?, *c.get(1)?);
    let object = b0 >> 3;
    let freq = ((b0 & 7) << 1) | (b1 >> 7);
    let channels = (b1 >> 3) & 0x0F;
    (freq < 13 && (1..=4).contains(&object) && (1..=7).contains(&channels)).then_some((object, freq, channels))
}

/// The codec setup of the first audio track of an init segment.
fn setup(d: &[u8]) -> Option<(u8, u8, u8)> {
    let moov = children(d, 0, d.len()).find(|b| &b.0 == b"moov")?;
    for trak in children(d, moov.1, moov.2).filter(|b| &b.0 == b"trak") {
        let (mdia, mdia_end) = find(d, trak.1, trak.2, b"mdia")?;
        let (minf, minf_end) = find(d, mdia, mdia_end, b"minf")?;
        let (stbl, stbl_end) = find(d, minf, minf_end, b"stbl")?;
        let (stsd, stsd_end) = find(d, stbl, stbl_end, b"stsd")?;
        // stsd: version/flags (4), entry count (4), then the sample entries
        let entry = read_box(d, stsd + 8, stsd_end)?;
        if &entry.0 != b"mp4a" {
            continue;
        }
        // an AudioSampleEntry: 6 reserved + 2 index + 8 + 2 channels + 2 size + 4 + 4 rate = 28 bytes, then the boxes
        if let Some((esds, esds_end)) = find(d, entry.1 + 28, entry.2, b"esds") {
            if let Some(cfg) = audio_config(d.get(esds..esds_end)?) {
                return parse_config(cfg);
            }
        }
    }
    None
}

fn adts_header(len: usize, object: u8, freq: u8, channels: u8) -> [u8; 7] {
    let total = len + 7;
    let profile = object.saturating_sub(1) & 3;
    [
        0xFF,
        0xF1,
        (profile << 6) | (freq << 2) | ((channels >> 2) & 1),
        ((channels & 3) << 6) | ((total >> 11) & 3) as u8,
        ((total >> 3) & 0xFF) as u8,
        (((total & 7) << 5) as u8) | 0x1F,
        0xFC,
    ]
}

/// The AAC frames of every fragment, in ADTS. None when this is not an AAC fragmented MP4.
pub fn to_adts(d: &[u8]) -> Option<Vec<u8>> {
    let (object, freq, channels) = setup(d)?;
    let mut out = Vec::with_capacity(d.len() + d.len() / 64);
    let mut frames = 0usize;
    for moof in children(d, 0, d.len()).filter(|b| &b.0 == b"moof") {
        let Some(traf) = children(d, moof.1, moof.2).find(|b| &b.0 == b"traf") else { continue };
        // defaults from tfhd
        let mut default_size = 0usize;
        let mut base: Option<usize> = None;
        if let Some((tfhd, _)) = find(d, traf.1, traf.2, b"tfhd") {
            let flags = u32::from_be_bytes([0, *d.get(tfhd + 1)?, *d.get(tfhd + 2)?, *d.get(tfhd + 3)?]);
            let mut p = tfhd + 8; // version/flags (4) + track id (4)
            if flags & 0x01 != 0 {
                base = Some(u64::from_be_bytes(d.get(p..p + 8)?.try_into().ok()?) as usize);
                p += 8;
            }
            if flags & 0x02 != 0 {
                p += 4;
            }
            if flags & 0x08 != 0 {
                p += 4;
            }
            if flags & 0x10 != 0 {
                default_size = u32::from_be_bytes(d.get(p..p + 4)?.try_into().ok()?) as usize;
            }
        }
        for trun in children(d, traf.1, traf.2).filter(|b| &b.0 == b"trun") {
            let flags = u32::from_be_bytes([0, *d.get(trun.1 + 1)?, *d.get(trun.1 + 2)?, *d.get(trun.1 + 3)?]);
            let count = u32::from_be_bytes(d.get(trun.1 + 4..trun.1 + 8)?.try_into().ok()?) as usize;
            let mut p = trun.1 + 8;
            let mut offset: Option<i32> = None;
            if flags & 0x01 != 0 {
                offset = Some(i32::from_be_bytes(d.get(p..p + 4)?.try_into().ok()?));
                p += 4;
            }
            if flags & 0x04 != 0 {
                p += 4;
            }
            // where the first sample is: relative to the moof (or to base-data-offset), else right after the mdat header
            let mut at = match offset {
                Some(o) => (base.unwrap_or(moof.1 - 8) as i64 + o as i64) as usize,
                None => children(d, moof.2, d.len()).find(|b| &b.0 == b"mdat")?.1,
            };
            for _ in 0..count {
                if flags & 0x100 != 0 {
                    p += 4;
                }
                let size = if flags & 0x200 != 0 {
                    let s = u32::from_be_bytes(d.get(p..p + 4)?.try_into().ok()?) as usize;
                    p += 4;
                    s
                } else {
                    default_size
                };
                if flags & 0x400 != 0 {
                    p += 4;
                }
                if flags & 0x800 != 0 {
                    p += 4;
                }
                let frame = d.get(at..at.checked_add(size)?)?;
                out.extend_from_slice(&adts_header(size, object, freq, channels));
                out.extend_from_slice(frame);
                at += size;
                frames += 1;
            }
        }
    }
    (frames > 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    /// An init segment with an AAC-LC 44.1 kHz stereo setup (AudioSpecificConfig 0x12 0x10).
    fn init() -> Vec<u8> {
        let esds = vec![0, 0, 0, 0, 0x03, 25, 0, 1, 0, 0x04, 17, 0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x05, 2, 0x12, 0x10, 0x06, 1, 2];
        let mut entry = vec![0u8; 28];
        entry.extend(boxed(b"esds", &esds));
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(boxed(b"mp4a", &entry));
        let stbl = boxed(b"stbl", &boxed(b"stsd", &stsd));
        let trak = boxed(b"trak", &boxed(b"mdia", &boxed(b"minf", &stbl)));
        let mut out = boxed(b"ftyp", b"isom\0\0\0\0");
        out.extend(boxed(b"moov", &trak));
        out
    }

    /// One fragment with these frames (trun: data offset + a size per sample).
    fn fragment(frames: &[&[u8]]) -> Vec<u8> {
        let mut trun = vec![0, 0, 0x02, 0x01]; // version 0, flags 0x000201: data offset + sample size
        trun.extend((frames.len() as u32).to_be_bytes());
        // the data offset is filled in once the moof size is known
        let off_at = trun.len();
        trun.extend([0, 0, 0, 0]);
        for f in frames {
            trun.extend((f.len() as u32).to_be_bytes());
        }
        let tfhd = boxed(b"tfhd", &[0, 0x02, 0, 0, 0, 0, 0, 1]);
        let moof_len = 8 + boxed(b"mfhd", &[0; 8]).len() + 8 + tfhd.len() + 8 + trun.len();
        let offset = (moof_len + 8) as i32;
        trun[off_at..off_at + 4].copy_from_slice(&offset.to_be_bytes());
        let traf = boxed(b"traf", &[tfhd, boxed(b"trun", &trun)].concat());
        let mut out = boxed(b"moof", &[boxed(b"mfhd", &[0; 8]), traf].concat());
        assert_eq!(out.len(), moof_len);
        out.extend(boxed(b"mdat", &frames.concat()));
        out
    }

    #[test]
    fn unpacks_fragments_into_adts() {
        let mut d = init();
        d.extend(fragment(&[&[1, 2, 3], &[4, 5]]));
        d.extend(fragment(&[&[6; 10]]));
        let a = to_adts(&d).expect("an AAC stream");
        // three frames, each with a 7-byte header
        assert_eq!(a.len(), 7 + 3 + 7 + 2 + 7 + 10);
        assert_eq!(&a[0..2], &[0xFF, 0xF1]);
        // AAC-LC (profile 1), 44.1 kHz (index 4), stereo
        assert_eq!(a[2], (1 << 6) | (4 << 2));
        assert_eq!(a[3] >> 6, 2);
        let len = (((a[3] & 3) as usize) << 11) | ((a[4] as usize) << 3) | (a[5] >> 5) as usize;
        assert_eq!(len, 10);
        assert_eq!(&a[7..10], &[1, 2, 3]);
    }

    #[test]
    fn not_aac_is_none() {
        assert!(to_adts(b"ID3\x04\0\0\0\0\0\0").is_none());
        assert!(to_adts(&[]).is_none());
    }
}
