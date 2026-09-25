//! PNG metadata stripping: drop text/eXIf/tIME chunks, keep the image data.

/// CRC-32 (ISO 3309 / PNG polynomial).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = crc & 1;
            crc >>= 1;
            if mask == 1 {
                crc ^= 0xedb8_8320;
            }
        }
    }
    !crc
}

/// Chunks that carry metadata (text, timestamps, EXIF) — always removed.
fn is_metadata_chunk(tag: &[u8; 4]) -> bool {
    matches!(
        tag,
        b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"tIME" | b"pCAL" | b"sCAL" | b"gIFg" | b"gIFx"
    )
}

/// Rebuild a PNG without metadata chunks. Critical chunks (IHDR, PLTE,
/// IDAT, IEND) and color/transparency data always survive; CRCs of all
/// remaining chunks are recomputed.
pub fn strip(data: &[u8]) -> Result<Vec<u8>, String> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if data.len() < 8 || data[..8] != SIG {
        return Err("not a PNG (bad signature)".into());
    }
    let mut out: Vec<u8> = Vec::with_capacity(data.len());
    out.extend_from_slice(&SIG);
    let mut pos = 8usize;
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
            as usize;
        let tag = [
            data[pos + 4],
            data[pos + 5],
            data[pos + 6],
            data[pos + 7],
        ];
        let chunk_end = pos + 12 + len;
        if chunk_end > data.len() {
            return Err("truncated PNG chunk".into());
        }
        if tag == *b"IEND" {
            out.extend_from_slice(&data[pos..chunk_end]);
            return Ok(out);
        }
        if !is_metadata_chunk(&tag) {
            let body = &data[pos + 8..pos + 8 + len];
            let crc = crc32_unfinished(body, crc32(&tag) ^ 0xffff_ffff);
            out.extend_from_slice(&(len as u32).to_be_bytes());
            out.extend_from_slice(&tag);
            out.extend_from_slice(body);
            out.extend_from_slice(&crc.to_be_bytes());
        }
        pos = chunk_end;
    }
    Err("PNG: missing IEND".into())
}

/// Continue a CRC over `data` from a running (pre-inverted) state.
fn crc32_unfinished(data: &[u8], mut crc: u32) -> u32 {
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = crc & 1;
            crc >>= 1;
            if mask == 1 {
                crc ^= 0xedb8_8320;
            }
        }
    }
    !crc
}

/// Metadata facts about a PNG file.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PngInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub text_chunks: usize,
    pub has_time: bool,
    pub has_exif: bool,
}

/// Walk chunks and collect metadata facts.
pub fn info(data: &[u8]) -> Result<PngInfo, String> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if data.len() < 8 || data[..8] != SIG {
        return Err("not a PNG (bad signature)".into());
    }
    let mut meta = PngInfo::default();
    let mut pos = 8usize;
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
            as usize;
        let tag = [
            data[pos + 4],
            data[pos + 5],
            data[pos + 6],
            data[pos + 7],
        ];
        match &tag {
            b"IHDR" if len >= 8 => {
                meta.width = Some(u32::from_be_bytes([
                    data[pos + 8],
                    data[pos + 9],
                    data[pos + 10],
                    data[pos + 11],
                ]));
                meta.height = Some(u32::from_be_bytes([
                    data[pos + 12],
                    data[pos + 13],
                    data[pos + 14],
                    data[pos + 15],
                ]));
            }
            b"tEXt" | b"zTXt" | b"iTXt" => meta.text_chunks += 1,
            b"tIME" => meta.has_time = true,
            b"eXIf" => meta.has_exif = true,
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(body.len() as u32).to_be_bytes());
        v.extend_from_slice(tag);
        v.extend_from_slice(body);
        v.extend_from_slice(&crc32(&[tag, body].concat()).to_be_bytes());
        v
    }

    fn sample_png(text: Option<&[u8]>) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        v.extend_from_slice(&chunk(b"IHDR", &[0, 0, 0, 8, 0, 0, 0, 4, 8, 2, 0, 0, 0]));
        if let Some(t) = text {
            v.extend_from_slice(&chunk(b"tEXt", t));
            v.extend_from_slice(&chunk(b"tIME", &[7, 230, 9, 25, 1, 2, 3]));
        }
        v.extend_from_slice(&chunk(b"IDAT", &[1, 2, 3, 4]));
        v.extend_from_slice(&chunk(b"IEND", &[]));
        v
    }

    #[test]
    fn info_counts_metadata() {
        let png = sample_png(Some(b"Comment\0made in secret"));
        let meta = info(&png).unwrap();
        assert_eq!(meta.width, Some(8));
        assert_eq!(meta.height, Some(4));
        assert_eq!(meta.text_chunks, 1);
        assert!(meta.has_time);
    }

    #[test]
    fn strip_removes_text_and_time() {
        let png = sample_png(Some(b"Comment\0made in secret"));
        let cleaned = strip(&png).unwrap();
        let meta = info(&cleaned).unwrap();
        assert_eq!(meta.text_chunks, 0);
        assert!(!meta.has_time);
        // image data intact
        assert!(cleaned.windows(4).any(|w| w == [1, 2, 3, 4]));
    }

    #[test]
    fn strip_preserves_crc_validity() {
        // verify every remaining chunk's CRC
        let png = sample_png(Some(b"Author\0me"));
        let cleaned = strip(&png).unwrap();
        let mut pos = 8usize;
        while pos + 8 <= cleaned.len() {
            let len = u32::from_be_bytes([
                cleaned[pos],
                cleaned[pos + 1],
                cleaned[pos + 2],
                cleaned[pos + 3],
            ]) as usize;
            let tag = [
                cleaned[pos + 4],
                cleaned[pos + 5],
                cleaned[pos + 6],
                cleaned[pos + 7],
            ];
            let body_end = pos + 8 + len;
            let crc = u32::from_be_bytes([
                cleaned[body_end],
                cleaned[body_end + 1],
                cleaned[body_end + 2],
                cleaned[body_end + 3],
            ]);
            let mut cat = tag.to_vec();
            cat.extend_from_slice(&cleaned[pos + 8..body_end]);
            assert_eq!(crc, crc32(&cat));
            if &tag == b"IEND" {
                break;
            }
            pos = body_end + 4;
        }
    }

    #[test]
    fn clean_file_is_byte_identical() {
        let png = sample_png(None);
        assert_eq!(strip(&png).unwrap(), png);
    }

    #[test]
    fn garbage_rejected() {
        assert!(strip(b"no").is_err());
        assert!(strip(&[137, 80, 78, 71, 13, 10, 26, 10]).is_err());
    }
}
