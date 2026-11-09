use super::*;
use crate::published::{self, Published};
use gpui::PathPromptOptions;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Default)]
pub(super) struct Settings {
    pub(super) draft: Option<Preferences>,
    tab: Tab,
    pub(super) message: Option<String>,
    picker: bool,
    testing: bool,
    reports: Option<Vec<Published>>,
    pending: Option<Receiver<Result<Vec<Published>, String>>>,
    report_error: bool,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Tab {
    Alert,
    Reports,
    #[default]
    Other,
    About,
}

impl Settings {
    pub(super) fn tab_label(&self) -> &'static str {
        match self.tab {
            Tab::Alert => "強震即時警報",
            Tab::Reports => "地震報告",
            Tab::Other => "其他",
            Tab::About => "關於",
        }
    }
    pub(super) fn poll(&mut self, hooks: Option<&Hooks>) -> bool {
        let mut changed = false;
        if let Some(r) = hooks.and_then(Hooks::result) {
            self.testing = false;
            self.message = Some(match r {
                Ok(()) => "測試成功：已傳入 4 和 10".into(),
                Err(err) => format!("測試失敗：{err}"),
            });
            changed = true;
        }
        let result = self.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("worker stopped".into())),
        });
        if let Some(result) = result {
            self.pending = None;
            self.report_error = result.is_err();
            match result {
                Ok(rows) => self.reports = Some(rows),
                Err(err) => eprintln!("published reports: {err}"),
            }
            changed = true;
        }
        changed
    }
}

impl Dashboard {
    pub(super) fn open_settings(&mut self) {
        if self.settings.draft.is_some() {
            return;
        }
        let mut draft = self.prefs.clone();
        draft.threshold = self.config.threshold;
        if self.selected < PLACES.len() {
            draft.place = self.selected;
        }
        self.settings.draft = Some(draft);
        self.settings.tab = Tab::Other;
    }

    fn settings_cancel(&mut self) {
        if !self.settings.picker && !self.settings.testing {
            self.settings.draft = None;
            self.settings.message = None;
        }
    }

    fn settings_save(&mut self) {
        if self.settings.picker || self.settings.testing {
            return;
        }
        let Some(draft) = self.settings.draft.as_ref() else {
            return;
        };
        if let Err(err) = draft.save() {
            self.settings.tab = Tab::Other;
            self.settings.message = Some(format!("無法儲存：{err}"));
            return;
        }
        let draft = self.settings.draft.take().expect("draft exists");
        if self.live.is_none() && self.demo.is_none() {
            self.config.threshold = draft.threshold;
            if self.selected < PLACES.len() {
                self.selected = draft.place;
            }
        }
        self.prefs = draft;
        self.settings.message = None;
    }

    fn choose_program(&mut self, cx: &mut Context<Self>) {
        if self.settings.picker || self.settings.testing || self.settings.draft.is_none() {
            return;
        }
        self.settings.picker = true;
        self.settings.message = None;
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("選擇連動程式".into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = selected.await;
            let _ = weak.update(cx, |this, cx| {
                this.settings.picker = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(draft) = this.settings.draft.as_mut() {
                            draft.program = paths.into_iter().next();
                            if let Err(err) = draft.program() {
                                this.settings.message = Some(err.to_string());
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => {
                        this.settings.message = Some(
                            "無法開啟檔案選擇器；Linux 需要桌面入口服務 (xdg-desktop-portal)。"
                                .into(),
                        )
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn test_program(&mut self) {
        if self.settings.picker || self.settings.testing {
            return;
        }
        let (Some(draft), Some(hooks)) = (self.settings.draft.as_ref(), self.hooks.as_ref()) else {
            return;
        };
        self.settings.message = Some(match hooks.test(draft) {
            Ok(()) => {
                self.settings.testing = true;
                "正在測試連動…".into()
            }
            Err(err) => format!("無法測試：{err}"),
        });
    }

    fn load_reports(&mut self) {
        if self.settings.pending.is_some() || self.demo.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("quakewatch-reports".into())
            .spawn(move || {
                let _ = tx.send(published::fetch());
            }) {
            Ok(_) => {
                self.settings.pending = Some(rx);
                self.settings.report_error = false;
            }
            Err(err) => {
                self.settings.report_error = true;
                eprintln!("report worker: {err}");
            }
        }
    }

    pub(super) fn settings_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match key {
            "escape" => self.settings_cancel(),
            "enter" => self.settings_save(),
            "1" => self.settings.tab = Tab::Alert,
            "2" => self.settings.tab = Tab::Reports,
            "3" => self.settings.tab = Tab::Other,
            "4" => self.settings.tab = Tab::About,
            "b" if self.settings.tab == Tab::Other => self.choose_program(cx),
            "r" if self.settings.tab == Tab::Reports => self.load_reports(),
            "t" if self.settings.tab == Tab::Other => self.test_program(),
            _ => return false,
        }
        true
    }

    pub(super) fn settings_panel(&self, cx: &Context<Self>) -> Div {
        let Some(draft) = self.settings.draft.as_ref() else {
            return div();
        };
        let tab = self.settings.tab;
        let busy = self.settings.picker || self.settings.testing;
        let mut nav = div().flex().flex_col().gap_2().w(px(172.)).flex_shrink_0();
        for (id, label, which, activate) in [
            (
                "set-eew",
                "強震即時警報 [1]",
                Tab::Alert,
                (|d: &mut Self| d.settings.tab = Tab::Alert) as fn(&mut Self),
            ),
            ("set-reports", "地震報告 [2]", Tab::Reports, |d| {
                d.settings.tab = Tab::Reports
            }),
            ("set-other", "其他 [3]", Tab::Other, |d| {
                d.settings.tab = Tab::Other
            }),
            ("set-about", "關於 [4]", Tab::About, |d| {
                d.settings.tab = Tab::About
            }),
        ] {
            nav = nav.child(self.button(id, label, true, which == tab, activate, cx));
        }
        let mut body = div()
            .id("settings-body")
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex_col()
            .gap_3()
            .overflow_y_scroll();
        match tab {
            Tab::Other => {
                let browse = self.button(
                    "browse",
                    if self.settings.picker {
                        "選擇中…"
                    } else {
                        "瀏覽… [B]"
                    },
                    false,
                    false,
                    |_| {},
                    cx,
                );
                let browse = if busy {
                    browse
                } else {
                    browse
                        .text_color(rgb(TEXT))
                        .cursor_pointer()
                        .hover(|b| b.bg(rgb(0x274455)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.choose_program(cx);
                            cx.notify();
                        }))
                };
                let scales: [(u16, fn(&mut Self)); 4] = [
                    (75, |d| d.set_scale(75)),
                    (100, |d| d.set_scale(100)),
                    (125, |d| d.set_scale(125)),
                    (150, |d| d.set_scale(150)),
                ];
                body = body.child(section("系統設定 · 介面縮放"))
                    .child(div().flex().flex_wrap().gap_2().children(scales.into_iter().map(|(scale, change)|
                        self.button(("scale", scale as u64), format!("{scale}%"), !busy, draft.scale == scale, change, cx))))
                    .child(div().text_sm().text_color(rgb(MUTED)).child("小視窗會限制放大比例，避免裁切。開機自啟與自動更新尚未提供。"))
                    .child(section("連動設定"))
                    .child(self.button("enable", check(draft.enabled, "收到速報且達到預警震度後執行程式"), !busy, draft.enabled, |d| d.settings.draft.as_mut().unwrap().enabled ^= true, cx))
                    .child(div().flex().items_center().gap_2()
                        .child(div().flex_1().min_w_0().text_sm().text_ellipsis().overflow_hidden().whitespace_nowrap().child(draft.program.as_ref().map_or_else(|| "未選擇程式".into(), |p| p.display().to_string())))
                        .child(browse)
                        .child(self.button("test", "測試 [T]", !busy && draft.program().is_ok() && self.hooks.is_some(), false, Self::test_program, cx)))
                    .child(div().text_sm().text_color(rgb(MUTED)).child(if cfg!(windows) { "Windows：.exe、.bat、.cmd" } else { "選擇本機執行檔或有執行權限的腳本。" }))
                    .child(div().text_sm().text_color(rgb(MUTED)).child("兩個參數：所在地震度、S 波抵達秒數。測試傳入 4 10；目前只估算至 ≥5弱，這一級傳入 5-。演練不執行程式。"))
                    .child(self.button("once", check(draft.once, "同一次地震僅執行一次（本次啟動期間）"), !busy, draft.once, |d| d.settings.draft.as_mut().unwrap().once ^= true, cx));
                if let Some(msg) = &self.settings.message {
                    body = body.child(div().text_sm().text_color(rgb(ORANGE)).child(msg.clone()));
                }
            }
            Tab::Alert => {
                let editable = self.live.is_none() && self.demo.is_none();
                body = body
                    .child(section("所在地與預警震度"))
                    .child(self.button(
                        "pref-place",
                        if self.selected == PLACES.len() {
                            "自訂座標（命令列）"
                        } else {
                            PLACES[draft.place].name
                        },
                        editable && self.selected != PLACES.len(),
                        false,
                        |d| {
                            let p = d.settings.draft.as_mut().unwrap();
                            p.place = (p.place + 1) % PLACES.len();
                        },
                        cx,
                    ))
                    .child(self.button(
                        "pref-min",
                        format!(
                            "門檻 {}",
                            if draft.threshold == 5 {
                                "5弱".to_string()
                            } else {
                                draft.threshold.to_string()
                            }
                        ),
                        editable,
                        false,
                        |d| {
                            let p = d.settings.draft.as_mut().unwrap();
                            p.threshold = (p.threshold + 1) % 6;
                        },
                        cx,
                    ))
                    .child(div().text_sm().text_color(rgb(MUTED)).child(if editable {
                        "點擊切換，儲存後套用。"
                    } else {
                        "停止監測或演練後才可變更。"
                    }));
            }
            Tab::Reports => {
                body = body
                    .child(section("最近 10 筆地震報告 · ExpTech"))
                    .child(self.button(
                        "fetch-reports",
                        if self.settings.pending.is_some() {
                            "讀取中…"
                        } else {
                            "更新報告 [R]"
                        },
                        self.demo.is_none() && self.settings.pending.is_none(),
                        false,
                        Self::load_reports,
                        cx,
                    ));
                if self.demo.is_some() {
                    body = body.child("演練期間不連網。");
                }
                if self.settings.report_error {
                    body = body.child(
                        div()
                            .text_color(rgb(ORANGE))
                            .child("無法取得最新報告，請稍後重試。下方可能是上一次的資料。"),
                    );
                }
                if let Some(rows) = &self.settings.reports {
                    if rows.is_empty() {
                        body = body.child("沒有可顯示的報告。");
                    }
                    body = body.children(rows.iter().map(|r| {
                        card().p_3().gap_1().child(r.loc.clone()).child(
                            div().text_sm().text_color(rgb(MUTED)).child(format!(
                                "M {:.1} · 深度 {:.0} km · 最大震度 {}",
                                r.mag,
                                r.depth,
                                r.intensity()
                            )),
                        )
                    }));
                }
            }
            Tab::About => {
                body = body.child(section("關於 Quakewatch"))
                    .child(format!("GPUI 桌面版 v{}", env!("CARGO_PKG_VERSION")))
                    .child("獨立 Rust 實作；不是地牛官方版。即時資料來自 ExpTech 轉送 CWA，震度及到時皆為粗估。不可用作唯一防災訊號。")
                    .child(self.button("releases", "GitHub 發行頁", true, false, |_| {}, cx).on_click(cx.listener(|_, _, _, cx| cx.open_url("https://github.com/Av1anJay/quakewatch-rs/releases"))));
            }
        }
        card()
            .p_4()
            .gap_4()
            .min_h_0()
            .flex_1()
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("Quakewatch 設定"),
            )
            .child(
                div().text_sm().text_color(rgb(ORANGE)).child(
                    self.reports()
                        .latest(self.clock)
                        .and_then(|entry| {
                            entry.notice.estimate.as_ref().map(|e| {
                                format!(
                                    "{} · {} · M {:.1} · 預估震度 {} · S 波剩餘 {} 秒",
                                    self.status().label(),
                                    e.location,
                                    e.magnitude,
                                    if e.level == 5 {
                                        "≥5弱".into()
                                    } else {
                                        e.level.to_string()
                                    },
                                    entry.eta(self.clock).unwrap_or(0)
                                )
                            })
                        })
                        .unwrap_or_else(|| {
                            format!("{} · 未收到速報不代表沒有地震", self.status().label())
                        }),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .min_h_0()
                    .flex_1()
                    .child(nav)
                    .child(body),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(self.button(
                        "cancel-settings",
                        "取消 [Esc]",
                        !busy,
                        false,
                        Self::settings_cancel,
                        cx,
                    ))
                    .child(self.button(
                        "save-settings",
                        "儲存 [Enter]",
                        !busy,
                        true,
                        Self::settings_save,
                        cx,
                    )),
            )
    }

    fn set_scale(&mut self, value: u16) {
        if let Some(draft) = self.settings.draft.as_mut() {
            draft.scale = value;
        }
    }
}

fn check(value: bool, label: &str) -> String {
    format!("{}  {label}", if value { "☑" } else { "☐" })
}
