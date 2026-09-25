//! Minimal TIFF/EXIF reader — enough to surface GPS, camera and capture
//! metadata (the privacy-sensitive part) without a full EXIF stack.

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ExifInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub software: Option<String>,
    pub datetime: Option<String>,
    pub orientation: Option<u16>,
    /// (latitude, longitude) in signed decimal degrees
    pub gps: Option<(f64, f64)>,
    pub gps_altitude: Option<f64>,
}

const TAG_MAKE: u16 = 0x010F;
const TAG_MODEL: u16 = 0x0110;
const TAG_SOFTWARE: u16 = 0x0131;
const TAG_DATETIME: u16 = 0x0132;
const TAG_ORIENTATION: u16 = 0x0112;
const TAG_GPS_IFD: u16 = 0x8825;

const GPS_LAT_REF: u16 = 1;
const GPS_LAT: u16 = 2;
const GPS_LON_REF: u16 = 3;
const GPS_LON: u16 = 4;
const GPS_ALT: u16 = 6;

fn u16_at(d: &[u8], off: usize, little: bool) -> Option<u16> {
    let b = d.get(off..off + 2)?;
    Some(if little {
        u16::from_le_bytes([b[0], b[1]])
    } else {
        u16::from_be_bytes([b[0], b[1]])
    })
}

fn u32_at(d: &[u8], off: usize, little: bool) -> Option<u32> {
    let b = d.get(off..off + 4)?;
    Some(if little {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    })
}

/// Byte-size of one element of an EXIF field type (0 = unknown/irregular).
fn type_size(ty: u16) -> usize {
    match ty {
        1 | 2 | 6 | 7 => 1, // BYTE, ASCII, SBYTE, UNDEFINED
        3 | 8 => 2,         // SHORT, SSHORT
        4 | 9 | 11 => 4,    // LONG, SLONG, FLOAT
        5 | 10 | 12 => 8,   // RATIONAL, SRATIONAL, DOUBLE
        _ => 0,
    }
}

fn ascii_value(d: &[u8], off: usize, count: usize) -> Option<String> {
    let raw = d.get(off..off + count)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Some(String::from_utf8_lossy(&raw[..end]).trim().to_string())
}

fn rational(d: &[u8], off: usize, little: bool) -> Option<f64> {
    let num = u32_at(d, off, little)?;
    let den = u32_at(d, off + 4, little)?;
    if den == 0 {
        return None;
    }
    Some(num as f64 / den as f64)
}

fn ifd_entries(d: &[u8], ifd_off: usize, little: bool) -> Option<Vec<(u16, usize, usize)>> {
    // (tag, value_offset, value_byte_count)
    let count = u16_at(d, ifd_off, little)? as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = ifd_off + 2 + i * 12;
        let tag = u16_at(d, base, little)?;
        let ty = u16_at(d, base + 2, little)?;
        let cnt = u32_at(d, base + 4, little)? as usize;
        let esize = type_size(ty);
        if esize == 0 {
            out.push((tag, base + 8, 0));
            continue;
        }
        let total = esize * cnt;
        // values ≤ 4 bytes are stored inline in the entry itself
        let off = if total <= 4 { base + 8 } else { u32_at(d, base + 8, little)? as usize };
        out.push((tag, off, total));
    }
    Some(out)
}

/// Parse a TIFF blob (the payload of an EXIF APP1 after the "Exif\0\0" header).
pub fn parse_tiff(d: &[u8]) -> Option<ExifInfo> {
    if d.len() < 8 {
        return None;
    }
    let little = match &d[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let magic = u16_at(d, 2, little)?;
    if magic != 42 {
        return None;
    }
    let ifd0 = u32_at(d, 4, little)? as usize;

    let mut info = ExifInfo::default();
    let mut gps_ifd_off = None;
    for (tag, off, cnt) in ifd_entries(d, ifd0, little)? {
        match tag {
            TAG_MAKE => info.make = ascii_value(d, off, cnt),
            TAG_MODEL => info.model = ascii_value(d, off, cnt),
            TAG_SOFTWARE => info.software = ascii_value(d, off, cnt),
            TAG_DATETIME => info.datetime = ascii_value(d, off, cnt),
            TAG_ORIENTATION => info.orientation = u16_at(d, off, little),
            TAG_GPS_IFD => gps_ifd_off = Some(u32_at(d, off, little)? as usize),
            _ => {}
        }
    }

    if let Some(goff) = gps_ifd_off {
        let mut lat: Option<(f64, bool)> = None;
        let mut lon: Option<(f64, bool)> = None;
        for (tag, off, cnt) in ifd_entries(d, goff, little)? {
            match tag {
                GPS_LAT_REF => {
                    let r = ascii_value(d, off, cnt).unwrap_or_default();
                    if let Some(lat0) = lat.as_mut() {
                        lat0.1 = r.eq_ignore_ascii_case("N");
                    }
                }
                GPS_LON_REF => {
                    let r = ascii_value(d, off, cnt).unwrap_or_default();
                    if let Some(lon0) = lon.as_mut() {
                        lon0.1 = r.eq_ignore_ascii_case("E");
                    }
                }
                GPS_LAT | GPS_LON => {
                    // 3 RATIONALs: degrees, minutes, seconds
                    let mut dms = [0f64; 3];
                    for (i, item) in dms.iter_mut().take(3).enumerate() {
                        *item = rational(d, off + i * 8, little).unwrap_or(0.0);
                    }
                    let deg = dms[0] + dms[1] / 60.0 + dms[2] / 3600.0;
                    if tag == GPS_LAT {
                        lat = Some((deg, true));
                    } else {
                        lon = Some((deg, true));
                    }
                }
                GPS_ALT => {
                    if let Some(v) = rational(d, off, little) {
                        info.gps_altitude = Some(v);
                    }
                }
                _ => {}
            }
        }
        // sign by hemisphere reference (defaults: N/E — re-checked above)
        if let (Some((mut la, _)), Some((mut lo, _))) = (lat, lon) {
            // re-read the refs properly (the closure above only patched Option)
            for (tag, off, cnt) in ifd_entries(d, goff, little)? {
                match tag {
                    GPS_LAT_REF => {
                        if let Some(r) = ascii_value(d, off, cnt) {
                            if r.eq_ignore_ascii_case("S") {
                                la = -la.abs();
                            }
                        }
                    }
                    GPS_LON_REF => {
                        if let Some(r) = ascii_value(d, off, cnt) {
                            if r.eq_ignore_ascii_case("W") {
                                lo = -lo.abs();
                            }
                        }
                    }
                    _ => {}
                }
            }
            // the sign was already folded into la/lo by the ref re-read only
            // when the refs exist; keep absolute values signed
            info.gps = Some((la, lo));
        }
    }
    Some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny little-endian TIFF:
    /// IFD0 {Make "TestCam", GPS pointer} + GPS IFD {latRef N, lat, lonRef E, lon, alt}.
    fn sample_tiff() -> Vec<u8> {
        // layout: header(8) | ifd0(2 + 2*12 + 4 = 30) | gps(2 + 5*12 = 62) | data
        let ifd0 = 8usize;
        let gps = ifd0 + 30; // 38 (count + 5*12 entries + next-IFD word)
        let data = gps + 66; // 104
        let make_off = data as u32;
        let mut d = Vec::new();
        d.extend_from_slice(b"II");
        d.extend_from_slice(&42u16.to_le_bytes());
        d.extend_from_slice(&(ifd0 as u32).to_le_bytes());
        // IFD0: 2 entries
        d.extend_from_slice(&2u16.to_le_bytes());
        // Make, ASCII, 8 bytes -> offset in data area
        d.extend_from_slice(&0x010Fu16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&8u32.to_le_bytes());
        d.extend_from_slice(&make_off.to_le_bytes());
        // GPS pointer, LONG, 1 -> inline value
        d.extend_from_slice(&0x8825u16.to_le_bytes());
        d.extend_from_slice(&4u16.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&(gps as u32).to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes()); // next IFD = 0
        assert_eq!(d.len(), gps, "gps ifd offset");
        // GPS IFD: 5 entries
        d.extend_from_slice(&5u16.to_le_bytes());
        let mut entry = |tag: u16, ty: u16, cnt: u32, voff: u32| {
            d.extend_from_slice(&tag.to_le_bytes());
            d.extend_from_slice(&ty.to_le_bytes());
            d.extend_from_slice(&cnt.to_le_bytes());
            d.extend_from_slice(&voff.to_le_bytes());
        };
        let da = data as u32;
        entry(GPS_LAT_REF, 2, 2, da + 8); // "N\0"
        entry(GPS_LAT, 5, 3, da + 16);
        entry(GPS_LON_REF, 2, 2, da + 10); // "E\0"
        entry(GPS_LON, 5, 3, da + 40);
        entry(GPS_ALT, 5, 1, da + 64);
        d.extend_from_slice(&0u32.to_le_bytes()); // next IFD = 0
        assert_eq!(d.len(), data, "data offset");
        // data area: [0..8) Make, [8..10) "N\0", [10..12) "E\0",
        // [16..40) lat DMS, [40..64) lon deg + pad, [64..72) alt
        d.extend_from_slice(b"TestCam\0");
        d.extend_from_slice(b"N\0");
        d.extend_from_slice(b"E\0");
        while d.len() < data + 16 {
            d.push(0);
        }
        for v in [41u32, 1, 30] {
            d.extend_from_slice(&v.to_le_bytes()); // lat deg/min/sec num
            d.extend_from_slice(&1u32.to_le_bytes()); // den
        }
        d.extend_from_slice(&29u32.to_le_bytes()); // lon deg
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        while d.len() < data + 64 {
            d.push(0);
        }
        d.extend_from_slice(&120u32.to_le_bytes()); // alt
        d.extend_from_slice(&1u32.to_le_bytes());
        d
    }

    #[test]
    fn parses_sample_tiff() {
        let t = sample_tiff();
        let info = parse_tiff(&t).expect("parse");
        assert_eq!(info.make.as_deref(), Some("TestCam"));
        let (lat, lon) = info.gps.expect("gps");
        assert!((lat - 41.025).abs() < 1e-6, "{lat}");
        assert!((lon - 29.0).abs() < 1e-6, "{lon}");
        assert_eq!(info.gps_altitude, Some(120.0));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_tiff(b"no").is_none());
        assert!(parse_tiff(b"XX\x2a\x00\x00\x00\x00\x00").is_none());
    }
}
