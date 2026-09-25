# exif-clean

**Privacy-first metadata stripper for photos — pure Rust, zero dependencies, lossless.**

[![CI](https://github.com/efealtiparmakoglu/exif-clean/actions/workflows/ci.yml/badge.svg)](https://github.com/efealtiparmakoglu/exif-clean/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.82%2B-orange.svg)](https://www.rust-lang.org)

```
$ exif-clean --info vacation.jpg
vacation.jpg
  size: 4032x3024
  jfif: yes
  exif: yes
  xmp: no
  photoshop: no
  icc profile: no
  comment: no
  camera make: Apple
  camera model: iPhone 15 Pro
  orientation: 6
  gps: 41.025000, 29.016667  <-- location leaked!
  altitude: 120.0 m

$ exif-clean vacation.jpg
cleaned vacation.cleaned.jpg (18432 metadata bytes removed)
```

Shows you exactly what a photo leaks (location, camera, timestamps,
editing software) and strips it without re-encoding the image — the
pixels stay bit-identical.

## Why exif-clean

- **Lossless by construction.** JPEG segment surgery: metadata segments
  (EXIF APP1, XMP APP1, Photoshop APP13, comments) are removed and the
  compressed entropy data is copied byte-for-byte. PNG: text/eXIf/tIME
  chunks dropped, CRCs recomputed, image data untouched.
- **Reads the scary parts.** A minimal TIFF/EXIF parser surfaces GPS
  coordinates (decimal degrees, hemisphere-aware), altitude, camera
  make/model, orientation, capture date and editing software — so
  `--info` tells you what you're about to publish.
- **Conservative by default.** JFIF (APP0) and color profiles survive —
  only tracking-adjacent metadata goes. `--strip-icc` and
  `--aggressive` opt into more.
- **Safe defaults.** Without flags the input is never touched: output
  goes to `*.cleaned.*` next to the input (or `--out-dir`). `-i`
  overwrites explicitly.
- **Zero dependencies.** Segment walking, TIFF/IFD parsing, CRC-32 —
  all hand-rolled.

## Usage

```sh
exif-clean --info photo.jpg            # report only
exif-clean --info --json photo.jpg     # machine-readable report
exif-clean photo.jpg                   # -> photo.cleaned.jpg
exif-clean -i photo.png                # strip text/eXIf/tIME in place
exif-clean --strip-icc -i *.jpg        # also drop color profiles
exif-clean --aggressive -o clean/ .    # nuke every APPn / ancillary chunk
```

Exit codes: 0 ok, 1 some/all files failed, 2 usage error.

## Known limitations

- JPEG and PNG only (no HEIC/WebP/TIFF input).
- The EXIF reader covers the privacy-relevant subset (IFD0, GPS IFD);
  exotic maker notes are stripped, not parsed.
- Orientation is reported but not baked into pixels — strip only if your
  publishing pipeline handles orientation.

## Development

```sh
cargo test
cargo clippy --all-targets
```

15 tests cover the TIFF/IFD walker (crafted GPS/Make blobs, garbage
rejection), JPEG segment walking (metadata detection, entropy stuffing
survival, comment/Photoshop stripping, byte-exact no-op), PNG chunk
surgery (CRC validity after strip, byte-identical clean files) and the
CLI behavior.

## License

MIT — see [LICENSE](LICENSE).
