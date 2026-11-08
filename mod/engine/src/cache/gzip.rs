// baritone-rs engine: gzip around miniz_oxide's deflate. Written for this project. LGPL-3.0-or-later, see lib.rs.
//
// Upstream's region files are `GZIPOutputStream` streams: a ten byte header, a raw deflate stream, the CRC-32 and the size of the data. This reads any
// gzip member (the optional header fields are skipped) and writes the plain header Java writes.

fn crc_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    })
}

/// CRC-32 (IEEE), as gzip writes it.
pub fn crc32(data: &[u8]) -> u32 {
    let t = crc_table();
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c = t[((c ^ u32::from(*b)) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// A gzip member holding `data`.
pub fn compress(data: &[u8], level: u8) -> Vec<u8> {
    let body = miniz_oxide::deflate::compress_to_vec(data, level);
    let mut out = Vec::with_capacity(body.len() + 18);
    out.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// The data of a gzip member, at most `limit` bytes of it. An error says what is wrong with the file.
pub fn decompress(bytes: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if bytes.len() < 18 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Err("not a gzip file".to_string());
    }
    if bytes[2] != 8 {
        return Err("unknown gzip compression method".to_string());
    }
    let flags = bytes[3];
    let mut at = 10usize;
    let need = |at: usize, n: usize| if at + n <= bytes.len() { Ok(()) } else { Err("truncated gzip header".to_string()) };
    if flags & 4 != 0 {
        need(at, 2)?;
        let n = usize::from(bytes[at]) | usize::from(bytes[at + 1]) << 8;
        at += 2;
        need(at, n)?;
        at += n;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            while at < bytes.len() && bytes[at] != 0 {
                at += 1;
            }
            at += 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    if at + 8 > bytes.len() {
        return Err("truncated gzip file".to_string());
    }
    let body = &bytes[at..bytes.len() - 8];
    let data = miniz_oxide::inflate::decompress_to_vec_with_limit(body, limit).map_err(|e| format!("bad deflate data: {:?}", e.status))?;
    let tail = &bytes[bytes.len() - 8..];
    let crc = u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]);
    let size = u32::from_le_bytes([tail[4], tail[5], tail[6], tail[7]]);
    if crc != crc32(&data) || size != data.len() as u32 {
        return Err("gzip checksum mismatch".to_string());
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn it_round_trips_and_rejects_damage() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let z = compress(&data, 6);
        assert!(z.len() < data.len());
        assert_eq!(decompress(&z, 1 << 20).unwrap(), data);
        let mut bad = z.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0x55;
        assert!(decompress(&bad, 1 << 20).is_err());
        assert!(decompress(&z[..z.len() - 3], 1 << 20).is_err());
        assert!(decompress(&z, 100).is_err(), "the limit holds");
        assert!(decompress(b"plain text, not gzip at all", 100).is_err());
    }

    #[test]
    fn it_reads_a_member_from_another_encoder() {
        // gzip of "hello" as zlib's gzip writes it: header, a fixed Huffman block, the crc and the size
        let z: [u8; 25] = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00, 0x86, 0xa6, 0x10, 0x36, 5, 0, 0, 0];
        assert_eq!(decompress(&z, 100).unwrap(), b"hello");
    }
}
