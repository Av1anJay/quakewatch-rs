use quakewatch::{Notice, Site, Tracker, read_frame};
use std::{
    io::BufReader,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const SOURCE: &str = "https://api.lb-tpe1.exptech.dev/api/v2/eq/eew?sse=1";
type BoxError = Box<dyn std::error::Error>;

#[derive(Debug)]
pub enum Event {
    Connecting,
    Online,
    Offline,
    Notice(Notice),
}

pub struct Feed {
    pub rx: std::sync::mpsc::Receiver<Event>,
    stop: Arc<AtomicBool>,
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn validate_url(url: &str) -> Result<(), BoxError> {
    let uri: ureq::http::Uri = url.parse()?;
    if uri.scheme_str() != Some("https")
        || uri.host().is_none_or(str::is_empty)
        || uri.authority().is_some_and(|a| a.as_str().contains('@'))
    {
        return Err("EEW URL must be HTTPS without embedded credentials".into());
    }
    Ok(())
}

pub fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

pub fn start(url: String, site: Site, minimum: u8) -> std::io::Result<Feed> {
    let (tx, rx) = sync_channel(32);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    thread::Builder::new()
        .name("quakewatch-eew".into())
        .spawn(move || {
            let config = ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(0)
                .timeout_resolve(Some(Duration::from_secs(8)))
                .timeout_connect(Some(Duration::from_secs(8)))
                .timeout_recv_response(Some(Duration::from_secs(12)))
                .timeout_per_read(Some(Duration::from_secs(300)))
                .build();
            let agent: ureq::Agent = config.into();
            let mut tracker = Tracker::new(site, minimum);
            let mut delay = 3;
            while !worker_stop.load(Ordering::Relaxed) {
                if tx.send(Event::Connecting).is_err() {
                    break;
                }
                let mut validated = false;
                if let Err(error) = session(
                    &agent,
                    &url,
                    &mut tracker,
                    &worker_stop,
                    &tx,
                    &mut validated,
                ) {
                    if worker_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    eprintln!("EEW stream: {error}");
                }
                if tx.send(Event::Offline).is_err() {
                    break;
                }
                if validated {
                    delay = 3;
                }
                for _ in 0..delay * 4 {
                    if worker_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                delay = (delay * 2).min(60);
            }
        })?;
    Ok(Feed { rx, stop })
}

fn session(
    agent: &ureq::Agent,
    url: &str,
    tracker: &mut Tracker,
    stop: &AtomicBool,
    tx: &SyncSender<Event>,
    validated: &mut bool,
) -> Result<(), BoxError> {
    let mut response = agent
        .get(url)
        .header("accept", "text/event-stream")
        .header(
            "user-agent",
            concat!("quakewatch-gui/", env!("CARGO_PKG_VERSION")),
        )
        .call()?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()).into());
    }
    let is_sse = response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(';').next())
        .is_some_and(|t| t.trim().eq_ignore_ascii_case("text/event-stream"));
    if !is_sse {
        return Err("expected text/event-stream".into());
    }
    let mut reader = BufReader::new(response.body_mut().as_reader());
    while !stop.load(Ordering::Relaxed) {
        let Some(frame) = read_frame(&mut reader)? else {
            break;
        };
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if frame.event != "message" {
            continue;
        }
        let (notices, invalid) = match tracker.ingest(&frame.data, unix_ms()) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("invalid EEW snapshot: {error}");
                continue;
            }
        };
        if invalid != 0 {
            eprintln!("ignored {invalid} invalid/stale EEW report(s)");
        }
        if !*validated && (invalid == 0 || !notices.is_empty()) {
            tx.send(Event::Online)?;
            *validated = true;
        }
        for notice in notices {
            tx.send(Event::Notice(notice))?;
        }
    }
    Err("stream closed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validate_stream_url() {
        assert!(validate_url(SOURCE).is_ok());
        for url in ["http://example.com", "/eew", "https://user@example.com"] {
            assert!(validate_url(url).is_err(), "{url}");
        }
    }
}
