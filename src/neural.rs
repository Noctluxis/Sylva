// SPDX-License-Identifier: MPL-2.0
//! Bounded, optional enhancement of already-confirmed text's next-word choices.
//! No model call runs on the editor, render, or VR input threads.
use crate::{
    engine::{Action, Snapshot},
    native::Native,
};
use chinese_ime_lm::{Reranker, SentenceModel};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const MODEL_BYTES: u64 = 4_486_280;
pub const MODEL_HASH: &str = "86ac529510cb3b4968a5e6ade83ec8080f5b34a0a681e75c362accbbd387d1c1";
const CONTEXT_CHARS: usize = 24;
const MAX_CANDIDATES: usize = 8;
const WORD_CHARS: usize = 8;
const TOTAL_CHARS: usize = 32;
const DEBOUNCE: Duration = Duration::from_millis(90);

#[derive(Clone)]
struct Request {
    generation: u64,
    revision: u64,
    created: Instant,
    context: String,
    candidates: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Prediction {
    pub revision: u64,
    pub candidates: Vec<String>,
    pub order: Vec<usize>,
}
#[derive(Default, Clone, serde::Serialize)]
struct Metrics {
    background_priority: bool,
    ready: bool,
    error: Option<String>,
    model_load_ms: f64,
    model_private_delta_mib: f64,
    requests: u64,
    runs: u64,
    cache_hits: u64,
    cancelled: u64,
    published: u64,
    promoted: u64,
    work_ms: Vec<f64>,
}
struct Shared {
    pending: Mutex<Option<Request>>,
    available: Condvar,
    generation: AtomicU64,
    health: AtomicU8,
    requests: AtomicU64,
    stop: AtomicBool,
    metrics: Mutex<Metrics>,
}
pub struct Predictor {
    shared: Arc<Shared>,
    results: mpsc::Receiver<Prediction>,
    thread: Option<thread::JoinHandle<()>>,
    report: PathBuf,
}
pub fn load_model(path: &Path) -> Result<Arc<SentenceModel>, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() != MODEL_BYTES {
        return Err("neural model size mismatch".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if digest != MODEL_HASH {
        return Err("neural model SHA256 mismatch".into());
    }
    let model = SentenceModel::load(&bytes).map_err(|e| e.to_string())?;
    if model.license() != "Apache-2.0" || model.attribution().is_empty() {
        return Err("neural model provenance missing".into());
    }
    Ok(Arc::new(model))
}

impl Predictor {
    pub fn start(
        model: PathBuf,
        notify: mpsc::SyncSender<Action>,
        report: PathBuf,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            pending: Mutex::new(None),
            available: Condvar::new(),
            generation: AtomicU64::new(0),
            health: AtomicU8::new(0),
            requests: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            metrics: Mutex::new(Metrics::default()),
        });
        let (tx, results) = mpsc::sync_channel(1);
        let task = shared.clone();
        let thread = thread::Builder::new()
            .name("sylva-neural".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(&model, &task, &tx, &notify);
                }));
                if outcome.is_err() {
                    task.health.store(2, Ordering::Release);
                    if let Ok(mut metrics) = task.metrics.lock() {
                        metrics.ready = false;
                        metrics.error = Some("neural worker stopped unexpectedly".into());
                    }
                    let _ = notify.try_send(Action::SuggestionsReady);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            results,
            thread: Some(thread),
            report,
        })
    }
    /// Fast invalidation, never waits for inference. At most one chunk can still
    /// finish on the background thread; its result is checked again before use.
    pub fn cancel(&self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut pending) = self.shared.pending.try_lock() {
            pending.take();
        }
        self.shared.available.notify_one();
    }
    pub fn request(&self, state: &Snapshot) {
        if !state.ready
            || state.english
            || !state.prediction
            || !state.preedit.is_empty()
            || state.before_caret().trim().is_empty()
            || state.candidates.len() < 2
        {
            return;
        }
        if self.shared.health.load(Ordering::Acquire) == 2 {
            return;
        }
        let tail: Vec<char> = state
            .before_caret()
            .chars()
            .rev()
            .take(CONTEXT_CHARS)
            .collect();
        let request = Request {
            generation: self.shared.generation.load(Ordering::Acquire),
            revision: state.candidate_revision,
            created: Instant::now(),
            context: tail.into_iter().rev().collect(),
            candidates: state
                .candidates
                .iter()
                .take(MAX_CANDIDATES)
                .cloned()
                .collect(),
        };
        // A contended slot simply skips this optional enhancement. The editor
        // never waits for a background thread, including its statistics lock.
        if let Ok(mut pending) = self.shared.pending.try_lock() {
            *pending = Some(request);
            self.shared.requests.fetch_add(1, Ordering::Relaxed);
            self.shared.available.notify_one();
        }
    }
    pub fn take_result(&self) -> Option<Prediction> {
        self.results.try_recv().ok()
    }
    pub fn status(&self) -> String {
        let health = self.shared.health.load(Ordering::Acquire);
        if health == 2 {
            "基础联想已开启"
        } else if health == 1 {
            "本地神经联想已就绪"
        } else {
            "智能联想已开启"
        }
        .into()
    }
    pub fn report(&self) -> serde_json::Value {
        let mut m = self.shared.metrics.lock().unwrap().clone();
        m.requests = self.shared.requests.load(Ordering::Relaxed);
        m.work_ms.sort_by(f64::total_cmp);
        let timing = if m.work_ms.is_empty() {
            serde_json::json!({"samples":0})
        } else {
            serde_json::json!({"samples":m.work_ms.len(),"p50":m.work_ms[m.work_ms.len()/2],"p95":m.work_ms[m.work_ms.len()*95/100],"max":m.work_ms.last()})
        };
        serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"metrics":m,"inference_ms":timing,
            "limits":{"context_chars":CONTEXT_CHARS,"candidates":MAX_CANDIDATES,"word_chars":WORD_CHARS,"candidate_char_budget":TOTAL_CHARS,"debounce_ms":90,"score_budget_ms":null,"request_expiry_ms":null,"cache_entries":16,"pending_requests":1},
            "scope":"Bounded local counters/timings only; no typed text. CPU worker, no GPU or network. Cancellation checked between two-candidate calls; current results are not rejected by elapsed time."})
    }
}
impl Drop for Predictor {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(parent) = self.report.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.report()) {
            let _ = std::fs::write(&self.report, bytes);
        }
    }
}
#[derive(Clone)]
struct Cached {
    context: String,
    candidates: Vec<String>,
    order: Vec<usize>,
}
fn obsolete(shared: &Shared, request: &Request) -> bool {
    shared.stop.load(Ordering::Acquire)
        || shared.generation.load(Ordering::Acquire) != request.generation
}
fn run(
    model_path: &Path,
    shared: &Shared,
    tx: &mpsc::SyncSender<Prediction>,
    notify: &mpsc::SyncSender<Action>,
) {
    shared.metrics.lock().unwrap().background_priority = Native::lower_thread_priority();
    let mut scorer = None;
    let mut cache: VecDeque<Cached> = VecDeque::new();
    loop {
        let request = {
            let mut pending = shared.pending.lock().unwrap();
            loop {
                if shared.stop.load(Ordering::Acquire) {
                    return;
                }
                if let Some(request) = pending.as_ref() {
                    let due = request.created + DEBOUNCE;
                    if Instant::now() >= due {
                        break pending.take().unwrap();
                    }
                    pending = shared
                        .available
                        .wait_timeout(pending, due.saturating_duration_since(Instant::now()))
                        .unwrap()
                        .0;
                } else {
                    pending = shared.available.wait(pending).unwrap();
                }
            }
        };
        if obsolete(shared, &request) {
            continue;
        }
        if scorer.is_none() {
            let before = Native::process_resources();
            let started = Instant::now();
            match load_model(model_path) {
                Ok(model) => {
                    scorer = Some(Reranker::new(model));
                    shared.health.store(1, Ordering::Release);
                    let mut m = shared.metrics.lock().unwrap();
                    m.ready = true;
                    m.model_load_ms = started.elapsed().as_secs_f64() * 1000.;
                    m.model_private_delta_mib = Native::process_resources()[2] - before[2];
                }
                Err(error) => {
                    shared.health.store(2, Ordering::Release);
                    shared.metrics.lock().unwrap().error = Some(error);
                    let _ = notify.try_send(Action::SuggestionsReady);
                    return;
                }
            }
            let _ = notify.try_send(Action::SuggestionsReady);
        }
        if obsolete(shared, &request) {
            shared.metrics.lock().unwrap().cancelled += 1;
            continue;
        }
        let cached = cache
            .iter()
            .position(|c| c.context == request.context && c.candidates == request.candidates);
        let order = if let Some(index) = cached {
            shared.metrics.lock().unwrap().cache_hits += 1;
            let item = cache.remove(index).unwrap();
            let order = item.order.clone();
            cache.push_back(item);
            order
        } else {
            let started = Instant::now();
            let scored = score(
                scorer.as_mut().unwrap(),
                &request.context,
                &request.candidates,
                || obsolete(shared, &request),
            );
            let elapsed = started.elapsed();
            {
                let mut m = shared.metrics.lock().unwrap();
                m.runs += 1;
                if m.work_ms.len() < 256 {
                    m.work_ms.push(elapsed.as_secs_f64() * 1000.);
                }
            }
            let order = match scored {
                Ok(order) => order,
                Err(ScoreStop::Cancelled) => {
                    shared.metrics.lock().unwrap().cancelled += 1;
                    continue;
                }
            };
            if cache.len() == 16 {
                cache.pop_front();
            }
            cache.push_back(Cached {
                context: request.context.clone(),
                candidates: request.candidates.clone(),
                order: order.clone(),
            });
            order
        };
        if obsolete(shared, &request) {
            shared.metrics.lock().unwrap().cancelled += 1;
            continue;
        }
        let promoted = order.first().is_some_and(|&i| i != 0);
        if tx
            .try_send(Prediction {
                revision: request.revision,
                candidates: request.candidates,
                order,
            })
            .is_ok()
        {
            let mut m = shared.metrics.lock().unwrap();
            m.published += 1;
            if promoted {
                m.promoted += 1;
            }
            drop(m);
            let _ = notify.try_send(Action::SuggestionsReady);
        }
    }
}
#[derive(Debug, PartialEq)]
enum ScoreStop {
    Cancelled,
}
fn score(
    scorer: &mut Reranker,
    context: &str,
    candidates: &[String],
    cancelled: impl Fn() -> bool,
) -> Result<Vec<usize>, ScoreStop> {
    let mut used = 0;
    let indices: Vec<usize> = candidates
        .iter()
        .take(MAX_CANDIDATES)
        .enumerate()
        .filter_map(|(i, word)| {
            let length = word.chars().count();
            if length == 0
                || length > WORD_CHARS
                || used + length > TOTAL_CHARS
                || !word.chars().all(|c| scorer.model().token(c).is_some())
            {
                return None;
            }
            used += length;
            Some(i)
        })
        .collect();
    let mut scores = vec![None; candidates.len()];
    if indices.len() >= 2 && indices[0] == 0 {
        for chunk in indices.chunks(2) {
            if cancelled() {
                return Err(ScoreStop::Cancelled);
            }
            let words: Vec<_> = chunk.iter().map(|&i| candidates[i].as_str()).collect();
            let values = scorer.log_probabilities(context, &words);
            for (&i, value) in chunk.iter().zip(values) {
                if value.is_finite() {
                    scores[i] = Some(value);
                }
            }
            if cancelled() {
                return Err(ScoreStop::Cancelled);
            }
            thread::yield_now();
        }
    }
    Ok(promote(&scores))
}
// Preserve the dictionary/personal order unless the model has a clear winner.
// Only one promotion, so a late result does not reshuffle the whole row.
fn promote(scores: &[Option<f32>]) -> Vec<usize> {
    let mut order: Vec<_> = (0..scores.len()).collect();
    let Some(Some(first)) = scores.first() else {
        return order;
    };
    let mut best = (0, *first);
    for (i, &score) in scores.iter().enumerate().skip(1) {
        if let Some(value) = score {
            let adjusted = value - 0.08 * i as f32;
            if adjusted > best.1 {
                best = (i, adjusted);
            }
        }
    }
    if best.0 != 0 && best.1 > *first + 0.35 {
        order.remove(best.0);
        order.insert(0, best.0);
    }
    order
}

pub fn evaluate_for_check(scorer: &mut Reranker, state: &Snapshot) -> Result<Vec<usize>, String> {
    let context: String = state
        .before_caret()
        .chars()
        .rev()
        .take(CONTEXT_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    score(scorer, &context, &state.candidates, || false).map_err(|e| format!("{e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conservative_promotion_preserves_all_candidate_indices() {
        assert_eq!(promote(&[Some(-2.), Some(-1.9), Some(-2.1)]), vec![0, 1, 2]);
        assert_eq!(
            promote(&[Some(-3.), Some(-1.), None, Some(-2.)]),
            vec![1, 0, 2, 3]
        );
        assert_eq!(promote(&[None, Some(-1.)]), vec![0, 1]);
    }
    #[test]
    fn model_lock_rejects_missing_or_wrong_size_files_without_loading() {
        assert!(load_model(Path::new("models/not-a-model.safetensors")).is_err());
        assert!(load_model(Path::new("Cargo.toml")).is_err());
    }
    #[test]
    fn busy_slot_does_not_block_input_and_missing_model_keeps_basic_prediction() {
        let (notify, notifications) = mpsc::sync_channel(4);
        let predictor = Predictor::start(
            "models/not-a-model.safetensors".into(),
            notify,
            "runtime/neural-unit/missing-model.json".into(),
        )
        .unwrap();
        let state = Snapshot {
            ready: true,
            prediction: true,
            draft: "你好".into(),
            caret: "你好".len(),
            candidates: vec!["世界".into(), "朋友".into()],
            ..Default::default()
        };
        let slot = predictor.shared.pending.lock().unwrap();
        predictor.cancel();
        predictor.request(&state);
        assert!(slot.is_none());
        drop(slot);
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            predictor.request(&state);
            if notifications
                .recv_timeout(Duration::from_millis(250))
                .is_ok()
            {
                break;
            }
            assert!(
                Instant::now() < until,
                "missing-model fallback did not finish"
            );
        }
        assert_eq!(predictor.status(), "基础联想已开启");
        assert!(predictor.take_result().is_none());
        assert!(!predictor.shared.metrics.lock().unwrap().ready);
    }
}
