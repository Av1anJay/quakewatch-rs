use quakewatch::{Action, Notice};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    pub program: Option<PathBuf>,
    pub once: bool,
    pub scale: u16,
    pub place: usize,
    pub threshold: u8,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: false,
            program: None,
            once: true,
            scale: 100,
            place: 0,
            threshold: 2,
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> io::Result<()> {
        self.validate_fields()?;
        if self.enabled {
            self.program()?;
        }
        Ok(())
    }
    fn validate_fields(&self) -> io::Result<()> {
        if ![75, 100, 125, 150].contains(&self.scale)
            || self.threshold > 5
            || self.place >= crate::state::PLACES.len()
        {
            return Err(io::Error::other("設定檔的縮放、門檻或所在地無效"));
        }
        Ok(())
    }
    pub fn program(&self) -> io::Result<&Path> {
        self.program
            .as_deref()
            .filter(|p| valid_program(p))
            .ok_or_else(|| io::Error::other("請先選擇有效的本機程式"))
    }
    fn config_path() -> io::Result<PathBuf> {
        let home = || {
            std::env::var_os("HOME")
                .filter(|x| !x.is_empty())
                .map(PathBuf::from)
        };
        let base = if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA")
                .filter(|x| !x.is_empty())
                .map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            home().map(|h| h.join("Library/Application Support"))
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .filter(|x| !x.is_empty())
                .map(PathBuf::from)
                .or_else(|| home().map(|h| h.join(".config")))
        };
        let base = base
            .filter(|p| p.is_absolute())
            .ok_or_else(|| io::Error::other("no absolute config directory"))?;
        Ok(base.join("quakewatch").join("desktop.json"))
    }
    pub fn load() -> io::Result<Self> {
        let path = Self::config_path()?;
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        let prefs: Self = serde_json::from_slice(&bytes)?;
        prefs.validate_fields()?;
        Ok(prefs)
    }
    pub fn save(&self) -> io::Result<()> {
        self.validate()?;
        let path = Self::config_path()?;
        let dir = path.parent().expect("config path has parent");
        std::fs::create_dir_all(dir)?;
        let temp = dir.join(format!("desktop.{}.tmp", std::process::id()));
        let result = (|| {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(&serde_json::to_vec_pretty(self)?)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, &path)
        })();
        if result
            .as_ref()
            .is_err_and(|e| e.kind() != io::ErrorKind::AlreadyExists)
        {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
}

fn valid_program(path: &Path) -> bool {
    if !path.is_absolute() || !path.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "exe" => true,
            "bat" | "cmd" => !path.to_string_lossy().contains(['%', '!', '\r', '\n', '"']),
            _ => false,
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(any(unix, windows)))]
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HookArgs {
    level: &'static str,
    seconds: u64,
}
impl HookArgs {
    fn from_notice(n: &Notice) -> Option<Self> {
        let e = n.estimate.as_ref()?;
        matches!(n.action, Action::Alert | Action::Update).then_some(Self {
            level: match e.level {
                0 => "0",
                1 => "1",
                2 => "2",
                3 => "3",
                4 => "4",
                _ => "5-",
            },
            seconds: e.eta_seconds,
        })
    }
}

const MAX_QUEUE_AGE: Duration = Duration::from_secs(180);
const MAX_SEEN_IDS: usize = 4096;

struct Job {
    program: PathBuf,
    args: HookArgs,
    manual: bool,
    queued_at: Instant,
}
impl Job {
    fn args_at(&self, now: Instant) -> Option<HookArgs> {
        if self.manual {
            return Some(self.args);
        }
        let age = now.saturating_duration_since(self.queued_at);
        if age >= MAX_QUEUE_AGE {
            return None;
        }
        Some(HookArgs {
            seconds: self.args.seconds.saturating_sub(age.as_secs()),
            ..self.args
        })
    }
}

pub struct Hooks {
    tx: SyncSender<Job>,
    results: Receiver<Result<(), String>>,
    seen: HashSet<String>,
}
impl Hooks {
    pub fn new() -> io::Result<Self> {
        let (tx, rx) = sync_channel::<Job>(8);
        let (result_tx, results) = sync_channel(4);
        std::thread::Builder::new()
            .name("quakewatch-hook".into())
            .spawn(move || {
                for job in rx {
                    let result = run(&job).map_err(|err| err.to_string());
                    if job.manual {
                        let _ = result_tx.try_send(result);
                    } else if let Err(err) = result {
                        eprintln!("alert integration: {err}");
                    }
                }
            })?;
        Ok(Self {
            tx,
            results,
            seen: HashSet::new(),
        })
    }
    pub fn test(&self, prefs: &Preferences) -> io::Result<()> {
        self.tx
            .try_send(Job {
                program: prefs.program()?.to_owned(),
                args: HookArgs {
                    level: "4",
                    seconds: 10,
                },
                manual: true,
                queued_at: Instant::now(),
            })
            .map_err(|_| io::Error::other("連動程式佇列忙碌，請稍後重試"))
    }
    pub fn result(&self) -> Option<Result<(), String>> {
        match self.results.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Disconnected | TryRecvError::Empty) => None,
        }
    }
    pub fn alert(&mut self, prefs: &Preferences, notice: &Notice, now: Instant) {
        if !prefs.enabled {
            return;
        }
        let Some(args) = HookArgs::from_notice(notice) else {
            return;
        };
        if prefs.once && (self.seen.contains(&notice.id) || self.seen.len() >= MAX_SEEN_IDS) {
            return;
        }
        match prefs.program().and_then(|path| {
            self.tx
                .try_send(Job {
                    program: path.into(),
                    args,
                    manual: false,
                    queued_at: now,
                })
                .map_err(io::Error::other)
        }) {
            Ok(()) => {
                if self.seen.len() < MAX_SEEN_IDS {
                    self.seen.insert(notice.id.clone());
                }
            }
            Err(err) => eprintln!("could not enqueue alert integration: {err}"),
        }
    }
}

fn run(job: &Job) -> io::Result<()> {
    if !valid_program(&job.program) {
        return Err(io::Error::other("程式不存在或不可執行"));
    }
    let Some(args) = job.args_at(Instant::now()) else {
        return Ok(());
    };
    let mut child = Command::new(&job.program)
        .arg(args.level)
        .arg(args.seconds.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(io::Error::other(format!("程式結束：{status}"))),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(other.err().unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "連動程式超過 10 秒")
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Demo, PLACES};

    fn job(queued_at: Instant, manual: bool) -> Job {
        Job {
            program: PathBuf::new(),
            args: HookArgs {
                level: "4",
                seconds: 10,
            },
            manual,
            queued_at,
        }
    }

    fn local_hooks() -> (Hooks, Receiver<Job>) {
        let (tx, rx) = sync_channel(8);
        let (_result_tx, results) = sync_channel(4);
        (
            Hooks {
                tx,
                results,
                seen: HashSet::new(),
            },
            rx,
        )
    }

    fn enabled_preferences() -> Preferences {
        Preferences {
            enabled: true,
            program: Some(std::env::current_exe().unwrap()),
            ..Preferences::default()
        }
    }

    fn notice(now: Instant) -> Notice {
        Demo::new(PLACES[0].site(), now).advance(now).pop().unwrap()
    }

    #[test]
    fn real_job_args_count_down_and_saturate() {
        let now = Instant::now();
        let job = job(now, false);
        assert_eq!(job.args_at(now), Some(job.args));
        assert_eq!(
            job.args_at(now + Duration::from_secs(3)),
            Some(HookArgs {
                level: "4",
                seconds: 7,
            })
        );
        for delay in [10, 30, 179] {
            assert_eq!(
                job.args_at(now + Duration::from_secs(delay)),
                Some(HookArgs {
                    level: "4",
                    seconds: 0,
                })
            );
        }
    }

    #[test]
    fn real_jobs_expire_at_three_minutes() {
        let now = Instant::now();
        let job = job(now, false);
        for delay in [180, 181, 301] {
            assert_eq!(job.args_at(now + Duration::from_secs(delay)), None);
        }
    }

    #[test]
    fn manual_jobs_keep_exact_test_args_after_delay() {
        let now = Instant::now();
        let job = job(now, true);
        for delay in [0, 3, 180, 301] {
            assert_eq!(
                job.args_at(now + Duration::from_secs(delay)),
                Some(HookArgs {
                    level: "4",
                    seconds: 10,
                })
            );
        }
    }

    #[test]
    fn once_suppresses_same_id_after_five_minutes() {
        let now = Instant::now();
        let (mut hooks, rx) = local_hooks();
        let prefs = enabled_preferences();
        let mut notice = notice(now);
        hooks.alert(&prefs, &notice, now);
        let queued = rx.try_recv().unwrap();
        assert!(!queued.manual);
        assert_eq!(queued.queued_at, now);

        notice.action = Action::Update;
        hooks.alert(&prefs, &notice, now + Duration::from_secs(301));
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        assert!(hooks.seen.contains(&notice.id));
        assert_eq!(hooks.seen.len(), 1);
    }

    #[test]
    fn disabled_below_threshold_and_cancel_do_not_queue_or_mark_seen() {
        let now = Instant::now();
        let (mut hooks, rx) = local_hooks();
        let mut prefs = enabled_preferences();
        let mut notice = notice(now);
        prefs.enabled = false;
        hooks.alert(&prefs, &notice, now);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        assert!(hooks.seen.is_empty());

        prefs.enabled = true;
        for action in [Action::BelowThreshold, Action::Cancel] {
            notice.action = action;
            hooks.alert(&prefs, &notice, now);
            assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
            assert!(hooks.seen.is_empty());
        }

        notice.action = Action::Alert;
        hooks.alert(&prefs, &notice, now);
        rx.try_recv().unwrap();
    }

    #[test]
    fn once_false_allows_updates_for_the_same_id() {
        let now = Instant::now();
        let (mut hooks, rx) = local_hooks();
        let mut prefs = enabled_preferences();
        prefs.once = false;
        let mut notice = notice(now);
        hooks.alert(&prefs, &notice, now);
        rx.try_recv().unwrap();

        notice.action = Action::Update;
        hooks.alert(&prefs, &notice, now + Duration::from_secs(1));
        rx.try_recv().unwrap();
        assert_eq!(hooks.seen.len(), 1);
    }

    #[test]
    fn id_limit_fails_closed_only_in_once_mode() {
        let now = Instant::now();
        let (mut hooks, rx) = local_hooks();
        let mut prefs = enabled_preferences();
        hooks.seen = (0..MAX_SEEN_IDS - 1)
            .map(|id| format!("previous-{id}"))
            .collect();
        let mut notice = notice(now);
        hooks.alert(&prefs, &notice, now);
        rx.try_recv().unwrap();
        assert_eq!(hooks.seen.len(), MAX_SEEN_IDS);

        notice.id = "overflow-id".into();
        hooks.alert(&prefs, &notice, now);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));

        prefs.once = false;
        hooks.alert(&prefs, &notice, now);
        rx.try_recv().unwrap();
        notice.action = Action::Update;
        hooks.alert(&prefs, &notice, now + Duration::from_secs(1));
        rx.try_recv().unwrap();
        assert_eq!(hooks.seen.len(), MAX_SEEN_IDS);
    }

    #[test]
    fn only_qualifying_updates_have_args_and_high_band_is_conservative() {
        let now = Instant::now();
        let mut demo = Demo::new(PLACES[0].site(), now);
        let mut alert = demo.advance(now).pop().unwrap();
        let estimate = alert.estimate.as_mut().unwrap();
        estimate.level = 5;
        estimate.eta_seconds = 17;
        assert_eq!(
            HookArgs::from_notice(&alert),
            Some(HookArgs {
                level: "5-",
                seconds: 17
            })
        );
        alert.action = Action::Cancel;
        assert_eq!(HookArgs::from_notice(&alert), None);
    }
    #[cfg(unix)]
    #[test]
    fn invokes_real_program_with_two_positional_args() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("quakewatch-hook-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let script = dir.join("script with spaces");
        std::fs::write(
            &script,
            "#!/bin/sh\n[ \"$#\" -eq 2 ] && [ \"$1\" = '4' ] && [ \"$2\" = '10' ]\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = run(&Job {
            program: script,
            args: HookArgs {
                level: "4",
                seconds: 10,
            },
            manual: true,
            queued_at: Instant::now(),
        });
        std::fs::remove_dir_all(dir).unwrap();
        result.unwrap();
    }
}
