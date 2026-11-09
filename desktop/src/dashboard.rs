use crate::{
    feed::{self, Event, Feed},
    map::{self, Waves},
    state::{Config, Demo, PLACES, Place, Reports, Status, action_label},
};
use gpui::{Context, Div, ElementId, FocusHandle, Stateful, Window, div, prelude::*, px, rgb};
use std::time::{Duration, Instant};

const BG: u32 = 0x0c1721;
const CARD: u32 = 0x132431;
const SEA: u32 = 0x102530;
const BORDER: u32 = 0x385362;
const TEXT: u32 = 0xf1f6f8;
const MUTED: u32 = 0xa9bfca;
const GREEN: u32 = 0x96efb7;
const BLUE: u32 = 0x65d7f3;
const ORANGE: u32 = 0xffb487;
const PINK: u32 = 0xffa7a8;

pub struct Dashboard {
    config: Config,
    selected: usize,
    status: Status,
    reports: Reports,
    live: Option<Feed>,
    demo: Option<Demo>,
    demo_reports: Reports,
    clock: Instant,
    fault: Option<&'static str>,
    focus: FocusHandle,
}

impl Dashboard {
    pub fn new(
        config: Config,
        live: bool,
        demo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        let mut this = Self {
            selected: if config.custom.is_some() {
                PLACES.len()
            } else {
                0
            },
            config,
            status: Status::Standby,
            reports: Reports::default(),
            live: None,
            demo: None,
            demo_reports: Reports::default(),
            clock: Instant::now(),
            fault: None,
            focus,
        };
        if live {
            this.toggle_live();
        }
        if demo {
            this.toggle_demo();
        }
        let executor = cx.background_executor().clone();
        cx.spawn(async move |weak, cx| {
            loop {
                executor.timer(Duration::from_millis(125)).await;
                if weak
                    .update(cx, |this, cx| {
                        if this.tick(Instant::now()) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn place(&self) -> Place {
        PLACES
            .get(self.selected)
            .copied()
            .or(self.config.custom)
            .unwrap_or(PLACES[0])
    }
    fn reports(&self) -> &Reports {
        if self.demo.is_some() {
            &self.demo_reports
        } else {
            &self.reports
        }
    }
    fn status(&self) -> Status {
        if self.demo.is_some() {
            Status::Demo
        } else {
            self.status
        }
    }
    fn clear_fault(&mut self) {
        self.fault = None;
    }

    fn toggle_live(&mut self) {
        if self.demo.is_some() {
            return;
        }
        self.clear_fault();
        if self.live.take().is_some() {
            self.status = Status::Standby;
            self.reports = Reports::default();
        } else {
            match feed::start(
                self.config.url.clone(),
                self.place().site(),
                self.config.threshold,
            ) {
                Ok(feed) => {
                    self.live = Some(feed);
                    self.status = Status::Connecting;
                }
                Err(err) => {
                    eprintln!("cannot start EEW worker: {err}");
                    self.fault = Some("無法啟動監測，請重試。");
                }
            }
        }
    }

    fn toggle_demo(&mut self) {
        if self.live.is_some() {
            return;
        }
        self.clear_fault();
        self.clock = Instant::now();
        self.demo_reports = Reports::default();
        if self.demo.take().is_none() {
            self.demo = Some(Demo::new(self.place().site(), self.clock));
            self.tick(self.clock);
        }
    }

    fn select_next(&mut self) {
        if self.live.is_none() && self.demo.is_none() {
            self.selected =
                (self.selected + 1) % (PLACES.len() + usize::from(self.config.custom.is_some()));
        }
    }

    fn next_threshold(&mut self) {
        if self.live.is_none() && self.demo.is_none() {
            self.config.threshold = (self.config.threshold + 1) % 6;
        }
    }

    fn tick(&mut self, now: Instant) -> bool {
        let mut changed = self.reports().latest(self.clock).is_some();
        self.clock = now;
        if let Some(demo) = self.demo.as_mut() {
            for n in demo.advance(now) {
                self.demo_reports.apply(n, now);
                changed = true;
            }
        }
        if let Some(feed) = self.live.as_ref() {
            for _ in 0..32 {
                match feed.rx.try_recv() {
                    Ok(event) => {
                        match event {
                            Event::Connecting => self.status = Status::Connecting,
                            Event::Online => self.status = Status::Online,
                            Event::Offline => self.status = Status::Offline,
                            Event::Notice(notice) => self.reports.apply(notice, now),
                        }
                        changed = true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.status = Status::Standby;
                        self.fault = Some("監測已停止，請重新連線。");
                        self.live = None;
                        changed = true;
                        break;
                    }
                }
            }
        }
        changed
    }

    fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<gpui::SharedString>,
        enabled: bool,
        emphasized: bool,
        shortcut: fn(&mut Self),
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let button = div()
            .id(id)
            .h(px(44.))
            .px_3()
            .rounded_md()
            .border_1()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(if emphasized { 0x203c38 } else { CARD }))
            .border_color(rgb(if emphasized { 0x547e6c } else { BORDER }))
            .text_color(rgb(if emphasized { GREEN } else { TEXT }))
            .text_sm()
            .child(label.into());
        if enabled {
            button
                .cursor_pointer()
                .hover(|b| b.bg(rgb(0x274455)))
                .active(|b| b.bg(rgb(0x305062)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    shortcut(this);
                    cx.notify();
                }))
        } else {
            button.text_color(rgb(MUTED)).border_color(rgb(0x293e4b))
        }
    }

    fn header(&self, cx: &Context<Self>) -> Div {
        let status = self.status();
        let color = match status {
            Status::Online => GREEN,
            Status::Demo | Status::Connecting => ORANGE,
            Status::Offline => PINK,
            Status::Standby => MUTED,
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .flex_shrink_0()
            .child(
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .w(px(46.))
                            .h(px(46.))
                            .rounded_md()
                            .bg(rgb(0x213d39))
                            .border_1()
                            .border_color(rgb(0x476b5d))
                            .text_color(rgb(GREEN))
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("QW"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Quakewatch 地震速報"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("臺灣強震監測桌面台"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(badge(status.label(), color))
                    .child(self.button(
                        "live",
                        if self.live.is_some() {
                            "停止監測 [L]"
                        } else {
                            "連線監測 [L]"
                        },
                        self.demo.is_none(),
                        self.live.is_none() && self.demo.is_none(),
                        Self::toggle_live,
                        cx,
                    ))
                    .child(self.button(
                        "demo",
                        if self.demo.is_some() {
                            "結束演練 [D]"
                        } else {
                            "演練 [D]"
                        },
                        self.live.is_none(),
                        false,
                        Self::toggle_demo,
                        cx,
                    )),
            )
    }

    fn map_panel(&self) -> Div {
        let wave = self.reports().latest(self.clock).and_then(|entry| {
            let eq = entry.notice.estimate.as_ref()?;
            let seconds = if self.config.static_map {
                0.
            } else {
                entry.age(self.clock).as_secs_f64()
            };
            Some(Waves {
                lat: eq.epicenter_lat,
                lon: eq.epicenter_lon,
                depth: eq.depth_km,
                elapsed: entry.wave_elapsed + seconds,
            })
        });
        card()
            .min_w_0()
            .min_h_0()
            .flex_1()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px_4()
                    .h(px(49.))
                    .flex_shrink_0()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("臺灣地震監測地圖"),
                    )
                    .child(div().text_xs().text_color(rgb(MUTED)).child(
                        if self.config.static_map {
                            "固定波圈"
                        } else {
                            "震波示意・非精密模型"
                        },
                    )),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .bg(rgb(SEA))
                    .child(map::view(self.place(), wave))
                    .child(div().absolute().top_3().left_3().child(badge("N ↑", MUTED)))
                    .child(
                        div()
                            .absolute()
                            .bottom_3()
                            .right_3()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("Natural Earth · 公開底圖"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_4()
                    .h(px(44.))
                    .flex_shrink_0()
                    .child(legend("●", "所在地", GREEN))
                    .child(legend("×", "震央", ORANGE))
                    .child(legend("─", "P 波", BLUE))
                    .child(legend("─", "S 波", ORANGE)),
            )
    }

    fn current(&self) -> Div {
        let title = section("目前速報");
        let Some(entry) = self.reports().latest(self.clock) else {
            let (label, info) = match self.status() {
                Status::Online => ("目前無活動速報", "低於所在地門檻的速報不顯示。"),
                Status::Connecting => ("等待資料", "收到第一份有效快照後才會顯示已連線。"),
                Status::Offline => ("連線中斷", "未收到速報不代表沒有地震。"),
                Status::Demo => ("演練已結束", "這是本機模擬，沒有對外送出訊息。"),
                Status::Standby => ("尚未連線", "可先開啟本機演練，或連線接收速報。"),
            };
            return card()
                .p_4()
                .gap_3()
                .child(title)
                .child(
                    div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(label),
                )
                .child(div().text_sm().text_color(rgb(MUTED)).child(info));
        };
        let e = entry
            .notice
            .estimate
            .as_ref()
            .expect("active reports always have an estimate");
        let eta = entry.eta(self.clock).unwrap_or(0);
        let intensity = if e.level == 5 {
            "≥5弱".into()
        } else {
            e.level.to_string()
        };
        let countdown = if eta == 0 {
            "已到時".into()
        } else {
            format!("{eta:02} 秒")
        };
        card()
            .p_4()
            .gap_2()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(title)
                    .child(badge(action_label(&entry.notice.action), ORANGE)),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_ellipsis()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(e.location.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(metric("預估震度", intensity, ORANGE))
                    .child(metric("S 波預估", countdown, BLUE)),
            )
            .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                "M {:.1}   深度 {:.0} km   震源距離 {:.0} km",
                e.magnitude, e.depth_km, e.distance_km
            )))
    }

    fn sidebar(&self, cx: &Context<Self>) -> Div {
        let place = self.place();
        let editable = self.live.is_none() && self.demo.is_none();
        div()
            .w(px(326.))
            .min_h_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.current())
            .child(
                card()
                    .p_4()
                    .gap_2()
                    .child(section("所在地設定"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button(
                                "place",
                                format!("{} [C]", place.name),
                                editable,
                                false,
                                Self::select_next,
                                cx,
                            ))
                            .child(self.button(
                                "threshold",
                                format!(
                                    "門檻 {} [T]",
                                    if self.config.threshold == 5 {
                                        "5弱".into()
                                    } else {
                                        self.config.threshold.to_string()
                                    }
                                ),
                                editable,
                                false,
                                Self::next_threshold,
                                cx,
                            )),
                    )
                    .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                        "{:.3}°N  {:.3}°E{}",
                        place.lat,
                        place.lon,
                        if editable {
                            ""
                        } else {
                            " · 停止後可調整"
                        }
                    ))),
            )
            .child(
                card()
                    .min_h_0()
                    .flex_1()
                    .p_4()
                    .gap_2()
                    .overflow_hidden()
                    .child(section("最近通知"))
                    .when(self.reports().recent.is_empty(), |b| {
                        b.child(div().text_sm().text_color(rgb(MUTED)).child("本次尚無通知"))
                    })
                    .children(self.reports().recent.iter().map(|(id, serial, label)| {
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .border_t_1()
                            .border_color(rgb(BORDER))
                            .pt_2()
                            .child(div().text_sm().text_color(rgb(TEXT)).child(*label))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(format!("{id} · 第 {serial} 報")),
                            )
                    })),
            )
    }
}

fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(BORDER))
        .rounded_lg()
}
fn section(label: &'static str) -> Div {
    div()
        .text_sm()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED))
        .child(label)
}
fn badge(label: &'static str, color: u32) -> Div {
    div()
        .rounded_md()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(BG))
        .px_2()
        .py_1()
        .text_xs()
        .text_color(rgb(color))
        .child(label)
}
fn legend(mark: &'static str, label: &'static str, color: u32) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .text_sm()
        .text_color(rgb(MUTED))
        .child(div().text_color(rgb(color)).child(mark))
        .child(label)
}
fn metric(label: &'static str, value: String, color: u32) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .rounded_md()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(BG))
        .p_2()
        .child(div().text_xs().text_color(rgb(MUTED)).child(label))
        .child(
            div()
                .text_xl()
                .text_color(rgb(color))
                .font_weight(gpui::FontWeight::BOLD)
                .child(value),
        )
}

impl Render for Dashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("dashboard")
            .track_focus(&self.focus)
            .size_full()
            .p_4()
            .gap_3()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .font_family("Noto Sans CJK TC")
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.is_held
                    || event.keystroke.modifiers.control
                    || event.keystroke.modifiers.platform
                    || event.keystroke.modifiers.alt
                {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "l" => this.toggle_live(),
                    "d" => this.toggle_demo(),
                    "c" => this.select_next(),
                    "t" => this.next_threshold(),
                    _ => return,
                }
                cx.notify();
            }))
            .child(self.header(cx))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .flex_1()
                    .min_h_0()
                    .child(self.map_panel())
                    .child(self.sidebar(cx)),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_3()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(
                        self.fault
                            .unwrap_or("研究原型 · 震度與到時皆為粗估，不可作為唯一防災訊號"),
                    )
                    .child("資料：ExpTech 轉送 CWA 速報"),
            )
    }
}
