use clap::{Parser, Subcommand};
use quakewatch::{Frame, Notice, Site, Tracker, read_frame};
use std::{
    error::Error,
    fs::File,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{SyncSender, sync_channel},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const SOURCE: &str = "https://api.lb-tpe1.exptech.dev/api/v2/eq/eew?sse=1";

#[derive(Parser)]
#[command(version, about = "台灣地震速報實驗工具；不可作為唯一防災訊號")]
struct Args {
    #[command(subcommand)]
    command: Mode,
}

#[derive(clap::Args)]
struct Location {
    #[arg(long, allow_negative_numbers = true)]
    lat: f64,
    #[arg(long, allow_negative_numbers = true)]
    lon: f64,
    /// Minimum estimated level: 0–4, or 5 for the undifferentiated ≥5弱 band.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(0..=5))]
    min: u8,
}
impl Location {
    fn tracker(&self) -> Result<Tracker> {
        Ok(Tracker::new(
            Site::new(self.lat, self.lon).ok_or("latitude/longitude out of range")?,
            self.min,
        ))
    }
}

#[derive(Subcommand)]
enum Mode {
    /// Listen to an authorized plain SSE feed. Emits JSON Lines to stdout.
    Live {
        #[command(flatten)]
        location: Location,
        #[arg(long, default_value = SOURCE)]
        url: String,
        #[arg(long)]
        backup: Option<String>,
        /// Validate the first snapshot and quit. Does not emit or run hooks.
        #[arg(long)]
        check: bool,
        /// Absolute path to an executable. Receives one JSON line on stdin.
        #[arg(long, conflicts_with = "check")]
        hook: Option<PathBuf>,
    },
    /// Replay a local SSE capture (or '-' for stdin). Never runs hooks.
    Replay {
        #[command(flatten)]
        location: Location,
        /// Evaluation clock (Unix milliseconds) for deterministic offline replay.
        #[arg(long)]
        at_ms: i64,
        file: PathBuf,
    },
}

fn now_ms() -> Result<i64> {
    Ok(i64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}

fn run_hook(exe: &Path, mut line: Vec<u8>) -> io::Result<()> {
    // A notice contains bounded input. Limit a single stdin write to a Linux pipe's
    // guaranteed atomic write size so a hook that never reads cannot block us.
    line.push(b'\n');
    if line.len() > 4096 {
        return Err(io::Error::other("hook input exceeds 4096 bytes"));
    }
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing hook stdin"))
        .and_then(|mut stdin| stdin.write_all(&line));
    let result = write_result.and_then(|()| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait()? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!("hook exited with {status}")))
                };
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "hook exceeded 5 seconds",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn start_hook(exe: PathBuf) -> Result<SyncSender<Vec<u8>>> {
    if !exe.is_absolute() || !exe.is_file() {
        return Err("--hook must be an absolute path to an executable file".into());
    }
    let (tx, rx) = sync_channel::<Vec<u8>>(16);
    thread::Builder::new()
        .name("alert-hook".into())
        .spawn(move || {
            for line in rx {
                if let Err(err) = run_hook(&exe, line) {
                    eprintln!("hook failed: {err}");
                }
            }
        })?;
    Ok(tx)
}

fn emit(
    out: &mut impl Write,
    notice: Notice,
    hook: Option<&SyncSender<Vec<u8>>>,
) -> io::Result<()> {
    let line = serde_json::to_vec(&notice)?;
    out.write_all(&line)?;
    out.write_all(b"\n")?;
    out.flush()?;
    if let Some(tx) = hook
        && let Err(err) = tx.try_send(line)
    {
        eprintln!(
            "hook skipped for event {}: {err} (JSON still emitted)",
            notice.id
        );
    }
    Ok(())
}

fn process(
    tracker: &mut Tracker,
    Frame { event, data }: Frame,
    now: i64,
    check: bool,
    hook: Option<&SyncSender<Vec<u8>>>,
    out: &mut impl Write,
) -> Result<bool> {
    if event != "message" {
        return Ok(false);
    } // info, heartbeats, compressed frames
    let (notices, invalid) = tracker.ingest(&data, now)?;
    if invalid > 0 {
        eprintln!("ignored {invalid} invalid, stale or overflow report(s)");
    }
    if check {
        if invalid > 0 {
            return Err("first snapshot contains invalid or stale reports".into());
        }
        eprintln!("SSE connected; first EEW JSON snapshot validated");
    } else {
        for notice in notices {
            emit(out, notice, hook)?;
        }
    }
    Ok(true)
}

fn live(
    location: Location,
    url: String,
    backup: Option<String>,
    check: bool,
    hook: Option<PathBuf>,
) -> Result<()> {
    let mut tracker = location.tracker()?;
    let sources: Vec<_> = std::iter::once(url).chain(backup).collect();
    for source in &sources {
        let uri: ureq::http::Uri = source.parse()?;
        if uri.scheme_str() != Some("https") || uri.host().is_none() {
            return Err("SSE URLs must use HTTPS".into());
        }
    }
    let hook = hook.map(start_hook).transpose()?;
    let config = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_resolve(Some(Duration::from_secs(8)))
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_recv_response(Some(Duration::from_secs(12)))
        .timeout_global(check.then_some(Duration::from_secs(25)))
        .timeout_per_read(Some(Duration::from_secs(if check { 12 } else { 300 })))
        .build();
    let agent: ureq::Agent = config.into();
    let mut out = io::stdout().lock();
    let mut backoff = 3;
    loop {
        for (index, source) in sources.iter().enumerate() {
            let mut connected = false;
            let result: Result<()> = (|| {
                let mut response = agent
                    .get(source)
                    .header("accept", "text/event-stream")
                    .header("user-agent", "quakewatch/0.1")
                    .call()?;
                if !response.status().is_success() {
                    return Err(format!("HTTP {}", response.status()).into());
                }
                let is_sse = response
                    .headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .is_some_and(|s| {
                        s.split(';')
                            .next()
                            .is_some_and(|t| t.trim().eq_ignore_ascii_case("text/event-stream"))
                    });
                if !is_sse {
                    return Err("expected text/event-stream".into());
                }
                eprintln!("SSE transport open (source {})", index + 1);
                let mut reader = BufReader::new(response.body_mut().as_reader());
                while let Some(frame) = read_frame(&mut reader)? {
                    match process(
                        &mut tracker,
                        frame,
                        now_ms()?,
                        check,
                        hook.as_ref(),
                        &mut out,
                    ) {
                        Ok(true) if check => return Ok(()),
                        Ok(true) => connected = true,
                        Ok(false) => {}
                        Err(err) if !check && err.is::<serde_json::Error>() => {
                            eprintln!("discarded malformed EEW snapshot: {err}")
                        }
                        Err(err) => return Err(err),
                    }
                }
                Err("SSE connection closed".into())
            })();
            match result {
                Ok(()) => return Ok(()),
                Err(err)
                    if err.is::<io::Error>()
                        && err
                            .downcast_ref::<io::Error>()
                            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe) =>
                {
                    return Err(err);
                }
                Err(err) => {
                    eprintln!("SSE source {}: {err}", index + 1);
                    if check && index + 1 == sources.len() {
                        return Err(err);
                    }
                }
            }
            if connected {
                backoff = 3;
            }
        }
        eprintln!("reconnecting in {backoff}s");
        thread::sleep(Duration::from_secs(backoff));
        backoff = (backoff * 2).min(60);
    }
}

fn replay(location: Location, at_ms: i64, file: &Path) -> Result<()> {
    if at_ms <= 0 {
        return Err("--at-ms must be a positive Unix timestamp in milliseconds".into());
    }
    let mut tracker = location.tracker()?;
    let mut reader: Box<dyn BufRead> = if file == Path::new("-") {
        Box::new(io::stdin().lock())
    } else {
        Box::new(BufReader::new(File::open(file)?))
    };
    let mut out = io::stdout().lock();
    let mut snapshot = false;
    while let Some(frame) = read_frame(&mut reader)? {
        snapshot |= process(&mut tracker, frame, at_ms, false, None, &mut out)?;
    }
    if !snapshot {
        return Err("replay contained no complete JSON snapshots".into());
    }
    Ok(())
}

fn main() -> Result<()> {
    match Args::parse().command {
        Mode::Live {
            location,
            url,
            backup,
            check,
            hook,
        } => live(location, url, backup, check, hook),
        Mode::Replay {
            location,
            at_ms,
            file,
        } => replay(location, at_ms, &file),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn hook_reports_exit_status_and_rejects_large_input() {
        assert!(run_hook(Path::new("/bin/cat"), b"{}".to_vec()).is_ok());
        let error = run_hook(Path::new("/bin/false"), b"{}".to_vec()).unwrap_err();
        assert!(
            error.kind() == io::ErrorKind::BrokenPipe || error.to_string().contains("hook exited")
        );
        assert!(run_hook(Path::new("/bin/cat"), vec![b'a'; 4096]).is_err());
    }
}
