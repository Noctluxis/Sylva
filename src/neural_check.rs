// SPDX-License-Identifier: MPL-2.0
//! Explicit offline acceptance/measurement mode. Only public test phrases are
//! written to its report; normal runtime reports never include input text.
use crate::input_check::exchange;
use crate::{
    engine::{Action, Editor, Snapshot, Worker},
    native::Native,
    neural,
};
use chinese_ime_lm::Reranker;
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

const CASES: [(&str, &str); 10] = [
    ("", "woxiangqu"),
    ("周末有时间，", "woxiangqu"),
    ("今晚一起", "kan"),
    ("欢迎来这里，", "nihao"),
    ("我们明天", "keyi"),
    ("这件事情", "wozhidao"),
    ("今天感觉", "hen"),
    ("天气不错，", "women"),
    ("刚才没听清，", "nishuo"),
    ("我喜欢和", "pengyou"),
];
fn phrase(editor: &mut Editor, prefix: &str, keys: &str) {
    editor.apply(Action::Clear);
    for c in prefix.chars() {
        editor.apply(Action::Literal(c));
    }
    for c in keys.chars() {
        editor.apply(Action::Key(c));
    }
    editor.apply(Action::Select(0, editor.state.candidate_revision));
}
pub fn check_mapping(editor: &mut Editor) -> Result<(), String> {
    phrase(editor, "", "woxiangqu");
    let before = editor.state.clone();
    if before.candidates.len() < 2 {
        return Err("mapping fixture needs two continuations".into());
    }
    let mut order: Vec<_> = (0..before.candidates.len()).collect();
    order.swap(0, 1);
    let result = neural::Prediction {
        revision: before.candidate_revision,
        candidates: before.candidates.clone(),
        order,
    };
    if !editor.apply_prediction(result.clone()) {
        return Err("valid neural revision rejected".into());
    }
    if editor.state.sequence != before.sequence {
        return Err("neural result acknowledged a key".into());
    }
    editor.apply(Action::Select(0, before.candidate_revision));
    if editor.state.draft != format!("{}{}", before.draft, before.candidates[0]) {
        return Err("late promotion changed a captured click".into());
    }
    phrase(editor, "", "woxiangqu");
    let stale = neural::Prediction {
        revision: editor.state.candidate_revision,
        candidates: editor.state.candidates.clone(),
        order: (0..editor.state.candidates.len()).collect(),
    };
    editor.apply(Action::Key('n'));
    let unchanged = editor.state.clone();
    if editor.apply_prediction(stale)
        || editor.state.draft != unchanged.draft
        || editor.state.preedit != unchanged.preedit
    {
        return Err("stale neural result replaced active composition".into());
    }
    Ok(())
}
fn stats(mut values: Vec<f64>) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return serde_json::json!({"samples":0});
    }
    serde_json::json!({"samples":values.len(),"p50":values[values.len()/2],"p95":values[values.len()*95/100],"max":values.last()})
}
fn worker_pass(
    resources: &Path,
    root: &Path,
    enabled: bool,
    pass: usize,
) -> Result<serde_json::Value, String> {
    let work = root.join(format!("runtime/neural-check/pass-{pass}"));
    let worker = Worker::start_for_check(resources.into(), work.clone(), None, enabled, false);
    let initial = worker
        .output
        .recv_timeout(Duration::from_secs(30))
        .map_err(|e| e.to_string())?;
    if !initial.ready {
        return Err(initial.status);
    }
    let mut sequence = initial.sequence;
    let mut roundtrip = Vec::new();
    let mut input = Vec::new();
    let mut state = initial;
    let start = Instant::now();
    let cpu = Native::process_resources()[0];
    for _ in 0..2 {
        for (prefix, keys) in CASES {
            exchange(&worker, Action::Clear, &mut sequence)?;
            for c in prefix.chars() {
                exchange(&worker, Action::Literal(c), &mut sequence)?;
            }
            for c in keys.chars() {
                let (s, ms) = exchange(&worker, Action::Key(c), &mut sequence)?;
                roundtrip.push(ms);
                input.push(s.latency_ms);
                state = s;
            }
            state = exchange(
                &worker,
                Action::Select(0, state.candidate_revision),
                &mut sequence,
            )?
            .0;
            // Interrupt some jobs just after their 90 ms debounce. Other jobs
            // are deliberately left idle to test actual result delivery.
            thread::sleep(Duration::from_millis(if sequence % 3 == 0 {
                180
            } else {
                98
            }));
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let cpu_delta = Native::process_resources()[0] - cpu;
    exchange(&worker, Action::Clear, &mut sequence)?;
    for c in "woxiangqu".chars() {
        state = exchange(&worker, Action::Key(c), &mut sequence)?.0;
    }
    state = exchange(
        &worker,
        Action::Select(0, state.candidate_revision),
        &mut sequence,
    )?
    .0;
    let before = state.candidates.clone();
    let until = Instant::now() + Duration::from_secs(2);
    let mut delivered = false;
    while enabled && Instant::now() < until {
        if let Ok(s) = worker.output.recv_timeout(Duration::from_millis(100)) {
            if s.sequence != sequence {
                return Err("unexpected input acknowledgement".into());
            }
            if s.neural_prediction {
                state = s;
                delivered = true;
                break;
            }
        }
    }
    if delivered {
        let chosen = state.candidates[0].clone();
        let draft = state.draft.clone();
        state = exchange(
            &worker,
            Action::Select(0, state.candidate_revision),
            &mut sequence,
        )?
        .0;
        if state.draft != format!("{draft}{chosen}") {
            return Err("neural candidate committed the wrong word".into());
        }
    }
    exchange(&worker, Action::PauseTyping, &mut sequence)?;
    let idle_cpu = Native::process_resources()[0];
    thread::sleep(Duration::from_millis(500));
    let idle_cpu_delta = Native::process_resources()[0] - idle_cpu;
    drop(worker);
    let neural_report = std::fs::read(work.join("neural-performance.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    Ok(
        serde_json::json!({"neural_enabled":enabled,"seconds":elapsed,"process_cpu_seconds":cpu_delta,"key_roundtrip_ms":stats(roundtrip),"editor_service_ms":stats(input),"paused_cpu_seconds_over_500ms":idle_cpu_delta,"real_result_delivered":delivered,"fixture_base_candidates":before,"neural":neural_report}),
    )
}
pub fn run(resources: &Path, root: &Path) -> Result<(), String> {
    let model_path = root.join("models/sentence-model.safetensors");
    let before = Native::process_resources();
    let load_start = Instant::now();
    let model = neural::load_model(&model_path)?;
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.;
    let loaded = Native::process_resources();
    let mut scorer = Reranker::new(model);
    let sample = "今天我们一起到这里游玩大家想先找一个地方休息然后";
    let stress = Snapshot {
        ready: true,
        prediction: true,
        draft: sample.into(),
        caret: sample.len(),
        candidates: [
            "看看风景",
            "吃点东西",
            "买些东西",
            "参观一下",
            "和你聊天",
            "了解情况",
            "放松心情",
            "参加活动",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        ..Default::default()
    };
    let stress_start = Instant::now();
    neural::evaluate_for_check(&mut scorer, &stress)?;
    let stress_ms = stress_start.elapsed().as_secs_f64() * 1000.;
    let warm = Native::process_resources();
    let mut editor = Editor::new(
        resources,
        &root.join("runtime/neural-check/direct"),
        None,
        true,
        false,
    )?;
    check_mapping(&mut editor)?;
    let mut samples = Vec::new();
    let mut fixtures = Vec::new();
    for cycle in 0..3 {
        for (prefix, keys) in CASES {
            phrase(&mut editor, prefix, keys);
            if !editor.state.prediction || editor.state.candidates.len() < 2 {
                continue;
            }
            let start = Instant::now();
            let order = neural::evaluate_for_check(&mut scorer, &editor.state)?;
            let ms = start.elapsed().as_secs_f64() * 1000.;
            samples.push(ms);
            if cycle == 0 {
                fixtures.push(serde_json::json!({"draft":editor.state.draft,"base":editor.state.candidates,"order":order,"ms":ms}));
            }
        }
    }
    if samples.is_empty() {
        return Err("no neural fixtures were scored".into());
    }
    drop(editor);
    msime_engine::shutdown();
    drop(scorer);
    let mut passes = Vec::new();
    for (i, on) in [false, true, true, false].into_iter().enumerate() {
        passes.push(worker_pass(resources, root, on, i)?);
    }
    let out = root.join("reports/neural-check.json");
    std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
    let report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"model_bytes":neural::MODEL_BYTES,"model_sha256":neural::MODEL_HASH,"load_ms":load_ms,"model_private_delta_mib":loaded[2]-before[2],"model_working_set_delta_mib":loaded[1]-before[1],"warm_model_private_delta_mib":warm[2]-before[2],"warm_model_working_set_delta_mib":warm[1]-before[1],"bounded_stress_ms":stress_ms,"score_ms":stats(samples),"fixtures":fixtures,"passes":passes,"mapping_check":"passed","scope":"Explicit public fixture benchmark, CPU only, ABBA editor roundtrip with/without background inference. Stress uses 24 context characters and 32 candidate characters, at most eight candidates. No SteamVR, VRChat, network or clipboard. Not headset frame pacing or quality accuracy evaluation."});
    std::fs::write(out, serde_json::to_vec_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}
