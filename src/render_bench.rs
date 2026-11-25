// SPDX-License-Identifier: MPL-2.0
use crate::{
    engine::{self, Action},
    native::Native,
    ui::{Settings, Ui},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
// Render the actual VR feedback atlas offline without starting SteamVR.
pub fn feedback_check(root: &Path) -> Result<(), String> {
    use crate::ui::UiAction;
    let native = Native::offscreen_mode(true)?;
    let out = root.join("screenshots/feedback");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    native.incremental(false);
    let mut previews = Vec::new();
    for (name, pressed) in [
        ("space-hover", false),
        ("space-pressed", true),
        ("letter-hover", false),
        ("candidate-hover", false),
        ("settings-hover", false),
    ] {
        let mut ui = Ui::new(Settings::default(), true, false);
        ui.state.ready = true;
        ui.settings_open = name == "settings-hover";
        ui.settings_general = false;
        if name == "candidate-hover" {
            ui.state.candidates = vec!["我们一起去玩吧".into(), "你好".into(), "明天见".into()];
        }
        ui.rebuild();
        let key = ui
            .buttons
            .iter()
            .find(|b| match name {
                "space-hover" | "space-pressed" => {
                    matches!(b.action, UiAction::Input(Action::Space))
                }
                "letter-hover" => matches!(b.action, UiAction::Input(Action::Key('n'))),
                "candidate-hover" => matches!(b.action, UiAction::Input(Action::Select(0, _))),
                _ => matches!(b.action, UiAction::TypingIndicator),
            })
            .ok_or("missing feedback preview target")?;
        let rect = [key.x, key.y, key.w, key.h];
        ui.render(&native);
        native.present()?;
        native.feedback_preview(rect, pressed)?;
        native.export_raw(&out.join(format!("{name}.png")))?;
        previews.push(serde_json::json!({"name":name,"rect":rect,"pressed":pressed}));
    }
    std::fs::create_dir_all(root.join("reports")).map_err(|e| e.to_string())?;
    std::fs::write(root.join("reports/feedback-check.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "version":env!("CARGO_PKG_VERSION"),
        "scope":"Offline composition of production VR atlas, slice UVs, cropped aspect and transforms. Not a SteamVR/HMD capture.",
        "atlas_bytes":40*432*4,"overlays_per_hand":3,"startup_uploads":6,
        "hover_texture_uploads":0,"previews":previews
    })).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

// Compare the exact RGBA buffer sent to SteamVR with a full redraw after every
// transition. This catches stale text, clipped borders and partial-copy errors.
pub fn check(resources: &Path, root: &Path) -> Result<(), String> {
    let n = Native::offscreen_mode(true)?;
    let mut editor = engine::Editor::new(
        resources,
        &root.join("runtime/render-check"),
        None,
        true,
        false,
    )?;
    let mut ui = Ui::new(Settings::default(), false, false);
    let out = root.join("reports/software/pixel-check");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for frame in 0..48 {
        match frame {
            3..=11 => {
                editor.apply(Action::Key("woxiangqu".chars().nth(frame - 3).unwrap()));
            }
            12 => {
                ui.expanded = true;
            }
            15 => {
                ui.expanded = false;
                editor.apply(Action::Select(0, editor.state.candidate_revision));
            }
            18 => {
                ui.settings.dark = true;
            }
            19 => {
                ui.symbols = true;
            }
            20 => {
                ui.symbols = false;
            }
            21 => {
                ui.settings_open = true;
                ui.settings_general = false;
            }
            22 => {
                ui.settings.typing_indicator = true;
                ui.settings.haptics = false;
            }
            23 => {
                ui.settings_general = true;
                ui.settings.close_to_tray = false;
                ui.settings.silent_start = false;
                ui.settings.typing_indicator = false;
                ui.settings.haptics = true;
            }
            24 => {
                ui.settings_open = false;
            }
            27 => {
                editor.apply(Action::Clear);
                editor.apply(Action::Language);
            }
            30 => {
                editor.apply(Action::Caps);
                editor.apply(Action::Key('a'));
            }
            33 => {
                editor.apply(Action::Caps);
                editor.apply(Action::Key('b'));
            }
            34 => {
                ui.symbols = true;
            }
            35 => {
                ui.symbols = false;
            }
            36 => {
                ui.settings.dark = false;
            }
            37 => {
                editor.apply(Action::Clear);
                for _ in 0..80 {
                    editor.apply(Action::Literal('字'));
                }
            }
            38 | 40 => {
                // Click the first visible character twice as the viewport reveals earlier text.
                let x = 31.;
                let y = 62.;
                let target = ui.hit(x, y).and_then(|id| ui.action(id, x, y));
                let Some(crate::ui::UiAction::Input(action)) = target else {
                    return Err("Long draft did not accept a pointer hit".into());
                };
                let previous = editor.state.caret;
                editor.apply(action);
                if editor.state.caret == 0 || editor.state.caret >= previous {
                    return Err("Long draft hit did not follow the horizontal viewport".into());
                }
            }
            39 => {
                editor.apply(Action::Backspace);
            }
            41 => {
                editor.apply(Action::Language);
                editor.apply(Action::Key('n'));
                editor.apply(Action::Key('i'));
            }
            42 => {
                ui.notice = "局部重绘检查：你好，VRChat！".into();
            }
            45 => {
                ui.notice.clear();
            }
            _ => {}
        }
        ui.state = editor.state.clone();
        ui.rebuild();
        ui.hover = [Some(11 + frame % 10), Some(21 + (frame * 3) % 9), None];
        ui.pressed = [if frame % 2 == 0 { ui.hover[0] } else { None }, None, None];
        n.incremental(true);
        ui.render(&n);
        n.present()?;
        let actual = out.join(format!("{frame:02}-incremental.png"));
        n.export_raw(&actual)?;
        n.incremental(false);
        ui.render(&n);
        n.present()?;
        let expected = out.join(format!("{frame:02}-full.png"));
        n.export_raw(&expected)?;
        if std::fs::read(&actual).map_err(|e| e.to_string())?
            != std::fs::read(&expected).map_err(|e| e.to_string())?
        {
            return Err(format!(
                "Incremental rendering mismatch in frame {frame}: {}",
                out.display()
            ));
        }
    }
    std::fs::write(out.parent().unwrap().join("raw-panel-pixel-check.json"),serde_json::to_vec_pretty(&serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"frames":48,"png_byte_identical":true,"scope":"Exact upload RGBA buffers: incremental vs full CPU rendering; Chinese composition/candidates, hover/press, themes, settings, Caps, deletion and status. No SteamVR/OSC."})).unwrap()).map_err(|e|e.to_string())?;
    drop(editor);
    msime_engine::shutdown();
    Ok(())
}
pub fn run(resources: &Path, root: &Path) -> Result<(), String> {
    let software = std::env::args().any(|a| a == "--software-panel");
    let native = Native::offscreen_mode(software)?;
    let mut editor = engine::Editor::new(
        resources,
        &root.join("runtime/render-bench"),
        None,
        true,
        false,
    )?;
    for c in "woxiangqu".chars() {
        editor.apply(Action::Key(c));
    }
    let mut ui = Ui::new(Settings::default(), false, false);
    ui.state = editor.state.clone();
    ui.rebuild();
    let out = root.join(if software {
        "reports/software"
    } else {
        "reports"
    });
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut passes = Vec::new();
    // ABBA: original DrawText vs cached layout in one binary, no OpenVR or OSC.
    for (pass, cached) in [false, true, true, false].into_iter().enumerate() {
        native.text_cache(cached);
        ui.hover = [None; 3];
        ui.settings.dark = false;
        ui.expanded = false;
        ui.rebuild();
        ui.render(&native);
        native.present()?;
        if pass < 2 {
            native.export(&out.join(if cached {
                "render-cached.png"
            } else {
                "render-baseline.png"
            }))?;
        }
        for frame in 0..40 {
            ui.hover[0] = Some(11 + frame % 10);
            ui.render(&native);
            native.present()?;
            native.wait(8);
        }
        native.reset_metrics();
        let cpu_before = native.process_metrics()[0];
        let start = Instant::now();
        let mut timings = Vec::new();
        for frame in 0..360 {
            let tick = Instant::now();
            if frame % 120 == 0 {
                ui.settings.dark = frame == 120;
                ui.expanded = frame == 240;
                ui.rebuild();
            }
            ui.hover[0] = Some(11 + frame % 10);
            let draw = Instant::now();
            ui.render(&native);
            native.present()?;
            timings.push(draw.elapsed().as_secs_f64() * 1000.);
            let remaining = Duration::from_secs_f64(1. / 120.).saturating_sub(tick.elapsed());
            if !remaining.is_zero() {
                std::thread::sleep(remaining);
            }
        }
        let seconds = start.elapsed().as_secs_f64();
        native.wait(20);
        timings.sort_by(f64::total_cmp);
        let process = native.process_metrics();
        passes.push(serde_json::json!({"pass":pass,"cached":cached,"frames":360,"wall_seconds":seconds,"target_draw_hz":120,"cpu_seconds":process[0]-cpu_before,"working_set_mib":process[1],"private_commit_mib":process[2],"draw_submit_ms":{"p50":timings[180],"p95":timings[342],"max":timings[359]},"graphics":native.graphics_metrics()}));
    }
    let report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"scope":"Offscreen ABBA full-panel benchmark, actual Chinese engine snapshot, light/dark/expanded candidates, no SteamVR/OSC/mirror. GPU timestamps may include scheduling by other processes. Not headset FPS.","passes":passes});
    std::fs::write(
        out.join("render-benchmark.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    drop(editor);
    msime_engine::shutdown();
    Ok(())
}
