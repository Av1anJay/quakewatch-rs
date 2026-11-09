use serde::Deserialize;
use std::time::Duration;

const URL: &str = "https://api.core-tyo1.exptech.dev/api/v2/eq/report?limit=10";
const LIMIT: u64 = 256 * 1024;

#[derive(Deserialize)]
pub struct Published {
    pub loc: String,
    pub mag: f64,
    pub depth: f64,
    pub int: u8,
}
impl Published {
    pub fn intensity(&self) -> &'static str {
        ["0", "1", "2", "3", "4", "5弱", "5強", "6弱", "6強", "7"][self.int as usize]
    }
}

pub fn fetch() -> Result<Vec<Published>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(12)))
        .https_only(true)
        .build()
        .into();
    let body = agent
        .get(URL)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(LIMIT)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    parse(&body)
}

fn parse(data: &[u8]) -> Result<Vec<Published>, String> {
    let mut rows: Vec<Published> = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    for row in &mut rows {
        row.loc = row.loc.trim().to_owned();
    }
    if rows.iter().any(|r| {
        r.int > 9
            || !r.mag.is_finite()
            || !(0.0..=10.0).contains(&r.mag)
            || !r.depth.is_finite()
            || !(0.0..=750.0).contains(&r.depth)
            || r.loc.is_empty()
            || r.loc.len() > 512
    }) {
        return Err("地震報告欄位無效".into());
    }
    rows.truncate(10);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(mag: f64, depth: f64, loc: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([
            {"loc": loc, "mag": mag, "depth": depth, "int": 6}
        ]))
        .unwrap()
    }

    #[test]
    fn accepts_inclusive_magnitude_and_depth_boundaries() {
        for mag in [0.0, 10.0] {
            for depth in [0.0, 750.0] {
                let rows = parse(&report(mag, depth, "花蓮近海")).unwrap();
                assert_eq!(rows[0].mag, mag);
                assert_eq!(rows[0].depth, depth);
            }
        }
    }

    #[test]
    fn rejects_out_of_range_magnitude_and_depth() {
        for mag in [-0.1, 10.1] {
            assert!(parse(&report(mag, 12.0, "花蓮近海")).is_err());
        }
        for depth in [-0.1, 750.1] {
            assert!(parse(&report(6.4, depth, "花蓮近海")).is_err());
        }
    }

    #[test]
    fn rejects_nonfinite_magnitude_and_depth() {
        for value in ["NaN", "Infinity", "-Infinity", "1e309", "-1e309"] {
            for (mag, depth) in [(value, "12.0"), ("6.4", value)] {
                let data = format!(r#"[{{"loc":"花蓮近海","mag":{mag},"depth":{depth},"int":6}}]"#);
                assert!(parse(data.as_bytes()).is_err());
            }
        }
    }

    #[test]
    fn trims_locations_and_rejects_blank_or_oversized_locations() {
        let rows = parse(&report(6.4, 12.0, " \t花蓮近海\n\u{3000}")).unwrap();
        assert_eq!(rows[0].loc, "花蓮近海");
        for loc in ["", " \t\n\u{3000}"] {
            assert!(parse(&report(6.4, 12.0, loc)).is_err());
        }
        assert!(parse(&report(6.4, 12.0, &"x".repeat(512))).is_ok());
        assert!(parse(&report(6.4, 12.0, &"x".repeat(513))).is_err());
    }

    #[test]
    fn parses_published_intensity_and_rejects_invalid_level() {
        let data = |int| format!(r#"[{{"loc":"花蓮近海","mag":6.4,"depth":12.0,"int":{int}}}]"#);
        assert_eq!(parse(data(6).as_bytes()).unwrap()[0].intensity(), "5強");
        assert!(parse(data(10).as_bytes()).is_err());
    }
}
