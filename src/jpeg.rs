//! JPEG segment surgery: walk markers, report metadata, strip it losslessly.

use crate::tiff::{parse_tiff, ExifInfo};

/// Metadata payload kinds inside APP segments.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MetaInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_jfif: bool,
    pub has_exif: bool,
    pub has_xmp: bool,
    pub has_photoshop: bool,
    pub has_icc: bool,
    pub has_comment: bool,
    pub exif: Option<ExifInfo>,
}

/// Options controlling what gets removed.
#[derive(Debug, Clone, Default)]
pub struct StripOptions {
    /// remove ICC color profiles (APP2)
    pub strip_icc: bool,
    /// remove every APPn segment except APP0/JFIF, plus all comments
    pub aggressive: bool,
}

const XMP_PREFIX: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const EXIF_PREFIX: &[u8] = b"Exif\0\0";
const PHOTOSHOP_PREFIX: &[u8] = b"Photoshop 3.0\0";

/// Walk top-level markers, invoking `visit(marker, seg_start, seg_end)` for
/// every segment (including its 2-byte length field). Entropy data after SOS
/// is skipped; scanning stops at EOI.
pub fn walk_segments<F>(data: &[u8], mut visit: F) -> Result<(), String>
where
    F: FnMut(u8, usize, usize),
{
    let mut pos = 2usize;
    loop {
        if pos + 1 >= data.len() {
            return Ok(()); // truncated tail: caller decides if that matters
        }
        if data[pos] != 0xff {
            return Err(format!("expected marker at byte {pos}"));
        }
        let marker = data[pos + 1];
        if marker == 0xff {
            pos += 1; // fill byte
            continue;
        }
        match marker {
            0xd8 => pos += 2, // SOI (shouldn't reappear)
            0xd9 => return Ok(()),
            0x01 | 0xd0..=0xd7 => pos += 2, // standalone markers
            _ => {
                if pos + 4 > data.len() {
                    return Ok(());
                }
                let len = usize::from(data[pos + 2]) << 8 | usize::from(data[pos + 3]);
                if len < 2 || pos + 2 + len > data.len() {
                    return Err(format!("bad segment length at byte {pos}"));
                }
                visit(marker, pos, pos + 2 + len);
                pos += 2 + len;
                if marker == 0xda {
                    // skip entropy: scan for the next real marker
                    let mut j = pos;
                    while j + 1 < data.len() {
                        if data[j] == 0xff && data[j + 1] != 0x00 {
                            if data[j + 1] == 0xff {
                                j += 1;
                                continue;
                            }
                            break;
                        }
                        j += 1;
                    }
                    pos = j;
                }
            }
        }
        if marker == 0xd9 {
            break;
        }
    }
    Ok(())
}

fn is_exif(payload: &[u8]) -> bool {
    payload.starts_with(EXIF_PREFIX)
}

fn is_xmp(payload: &[u8]) -> bool {
    payload.starts_with(XMP_PREFIX)
}

fn is_photoshop(payload: &[u8]) -> bool {
    payload.starts_with(PHOTOSHOP_PREFIX)
}

fn dims_from_sof(payload: &[u8]) -> Option<(u32, u32)> {
    if payload.len() < 6 {
        return None;
    }
    let h = u32::from(payload[1]) << 8 | u32::from(payload[2]);
    let w = u32::from(payload[3]) << 8 | u32::from(payload[4]);
    Some((w, h))
}

/// Collect metadata facts about a JPEG file.
pub fn info(data: &[u8]) -> Result<MetaInfo, String> {
    let mut meta = MetaInfo::default();
    walk_segments(data, |marker, seg_start, seg_end| {
        // payload = after the 2 length bytes
        let payload = &data[seg_start + 4..seg_end];
        match marker {
            0xe0 => meta.has_jfif = payload.starts_with(b"JFIF"),
            0xe1 => {
                if is_exif(payload) {
                    meta.has_exif = true;
                    // payload = "Exif\0\0" (6 bytes) + TIFF
                    if payload.len() > 6 {
                        meta.exif = parse_tiff(&payload[6..]);
                    }
                } else if is_xmp(payload) {
                    meta.has_xmp = true;
                }
            }
            0xed => meta.has_photoshop = is_photoshop(payload),
            0xe2 => {
                if payload.starts_with(b"ICC_PROFILE") {
                    meta.has_icc = true;
                }
            }
            0xfe => meta.has_comment = true,
            0xc0 | 0xc1 => {
                if let Some((w, h)) = dims_from_sof(&data[seg_start + 4..seg_end]) {
                    meta.width = Some(w);
                    meta.height = Some(h);
                }
            }
            _ => {}
        }
    })?;
    Ok(meta)
}

/// Rebuild the JPEG without the metadata segments selected by `opts`.
/// The image data (DQT/SOF/DHT/SOS/entropy) is copied byte-exact.
pub fn strip(data: &[u8], opts: &StripOptions) -> Result<Vec<u8>, String> {
    if data.len() < 4 || data[0] != 0xff || data[1] != 0xd8 {
        return Err("not a JPEG (missing SOI)".into());
    }
    let mut out: Vec<u8> = data[..2].to_vec(); // SOI
    let mut pos = 2usize;
    while pos + 1 < data.len() {
        if data[pos] != 0xff {
            return Err(format!("expected marker at byte {pos}"));
        }
        let marker = data[pos + 1];
        if marker == 0xff {
            pos += 1;
            continue;
        }
        match marker {
            0xd9 => {
                out.extend_from_slice(&data[pos..pos + 2]);
                return Ok(out);
            }
            0x01 | 0xd0..=0xd7 => {
                out.extend_from_slice(&data[pos..pos + 2]);
                pos += 2;
            }
            _ => {
                if pos + 4 > data.len() {
                    // truncated tail: keep what's there
                    out.extend_from_slice(&data[pos..]);
                    return Ok(out);
                }
                let len = usize::from(data[pos + 2]) << 8 | usize::from(data[pos + 3]);
                let seg_end = (pos + 2 + len).min(data.len());
                let payload = &data[pos + 4..seg_end];
                let drop = match marker {
                    0xe0 => false,
                    0xe1 => is_exif(payload) || is_xmp(payload),
                    0xed => is_photoshop(payload),
                    0xe2 => opts.strip_icc && payload.starts_with(b"ICC_PROFILE"),
                    0xfe => true,
                    0xe3..=0xef => opts.aggressive,
                    _ => false,
                };
                if !drop {
                    out.extend_from_slice(&data[pos..seg_end]);
                }
                pos = seg_end;
                if marker == 0xda {
                    // copy entropy until the next real marker
                    let mut j = pos;
                    while j + 1 < data.len() {
                        if data[j] == 0xff && data[j + 1] != 0x00 {
                            if data[j + 1] == 0xff {
                                j += 1;
                                continue;
                            }
                            break;
                        }
                        j += 1;
                    }
                    out.extend_from_slice(&data[pos..j]);
                    pos = j;
                }
            }
        }
    }
    if !out.ends_with(&[0xff, 0xd9]) {
        out.extend_from_slice(&[0xff, 0xd9]);
    }
    Ok(out)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_jpeg(payload_extra: &[(u8, &[u8])]) -> Vec<u8> {
        let mut v = vec![0xff, 0xd8];
        for (marker, payload) in payload_extra {
            v.push(0xff);
            v.push(*marker);
            let len = payload.len() + 2;
            v.extend_from_slice(&(len as u16).to_be_bytes());
            v.extend_from_slice(payload);
        }
        // minimal SOF0: 8x4, 1 component
        v.extend_from_slice(&[
            0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x04, 0x00, 0x08, 0x01, 0x00, 0x11, 0x00,
        ]);
        // SOS + entropy + EOI
        v.extend_from_slice(&[0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00]);
        v.extend_from_slice(&[0x12, 0x34, 0x56]);
        v.extend_from_slice(&[0xff, 0xd9]);
        v
    }

    const COMMENT: &[u8] = b"created by a spy camera";

    #[test]
    fn info_detects_metadata() {
        let j = minimal_jpeg(&[(0xfe, COMMENT), (0xed, PHOTOSHOP_PREFIX)]);
        let meta = info(&j).unwrap();
        assert!(meta.has_comment);
        assert!(meta.has_photoshop);
        assert_eq!(meta.width, Some(8));
        assert_eq!(meta.height, Some(4));
    }

    #[test]
    fn strip_removes_comments_and_photoshop() {
        let j = minimal_jpeg(&[(0xfe, COMMENT), (0xed, PHOTOSHOP_PREFIX)]);
        let cleaned = strip(&j, &StripOptions::default()).unwrap();
        let meta = info(&cleaned).unwrap();
        assert!(!meta.has_comment);
        assert!(!meta.has_photoshop);
        assert_eq!(meta.width, Some(8));
        assert!(cleaned.windows(3).any(|w| w == [0x12, 0x34, 0x56]));
    }

    #[test]
    fn strip_keeps_jfif() {
        let j = minimal_jpeg(&[(0xe0, b"JFIF\0\0"), (0xfe, COMMENT)]);
        let cleaned = strip(&j, &StripOptions::default()).unwrap();
        assert!(info(&cleaned).unwrap().has_jfif);
    }

    #[test]
    fn non_metadata_survives_byte_exact() {
        let j = minimal_jpeg(&[]);
        let cleaned = strip(&j, &StripOptions::default()).unwrap();
        assert_eq!(cleaned, j);
    }

    #[test]
    fn entropy_survives_multi_scan_style_tail() {
        // entropy bytes containing 0xff 0x00 stuffing
        let mut j = minimal_jpeg(&[]);
        let eoi = j.split_off(j.len() - 2);
        j.extend_from_slice(&[0xab, 0xff, 0x00, 0xcd]);
        j.extend_from_slice(&eoi);
        let cleaned = strip(&j, &StripOptions::default()).unwrap();
        assert!(cleaned.windows(4).any(|w| w == [0xab, 0xff, 0x00, 0xcd]));
        assert!(cleaned.ends_with(&[0xff, 0xd9]));
    }

    #[test]
    fn garbage_rejected() {
        assert!(info(b"nope").is_err());
        assert!(strip(&[0xff, 0xd8, 0x00], &StripOptions::default()).is_err());
    }
}
