// SPDX-License-Identifier: MPL-2.0
//! Offline checks through the same queue used by the desktop and VR keyboards.
use crate::engine::{Action, Snapshot, Worker};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub fn exchange(
    worker: &Worker,
    action: Action,
    sequence: &mut u64,
) -> Result<(Snapshot, f64), String> {
    let started = Instant::now();
    worker.input.send(action).map_err(|e| e.to_string())?;
    loop {
        let s = worker
            .output
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        if s.sequence > *sequence {
            *sequence = s.sequence;
            return Ok((s, started.elapsed().as_secs_f64() * 1000.));
        }
        if !s.ready && !s.status.is_empty() {
            return Err(s.status);
        }
    }
}

pub fn rows(worker: &Worker, seq: u64) -> Result<Snapshot, String> {
    loop {
        let s = worker
            .output
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        if s.sequence == seq && !s.candidates.is_empty() {
            return Ok(s);
        }
    }
}

pub fn english(resources: &Path, work: &Path) -> Result<String, String> {
    let worker = Worker::start_for_check(resources.into(), work.into(), None, false, false);
    let mut seq = 0;
    exchange(&worker, Action::Language, &mut seq)?;
    let mut ack_ms = Vec::new();
    let mut row_ms = Vec::new();
    for _ in 0..8 {
        for prefix in ["hel", "wor", "fri", "tha", "ple", "qui"] {
            exchange(&worker, Action::Clear, &mut seq)?;
            let mut last_key = Instant::now();
            for (i, c) in prefix.char_indices() {
                last_key = Instant::now();
                let (s, ms) = exchange(&worker, Action::Key(c), &mut seq)?;
                if s.draft != prefix[..i + 1] || !s.preedit.is_empty() {
                    return Err("English key acknowledgement lost the literal input".into());
                }
                ack_ms.push(ms);
            }
            let s = rows(&worker, seq)?;
            row_ms.push(last_key.elapsed().as_secs_f64() * 1000.);
            if s.draft != prefix
                || !s
                    .candidates
                    .iter()
                    .all(|s| s.to_ascii_lowercase().starts_with(prefix))
            {
                return Err("English suggestions did not match the latest prefix".into());
            }
        }
    }
    exchange(&worker, Action::Clear, &mut seq)?;
    for c in "hel".chars() {
        exchange(&worker, Action::Key(c), &mut seq)?;
    }
    let list = rows(&worker, seq)?;
    let hello = list
        .candidates
        .iter()
        .position(|w| w == "hello")
        .ok_or("missing hello completion")?;
    let (s, _) = exchange(&worker, Action::Space, &mut seq)?;
    if s.draft != "hel " || !s.candidates.is_empty() {
        return Err("Space replaced raw English".into());
    }
    for c in "world!".chars() {
        exchange(&worker, Action::Key(c), &mut seq)?;
    }
    let click = Action::MoveCaret {
        offset: 3,
        sequence: seq,
    };
    exchange(&worker, click, &mut seq)?;
    let current = rows(&worker, seq)?;
    let (s, _) = exchange(
        &worker,
        Action::Select(hello, list.candidate_revision),
        &mut seq,
    )?;
    if s.draft != "hel world!" || s.candidates != current.candidates {
        return Err("A stale completion changed the draft or current candidates".into());
    }
    let index = s
        .candidates
        .iter()
        .position(|w| w == "hello")
        .ok_or("missing restored hello")?;
    let (s, _) = exchange(
        &worker,
        Action::Select(index, s.candidate_revision),
        &mut seq,
    )?;
    if s.draft != "hello world!" || s.caret != 5 || !s.candidates.is_empty() {
        return Err("English candidate selection failed".into());
    }
    let click = Action::MoveCaret {
        offset: 8,
        sequence: seq,
    };
    exchange(&worker, click, &mut seq)?;
    let list = rows(&worker, seq)?;
    let index = list
        .candidates
        .iter()
        .position(|w| w == "work")
        .ok_or("missing work completion")?;
    let (s, _) = exchange(
        &worker,
        Action::Select(index, list.candidate_revision),
        &mut seq,
    )?;
    if s.draft != "hello work!" {
        return Err("Mid-word completion left an old suffix".into());
    }
    exchange(&worker, Action::Clear, &mut seq)?;
    for c in "VRChat".chars() {
        worker
            .input
            .send(Action::Key(c))
            .map_err(|e| e.to_string())?;
    }
    worker
        .input
        .send(Action::Space)
        .map_err(|e| e.to_string())?;
    loop {
        let s = worker
            .output
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        if s.sequence == seq + 7 {
            if s.draft != "VRChat " || !s.candidates.is_empty() {
                return Err("Burst typing changed the original word".into());
            }
            break;
        }
    }
    ack_ms.sort_by(f64::total_cmp);
    row_ms.sort_by(f64::total_cmp);
    Ok(format!(
        "PASS English completion, raw space and stale/mid-word clicks. Worker round-trip p95 {:.3} ms ({} keys); final prefix to candidate snapshot p95 {:.3} ms ({} lookups, mixed cache). Excludes UI and HMD.\n",
        ack_ms[ack_ms.len() * 95 / 100],
        ack_ms.len(),
        row_ms[row_ms.len() * 95 / 100],
        row_ms.len()
    ))
}
