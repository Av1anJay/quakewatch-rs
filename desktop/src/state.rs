use quakewatch::{Action, Notice, Site, Tracker};
use serde_json::json;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub struct Place {
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
}
pub const PLACES: [Place; 6] = [
    Place {
        name: "臺北",
        lat: 25.0375,
        lon: 121.5637,
    },
    Place {
        name: "新竹",
        lat: 24.8039,
        lon: 120.9647,
    },
    Place {
        name: "臺中",
        lat: 24.1477,
        lon: 120.6736,
    },
    Place {
        name: "臺南",
        lat: 22.9999,
        lon: 120.2270,
    },
    Place {
        name: "高雄",
        lat: 22.6273,
        lon: 120.3014,
    },
    Place {
        name: "花蓮",
        lat: 23.9872,
        lon: 121.6016,
    },
];
impl Place {
    pub fn custom(lat: f64, lon: f64) -> Result<Self, &'static str> {
        Site::new(lat, lon)
            .map(|_| Self {
                name: "自訂座標",
                lat,
                lon,
            })
            .ok_or("latitude/longitude out of range")
    }
    pub fn site(self) -> Site {
        Site::new(self.lat, self.lon).expect("place coordinates are validated")
    }
}

pub struct Config {
    pub custom: Option<Place>,
    pub threshold: u8,
    pub static_map: bool,
    pub url: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Standby,
    Connecting,
    Online,
    Offline,
    Demo,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standby => "未連線",
            Self::Connecting => "連線中",
            Self::Online => "即時監測中",
            Self::Offline => "連線中斷・重試中",
            Self::Demo => "本機演練・非真實地震",
        }
    }
}

pub struct Entry {
    pub notice: Notice,
    pub arrived: Instant,
    pub wave_elapsed: f64,
}
impl Entry {
    pub fn new(notice: Notice, arrived: Instant) -> Self {
        let wave_elapsed = notice.estimate.as_ref().map_or(0., |e| {
            notice
                .received_at_ms
                .saturating_sub(e.origin_time_ms)
                .max(0) as f64
                / 1_000.
        });
        Self {
            notice,
            arrived,
            wave_elapsed,
        }
    }
    pub fn eta(&self, now: Instant) -> Option<u64> {
        self.notice.estimate.as_ref().map(|e| {
            e.eta_seconds
                .saturating_sub(now.saturating_duration_since(self.arrived).as_secs())
        })
    }
    pub fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.arrived)
    }
    pub fn is_active(&self, now: Instant) -> bool {
        matches!(self.notice.action, Action::Alert | Action::Update)
            && self.age(now) < Duration::from_secs(180)
    }
}

#[derive(Default)]
pub struct Reports {
    pub active: Vec<Entry>,
    pub recent: VecDeque<(String, u32, &'static str)>,
}
impl Reports {
    pub fn apply(&mut self, notice: Notice, now: Instant) {
        let id = notice.id.clone();
        let label = action_label(&notice.action);
        self.recent.push_front((id.clone(), notice.serial, label));
        self.recent.truncate(3);
        self.active
            .retain(|entry| entry.notice.id != id && entry.is_active(now));
        if matches!(notice.action, Action::Alert | Action::Update) {
            self.active.insert(0, Entry::new(notice, now));
        }
        self.active.truncate(24);
    }
    pub fn latest(&self, now: Instant) -> Option<&Entry> {
        self.active.iter().find(|e| e.is_active(now))
    }
}

pub fn action_label(action: &Action) -> &'static str {
    match action {
        Action::Alert => "速報",
        Action::Update => "更正報",
        Action::BelowThreshold => "低於門檻",
        Action::Cancel => "已撤報",
    }
}

pub struct Demo {
    start: Instant,
    step: usize,
    tracker: Tracker,
}
impl Demo {
    pub fn new(site: Site, start: Instant) -> Self {
        Self {
            start,
            step: 0,
            tracker: Tracker::new(site, 0),
        }
    }
    pub fn advance(&mut self, now: Instant) -> Vec<Notice> {
        const ORIGIN: i64 = 1_700_000_000_000;
        let elapsed = now
            .saturating_duration_since(self.start)
            .as_millis()
            .min(60_000) as i64;
        let mut notices = Vec::new();
        while let Some(&at) = [0, 12_000, 45_000].get(self.step) {
            if elapsed < at {
                break;
            }
            let cancelled = self.step == 2;
            let eq = (!cancelled).then(|| json!({"lat": 23.93, "lon":121.71, "depth":15.0,
                "mag":if self.step == 0 { 6.3 } else { 6.5 }, "loc":"演練：花蓮近海", "time":ORIGIN - 10_000}));
            let sample = json!([{"author":"cwa", "id":"DEMO-001", "serial":self.step+1,
                "status":if cancelled { 3 } else { 1 }, "eq":eq, "time": ORIGIN+at}]);
            notices.extend(
                self.tracker
                    .ingest(&sample.to_string(), ORIGIN + elapsed)
                    .expect("valid built-in simulation")
                    .0,
            );
            self.step += 1;
        }
        notices
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demo_counts_down_updates_and_removes_on_cancel() {
        let start = Instant::now();
        let mut demo = Demo::new(PLACES[0].site(), start);
        let mut reports = Reports::default();
        for n in demo.advance(start) {
            reports.apply(n, start);
        }
        let first = reports.latest(start).unwrap();
        assert_eq!(first.notice.action, Action::Alert);
        assert!(first.eta(start).unwrap() > first.eta(start + Duration::from_secs(1)).unwrap());
        let updated = start + Duration::from_secs(12);
        for n in demo.advance(updated) {
            reports.apply(n, updated);
        }
        assert_eq!(
            reports.latest(updated).unwrap().notice.action,
            Action::Update
        );
        let cancelled = start + Duration::from_secs(45);
        for n in demo.advance(cancelled) {
            reports.apply(n, cancelled);
        }
        assert!(reports.latest(cancelled).is_none());
        assert_eq!(reports.recent[0].2, "已撤報");
        assert!(demo.advance(cancelled).is_empty());
    }
    #[test]
    fn old_report_is_not_presented_as_active() {
        let start = Instant::now();
        let mut demo = Demo::new(PLACES[0].site(), start);
        let mut reports = Reports::default();
        for n in demo.advance(start) {
            reports.apply(n, start);
        }
        assert!(reports.latest(start + Duration::from_secs(180)).is_none());
    }
}
