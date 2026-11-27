// SPDX-License-Identifier: MPL-2.0
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod engine;
mod english;
mod input_check;
mod key_sound;
mod native;
mod native_metrics;
mod neural;
mod neural_check;
mod osc;
mod perf;
mod pointer;
mod render_bench;
mod startup;
mod translation;
mod ui;
use engine::{Action, Worker};
use native::Native;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::TrySendError,
    time::{Duration, Instant},
};
use ui::{Settings, Ui, UiAction};

fn root_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let parent = exe.parent().unwrap();
    if !parent.join("bindings/actions.json").is_file()
        && (parent.ends_with("debug") || parent.ends_with("release"))
    {
        // Support cargo run and an executable launched directly from target/.
        let manifest = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from);
        if let Some(checkout) = manifest
            .as_deref()
            .into_iter()
            .chain(parent.ancestors())
            .find(|p| p.join("Cargo.toml").is_file() && p.join("bindings/actions.json").is_file())
        {
            return checkout.to_path_buf();
        }
    }
    parent.into()
}
fn save_settings(root: &Path, settings: &Settings) {
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(root.join("settings.json"), json);
    }
}
fn send(worker: &Worker, ui: &mut Ui, action: Action) {
    let play_key_sound = ui.settings.plays_key_sound(&action);
    let action = if matches!(action, Action::Send) && ui.settings.translation.enabled {
        Action::SendTranslated(ui.settings.translation.clone())
    } else {
        action
    };
    match worker.input.try_send(action) {
        Ok(()) => {
            if play_key_sound {
                if let Some(audio) = &ui.key_audio {
                    audio.play();
                }
            }
            ui.pending += 1;
            ui.notice.clear();
            ui.copied = false;
        }
        Err(TrySendError::Full(_)) => ui.notice = "输入队列繁忙，请稍候".into(),
        Err(_) => ui.notice = "输入引擎已停止，请重启".into(),
    }
}
fn activate(
    action: UiAction,
    ui: &mut Ui,
    worker: &Worker,
    native: &Native,
    root: &Path,
    startup: &mut Option<startup::Task>,
) {
    let settings_changed = matches!(
        action,
        UiAction::CloseToTray
            | UiAction::SilentStart
            | UiAction::KeySound
            | UiAction::TypingIndicator
            | UiAction::Haptics
            | UiAction::Dark
            | UiAction::Block
            | UiAction::Narrow
            | UiAction::Widen
            | UiAction::ToggleTranslation
            | UiAction::Pick(_)
    );
    match action {
        UiAction::Input(action) => {
            send(worker, ui, action);
            ui.expanded = false;
            ui.page = 0;
        }
        UiAction::Copy => match native.copy_text(&ui.state.draft) {
            Ok(()) => {
                ui.copied = true;
                ui.notice.clear();
            }
            Err(error) => {
                ui.copied = false;
                ui.notice = error;
            }
        },
        UiAction::Expand => {
            ui.expanded = !ui.expanded;
            ui.settings_open = false;
            ui.page = 0;
        }
        UiAction::Previous => ui.page = ui.page.saturating_sub(1),
        UiAction::Next => ui.page += 1,
        UiAction::Symbols => {
            ui.symbols = !ui.symbols;
            ui.expanded = false;
            ui.settings_open = false;
        }
        UiAction::Settings => {
            ui.settings_open = !ui.settings_open;
            ui.choices = None;
            ui.expanded = false;
        }
        UiAction::SettingsPage(page) => {
            ui.settings_page = page;
            ui.choices = None;
        }
        UiAction::ToggleTranslation => {
            ui.settings.translation.enabled = !ui.settings.translation.enabled
        }
        UiAction::Choose(choice) => ui.choices = Some((choice, 0)),
        UiAction::Pick(index) => ui.pick(index),
        UiAction::ChoicePage(page) => {
            if let Some((choice, _)) = ui.choices {
                ui.choices = Some((choice, page));
            }
        }
        UiAction::CloseChoices => ui.choices = None,
        UiAction::ConfigureService => {
            let result = std::env::current_exe().and_then(|exe| {
                std::process::Command::new(exe)
                    .args(["--translation-account", &ui.settings.translation.provider])
                    .spawn()
            });
            ui.notice = if result.is_ok() {
                "请在桌面窗口配置翻译服务"
            } else {
                "无法打开服务配置窗口"
            }
            .into();
        }
        UiAction::CloseToTray => {
            ui.settings.close_to_tray = !ui.settings.close_to_tray;
            native.lifecycle_settings(ui.settings.close_to_tray);
        }
        UiAction::SilentStart => ui.settings.silent_start = !ui.settings.silent_start,
        UiAction::SteamVrStartup => {
            if startup.is_none() {
                *startup = Some(startup::Task::start(!ui.settings.steamvr_startup));
                ui.startup_pending = true;
            }
        }
        UiAction::KeySound => {
            ui.settings.key_sound = !ui.settings.key_sound;
            if ui.settings.key_sound {
                if let Some(audio) = &ui.key_audio {
                    audio.play();
                }
            }
        }
        UiAction::TypingIndicator => {
            ui.settings.typing_indicator = !ui.settings.typing_indicator;
            send(
                worker,
                ui,
                Action::SetTypingIndicator(ui.settings.typing_indicator),
            );
        }
        UiAction::Haptics => ui.settings.haptics = !ui.settings.haptics,
        UiAction::Dark => ui.settings.dark = !ui.settings.dark,
        UiAction::Block => ui.settings.block_input = !ui.settings.block_input,
        UiAction::Narrow => ui.settings.width_m = (ui.settings.width_m - 0.05).max(0.45),
        UiAction::Widen => ui.settings.width_m = (ui.settings.width_m + 0.05).min(1.4),
        UiAction::Center => native.settings(
            ui.settings.width_m,
            ui.settings.block_input,
            ui.settings.haptics,
            true,
        ),
    }
    if settings_changed {
        native.settings(
            ui.settings.width_m,
            ui.settings.block_input,
            ui.settings.haptics,
            false,
        );
        save_settings(root, &ui.settings);
    }
    ui.rebuild();
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let root = root_dir();
    if let Some(index) = args.iter().position(|s| s == "--translation-account") {
        let service = args
            .get(index + 1)
            .and_then(|s| translation::service(s))
            .filter(|s| s.account())
            .ok_or("未知翻译服务")?;
        return translation::edit_account(service);
    }
    if args.iter().any(|a| a == "--lifecycle-forward-check") {
        return Native::forward_check(args.iter().any(|a| a == "steam"));
    }
    if args.iter().any(|a| a == "--lifecycle-check") {
        let result = Native::lifecycle_check();
        std::fs::create_dir_all(root.join("reports")).map_err(|e| e.to_string())?;
        std::fs::write(
            root.join("reports/lifecycle-check.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"result":result}),
            )
            .unwrap(),
        )
        .map_err(|e| e.to_string())?;
        return result.map(|_| ());
    }
    if let Some(index) = args
        .iter()
        .position(|a| a == "--configure-steamvr-autostart")
    {
        let result = match args.get(index + 1).map(String::as_str) {
            Some("on") => startup::configure(&root, true, false),
            Some("off") => startup::configure(&root, false, false),
            _ => Err("Invalid startup option".into()),
        };
        println!("{}", serde_json::to_string(&result).unwrap());
        return Ok(());
    }
    if args.iter().any(|a| a == "--autostart-check") {
        let result = startup::configure(&root, true, true);
        std::fs::create_dir_all(root.join("reports")).map_err(|e| e.to_string())?;
        std::fs::write(
            root.join("reports/autostart-check.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        return result;
    }
    let profiling = args
        .iter()
        .any(|a| a == "--profile-vr" || a == "--profile-vr-hidden");
    let profile_seconds = if args.iter().any(|a| a == "--profile-vr-hidden") {
        3
    } else {
        10
    };
    let resources = root.join("data");
    let runtime = root.join(if profiling {
        "runtime/vr-profile"
    } else {
        "runtime"
    });
    if args.iter().any(|a| a == "--self-test") {
        return self_test(&resources, &root);
    }
    if args.iter().any(|a| a == "--render-demo") {
        return render_demo(&resources, &root);
    }
    if args.iter().any(|a| a == "--render-bench") {
        return render_bench::run(&resources, &root);
    }
    if args.iter().any(|a| a == "--render-check") {
        return render_bench::check(&resources, &root);
    }
    if args.iter().any(|a| a == "--feedback-check") {
        return render_bench::feedback_check(&root);
    }
    if args.iter().any(|a| a == "--neural-check") {
        return neural_check::run(&resources, &root);
    }
    let mut settings: Settings = std::fs::read(root.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_slice(&s).ok())
        .unwrap_or_default();
    settings.width_m = settings.width_m.clamp(0.45, 1.4);
    let osc = if profiling || args.iter().any(|a| a == "--no-osc") {
        None
    } else {
        let mut osc = osc::Osc::new(
            settings
                .osc_address
                .parse()
                .map_err(|_| "OSC 地址格式须为 IP:端口")?,
        )
        .map_err(|e| e.to_string())?;
        osc.set_typing_enabled(settings.typing_indicator)
            .map_err(|e| e.to_string())?;
        Some(osc)
    };
    let Some(native) = Native::open(
        &root.join("bindings/actions.json"),
        args.iter().any(|a| a == "--desktop"),
        &settings,
    )?
    else {
        return Ok(());
    };
    if profiling && !native.vr() {
        return Err("性能采样需要先连接 SteamVR 头显".into());
    }
    native.settings(
        settings.width_m,
        settings.block_input,
        settings.haptics,
        false,
    );
    let mut startup_task = if settings.steamvr_startup && !profiling {
        Some(startup::Task::start(true))
    } else {
        None
    };
    let mut attach_until = if args.iter().any(|a| a == "--from-steamvr") && !native.vr() {
        Some(Instant::now() + Duration::from_secs(30))
    } else {
        None
    };
    let mut attach_next = Instant::now();
    let worker = Worker::start(resources, runtime, osc);
    let mut ui = Ui::new(
        settings,
        native.vr(),
        !profiling && !args.iter().any(|a| a == "--no-osc"),
    );
    if !Native::error().is_empty() {
        ui.runtime_status = Native::error();
    }
    if !profiling {
        ui.key_audio = Some(key_sound::KeyAudio::start());
    }
    ui.startup_pending = startup_task.is_some();
    ui.rebuild();
    let mut dirty = true;
    let mut running = true;
    let mut pointers: [pointer::Pointer; 3] = std::array::from_fn(|_| pointer::Pointer::default());
    let mut candidate_revision = 0;
    let mut acknowledged_sequence = 0;
    let mut deferred_neural: Option<engine::Snapshot> = None;
    let mut performance = perf::Perf::new();
    let mut feedback_until = [None; 3];
    let mut profile_frame = 0;
    while running {
        native.pump();
        while let Some(e) = native.event() {
            let h = e.hand.clamp(0, 2) as usize;
            match e.kind {
                1 => running = false,
                2 => {
                    let next = ui.hit(e.x, e.y);
                    if ui.hover[h] != next {
                        ui.hover[h] = next;
                        dirty = true;
                    }
                }
                3 => {
                    ui.pressed[h] = ui.hit(e.x, e.y);
                    let action = ui.pressed[h].and_then(|id| ui.action(id, e.x, e.y));
                    if let Some(action) =
                        pointers[h].down(ui.pressed[h], action, h < 2, Instant::now())
                    {
                        feedback_until[h] = Some(Instant::now() + Duration::from_millis(100));
                        activate(action, &mut ui, &worker, &native, &root, &mut startup_task);
                        if ui.settings.haptics {
                            native.feedback(h);
                        }
                    }
                    dirty = true;
                }
                4 => {
                    if let Some(action) = pointers[h].up(ui.hit(e.x, e.y)) {
                        activate(action, &mut ui, &worker, &native, &root, &mut startup_task);
                    }
                    if feedback_until[h].is_none() {
                        ui.pressed[h] = None;
                    }
                    dirty = true;
                }
                5 => {
                    let action = match e.value {
                        8 => Some(Action::Backspace),
                        13 => Some(if ui.state.preedit.is_empty() {
                            Action::Send
                        } else {
                            Action::Space
                        }),
                        27 => Some(Action::Cancel),
                        32 => Some(Action::Space),
                        0x110000 => Some(Action::Caps),
                        _ => char::from_u32(e.value).map(Action::Key),
                    };
                    if let Some(action) = action {
                        send(&worker, &mut ui, action);
                        ui.rebuild();
                        dirty = true;
                    }
                }
                6 => dirty = true,
                7 => {
                    ui.pressed[h] = None;
                    ui.hover[h] = None;
                    pointers[h].cancel();
                    feedback_until[h] = None;
                    dirty = true;
                }
                8 => {
                    if e.value == 0 {
                        send(&worker, &mut ui, Action::PauseTyping);
                    }
                    dirty = true;
                }
                10 => {
                    if !native.vr() && !args.iter().any(|a| a == "--desktop") {
                        attach_until = Some(Instant::now() + Duration::from_secs(30));
                        attach_next = Instant::now();
                    }
                }
                _ => {}
            }
        }
        if let Some(task) = &startup_task {
            if let Some(result) = task.poll() {
                match result {
                    Ok(()) => {
                        ui.settings.steamvr_startup = task.enabled;
                        save_settings(&root, &ui.settings);
                        ui.notice.clear();
                    }
                    Err(error) => ui.notice = format!("启动设置失败：{error}"),
                }
                startup_task = None;
                ui.startup_pending = false;
                ui.rebuild();
                dirty = true;
            }
        }
        if let Some(until) = attach_until {
            if Instant::now() >= attach_next {
                if native.attach_vr() {
                    ui.vr = true;
                    ui.runtime_status = "SteamVR 已连接".into();
                    attach_until = None;
                    ui.rebuild();
                    dirty = true;
                } else if Instant::now() >= until {
                    attach_until = None;
                    ui.runtime_status = Native::error();
                    ui.rebuild();
                    dirty = true;
                }
                attach_next = Instant::now() + Duration::from_secs(2);
            }
        }
        for h in 0..3 {
            if feedback_until[h].is_some_and(|until| Instant::now() >= until) {
                feedback_until[h] = None;
                if pointers[h].pressed.is_none() {
                    ui.pressed[h] = None;
                    dirty = true;
                }
            }
            if pointers[h].repeat(ui.hover[h], Instant::now()) {
                send(&worker, &mut ui, Action::Backspace);
                dirty = true;
            }
        }
        while let Ok(state) = worker.output.try_recv() {
            // Neural/status snapshots do not acknowledge another keypress.
            let acknowledged = state.sequence.saturating_sub(acknowledged_sequence);
            if acknowledged == 0
                && state.neural_prediction
                && state.translating == ui.state.translating
                && state.recoverable == ui.state.recoverable
                && (ui.pending > 0 || ui.candidate_targeted())
            {
                deferred_neural = Some(state);
                continue;
            }
            deferred_neural = None;
            ui.pending = ui.pending.saturating_sub(acknowledged as usize);
            acknowledged_sequence = acknowledged_sequence.max(state.sequence);
            if candidate_revision != state.candidate_revision {
                ui.page = 0;
                candidate_revision = state.candidate_revision;
            }
            ui.state = state;
            ui.rebuild();
            dirty = true;
        }
        // Keep the row still while either hand (or the mouse) is aiming at a
        // candidate. The editor retains the prior mapping for captured clicks.
        if ui.pending == 0 && !ui.candidate_targeted() {
            if let Some(state) = deferred_neural.take() {
                if state.sequence == acknowledged_sequence {
                    if candidate_revision != state.candidate_revision {
                        ui.page = 0;
                        candidate_revision = state.candidate_revision;
                    }
                    ui.state = state;
                    ui.rebuild();
                    dirty = true;
                }
            }
        }
        if (dirty || profiling) && running {
            if profiling {
                // Force actual bitmap changes every profiling iteration. Normal
                // VR hover uses preloaded overlays and does not redraw keycaps.
                profile_frame += 1;
                ui.hover[2] = Some(11 + profile_frame % 10);
                ui.vr = false;
            }
            let start = Instant::now();
            ui.render(&native);
            ui.vr = native.vr();
            native.present()?;
            performance.render(start.elapsed().as_secs_f64() * 1000.);
            dirty = false;
        }
        let start = Instant::now();
        native.mirror()?;
        performance.mirror(start.elapsed().as_secs_f64() * 1000.);
        if profiling && performance.elapsed() >= Duration::from_secs(profile_seconds) {
            running = false;
        }
        performance.save(&native, &root, !running);
        native.wait(if native.visible() {
            11
        } else if native.vr() {
            33
        } else if native.desktop_visible() {
            25
        } else {
            250
        });
    }
    if let Some(task) = startup_task {
        let (enabled, result) = task.finish();
        if result.is_ok() {
            ui.settings.steamvr_startup = enabled;
            save_settings(&root, &ui.settings);
        }
    }
    drop(worker);
    drop(native);
    Ok(())
}
fn self_test(resources: &Path, root: &Path) -> Result<(), String> {
    let mut log = String::new();
    let test_dir = root.join("runtime/self-test");
    let mut e = engine::Editor::new(resources, &test_dir, None, true, false)?;
    for pinyin in ["nihao", "nh", "dwyx", "woxiangqu", "zhongguo"] {
        e.apply(Action::Clear);
        let start = Instant::now();
        for c in pinyin.chars() {
            e.apply(Action::Key(c));
        }
        if e.state.candidates.is_empty() {
            return Err(format!("{pinyin}: no candidates"));
        }
        log.push_str(&format!(
            "{pinyin}: {:?}; {:.2} ms\n",
            &e.state.candidates[..e.state.candidates.len().min(5)],
            start.elapsed().as_secs_f64() * 1000.
        ));
        if pinyin == "nihao" && !e.state.candidates.iter().any(|c| c == "你好") {
            return Err("nihao missing 你好".into());
        }
        if pinyin == "nh" && !e.state.candidates.iter().any(|c| c == "你好") {
            return Err("nh missing 你好".into());
        }
        e.apply(Action::Select(0, e.state.candidate_revision));
        log.push_str(&format!(
            "commit: {}; prediction: {:?}\n",
            e.state.draft,
            &e.state.candidates[..e.state.candidates.len().min(8)]
        ));
    }
    e.apply(Action::Clear);
    e.apply(Action::Language);
    e.apply(Action::Caps);
    e.apply(Action::Key('a'));
    e.apply(Action::Key('1'));
    e.apply(Action::Caps);
    e.apply(Action::Key('b'));
    if e.state.draft != "A1b" {
        return Err(format!("caps mismatch: {}", e.state.draft));
    }
    e.apply(Action::Caps);
    e.apply(Action::Language);
    e.apply(Action::Key('n'));
    e.apply(Action::Key('h'));
    if e.state.preedit.to_lowercase() != e.state.preedit {
        return Err("caps affected pinyin".into());
    }
    let revision = e.state.candidate_revision;
    e.apply(Action::Key('a'));
    let before = e.state.draft.clone();
    e.apply(Action::Select(0, revision));
    if e.state.draft != before {
        return Err("stale candidate committed".into());
    }
    log.push_str("PASS Caps, language isolation, stale candidate rejection.\n");
    for symbol in ['2', '“', '”', '、', '@', '\''] {
        e.apply(Action::Clear);
        for c in "nihao".chars() {
            e.apply(Action::Key(c));
        }
        let first = e
            .state
            .candidates
            .first()
            .ok_or("missing literal-layer candidate")?
            .clone();
        e.apply(Action::Literal(symbol));
        if !e.state.preedit.is_empty() || e.state.draft != format!("{first}{symbol}") {
            return Err(format!("literal layer failed for {symbol}: {:?}", e.state));
        }
    }
    e.apply(Action::Clear);
    e.apply(Action::Language);
    for c in "1234567890~!@#%\"'*?()-_:;/".chars() {
        e.apply(Action::Literal(c));
    }
    if e.state.draft != "1234567890~!@#%\"'*?()-_:;/" {
        return Err("English symbol layer changed literal characters".into());
    }
    e.apply(Action::Language);
    log.push_str("PASS literal number/symbol layer, composition commit, English symbols.\n");
    e.apply(Action::Clear);
    e.apply(Action::SetPrediction(false));
    for c in "nihao".chars() {
        e.apply(Action::Key(c));
    }
    e.apply(Action::Select(0, e.state.candidate_revision));
    if e.state.prediction {
        return Err("prediction disable failed".into());
    }
    e.apply(Action::Clear);
    e.apply(Action::SetPrediction(true));
    for c in "woxiangqu".chars() {
        e.apply(Action::Key(c));
    }
    e.apply(Action::Select(0, e.state.candidate_revision));
    if !e.state.prediction || e.state.candidates.is_empty() {
        return Err("prediction enable failed".into());
    }
    let continuation = e.state.candidates[0].clone();
    e.apply(Action::Select(0, e.state.candidate_revision));
    if e.state.draft != format!("我想去{continuation}") {
        return Err("prediction selection failed".into());
    }
    log.push_str("PASS prediction disable/enable and explicit continuation selection.\n");
    let mut timings = Vec::new();
    for _ in 0..20 {
        for phrase in ["nihao", "nh", "dwyx", "woxiangqu", "zhongguo"] {
            e.apply(Action::Clear);
            for c in phrase.chars() {
                e.apply(Action::Key(c));
                timings.push(e.state.latency_ms);
            }
        }
    }
    timings.sort_by(f64::total_cmp);
    log.push_str(&format!("Engine key latency: {} samples, p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms. Includes query+snapshot; excludes UI.\n",timings.len(),timings[timings.len()/2],timings[timings.len()*95/100],timings[timings.len()-1]));
    drop(e);
    msime_engine::shutdown();
    log.push_str(&input_check::english(resources, &test_dir.join("english"))?);
    std::fs::write(root.join("self-test.txt"), &log).map_err(|e| e.to_string())?;
    Ok(())
}
fn render_demo(resources: &Path, root: &Path) -> Result<(), String> {
    let software = std::env::args().any(|a| a == "--software-panel");
    let n = Native::offscreen_mode(software)?;
    let mut e = engine::Editor::new(
        resources,
        &root.join("runtime/render-test"),
        None,
        true,
        false,
    )?;
    let out = root.join(if software {
        "screenshots/software"
    } else {
        "screenshots"
    });
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut ui = Ui::new(Settings::default(), false, false);
    ui.state = e.state.clone();
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-idle.png"))?;
    for c in "nihao".chars() {
        e.apply(Action::Key(c));
    }
    e.apply(Action::Select(0, e.state.candidate_revision));
    e.apply(Action::Key(','));
    for c in "woxiangqu".chars() {
        e.apply(Action::Key(c));
    }
    ui.state = e.state.clone();
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-pinyin.png"))?;
    ui.settings_open = true;
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-settings-composing.png"))?;
    ui.settings_page = crate::ui::SettingsPage::Input;
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-settings-input.png"))?;
    ui.settings_page = crate::ui::SettingsPage::Translation;
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-settings-translation.png"))?;
    ui.settings_page = crate::ui::SettingsPage::General;
    ui.settings_open = false;
    ui.symbols = true;
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-symbols.png"))?;
    ui.symbols = false;
    ui.expanded = true;
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-candidates.png"))?;
    ui.expanded = false;
    e.apply(Action::Select(0, e.state.candidate_revision));
    ui.state = e.state.clone();
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-prediction.png"))?;
    e.apply(Action::Clear);
    e.apply(Action::Language);
    e.apply(Action::Caps);
    for c in "hello vrchat".chars() {
        if c == ' ' {
            e.apply(Action::Space);
        } else {
            e.apply(Action::Key(c));
        }
    }
    ui.state = e.state.clone();
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-caps.png"))?;
    ui.settings.dark = true;
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-dark.png"))?;
    ui.settings_open = true;
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-settings.png"))?;
    // Exercise the actual shaped line and trigger hit before capturing a mid-sentence edit.
    e.apply(Action::Clear);
    e.apply(Action::Language);
    for c in "我们晚上九点在门口见".chars() {
        e.apply(Action::Literal(c));
    }
    ui.settings_open = false;
    ui.settings.dark = false;
    ui.state = e.state.clone();
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    let x = 30. + Native::text_width("我们晚上九", 26.);
    let target = ui.hit(x, 62.).and_then(|id| ui.action(id, x, 62.));
    let Some(UiAction::Input(action)) = target else {
        return Err("草稿文字未命中".into());
    };
    e.apply(action);
    if e.state.before_caret() != "我们晚上九" {
        return Err("草稿光标定位错误".into());
    }
    e.apply(Action::Backspace);
    for c in "ba".chars() {
        e.apply(Action::Key(c));
    }
    ui.state = e.state.clone();
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-draft-edit.png"))?;
    let index = e
        .state
        .candidates
        .iter()
        .position(|s| s == "八")
        .ok_or("缺少候选「八」")?;
    e.apply(Action::Select(index, e.state.candidate_revision));
    if e.state.draft != "我们晚上八点在门口见" {
        return Err("句中选词错误".into());
    }
    drop(e);
    msime_engine::shutdown();
    let worker = Worker::start_for_check(
        resources.into(),
        root.join("runtime/render-english"),
        None,
        false,
        false,
    );
    let mut sequence = 0;
    input_check::exchange(&worker, Action::Language, &mut sequence)?;
    for c in "hel".chars() {
        input_check::exchange(&worker, Action::Key(c), &mut sequence)?;
    }
    ui.state = input_check::rows(&worker, sequence)?;
    ui.rebuild();
    ui.render(&n);
    n.present()?;
    n.export(&out.join("native-english-completion.png"))?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        let root = root_dir();
        let _ = std::fs::write(root.join("last-error.txt"), &error);
        if !std::env::args().any(|a| {
            a == "--self-test"
                || a == "--lifecycle-check"
                || a == "--lifecycle-forward-check"
                || a == "--render-demo"
                || a == "--render-check"
                || a == "--feedback-check"
                || a == "--neural-check"
                || a == "--render-bench"
                || a == "--profile-vr"
                || a == "--autostart-check"
                || a == "--profile-vr-hidden"
        }) {
            Native::message(&error);
        }
        std::process::exit(1);
    }
}
