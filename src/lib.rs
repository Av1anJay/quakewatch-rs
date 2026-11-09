use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, BufRead},
};

const MAX_AGE_MS: i64 = 180_000;
const FUTURE_SKEW_MS: i64 = 10_000;
const MAX_TRACKED: usize = 1024;
const MAX_FRAME: usize = 256 * 1024;
const EARTH_RADIUS_KM: f64 = 6_371.0;
const S_WAVE_KM_S: f64 = 3.5;

#[derive(Clone, Copy, Debug)]
pub struct Site {
    lat: f64,
    lon: f64,
}
impl Site {
    pub fn new(lat: f64, lon: f64) -> Option<Self> {
        ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon))
            .then_some(Self { lat, lon })
    }
}

#[derive(Debug, Deserialize)]
struct Report {
    author: String,
    id: String,
    serial: u32,
    status: u8,
    #[serde(default)]
    eq: Option<Origin>,
    time: i64,
}

#[derive(Debug, Deserialize)]
struct Origin {
    lat: f64,
    lon: f64,
    depth: f64,
    mag: f64,
    time: i64,
    loc: String,
}
impl Origin {
    fn valid(&self, now: i64) -> bool {
        Site::new(self.lat, self.lon).is_some()
            && (0.0..=750.0).contains(&self.depth)
            && (0.0..=10.0).contains(&self.mag)
            && recent(self.time, now)
    }
}

fn recent(ms: i64, now: i64) -> bool {
    ms > 0 && ms >= now.saturating_sub(MAX_AGE_MS) && ms <= now.saturating_add(FUTURE_SKEW_MS)
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Alert,
    Update,
    BelowThreshold,
    Cancel,
}

#[derive(Debug, Serialize)]
pub struct Estimate {
    pub location: String,
    pub epicenter_lat: f64,
    pub epicenter_lon: f64,
    pub origin_time_ms: i64,
    pub magnitude: f64,
    pub depth_km: f64,
    pub distance_km: f64,
    pub pga_gal: f64,
    pub level: u8,
    pub intensity: &'static str,
    pub eta_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct Notice {
    pub action: Action,
    pub id: String,
    pub author: String,
    pub serial: u32,
    pub upstream_status: u8,
    pub received_at_ms: i64,
    pub estimate: Option<Estimate>,
}

fn estimate(site: Site, eq: Origin, now: i64) -> Estimate {
    let h = (((eq.lat - site.lat).to_radians() / 2.0).sin().powi(2)
        + site.lat.to_radians().cos()
            * eq.lat.to_radians().cos()
            * (((eq.lon - site.lon).to_radians() / 2.0).sin().powi(2)))
    .clamp(0.0, 1.0);
    let distance = (2.0 * EARTH_RADIUS_KM * h.sqrt().asin()).hypot(eq.depth);
    let pga = 1.657 * (1.533 * eq.mag).exp() * distance.max(1.0).powf(-1.607);
    let level = [0.8, 2.5, 8.0, 25.0, 80.0]
        .iter()
        .filter(|&&x| pga >= x)
        .count() as u8;
    let elapsed = now.saturating_sub(eq.time) as f64 / 1_000.0;
    let eta = (distance / S_WAVE_KM_S - elapsed).max(0.0).ceil() as u64;
    Estimate {
        location: eq.loc,
        epicenter_lat: eq.lat,
        epicenter_lon: eq.lon,
        origin_time_ms: eq.time,
        magnitude: eq.mag,
        depth_km: eq.depth,
        distance_km: distance,
        pga_gal: pga,
        level,
        intensity: ["0", "1", "2", "3", "4", "≥5弱（粗估，未細分）"][level as usize],
        eta_seconds: eta,
    }
}

#[derive(Debug)]
struct Seen {
    serial: u32,
    expires_at: i64,
    cancelled: bool,
    active: bool,
}

pub struct Tracker {
    site: Site,
    minimum: u8,
    seen: HashMap<(String, String), Seen>,
}

impl Tracker {
    pub fn new(site: Site, minimum: u8) -> Self {
        Self {
            site,
            minimum,
            seen: HashMap::new(),
        }
    }

    // The SSE feed sends a snapshot, not a stream of individual report objects.
    pub fn ingest(&mut self, data: &str, now: i64) -> serde_json::Result<(Vec<Notice>, usize)> {
        let entries: Vec<serde_json::Value> = serde_json::from_str(data)?;
        self.seen.retain(|_, item| item.expires_at >= now);
        let mut invalid = 0;
        let mut latest: HashMap<(String, String), Report> = HashMap::new();
        for entry in entries {
            let report: Report = match serde_json::from_value(entry) {
                Ok(report) => report,
                Err(_) => {
                    invalid += 1;
                    continue;
                }
            };
            if !report.author.eq_ignore_ascii_case("cwa") {
                continue;
            }
            if report.id.is_empty()
                || report.id.len() > 64
                || report.serial == 0
                || !recent(report.time, now)
                || !matches!(report.status, 0 | 1 | 3)
                || (report.status != 3 && !report.eq.as_ref().is_some_and(|eq| eq.valid(now)))
            {
                invalid += 1;
                continue;
            }
            let key = ("cwa".to_owned(), report.id.clone());
            let rank = |r: &Report| (r.serial, r.status == 3, r.time);
            if latest.get(&key).is_none_or(|old| rank(&report) > rank(old)) {
                latest.insert(key, report);
            }
        }
        let mut latest: Vec<_> = latest.into_iter().collect();
        latest.sort_unstable_by(|a, b| (a.1.time, &a.0).cmp(&(b.1.time, &b.0)));
        let mut notices = Vec::new();
        for (key, report) in latest {
            let old = self.seen.get(&key);
            let cancelled = report.status == 3;
            if let Some(old) = old {
                if old.cancelled
                    || report.serial < old.serial
                    || (report.serial == old.serial && !cancelled)
                {
                    continue;
                }
            } else if self.seen.len() >= MAX_TRACKED {
                invalid += 1;
                continue;
            }
            let was_active = old.is_some_and(|old| old.active);
            let estimate = if cancelled {
                None
            } else {
                report.eq.map(|eq| estimate(self.site, eq, now))
            };
            let active = estimate.as_ref().is_some_and(|e| e.level >= self.minimum);
            self.seen.insert(
                key,
                Seen {
                    serial: report.serial,
                    expires_at: report.time.saturating_add(MAX_AGE_MS),
                    cancelled,
                    active,
                },
            );
            let action = match (cancelled, was_active, active) {
                (true, true, _) => Some(Action::Cancel),
                (false, true, true) => Some(Action::Update),
                (false, false, true) => Some(Action::Alert),
                (false, true, false) => Some(Action::BelowThreshold),
                _ => None,
            };
            if let Some(action) = action {
                notices.push(Notice {
                    action,
                    id: report.id,
                    author: "cwa".into(),
                    serial: report.serial,
                    upstream_status: report.status,
                    received_at_ms: now,
                    estimate,
                });
            }
        }
        Ok((notices, invalid))
    }
}

#[derive(Debug, PartialEq)]
pub struct Frame {
    pub event: String,
    pub data: String,
}

pub fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Frame>> {
    let mut frame = Frame {
        event: "message".into(),
        data: String::new(),
    };
    let mut line = String::new();
    let mut has_data = false;
    let mut consumed = 0;
    loop {
        line.clear();
        let n =
            io::Read::take(&mut *reader, (MAX_FRAME - consumed + 1) as u64).read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        } // SSE dispatches only after a blank line.
        consumed += n;
        if consumed > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SSE frame exceeds 256 KiB",
            ));
        }
        let line = line.strip_suffix('\n').unwrap_or(&line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if has_data {
                return Ok(Some(frame));
            }
            consumed = 0;
            frame.event = "message".into();
            continue;
        }
        let (field, value) = line
            .split_once(':')
            .map_or((line, ""), |(k, v)| (k, v.strip_prefix(' ').unwrap_or(v)));
        match field {
            "event" => {
                frame.event = if value.is_empty() {
                    "message".into()
                } else {
                    value.into()
                }
            }
            "data" => {
                if has_data {
                    frame.data.push('\n');
                }
                frame.data.push_str(value);
                has_data = true;
            }
            _ => {}
        }
    }
}
