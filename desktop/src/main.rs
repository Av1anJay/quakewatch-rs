mod dashboard;
mod feed;
mod map;
mod state;

use clap::Parser;
use dashboard::Dashboard;
use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use state::{Config, Place};

#[derive(Parser)]
#[command(version, about = "Quakewatch — 台灣地震速報桌面版")]
struct Args {
    /// Connect on startup; otherwise start offline.
    #[arg(long)]
    live: bool,
    /// Start a local simulation, with no network connection.
    #[arg(long, conflicts_with = "live")]
    demo: bool,
    /// An authorized, uncompressed HTTPS SSE endpoint.
    #[arg(long, default_value = feed::SOURCE)]
    url: String,
    #[arg(long, requires = "lon", allow_negative_numbers = true)]
    lat: Option<f64>,
    #[arg(long, requires = "lat", allow_negative_numbers = true)]
    lon: Option<f64>,
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(0..=5))]
    min: u8,
    /// Do not animate the wave rings.
    #[arg(long)]
    static_map: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let custom = args
        .lat
        .zip(args.lon)
        .map(|(lat, lon)| Place::custom(lat, lon))
        .transpose()?;
    feed::validate_url(&args.url)?;
    let config = Config {
        custom,
        threshold: args.min,
        static_map: args.static_map,
        url: args.url,
    };
    if cfg!(target_os = "linux")
        && std::env::var_os("DISPLAY")
            .filter(|d| !d.is_empty())
            .is_none()
    {
        return Err("Quakewatch GPUI requires X11 (or XWayland with DISPLAY set)".into());
    }
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(745.)), cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        if let Err(err) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(850.), px(640.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Quakewatch · 地震速報".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Dashboard::new(config, args.live, args.demo, window, cx)),
        ) {
            eprintln!("cannot open Quakewatch window: {err}");
            std::process::exit(1);
        }
        cx.activate(true);
    });
    Ok(())
}
