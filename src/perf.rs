// SPDX-License-Identifier: MPL-2.0
use crate::native::Native;
use std::{
    path::Path,
    time::{Duration, Instant},
};

// Bounded, local timing samples only. No text, key identities or controller poses.
pub struct Perf {
    start: Instant,
    written: bool,
    render: Vec<f64>,
    mirror: Vec<f64>,
}
impl Perf {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            written: false,
            render: Vec::new(),
            mirror: Vec::new(),
        }
    }
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
    pub fn render(&mut self, ms: f64) {
        if !self.written && self.render.len() < 4096 {
            self.render.push(ms);
        }
    }
    pub fn mirror(&mut self, ms: f64) {
        if !self.written && ms > 0.01 && self.mirror.len() < 4096 {
            self.mirror.push(ms);
        }
    }
    pub fn save(&mut self, native: &Native, root: &Path, exiting: bool) {
        if self.written || !native.vr() || (!exiting && self.elapsed() < Duration::from_secs(30)) {
            return;
        }
        fn stats(values: &mut [f64]) -> serde_json::Value {
            values.sort_by(f64::total_cmp);
            if values.is_empty() {
                return serde_json::json!({"samples":0});
            }
            serde_json::json!({"samples":values.len(),"p50":values[values.len()/2],"p95":values[values.len()*95/100],"max":values[values.len()-1]})
        }
        let resources = native.process_metrics();
        let report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"seconds":self.elapsed().as_secs_f64(),"vr_input":native.vr_metrics(),"frame_pacing":native.frame_pacing_metrics(),"graphics":native.graphics_metrics(),"steamvr_scene":native.scene_metrics(),"ui_draw_submit_ms":stats(&mut self.render),"desktop_mirror_ms":stats(&mut self.mirror),"process":{"cpu_seconds_since_start":resources[0],"working_set_mib":resources[1],"private_commit_mib":resources[2]},"scope":"First bounded local samples, includes startup. GPU timestamps are asynchronous, not motion-to-photon latency; completion-observed delay includes polling interval. Scene timings are separate from keyboard timings. No chat data."});
        let folder = root.join("runtime");
        if std::fs::create_dir_all(&folder).is_ok() {
            if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
                self.written = std::fs::write(folder.join("vr-performance.json"), bytes).is_ok();
                if self.written {
                    native.stop_metrics();
                }
            }
        }
    }
}
