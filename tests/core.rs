use quakewatch::{Action, Frame, Site, Tracker, read_frame};
use serde_json::{Value, json};
use std::io::{self, Cursor};

const NOW: i64 = 1_700_000_000_000;
fn tracker(min: u8) -> Tracker {
    Tracker::new(Site::new(25.0, 121.5).unwrap(), min)
}
fn report(id: &str, serial: u32, status: u8, mag: f64) -> Value {
    json!({"author":"cwa", "id":id, "serial":serial, "status":status, "time":NOW,
        "eq":{"lat":25.0,"lon":121.5,"depth":14.0,"mag":mag,"time":NOW-1000,"loc":"test"}})
}
fn ingest(t: &mut Tracker, entries: Vec<Value>) -> (Vec<quakewatch::Notice>, usize) {
    t.ingest(&json!(entries).to_string(), NOW).unwrap()
}

#[test]
fn rejects_invalid_coordinates() {
    for (lat, lon) in [
        (91.0, 0.0),
        (0.0, -181.0),
        (f64::NAN, 0.0),
        (0.0, f64::INFINITY),
    ] {
        assert!(Site::new(lat, lon).is_none());
    }
    assert!(Site::new(90.0, -180.0).is_some());
}
#[test]
fn estimate_and_threshold_crossing() {
    let mut t = tracker(3);
    assert!(ingest(&mut t, vec![report("x", 1, 0, 2.0)]).0.is_empty());
    let (notices, _) = ingest(&mut t, vec![report("x", 2, 1, 5.0)]);
    assert_eq!(notices[0].action, Action::Alert);
    let e = notices[0].estimate.as_ref().unwrap();
    assert_eq!(e.distance_km, 14.0);
    assert_eq!(e.eta_seconds, 3);
    assert_eq!(e.level, 4);
    assert_eq!(
        ingest(&mut t, vec![report("x", 3, 0, 2.0)]).0[0].action,
        Action::BelowThreshold
    );
    assert_eq!(
        ingest(&mut t, vec![report("x", 4, 0, 5.0)]).0[0].action,
        Action::Alert
    );
}
#[test]
fn duplicate_update_stale_cancel_and_no_resurrection() {
    let mut t = tracker(1);
    let first = report("x", 1, 0, 5.0);
    assert_eq!(
        ingest(&mut t, vec![first.clone()]).0[0].action,
        Action::Alert
    );
    assert!(ingest(&mut t, vec![first.clone()]).0.is_empty());
    assert_eq!(
        ingest(&mut t, vec![report("x", 2, 1, 5.0)]).0[0].action,
        Action::Update
    );
    assert!(ingest(&mut t, vec![first]).0.is_empty());
    assert!(ingest(&mut t, vec![report("x", 1, 3, 5.0)]).0.is_empty());
    let cancel = json!({"author":"CWA","id":"x","serial":2,"status":3,"time":NOW});
    let (n, invalid) = ingest(&mut t, vec![cancel.clone()]);
    assert_eq!(invalid, 0);
    assert_eq!(n[0].action, Action::Cancel);
    assert!(n[0].estimate.is_none());
    assert!(
        ingest(&mut t, vec![cancel, report("x", 3, 1, 5.0)])
            .0
            .is_empty()
    );
}
#[test]
fn cancellation_wins_same_serial_within_snapshot() {
    let mut t = tracker(1);
    let (n, invalid) = ingest(&mut t, vec![report("x", 3, 3, 5.0), report("x", 3, 1, 5.0)]);
    assert!(n.is_empty());
    assert_eq!(invalid, 0);
}
#[test]
fn ignores_other_sources_and_isolates_invalid_entries() {
    let mut t = tracker(0);
    let mut stale = report("stale", 1, 0, 5.0);
    stale["time"] = json!(NOW - 181_000);
    let mut future = report("future", 1, 0, 5.0);
    future["eq"]["time"] = json!(NOW + 11_000);
    let mut other = report("other", 1, 0, 5.0);
    other["author"] = json!("trem");
    let (n, invalid) = ingest(
        &mut t,
        vec![stale, future, other, json!({}), report("ok", 1, 0, 5.0)],
    );
    assert_eq!(invalid, 3);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].id, "ok");
    assert!(t.ingest("not json", NOW).is_err());
}
#[test]
fn landed_eta_does_not_go_negative() {
    let mut t = tracker(0);
    let mut r = report("x", 1, 0, 5.0);
    r["eq"]["time"] = json!(NOW - 30_000);
    assert_eq!(
        ingest(&mut t, vec![r]).0[0]
            .estimate
            .as_ref()
            .unwrap()
            .eta_seconds,
        0
    );
}
#[test]
fn sse_framing_comments_named_multiline_and_incomplete() {
    let input = b": heartbeat\r\nretry: 3000\r\n\r\nevent: info\r\ndata: hello\r\n\r\ndata: [\r\ndata: ]\r\n\r\ndata: incomplete\n";
    let mut r = Cursor::new(input);
    assert_eq!(
        read_frame(&mut r).unwrap(),
        Some(Frame {
            event: "info".into(),
            data: "hello".into()
        })
    );
    assert_eq!(
        read_frame(&mut r).unwrap(),
        Some(Frame {
            event: "message".into(),
            data: "[\n]".into()
        })
    );
    assert!(read_frame(&mut r).unwrap().is_none());
}
#[test]
fn sse_rejects_oversized_frame() {
    assert_eq!(
        read_frame(&mut Cursor::new(vec![b'a'; 300 * 1024]))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
}
