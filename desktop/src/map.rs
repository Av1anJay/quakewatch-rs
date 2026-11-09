use crate::state::Place;
use gpui::{
    App, Bounds, PathBuilder, Pixels, Point, Window, canvas, point, prelude::*, px, rgb, rgba,
};
use std::{f32::consts::TAU, sync::OnceLock};

static COAST: OnceLock<Vec<Vec<[f32; 2]>>> = OnceLock::new();
const LON: f32 = 121.05;
const LAT: f32 = 23.67;
const X_KM: f32 = 102.;
const Y_KM: f32 = 111.;

#[derive(Clone, Copy)]
pub struct Waves {
    pub lat: f64,
    pub lon: f64,
    pub depth: f64,
    pub elapsed: f64,
}

struct Project {
    center: Point<Pixels>,
    scale: f32,
}
impl Project {
    fn new(bounds: Bounds<Pixels>) -> Self {
        Self {
            center: bounds.center(),
            scale: (f32::from(bounds.size.height) / 400.).min(f32::from(bounds.size.width) / 330.),
        }
    }
    fn at(&self, lon: f32, lat: f32) -> Point<Pixels> {
        self.center
            + point(
                px((lon - LON) * X_KM * self.scale),
                px((LAT - lat) * Y_KM * self.scale),
            )
    }
}

fn stroke(
    window: &mut Window,
    points: impl IntoIterator<Item = Point<Pixels>>,
    width: f32,
    color: gpui::Rgba,
    closed: bool,
) {
    let mut points = points.into_iter();
    let Some(first) = points.next() else {
        return;
    };
    let mut builder = PathBuilder::stroke(px(width));
    builder.move_to(first);
    for next in points {
        builder.line_to(next);
    }
    if closed {
        builder.close();
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn circle(window: &mut Window, center: Point<Pixels>, radius: f32, color: gpui::Rgba, fill: bool) {
    let mut builder = if fill {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(px(1.5))
    };
    builder.move_to(center + point(px(radius), px(0.)));
    for i in 1..72 {
        let (sin, cos) = (i as f32 * TAU / 72.).sin_cos();
        builder.line_to(center + point(px(radius * cos), px(radius * sin)));
    }
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn inside(bounds: Bounds<Pixels>, p: Point<Pixels>) -> bool {
    p.x >= bounds.origin.x
        && p.x < bounds.origin.x + bounds.size.width
        && p.y >= bounds.origin.y
        && p.y < bounds.origin.y + bounds.size.height
}

fn draw(bounds: Bounds<Pixels>, window: &mut Window, place: Place, waves: Option<Waves>) {
    let projection = Project::new(bounds);
    for x in 119..=124 {
        let p = projection.at(x as f32, LAT);
        if p.x > bounds.origin.x && p.x < bounds.origin.x + bounds.size.width {
            stroke(
                window,
                [
                    point(p.x, bounds.origin.y),
                    point(p.x, bounds.origin.y + bounds.size.height),
                ],
                1.,
                rgb(0x1b3544),
                false,
            );
        }
    }
    for y in 21..=27 {
        let p = projection.at(LON, y as f32);
        if p.y > bounds.origin.y && p.y < bounds.origin.y + bounds.size.height {
            stroke(
                window,
                [
                    point(bounds.origin.x, p.y),
                    point(bounds.origin.x + bounds.size.width, p.y),
                ],
                1.,
                rgb(0x1b3544),
                false,
            );
        }
    }
    for ring in COAST.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/taiwan-rings.json"))
            .expect("valid bundled coast")
    }) {
        let mut vertices = ring.iter().map(|v| projection.at(v[0], v[1]));
        let Some(first) = vertices.next() else {
            continue;
        };
        let mut land = PathBuilder::fill();
        land.move_to(first);
        for v in vertices {
            land.line_to(v);
        }
        land.close();
        if let Ok(path) = land.build() {
            window.paint_path(path, rgb(0x224958));
        }
        stroke(
            window,
            ring.iter().map(|v| projection.at(v[0], v[1])),
            1.5,
            rgb(0x71aab5),
            true,
        );
    }
    if let Some(wave) = waves {
        let p = projection.at(wave.lon as f32, wave.lat as f32);
        // Draw only circles whose rims can intersect this viewport. The outer
        // GPUI clip keeps nearby offshore waves from leaking into the sidebar.
        let diagonal = f32::from(bounds.size.width).hypot(f32::from(bounds.size.height));
        let to_center =
            f32::from(p.x - projection.center.x).hypot(f32::from(p.y - projection.center.y));
        for (speed, color) in [(6.0, rgb(0x65d7f3)), (3.5, rgb(0xffa362))] {
            let r2 = (wave.elapsed * speed).powi(2) - wave.depth.powi(2);
            let radius = r2.max(0.).sqrt() as f32 * projection.scale;
            if radius >= 2. && (to_center - radius).abs() <= diagonal {
                circle(window, p, radius, color, false);
            }
        }
        if inside(bounds, p) {
            circle(window, p, 12., rgba(0xff9a5a45), true);
            stroke(
                window,
                [p + point(px(-5.), px(-5.)), p + point(px(5.), px(5.))],
                2.5,
                rgb(0xffb487),
                false,
            );
            stroke(
                window,
                [p + point(px(-5.), px(5.)), p + point(px(5.), px(-5.))],
                2.5,
                rgb(0xffb487),
                false,
            );
        }
    }
    let p = projection.at(place.lon as f32, place.lat as f32);
    if inside(bounds, p) {
        circle(window, p, 9., rgba(0x8df7ba44), true);
        circle(window, p, 4., rgb(0x96efb7), true);
    }
}

pub fn view(place: Place, waves: Option<Waves>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window: &mut Window, _: &mut App| draw(bounds, window, place, waves),
    )
    .size_full()
}
