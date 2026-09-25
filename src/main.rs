//! exif-clean CLI — strip location/camera metadata from JPEG and PNG files.

use exif_clean::{jpeg, png};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
exif-clean — strip location & camera metadata from photos (JPEG / PNG)

USAGE:
  exif-clean [OPTIONS] FILE...

OPTIONS:
  --info              only show what metadata was found, don't clean
  -i, --inplace       overwrite the input file (default: write *.cleaned.*)
  -o, --out-dir <DIR> write cleaned files into DIR
  --strip-icc         also remove color profiles (may shift colors slightly)
  --aggressive        remove every APPn segment / ancillary chunk
  --json              machine-readable info output (with --info)
  --quiet             suppress the per-file report
  -h, --help          show this help
  -V, --version       show version

EXAMPLES:
  exif-clean --info vacation.jpg     # show EXIF/GPS before deciding
  exif-clean vacation.jpg            # -> vacation.cleaned.jpg
  exif-clean -i photos/*.png         # overwrite in place
";

struct Args {
    files: Vec<PathBuf>,
    info_only: bool,
    inplace: bool,
    out_dir: Option<PathBuf>,
    strip_icc: bool,
    aggressive: bool,
    json: bool,
    quiet: bool,
}

enum Outcome {
    Run(Args),
    Print(String),
}

fn usage_error(msg: &str) -> String {
    format!("exif-clean: {msg}\nrun `exif-clean --help` for usage")
}

fn parse_args<I: Iterator<Item = String>>(mut args: I) -> Result<Outcome, String> {
    let mut out = Args {
        files: Vec::new(),
        info_only: false,
        inplace: false,
        out_dir: None,
        strip_icc: false,
        aggressive: false,
        json: false,
        quiet: false,
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(Outcome::Print(HELP.to_string())),
            "-V" | "--version" => return Ok(Outcome::Print(format!("exif-clean {VERSION}\n"))),
            "--info" => out.info_only = true,
            "-i" | "--inplace" => out.inplace = true,
            "-o" | "--out-dir" => {
                let v = args
                    .next()
                    .ok_or_else(|| usage_error("--out-dir needs a value"))?;
                out.out_dir = Some(PathBuf::from(v));
            }
            "--strip-icc" => out.strip_icc = true,
            "--aggressive" => out.aggressive = true,
            "--json" => out.json = true,
            "--quiet" => out.quiet = true,
            other => {
                if other.starts_with('-') && other != "-" {
                    return Err(usage_error(&format!("unknown option {other:?}")));
                }
                out.files.push(PathBuf::from(other));
            }
        }
    }
    if out.files.is_empty() {
        return Err(usage_error("no input files"));
    }
    if out.inplace && out.out_dir.is_some() {
        return Err(usage_error("--inplace and --out-dir are mutually exclusive"));
    }
    Ok(Outcome::Run(out))
}

enum Kind {
    Jpeg,
    Png,
}

fn sniff(data: &[u8]) -> Option<Kind> {
    if data.len() >= 3 && data[0] == 0xff && data[1] == 0xd8 && data[2] == 0xff {
        return Some(Kind::Jpeg);
    }
    if data.len() >= 8 && data[..8] == [137, 80, 78, 71, 13, 10, 26, 10] {
        return Some(Kind::Png);
    }
    None
}

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn describe(meta: &jpeg::MetaInfo, path: &Path) -> String {
    let mut lines = vec![path.display().to_string()];
    if let (Some(w), Some(h)) = (meta.width, meta.height) {
        lines.push(format!("  size: {w}x{h}"));
    }
    lines.push(format!("  jfif: {}", yes(meta.has_jfif)));
    lines.push(format!("  exif: {}", yes(meta.has_exif)));
    lines.push(format!("  xmp: {}", yes(meta.has_xmp)));
    lines.push(format!("  photoshop: {}", yes(meta.has_photoshop)));
    lines.push(format!("  icc profile: {}", yes(meta.has_icc)));
    lines.push(format!("  comment: {}", yes(meta.has_comment)));
    if let Some(exif) = &meta.exif {
        if let Some(make) = &exif.make {
            lines.push(format!("  camera make: {make}"));
        }
        if let Some(model) = &exif.model {
            lines.push(format!("  camera model: {model}"));
        }
        if let Some(software) = &exif.software {
            lines.push(format!("  software: {software}"));
        }
        if let Some(dt) = &exif.datetime {
            lines.push(format!("  captured: {dt}"));
        }
        if let Some(o) = exif.orientation {
            lines.push(format!("  orientation: {o}"));
        }
        match exif.gps {
            Some((lat, lon)) => {
                lines.push(format!("  gps: {lat:.6}, {lon:.6}  <-- location leaked!"));
            }
            None => lines.push("  gps: none".into()),
        }
        if let Some(alt) = exif.gps_altitude {
            lines.push(format!("  altitude: {alt:.1} m"));
        }
    }
    lines.join("\n")
}

fn info_json(meta: &jpeg::MetaInfo, path: &Path) -> String {
    let mut parts = vec![format!("\"file\": {:?}", path.display().to_string())];
    if let (Some(w), Some(h)) = (meta.width, meta.height) {
        parts.push(format!("\"width\": {w}, \"height\": {h}"));
    }
    parts.push(format!("\"exif\": {}", meta.has_exif));
    parts.push(format!("\"xmp\": {}", meta.has_xmp));
    parts.push(format!("\"photoshop\": {}", meta.has_photoshop));
    parts.push(format!("\"icc\": {}", meta.has_icc));
    parts.push(format!("\"comment\": {}", meta.has_comment));
    let esc = |s: &str| format!("{s:?}");
    if let Some(exif) = &meta.exif {
        if let Some(make) = &exif.make {
            parts.push(format!("\"make\": {}", esc(make)));
        }
        if let Some(model) = &exif.model {
            parts.push(format!("\"model\": {}", esc(model)));
        }
        if let Some(software) = &exif.software {
            parts.push(format!("\"software\": {}", esc(software)));
        }
        if let Some(dt) = &exif.datetime {
            parts.push(format!("\"captured\": {}", esc(dt)));
        }
        if let Some((lat, lon)) = exif.gps {
            parts.push(format!("\"gps\": [{lat:.6}, {lon:.6}]"));
        }
        if let Some(alt) = exif.gps_altitude {
            parts.push(format!("\"altitude_m\": {alt:.1}"));
        }
    }
    format!("{{{}}}", parts.join(", "))
}

fn output_path(path: &Path, args: &Args) -> PathBuf {
    if args.inplace {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "jpg".into());
    let name = format!("{stem}.cleaned.{ext}");
    match &args.out_dir {
        Some(dir) => dir.join(name),
        None => match path.parent() {
            Some(parent) => parent.join(name),
            None => PathBuf::from(name),
        },
    }
}

fn write_output(path: &Path, cleaned: &[u8], args: &Args) -> Result<(), String> {
    let target = output_path(path, args);
    if let Some(dir) = target.parent() {
        let _ = fs::create_dir_all(dir);
    }
    fs::write(&target, cleaned).map_err(|e| format!("write {}: {e}", target.display()))
}

fn process_jpeg(path: &Path, data: &[u8], args: &Args) -> Result<(), String> {
    let meta = jpeg::info(data)?;
    if args.info_only {
        if args.json {
            println!("{}", info_json(&meta, path));
        } else {
            println!("{}", describe(&meta, path));
        }
        return Ok(());
    }
    let opts = jpeg::StripOptions { strip_icc: args.strip_icc, aggressive: args.aggressive };
    let cleaned = jpeg::strip(data, &opts)?;
    write_output(path, &cleaned, args)?;
    if !args.quiet {
        let removed = data.len().saturating_sub(cleaned.len());
        println!(
            "cleaned {} ({} metadata bytes removed)",
            output_path(path, args).display(),
            removed
        );
    }
    Ok(())
}

fn process_png(path: &Path, data: &[u8], args: &Args) -> Result<(), String> {
    let meta = png::info(data)?;
    if args.info_only {
        if args.json {
            let w = meta.width.map(|v| v.to_string()).unwrap_or_default();
            let h = meta.height.map(|v| v.to_string()).unwrap_or_default();
            println!(
                "{{\"file\": {:?}, \"width\": {}, \"height\": {}, \"text_chunks\": {}, \"time\": {}, \"exif\": {}}}",
                path.display().to_string(), w, h, meta.text_chunks, meta.has_time, meta.has_exif
            );
        } else {
            println!("{}", path.display());
            if let (Some(w), Some(h)) = (meta.width, meta.height) {
                println!("  size: {w}x{h}");
            }
            println!("  text chunks: {}", meta.text_chunks);
            println!("  tIME chunk: {}", yes(meta.has_time));
            println!("  eXIf chunk: {}", yes(meta.has_exif));
        }
        return Ok(());
    }
    let cleaned = png::strip(data)?;
    write_output(path, &cleaned, args)?;
    if !args.quiet {
        let removed = data.len().saturating_sub(cleaned.len());
        println!(
            "cleaned {} ({} metadata bytes removed)",
            output_path(path, args).display(),
            removed
        );
    }
    Ok(())
}

fn run(args: Args) -> ExitCode {
    let mut failures = 0usize;
    let mut count = 0usize;
    for path in &args.files {
        count += 1;
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("exif-clean: {}: {e}", path.display());
                failures += 1;
                continue;
            }
        };
        let kind = match sniff(&data) {
            Some(k) => k,
            None => {
                eprintln!(
                    "exif-clean: {}: unsupported format (JPEG/PNG only)",
                    path.display()
                );
                failures += 1;
                continue;
            }
        };
        let result = match kind {
            Kind::Jpeg => process_jpeg(path, &data, &args),
            Kind::Png => process_png(path, &data, &args),
        };
        if let Err(e) = result {
            eprintln!("exif-clean: {}: {e}", path.display());
            failures += 1;
        }
    }
    if failures > 0 {
        eprintln!("exif-clean: {failures}/{count} file(s) failed");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)) {
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(2)
        }
        Ok(Outcome::Print(text)) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Ok(Outcome::Run(args)) => run(args),
    }
}
