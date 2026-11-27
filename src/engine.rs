// SPDX-License-Identifier: MPL-2.0
use crate::{english, osc::Osc};
use msime_engine::{
    Command, DictionaryAccess, KeyResult, SchemeSet, SchemeType, Session, SessionOptions,
    prepare_runtime_paths_for,
};
use std::{
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug)]
pub enum Action {
    Key(char),
    Literal(char),
    Select(usize, u64),
    Space,
    Backspace,
    MoveCaret { offset: usize, sequence: u64 },
    Language,
    Caps,
    Clear,
    Cancel,
    Send,
    SendTranslated(crate::translation::Config),
    RestoreUnsent,
    SetPrediction(bool),
    SetTypingIndicator(bool),
    PauseTyping,
    SuggestionsReady,
    Stop,
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub sequence: u64,
    pub candidate_revision: u64,
    pub draft: String,
    /// Byte offset at an extended grapheme boundary in the committed draft.
    pub caret: usize,
    pub preedit: String,
    pub candidates: Vec<String>,
    pub prediction: bool,
    pub english: bool,
    pub caps: bool,
    pub ready: bool,
    pub translating: bool,
    pub recoverable: bool,
    pub status: String,
    pub latency_ms: f64,
    pub neural_status: String,
    pub neural_prediction: bool,
}
impl Snapshot {
    pub fn before_caret(&self) -> &str {
        &self.draft[..self.caret]
    }
    fn boundary_at_or_after(&self, offset: usize) -> usize {
        if offset == self.draft.len() {
            return offset;
        }
        self.draft
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|&i| i >= offset)
            .unwrap_or(self.draft.len())
    }
    fn insert(&mut self, text: &str) {
        self.draft.insert_str(self.caret, text);
        self.caret = self.boundary_at_or_after(self.caret + text.len());
    }
    fn backspace(&mut self) {
        if let Some((start, _)) = self.before_caret().grapheme_indices(true).next_back() {
            self.draft.replace_range(start..self.caret, "");
            self.caret = self.boundary_at_or_after(start);
        }
    }
    fn clear_draft(&mut self) {
        self.draft.clear();
        self.caret = 0;
    }
    pub fn count(&self) -> usize {
        self.draft.encode_utf16().count()
    }
    pub fn can_send(&self) -> bool {
        self.ready
            && !self.translating
            && self.preedit.is_empty()
            && !self.draft.trim().is_empty()
            && self.count() <= 144
            && self.draft.lines().count() <= 9
    }
}

pub struct Editor {
    session: Session,
    _access: DictionaryAccess,
    pub state: Snapshot,
    osc: Option<Osc>,
    last_input: Instant,
    engine_revision: u64,
    candidate_map: Vec<usize>,
    previous_map: Option<(u64, Vec<usize>)>,
    unsent: std::collections::VecDeque<String>,
}
impl Editor {
    pub fn new(
        resources: &Path,
        work: &Path,
        osc: Option<Osc>,
        prediction: bool,
        learning: bool,
    ) -> Result<Self, String> {
        let schemes = SchemeSet::of(&[SchemeType::Quanpin]);
        let paths = prepare_runtime_paths_for(
            resources,
            &work.join("user"),
            &work.join("cache"),
            "quanpin-2_0_41",
            schemes,
        )
        .map_err(|e| e.to_string())?;
        let access = DictionaryAccess::try_session(&paths.user_data, &paths.dictionaries)
            .map_err(|e| e.to_string())?
            .ok_or("词库正在维护，请稍后重试")?;
        let mut options = SessionOptions::new(paths);
        options.enabled_schemes = schemes;
        options.next_word_prediction = true;
        options.personal_context = learning;
        options.learning = learning;
        options.first_page_rows = 8;
        // Only the Chinese and English dictionaries are distributed; no online or expressive providers.
        options.mixed_candidates = false;
        let mut session = Session::new(options).map_err(|e| e.to_string())?;
        session.set_cloud_candidates_enabled(false);
        session.set_english_online_candidates_enabled(false);
        session.set_prediction_allowed(prediction);
        let mut editor = Self {
            session,
            _access: access,
            state: Snapshot {
                ready: true,
                status: "水杉已就绪".into(),
                ..Default::default()
            },
            osc,
            last_input: Instant::now(),
            engine_revision: 0,
            candidate_map: Vec::new(),
            previous_map: None,
            unsent: Default::default(),
        };
        editor.refresh();
        Ok(editor)
    }
    fn refresh(&mut self) {
        let s = self.session.display_snapshot();
        self.state.preedit = s.preedit;
        self.state.prediction = s.prediction;
        self.state.candidates = if self.state.english {
            Vec::new()
        } else {
            s.candidates
                .into_iter()
                .take(256)
                .map(|x| x.word.to_string())
                .collect()
        };
        self.engine_revision = self.session.candidate_revision();
        self.candidate_map = (0..self.state.candidates.len()).collect();
        self.previous_map = None;
        self.state.candidate_revision = self.state.candidate_revision.wrapping_add(1);
        self.state.neural_prediction = false;
    }
    pub fn apply_prediction(&mut self, result: crate::neural::Prediction) -> bool {
        let n = result.candidates.len();
        if result.revision != self.state.candidate_revision
            || !self.state.prediction
            || self.state.english
            || !self.state.preedit.is_empty()
            || n > self.state.candidates.len()
            || self.state.candidates[..n] != result.candidates
            || self.engine_revision != self.session.candidate_revision()
        {
            return false;
        }
        let mut sorted = result.order.clone();
        sorted.sort_unstable();
        if sorted != (0..n).collect::<Vec<_>>() {
            return false;
        }
        if result.order.iter().enumerate().any(|(i, &j)| i != j) {
            // A click already captured from the original row must still select
            // the original word when this asynchronous promotion overtakes it.
            self.previous_map = Some((self.state.candidate_revision, self.candidate_map.clone()));
            let mut candidates: Vec<_> = result
                .order
                .iter()
                .map(|&i| self.state.candidates[i].clone())
                .collect();
            let mut mapping: Vec<_> = result
                .order
                .iter()
                .map(|&i| self.candidate_map[i])
                .collect();
            candidates.extend_from_slice(&self.state.candidates[n..]);
            mapping.extend_from_slice(&self.candidate_map[n..]);
            self.state.candidates = candidates;
            self.candidate_map = mapping;
            self.state.candidate_revision = self.state.candidate_revision.wrapping_add(1);
        }
        self.state.neural_prediction = true;
        true
    }
    fn accept_english(&mut self, revision: u64, rows: Vec<String>) -> bool {
        if !self.state.english || revision != self.state.candidate_revision || rows.is_empty() {
            return false;
        }
        self.state.candidates = rows;
        self.state.candidate_revision = revision.wrapping_add(1);
        true
    }
    fn consume(&mut self, result: KeyResult) {
        if let Some(text) = result.commit {
            self.state.insert(&text);
        }
        if let Some(message) = result.diagnostic {
            self.state.status = message;
        }
    }
    fn reset_context(&mut self) {
        self.session.reset_context();
        self.session
            .set_rescoring_context(self.state.before_caret());
    }
    fn type_literal(&mut self, c: char) {
        self.session.command(Command::Cancel);
        self.state.insert(c.encode_utf8(&mut [0; 4]));
        self.reset_context();
    }
    fn composing(&self) -> bool {
        !self.session.display_snapshot().preedit.is_empty()
    }
    fn begin_translation(
        &mut self,
        config: crate::translation::Config,
        notify: mpsc::SyncSender<Action>,
    ) -> Option<crate::translation::Task> {
        self.state.sequence += 1;
        if self.state.translating {
            return None;
        }
        if !self.state.can_send() {
            self.state.status = "请先确认拼音，并将原文控制在 144 字符以内".into();
            return None;
        }
        if self.osc.is_none() {
            self.state.status = "试打模式，未连接 OSC".into();
            return None;
        }
        match crate::translation::Task::start(config, self.state.draft.clone(), notify) {
            Ok(task) => {
                if let Some(osc) = &mut self.osc {
                    let _ = osc.typing(false);
                }
                self.state.clear_draft();
                self.session.command(Command::Cancel);
                self.reset_context();
                self.refresh();
                self.state.translating = true;
                self.state.status.clear();
                Some(task)
            }
            Err(e) => {
                self.state.status = e;
                None
            }
        }
    }
    fn finish_translation(&mut self, original: String, result: Result<String, String>) {
        self.state.translating = false;
        let sent = result.and_then(|message| {
            self.osc
                .as_ref()
                .ok_or_else(|| "OSC 未连接".to_string())?
                .send(&message)
                .map_err(|e| e.to_string())
        });
        match sent {
            Ok(()) => self.state.status = "译文已发出".into(),
            Err(e) => {
                self.state.status = format!("未发送 · {e}");
                if self.state.draft.is_empty() && !self.composing() {
                    self.state.insert(&original);
                    self.reset_context();
                    self.refresh();
                } else {
                    self.unsent.push_back(original);
                    self.state.recoverable = true;
                }
            }
        }
    }
    pub fn apply(&mut self, action: Action) {
        if matches!(action, Action::SuggestionsReady) {
            return;
        }
        let start = Instant::now();
        self.state.status.clear();
        let mut refresh_candidates = true;
        let edited = matches!(
            action,
            Action::Key(_)
                | Action::Literal(_)
                | Action::Select(..)
                | Action::Space
                | Action::Backspace
        );
        if self.state.draft.encode_utf16().count() > 4096
            && matches!(
                action,
                Action::Key(_) | Action::Literal(_) | Action::Space | Action::Select(..)
            )
        {
            self.state.status = "草稿过长，请清理后继续".into();
            self.state.sequence += 1;
            return;
        }
        match action {
            Action::SendTranslated(_) => unreachable!("handled by the worker"),
            Action::RestoreUnsent => {
                if self.state.draft.is_empty()
                    && !self.composing()
                    && let Some(text) = self.unsent.pop_front()
                {
                    self.state.insert(&text);
                    self.reset_context();
                    self.state.recoverable = !self.unsent.is_empty();
                }
            }
            // The number/symbol layer inserts the displayed character. Desktop
            // digit shortcuts still use Key, so they can select candidates.
            Action::Literal(c) => {
                if self.composing() {
                    let r = self.session.command(Command::CommitCandidate);
                    self.consume(r);
                }
                if self.composing() {
                    self.state.status = "请先确认拼音，再输入符号".into();
                } else {
                    self.type_literal(c);
                }
            }
            Action::Key(c) => {
                if self.state.english {
                    self.type_literal(if self.state.caps {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    });
                } else if c.is_ascii_alphabetic() || c == '\'' {
                    self.session
                        .set_rescoring_context(self.state.before_caret());
                    let r = self.session.character(c.to_ascii_lowercase() as u8, false);
                    self.consume(r);
                } else if c.is_ascii_digit() && self.composing() {
                    let r = self.session.candidate_key(c as u8);
                    self.consume(r);
                } else if c.is_ascii_punctuation() {
                    let r = self.session.punctuation(c as u8);
                    let handled = r.handled;
                    self.consume(r);
                    if !handled {
                        self.type_literal(c);
                    }
                } else if !self.composing() {
                    self.type_literal(c);
                } else {
                    self.state.status = "请先确认拼音，再输入符号".into();
                }
            }
            Action::Select(index, revision) if self.state.english => {
                if revision == self.state.candidate_revision
                    && let Some(range) = english::word_at_caret(&self.state)
                    && let Some(word) = self.state.candidates.get(index)
                {
                    let end = range.start + word.len();
                    self.state.draft.replace_range(range, word);
                    self.state.caret = self.state.boundary_at_or_after(end);
                    self.reset_context();
                } else {
                    self.state.status = "候选已更新，请重新选择".into();
                    // A late click must also leave the newer prefix's list (or pending lookup) intact.
                    refresh_candidates = false;
                }
            }
            Action::Select(index, revision) => {
                let mapping = if revision == self.state.candidate_revision {
                    Some(&self.candidate_map)
                } else {
                    self.previous_map
                        .as_ref()
                        .filter(|(r, _)| *r == revision)
                        .map(|(_, m)| m)
                };
                let actual = mapping.and_then(|m| m.get(index)).copied();
                if let Some(index) =
                    actual.filter(|_| self.engine_revision == self.session.candidate_revision())
                {
                    let r = self.session.select(index);
                    self.consume(r);
                } else {
                    self.state.status = "候选已更新，请重新选择".into();
                }
            }
            Action::Space => {
                if self.composing() {
                    let r = self.session.command(Command::CommitCandidate);
                    self.consume(r);
                } else {
                    self.type_literal(' ');
                }
            }
            Action::Backspace => {
                if self.composing() {
                    let r = self.session.command(Command::Backspace);
                    self.consume(r);
                } else {
                    self.session.command(Command::Cancel);
                    self.state.backspace();
                    self.reset_context();
                }
            }
            Action::MoveCaret { offset, sequence } => {
                // A desktop press can be released after another hand has edited the line.
                if sequence == self.state.sequence && offset != self.state.caret {
                    let mut target = offset.min(self.state.draft.len());
                    if self.composing() {
                        let old_len = self.state.draft.len();
                        let old_caret = self.state.caret;
                        let r = self.session.command(Command::CommitCandidate);
                        self.consume(r);
                        if target > old_caret {
                            target += self.state.draft.len() - old_len;
                        }
                    }
                    if self.composing() {
                        self.state.status = "请先选完当前拼音".into();
                    } else {
                        self.state.caret = self.state.boundary_at_or_after(target);
                        self.session.command(Command::Cancel);
                        self.reset_context();
                    }
                }
            }
            Action::Language => {
                if self.composing() {
                    self.state.status = "请先选词，或按 Esc 取消拼音".into();
                } else {
                    self.session.command(Command::Cancel);
                    self.state.english = !self.state.english;
                    self.reset_context();
                }
            }
            Action::Caps => {
                if self.state.english {
                    self.state.caps = !self.state.caps;
                }
            }
            Action::Clear => {
                self.session.command(Command::Cancel);
                self.state.clear_draft();
                self.reset_context();
            }
            Action::Cancel => {
                self.session.command(Command::Cancel);
            }
            Action::Send => {
                self.refresh();
                if self.state.can_send() {
                    if let Some(osc) = &self.osc {
                        match osc.send(&self.state.draft) {
                            Ok(()) => {
                                self.state.clear_draft();
                                self.session.command(Command::Cancel);
                                self.reset_context();
                                self.state.status = "已发出".into();
                            }
                            Err(e) => self.state.status = format!("发送失败：{e}"),
                        }
                    } else {
                        self.state.status = "试打模式，未连接 OSC".into();
                    }
                } else {
                    self.state.status = "请确认拼音，并将消息控制在 144 字符、9 行以内".into();
                }
                if let Some(osc) = &mut self.osc {
                    let _ = osc.typing(false);
                }
            }
            Action::SetPrediction(enabled) => self.session.set_prediction_allowed(enabled),
            Action::SetTypingIndicator(enabled) => {
                refresh_candidates = !self.state.english;
                if let Some(osc) = &mut self.osc {
                    if let Err(error) = osc.set_typing_enabled(enabled) {
                        self.state.status = format!("输入状态更新失败：{error}");
                    }
                }
            }
            Action::PauseTyping | Action::Stop => {
                refresh_candidates = !self.state.english;
                if let Some(osc) = &mut self.osc {
                    let _ = osc.typing(false);
                }
            }
            Action::SuggestionsReady => unreachable!(),
        }
        if edited {
            self.last_input = Instant::now();
            if let Some(osc) = &mut self.osc {
                let _ = osc.typing(true);
            }
        }
        if refresh_candidates {
            self.refresh();
        }
        self.state.sequence += 1;
        self.state.latency_ms = start.elapsed().as_secs_f64() * 1000.;
    }
    pub fn idle(&mut self) {
        if self.last_input.elapsed() > Duration::from_secs(4) {
            if let Some(osc) = &mut self.osc {
                let _ = osc.typing(false);
            }
        }
    }
}

pub struct Worker {
    pub input: mpsc::SyncSender<Action>,
    pub output: mpsc::Receiver<Snapshot>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(
        resources: std::path::PathBuf,
        work: std::path::PathBuf,
        osc: Option<Osc>,
    ) -> Self {
        Self::start_for_check(resources, work, osc, true, true)
    }
    pub fn start_for_check(
        resources: std::path::PathBuf,
        work: std::path::PathBuf,
        osc: Option<Osc>,
        neural_enabled: bool,
        learning: bool,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel(128);
        let (state_tx, state_rx) = mpsc::channel();
        let notify = tx.clone();
        let thread = thread::Builder::new()
            .name("msime".into())
            .spawn(move || {
                match Editor::new(&resources, &work, osc, true, learning) {
                    Ok(mut editor) => {
                        let english_lookup = std::cell::OnceCell::<Option<english::Lookup>>::new();
                        let neural = if neural_enabled {
                            crate::neural::Predictor::start(
                                resources
                                    .parent()
                                    .unwrap_or(&resources)
                                    .join("models/sentence-model.safetensors"),
                                notify.clone(),
                                work.join("neural-performance.json"),
                            )
                            .ok()
                        } else {
                            None
                        };
                        editor.state.neural_status = neural
                            .as_ref()
                            .map_or("基础联想已开启".into(), |p| p.status());
                        let _ = state_tx.send(editor.state.clone());
                        crate::native::Native::wake();
                        let mut translation: Option<crate::translation::Task> = None;
                        loop {
                            match rx.recv_timeout(Duration::from_millis(500)) {
                                Ok(Action::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                                    break;
                                }
                                Ok(Action::SuggestionsReady) => {}
                                Ok(action) => {
                                    if let Some(p) = &neural {
                                        p.cancel();
                                    }
                                    let paused = matches!(action, Action::PauseTyping);
                                    let complete_word = matches!(
                                        action,
                                        Action::Key(_)
                                            | Action::Literal(_)
                                            | Action::Backspace
                                            | Action::MoveCaret { .. }
                                            | Action::Language
                                            | Action::Caps
                                    );
                                    if let Action::SendTranslated(config) = action {
                                        if let Some(task) =
                                            editor.begin_translation(config, notify.clone())
                                        {
                                            translation = Some(task);
                                        }
                                    } else {
                                        editor.apply(action);
                                    }
                                    if state_tx.send(editor.state.clone()).is_err() {
                                        break;
                                    }
                                    crate::native::Native::wake();
                                    if complete_word
                                        && let Some(word) = english::word_at_caret(&editor.state)
                                        && let Some(lookup) = english_lookup.get_or_init(|| {
                                            english::Lookup::start(
                                                resources.clone(),
                                                notify.clone(),
                                            )
                                            .ok()
                                        })
                                    {
                                        lookup.request(
                                            editor.state.candidate_revision,
                                            &editor.state.draft[word.start..editor.state.caret],
                                            editor.state.caps,
                                        );
                                    }
                                    if !paused {
                                        if let Some(p) = &neural {
                                            p.request(&editor.state);
                                        }
                                    }
                                }
                                Err(mpsc::RecvTimeoutError::Timeout) => editor.idle(),
                            }
                            let mut changed = false;
                            if let Some(task) = &translation {
                                let result = match task.result.try_recv() {
                                    Ok(r) => Some(r),
                                    Err(mpsc::TryRecvError::Disconnected) => {
                                        Some(Err("翻译任务已停止".into()))
                                    }
                                    Err(mpsc::TryRecvError::Empty) => None,
                                };
                                if let Some(result) = result {
                                    editor.finish_translation(
                                        translation.take().unwrap().original,
                                        result,
                                    );
                                    changed = true;
                                }
                            }
                            if let Some(Some(lookup)) = english_lookup.get() {
                                while let Ok((revision, rows)) = lookup.results.try_recv() {
                                    changed |= editor.accept_english(revision, rows);
                                }
                            }
                            if let Some(p) = &neural {
                                while let Some(result) = p.take_result() {
                                    changed |= editor.apply_prediction(result);
                                }
                                let status = p.status();
                                if editor.state.neural_status != status {
                                    editor.state.neural_status = status;
                                    changed = true;
                                }
                            }
                            if changed {
                                if state_tx.send(editor.state.clone()).is_err() {
                                    break;
                                }
                                crate::native::Native::wake();
                            }
                        }
                    }
                    Err(error) => {
                        let _ = state_tx.send(Snapshot {
                            status: format!("词库初始化失败：{error}"),
                            ..Default::default()
                        });
                        crate::native::Native::wake();
                    }
                }
                msime_engine::shutdown();
            })
            .expect("engine worker");
        Self {
            input: tx,
            output: state_rx,
            thread: Some(thread),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.input.send(Action::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires dictionaries, public Edge translation and loopback UDP"]
    fn translation_allows_typing_until_one_multiline_osc_packet() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let receiver = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(12)))
            .unwrap();
        let worker = Worker::start_for_check(
            root.join("data"),
            root.join("runtime/translation-osc-test"),
            Some(Osc::new(receiver.local_addr().unwrap()).unwrap()),
            false,
            false,
        );
        let wait = |f: &dyn Fn(&Snapshot) -> bool| {
            let end = Instant::now() + Duration::from_secs(15);
            loop {
                let s = worker
                    .output
                    .recv_timeout(end.saturating_duration_since(Instant::now()))
                    .unwrap();
                if f(&s) {
                    break s;
                }
            }
        };
        wait(&|s| s.ready);
        for c in "你好！".chars() {
            worker.input.send(Action::Literal(c)).unwrap();
        }
        worker
            .input
            .send(Action::SendTranslated(crate::translation::Config {
                enabled: true,
                second: Some("ja".into()),
                ..Default::default()
            }))
            .unwrap();
        let start = Instant::now();
        worker.input.send(Action::Literal('新')).unwrap();
        wait(&|s| s.translating && s.draft == "新");
        println!(
            "Input acknowledged while translation was pending: {:?}",
            start.elapsed()
        );
        let mut bytes = [0u8; 1024];
        let n = receiver.recv(&mut bytes).unwrap();
        assert_eq!(&bytes[..24], b"/chatbox/input\0\0,sTF\0\0\0\0");
        let body = std::str::from_utf8(&bytes[24..n])
            .unwrap()
            .trim_end_matches('\0');
        let lines: Vec<_> = body.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "你好！");
        assert!(lines[1].contains(|c: char| c.is_ascii_alphabetic()));
        assert!(lines[2].contains(|c: char| ('ぁ'..='ヿ').contains(&c)));
        wait(&|s| !s.translating && s.draft == "新");
        receiver
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        assert!(receiver.recv(&mut bytes).is_err());
    }
    #[test]
    #[ignore = "requires dictionaries"]
    fn failed_translation_preserves_new_edits_and_unfinished_pinyin() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut e = Editor::new(
            &root.join("data"),
            &root.join("runtime/translation-recovery"),
            None,
            false,
            false,
        )
        .unwrap();
        e.apply(Action::Key('n'));
        let preedit = e.state.preedit.clone();
        assert!(!preedit.is_empty());
        e.finish_translation("旧草稿".into(), Err("连接超时".into()));
        assert!(e.state.recoverable);
        assert_eq!(e.state.preedit, preedit);
        e.apply(Action::Cancel);
        e.apply(Action::RestoreUnsent);
        assert_eq!(e.state.draft, "旧草稿");
        assert!(!e.state.recoverable);
        e.apply(Action::Clear);
        e.finish_translation("再试一次".into(), Err("超出字符限制".into()));
        assert_eq!(e.state.draft, "再试一次");
    }
    #[test]
    #[ignore = "requires the downloaded real dictionaries and loopback UDP"]
    fn real_engine_to_osc_only_sends_confirmed_text() {
        use std::net::UdpSocket;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let osc = Osc::new(receiver.local_addr().unwrap()).unwrap();
        let mut editor = Editor::new(
            &root.join("data"),
            &root.join("runtime/osc-test"),
            Some(osc),
            true,
            false,
        )
        .unwrap();
        for c in "nihao".chars() {
            editor.apply(Action::Key(c));
        }
        editor.apply(Action::Send);
        let mut packet = [0u8; 1024];
        // With the default settings, composing must emit no OSC at all.
        assert!(receiver.recv(&mut packet).is_err());
        assert!(!editor.state.preedit.is_empty());
        let index = editor
            .state
            .candidates
            .iter()
            .position(|c| c == "你好")
            .unwrap();
        editor.apply(Action::Select(index, editor.state.candidate_revision));
        assert_eq!(editor.state.draft, "你好");
        editor.apply(Action::Literal('！'));
        editor.apply(Action::MoveCaret {
            offset: "你好".len(),
            sequence: editor.state.sequence,
        });
        for c in "shijie".chars() {
            editor.apply(Action::Key(c));
        }
        let index = editor
            .state
            .candidates
            .iter()
            .position(|s| s == "世界")
            .unwrap();
        editor.apply(Action::Select(index, editor.state.candidate_revision));
        assert_eq!(editor.state.before_caret(), "你好世界");
        editor.apply(Action::Send);
        let mut messages = Vec::new();
        while let Ok(len) = receiver.recv(&mut packet) {
            if packet[..len].starts_with(b"/chatbox/input") {
                messages.push(packet[..len].to_vec());
            }
        }
        assert_eq!(
            messages,
            vec![crate::osc::chat_packet("你好世界！").unwrap()]
        );
        assert!(editor.state.draft.is_empty());
        assert_eq!(editor.state.caret, 0);
        editor.apply(Action::Send);
        assert!(receiver.recv(&mut packet).is_err());
        for c in "你呀".chars() {
            editor.apply(Action::Literal(c));
        }
        editor.apply(Action::MoveCaret {
            offset: "你".len(),
            sequence: editor.state.sequence,
        });
        for c in "hao".chars() {
            editor.apply(Action::Key(c));
        }
        let first = editor.state.candidates[0].clone();
        let end = editor.state.draft.len();
        let seq = editor.state.sequence;
        editor.apply(Action::MoveCaret {
            offset: end,
            sequence: seq,
        });
        assert_eq!(editor.state.before_caret(), format!("你{first}呀"));
        editor.apply(Action::MoveCaret {
            offset: 0,
            sequence: seq,
        });
        assert_eq!(editor.state.caret, editor.state.draft.len());
        editor.apply(Action::Clear);
        crate::neural_check::check_mapping(&mut editor).unwrap();
        assert!(receiver.recv(&mut packet).is_err());
        editor.apply(Action::Clear);
        editor.apply(Action::SetTypingIndicator(true));
        assert!(receiver.recv(&mut packet).is_err());
        editor.apply(Action::Key('n'));
        let len = receiver.recv(&mut packet).unwrap();
        assert_eq!(&packet[..len], b"/chatbox/typing\0,T\0\0");
        editor.apply(Action::SetTypingIndicator(false));
        let len = receiver.recv(&mut packet).unwrap();
        assert_eq!(&packet[..len], b"/chatbox/typing\0,F\0\0");
        editor.apply(Action::Key('i'));
        assert!(receiver.recv(&mut packet).is_err());
        editor.apply(Action::SetTypingIndicator(true));
        editor.apply(Action::Key('h'));
        receiver.recv(&mut packet).unwrap();
        editor.apply(Action::PauseTyping);
        let len = receiver.recv(&mut packet).unwrap();
        assert_eq!(&packet[..len], b"/chatbox/typing\0,F\0\0");
        drop(editor);
        msime_engine::shutdown();
    }
    #[test]
    fn deletion_preserves_graphemes() {
        let mut s = Snapshot::default();
        s.insert("中👨‍👩‍👧‍👦e\u{301}文");
        s.caret -= "文".len();
        s.backspace();
        assert_eq!(s.draft, "中👨‍👩‍👧‍👦文");
        s.backspace();
        assert_eq!(s.draft, "中文");
        s.insert("间");
        assert_eq!(s.draft, "中间文");
    }
    #[test]
    fn pending_composition_never_sends() {
        let s = Snapshot {
            ready: true,
            draft: "你好".into(),
            preedit: "wo".into(),
            ..Default::default()
        };
        assert!(!s.can_send());
    }
}
