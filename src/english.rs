// SPDX-License-Identifier: MPL-2.0
use crate::{
    engine::{Action, Snapshot},
    native::Native,
};
use std::{
    collections::VecDeque,
    ops::Range,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};
use unicode_segmentation::UnicodeSegmentation;

const ROWS: usize = 8;

/// The prefix ends at the caret; selecting a row replaces the whole word there.
pub fn word_at_caret(state: &Snapshot) -> Option<Range<usize>> {
    if !state.english
        || !state
            .before_caret()
            .as_bytes()
            .last()?
            .is_ascii_alphabetic()
    {
        return None;
    }
    let (start, word) = state
        .draft
        .split_word_bound_indices()
        .find(|(start, word)| start + word.len() >= state.caret)?;
    word.bytes()
        .all(|b| b.is_ascii_alphabetic())
        .then_some(start..start + word.len())
}

struct Query {
    revision: u64,
    prefix: String,
    caps: bool,
}

fn display_words(words: &[String], prefix: &str, caps: bool) -> Vec<String> {
    let uppercase = (prefix.len() > 1 || caps) && prefix.bytes().all(|b| b.is_ascii_uppercase());
    let mut rows = Vec::with_capacity(ROWS);
    for word in words {
        let Some(head) = word.get(..prefix.len()) else {
            continue;
        };
        if !head.eq_ignore_ascii_case(prefix) || word.len() <= prefix.len() {
            continue;
        }
        let display = if uppercase {
            word.to_ascii_uppercase()
        } else if prefix.bytes().any(|b| b.is_ascii_uppercase()) {
            format!("{prefix}{}", &word[prefix.len()..])
        } else {
            word.clone()
        };
        if !rows.contains(&display) {
            rows.push(display);
            if rows.len() == ROWS {
                break;
            }
        }
    }
    rows
}

pub struct Lookup {
    pending: Arc<Mutex<Option<Query>>>,
    wake: Option<mpsc::SyncSender<()>>,
    pub results: mpsc::Receiver<(u64, Vec<String>)>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Lookup {
    pub fn start(resources: PathBuf, notify: mpsc::SyncSender<Action>) -> std::io::Result<Self> {
        let pending = Arc::new(Mutex::new(None::<Query>));
        let slot = Arc::clone(&pending);
        let (wake, requests) = mpsc::sync_channel(1);
        let (output, results) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("english-completion".into())
            .spawn(move || {
                Native::lower_thread_priority();
                let mut cache: VecDeque<(String, Vec<String>)> = VecDeque::new();
                while requests.recv().is_ok() {
                    let query = slot.lock().unwrap().take();
                    let Some(query) = query else {
                        continue;
                    };
                    let key = query.prefix.to_ascii_lowercase();
                    let cached = cache.iter().position(|(prefix, _)| prefix == &key);
                    let entry = if let Some(i) = cached {
                        cache.remove(i).unwrap()
                    } else {
                        let Ok(words) = msime_engine::host::english_completions(
                            &resources.to_string_lossy(),
                            &key,
                            ROWS + 1,
                        ) else {
                            continue;
                        };
                        if cache.len() == 32 {
                            cache.pop_front();
                        }
                        (key, words)
                    };
                    let rows = display_words(&entry.1, &query.prefix, query.caps);
                    cache.push_back(entry);
                    if output.send((query.revision, rows)).is_err() {
                        break;
                    }
                    // A full editor queue already has a wakeup; results are drained after every event.
                    let _ = notify.try_send(Action::SuggestionsReady);
                }
            })?;
        Ok(Self {
            pending,
            wake: Some(wake),
            results,
            thread: Some(worker),
        })
    }
    pub fn request(&self, revision: u64, prefix: &str, caps: bool) {
        *self.pending.lock().unwrap() = Some(Query {
            revision,
            prefix: prefix.to_owned(),
            caps,
        });
        let _ = self.wake.as_ref().unwrap().try_send(());
    }
}
impl Drop for Lookup {
    fn drop(&mut self) {
        self.pending.lock().unwrap().take();
        self.wake.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_keeps_typed_case_and_omits_the_literal_prefix() {
        let words = ["hel", "hello", "Helen"].map(str::to_owned);
        assert_eq!(display_words(&words, "hel", false), ["hello", "Helen"]);
        assert_eq!(display_words(&words, "Hel", false), ["Hello", "Helen"]);
        assert_eq!(display_words(&words, "HEL", true), ["HELLO", "HELEN"]);
        assert_eq!(display_words(&words, "hEl", false), ["hEllo", "hElen"]);
        let words = ["have".to_owned()];
        assert_eq!(display_words(&words, "H", false), ["Have"]);
        assert_eq!(display_words(&words, "H", true), ["HAVE"]);
    }
    #[test]
    fn lookup_is_limited_to_the_english_word_at_the_caret() {
        let mut state = Snapshot {
            draft: "我hel朋友".into(),
            caret: "我he".len(),
            english: true,
            ..Default::default()
        };
        assert_eq!(&state.draft[word_at_caret(&state).unwrap()], "hel");
        state.english = false;
        assert!(word_at_caret(&state).is_none());
        for text in ["", "hello ", "don't", "a_b", "VR2D", "hello!", "中文"] {
            let state = Snapshot {
                draft: text.into(),
                caret: text.len(),
                english: true,
                ..Default::default()
            };
            assert!(word_at_caret(&state).is_none(), "{text}");
        }
    }
}
