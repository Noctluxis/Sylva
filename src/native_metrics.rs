// SPDX-License-Identifier: MPL-2.0
use crate::native::Native;
use std::ffi::{CStr, c_char};
unsafe extern "C" {
    fn native_graphics_metrics(values: *mut f64);
    fn native_metrics_reset();
    fn native_metrics_stop();
    fn native_text_cache(enabled: i32);
    fn native_cache_metrics(values: *mut f64);
    fn native_wait_metrics(values: *mut f64);
    fn native_process_metrics(values: *mut f64);
    fn native_scene_metrics(values: *mut f64) -> u32;
    fn native_adapter() -> *const c_char;
    fn native_runtime_version() -> *const c_char;
    fn native_cpu_panel() -> i32;
    fn native_prepare_metrics(values: *mut f64);
    fn native_raw_metrics(values: *mut f64);
}
fn timing(v: &[f64]) -> serde_json::Value {
    serde_json::json!({"samples":v[0] as u64,"p50":v[1],"p95":v[2],"max":v[3]})
}
impl Native {
    pub fn reset_metrics(&self) {
        unsafe { native_metrics_reset() }
    }
    pub fn stop_metrics(&self) {
        unsafe { native_metrics_stop() }
    }
    pub fn text_cache(&self, enabled: bool) {
        unsafe { native_text_cache(enabled as i32) }
    }
    pub fn process_metrics(&self) -> [f64; 3] {
        Self::process_resources()
    }
    pub fn process_resources() -> [f64; 3] {
        let mut v = [0.; 3];
        unsafe { native_process_metrics(v.as_mut_ptr()) };
        v
    }
    pub fn graphics_metrics(&self) -> serde_json::Value {
        let mut values = [0.; 30];
        let mut cache = [0.; 3];
        unsafe {
            native_graphics_metrics(values.as_mut_ptr());
            native_cache_metrics(cache.as_mut_ptr());
        }
        let mut report = serde_json::Map::new();
        for (i, name) in [
            "cpu_record_ms",
            "cpu_end_draw_ms",
            "cpu_flush_ms",
            "steamvr_texture_submit_ms",
            "gpu_draw_ms",
            "gpu_completion_observed_delay_ms",
            "desktop_mirror_ms",
        ]
        .iter()
        .enumerate()
        {
            report.insert(name.to_string(), timing(&values[i * 4..i * 4 + 4]));
        }
        report.insert("gpu_query_skipped".into(), serde_json::json!(values[28]));
        report.insert("gpu_query_disjoint".into(), serde_json::json!(values[29]));
        report.insert(
            "text_layout_cache".into(),
            serde_json::json!({"hits":cache[0],"misses":cache[1],"entries":cache[2],"limit":256}),
        );
        report.insert(
            "adapter".into(),
            serde_json::json!(unsafe { CStr::from_ptr(native_adapter()) }.to_string_lossy()),
        );
        report.insert(
            "texture_pixels".into(),
            serde_json::json!([
                (crate::ui::WIDTH * 1.5) as u32,
                (crate::ui::HEIGHT * 1.5) as u32
            ]),
        );
        let software = unsafe { native_cpu_panel() } != 0;
        let mut prepared = [0.; 4];
        unsafe { native_prepare_metrics(prepared.as_mut_ptr()) };
        report.insert(
            "render_path".into(),
            serde_json::json!(if software {
                "cpu_wic_to_steamvr_raw"
            } else {
                "d3d11_texture"
            }),
        );
        report.insert("cpu_pixel_prepare_ms".into(), timing(&prepared));
        let mut raw = [0.; 15];
        unsafe { native_raw_metrics(raw.as_mut_ptr()) };
        report.insert(
            "raw_upload".into(),
            serde_json::json!({
                "image_loaded_observed_ms":timing(&raw[..4]),
                "submitted":raw[4], "loaded":raw[5],
                "superseded_before_submit":raw[6], "in_flight":raw[7],
            "load_failed":raw[8], "max_in_flight":1,
            "preloaded_feedback_images":raw[9],
            "buffer_count":2,"completed_handoffs":raw[10],
            "front_texture_checks_during_upload":raw[11],"missing_front_texture":raw[12],
            "hidden_diagnostic":raw[13]!=0.,
            "in_flight_age_ms":raw[14],
            "hover_press_path":"Preloaded overlays, transform/color only on the controller thread",
            "scope":"ImageLoaded event observed by UI loop, not HMD scanout."
            }),
        );
        report.insert(
            "app_gpu_queries_applicable".into(),
            serde_json::json!(!software),
        );
        serde_json::Value::Object(report)
    }
    pub fn frame_pacing_metrics(&self) -> serde_json::Value {
        let mut v = [0.; 7];
        unsafe { native_wait_metrics(v.as_mut_ptr()) };
        serde_json::json!({"wait_frame_sync_ms":timing(&v[..4]),"timeouts":v[4],"errors":v[5],"display_hz":v[6],"runtime_version":unsafe{CStr::from_ptr(native_runtime_version())}.to_string_lossy()})
    }
    pub fn scene_metrics(&self) -> serde_json::Value {
        let mut v = [0.; 21];
        let frames = unsafe { native_scene_metrics(v.as_mut_ptr()) };
        serde_json::json!({"available":frames>0,"recent_frames":frames,"scene_pre_submit_gpu_ms":timing(&v[..4]),"compositor_gpu_ms":timing(&v[4..8]),"scene_frame_interval_ms":timing(&v[8..12]),"scene_submit_ms":timing(&v[12..16]),"cumulative_pid":v[16],"cumulative_presents":v[17],"cumulative_dropped":v[18],"cumulative_reprojected":v[19],"scene_focus_pid":v[20],"scope":"Recent SteamVR scene frames and cumulative counters; NOT keyboard-only timings; unavailable on some overlay runtimes."})
    }
}
