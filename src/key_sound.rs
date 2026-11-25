// SPDX-License-Identifier: MPL-2.0
use std::{
    sync::mpsc::{self, RecvTimeoutError, SyncSender},
    thread,
    time::{Duration, Instant},
};

const WOOD_TAP: &[u8] = include_bytes!("../assets/key-sounds/E-wood.pcm");

unsafe extern "C" {
    fn key_audio_play(pcm: *const u8, bytes: u32) -> bool;
    fn key_audio_close();
}

pub struct KeyAudio(Option<SyncSender<Instant>>);
impl KeyAudio {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::sync_channel::<Instant>(4);
        let thread = thread::Builder::new()
            .name("key-audio".into())
            .spawn(move || {
                while let Ok(mut tap) = rx.recv() {
                    let mut retry_after = Instant::now();
                    loop {
                        let now = Instant::now();
                        if now >= retry_after && tap.elapsed() < Duration::from_millis(100) {
                            if !unsafe { key_audio_play(WOOD_TAP.as_ptr(), WOOD_TAP.len() as u32) }
                            {
                                retry_after = now + Duration::from_secs(1);
                            }
                        }
                        match rx.recv_timeout(Duration::from_secs(5)) {
                            Ok(next) => tap = next,
                            Err(RecvTimeoutError::Disconnected | RecvTimeoutError::Timeout) => {
                                break;
                            }
                        }
                    }
                    unsafe {
                        key_audio_close();
                    }
                }
            });
        Self(thread.ok().map(|_| tx))
    }
    pub fn play(&self) {
        if let Some(tx) = &self.0 {
            let _ = tx.try_send(Instant::now());
        }
    }
}
