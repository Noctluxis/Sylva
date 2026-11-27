// SPDX-License-Identifier: MPL-2.0
use std::ffi::{CStr, CString, c_char};
#[repr(C)]
#[derive(Default, Copy, Clone)]
pub struct Event {
    pub kind: i32,
    pub hand: i32,
    pub x: f32,
    pub y: f32,
    pub value: u32,
}
unsafe extern "C" {
    fn native_lifecycle_check() -> i32;
    fn native_forward_check(steam: i32) -> i32;
    fn native_init(manifest: *const c_char, desktop: i32, flags: u32) -> i32;
    fn native_lifecycle_settings(close: i32);
    fn native_desktop_visible() -> i32;
    fn native_attach_vr() -> i32;
    fn native_autostart(
        manifest: *const c_char,
        key: *const c_char,
        enabled: i32,
        temporary: i32,
    ) -> i32;
    fn native_error() -> *const c_char;
    fn native_vr_ready() -> i32;
    fn native_incremental(enabled: i32);
    fn native_key_regions(regions: *const [f32; 4], count: u32);
    fn native_visible() -> i32;
    fn native_vr_settings(width: f32, block: i32, haptics: i32, center: i32);
    fn native_pump();
    fn native_event(event: *mut Event) -> i32;
    fn native_wait(ms: u32);
    fn native_wake();
    fn native_feedback(hand: u32);
    fn native_feedback_preview(rect: *const f32, pressed: i32) -> i32;
    fn native_vr_metrics(values: *mut f64) -> u32;
    fn native_begin(bg: u32);
    fn native_rect(x: f32, y: f32, w: f32, h: f32, r: f32, fill: u32, border: u32);
    fn native_text(
        text: *const u16,
        len: u32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        size: f32,
        fill: u32,
        align: i32,
    );
    fn native_text_width(text: *const u16, len: u32, size: f32) -> f32;
    fn native_draft(
        text: *const u16,
        len: u32,
        start: u32,
        length: u32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        size: f32,
        fill: u32,
    );
    fn native_draft_hit(x: f32, y: f32) -> i32;
    fn native_icon(id: u32, x: f32, y: f32, size: f32, fill: u32);
    fn native_copy_text(text: *const u16, len: u32) -> u32;
    fn native_paste_text(text: *mut u16, capacity: u32, len: *mut u32) -> u32;
    fn native_neural_background() -> i32;
    fn native_present() -> i32;
    fn native_mirror() -> i32;
    fn native_export(path: *const u16) -> i32;
    fn native_export_raw(path: *const u16) -> i32;
    fn native_close();
    fn native_message(text: *const u16);
}
pub struct Native;
#[repr(u32)]
#[derive(Clone, Copy)]
pub enum Icon {
    Clear,
    Copy,
    ChevronDown,
    ChevronUp,
    Backspace,
}
impl Native {
    pub fn clipboard_text() -> Result<String, String> {
        let mut data = [0u16; 2561];
        let mut n = 0;
        match unsafe { native_paste_text(data.as_mut_ptr(), data.len() as u32, &mut n) } {
            0 if n > 0 => {
                String::from_utf16(&data[..n as usize]).map_err(|_| "剪贴板文本无效".into())
            }
            0 | 232 => Err("剪贴板中没有文字".into()),
            122 => Err("剪贴板内容过长".into()),
            _ => Err("剪贴板暂不可用，请重试".into()),
        }
    }
    pub fn lifecycle_check() -> Result<i32, String> {
        let checks = unsafe { native_lifecycle_check() };
        if checks > 0 {
            Ok(checks)
        } else {
            Err(Self::error())
        }
    }
    pub fn forward_check(steam: bool) -> Result<(), String> {
        if unsafe { native_forward_check(steam as i32) } != 0 {
            Ok(())
        } else {
            Err("No running test instance".into())
        }
    }
    pub fn lower_thread_priority() -> bool {
        unsafe { native_neural_background() != 0 }
    }
    pub fn icon(&self, icon: Icon, x: f32, y: f32, size: f32, fill: u32) {
        unsafe { native_icon(icon as u32, x, y, size, fill) }
    }
    pub fn copy_text(&self, text: &str) -> Result<(), String> {
        if text.contains('\0') {
            return Err("内容含有无法复制的空字符".into());
        }
        let utf16: Vec<u16> = text
            .replace("\r\n", "\n")
            .replace('\n', "\r\n")
            .encode_utf16()
            .collect();
        let error = unsafe { native_copy_text(utf16.as_ptr(), utf16.len() as u32) };
        if error == 0 {
            Ok(())
        } else {
            Err(format!("剪贴板暂不可用，请重试（{error}）"))
        }
    }
    pub fn text_width(text: &str, size: f32) -> f32 {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let width = unsafe { native_text_width(utf16.as_ptr(), utf16.len() as u32, size) };
        if width >= 0. {
            width
        } else {
            text.chars()
                .map(|c| if c.is_ascii() { size * 0.56 } else { size })
                .sum()
        }
    }
    pub fn key_regions(&self, buttons: &[crate::ui::Button]) {
        let regions: Vec<[f32; 4]> = buttons
            .iter()
            .filter(|b| b.enabled)
            .map(|b| [b.x, b.y, b.w, b.h])
            .collect();
        unsafe { native_key_regions(regions.as_ptr(), regions.len() as u32) }
    }
    pub fn incremental(&self, enabled: bool) {
        unsafe { native_incremental(enabled as i32) }
    }
    pub fn offscreen_mode(software: bool) -> Result<Self, String> {
        let manifest = CString::new("").unwrap();
        if unsafe { native_init(manifest.as_ptr(), if software { 3 } else { 2 }, 0) } == 0 {
            let error = Self::error();
            unsafe { native_close() };
            Err(error)
        } else {
            Ok(Self)
        }
    }
    pub fn export(&self, path: &std::path::Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { native_export(path.as_ptr()) } == 0 {
            Err(Self::error())
        } else {
            Ok(())
        }
    }
    pub fn export_raw(&self, path: &std::path::Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { native_export_raw(path.as_ptr()) } == 0 {
            Err(Self::error())
        } else {
            Ok(())
        }
    }
    pub fn open(
        manifest: &std::path::Path,
        desktop: bool,
        settings: &crate::ui::Settings,
    ) -> Result<Option<Self>, String> {
        let path =
            CString::new(manifest.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let mode = if desktop {
            1
        } else if std::env::args().any(|a| a == "--profile-vr-hidden") {
            5
        } else if std::env::args().any(|a| a == "--gpu-panel") {
            4
        } else {
            0
        };
        let diagnostic = std::env::args().any(|a| a.starts_with("--profile-vr"));
        let quiet = settings.silent_start && !std::env::args().any(|a| a == "--show");
        let flags = if diagnostic {
            0
        } else {
            4 | 8
                | settings.close_to_tray as u32
                | ((quiet as u32) << 1)
                | ((std::env::args().any(|a| a == "--from-steamvr") as u32) << 4)
        };
        match unsafe { native_init(path.as_ptr(), mode, flags) } {
            0 => {
                let err = Self::error();
                unsafe { native_close() };
                Err(err)
            }
            2 => Ok(None),
            _ => Ok(Some(Self)),
        }
    }
    pub fn lifecycle_settings(&self, close: bool) {
        unsafe { native_lifecycle_settings(close as i32) }
    }
    pub fn desktop_visible(&self) -> bool {
        unsafe { native_desktop_visible() != 0 }
    }
    pub fn attach_vr(&self) -> bool {
        unsafe { native_attach_vr() != 0 }
    }
    // Called only in the short-lived utility process: never changes the renderer's OpenVR context.
    pub fn autostart(
        manifest: &std::path::Path,
        key: &str,
        enabled: bool,
        temporary: bool,
    ) -> Result<(), String> {
        let path =
            CString::new(manifest.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let key = CString::new(key).map_err(|e| e.to_string())?;
        if unsafe {
            native_autostart(
                path.as_ptr(),
                key.as_ptr(),
                enabled as i32,
                temporary as i32,
            )
        } != 0
        {
            Ok(())
        } else {
            Err(Self::error())
        }
    }
    pub fn error() -> String {
        unsafe { CStr::from_ptr(native_error()) }
            .to_string_lossy()
            .into_owned()
    }
    pub fn vr(&self) -> bool {
        unsafe { native_vr_ready() != 0 }
    }
    pub fn visible(&self) -> bool {
        unsafe { native_visible() != 0 }
    }
    pub fn settings(&self, width: f32, block: bool, haptics: bool, center: bool) {
        unsafe { native_vr_settings(width, block as i32, haptics as i32, center as i32) }
    }
    pub fn pump(&self) {
        unsafe { native_pump() }
    }
    pub fn event(&self) -> Option<Event> {
        let mut event = Event::default();
        if unsafe { native_event(&mut event) } != 0 {
            Some(event)
        } else {
            None
        }
    }
    pub fn wait(&self, ms: u32) {
        unsafe { native_wait(ms) }
    }
    pub fn wake() {
        unsafe { native_wake() }
    }
    pub fn feedback(&self, hand: usize) {
        unsafe { native_feedback(hand as u32) }
    }
    pub fn feedback_preview(&self, rect: [f32; 4], pressed: bool) -> Result<(), String> {
        if unsafe { native_feedback_preview(rect.as_ptr(), pressed as i32) } == 0 {
            Err("无法绘制离线 VR 高亮预览".into())
        } else {
            Ok(())
        }
    }
    pub fn vr_metrics(&self) -> serde_json::Value {
        let mut values = [0.; 6];
        let count = unsafe { native_vr_metrics(values.as_mut_ptr()) };
        serde_json::json!({"samples":count,"poll_ms":{"p50":values[0],"p95":values[1],"max":values[2]},"pointer_frame_interval_ms":{"p50":values[3],"p95":values[4],"max":values[5]}})
    }
    pub fn begin(&self, bg: u32) {
        unsafe { native_begin(bg) }
    }
    pub fn draft(&self, text: &[u16], start: u32, length: u32, bounds: [f32; 4], fill: u32) {
        let [x, y, w, h] = bounds;
        unsafe {
            native_draft(
                text.as_ptr(),
                text.len() as u32,
                start,
                length,
                x,
                y,
                w,
                h,
                26.,
                fill,
            )
        }
    }
    pub fn draft_hit(x: f32, y: f32) -> Option<u32> {
        u32::try_from(unsafe { native_draft_hit(x, y) }).ok()
    }
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32, r: f32, fill: u32, border: u32) {
        unsafe { native_rect(x, y, w, h, r, fill, border) }
    }
    pub fn text(
        &self,
        text: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        size: f32,
        fill: u32,
        align: i32,
    ) {
        let text: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            native_text(
                text.as_ptr(),
                text.len() as u32,
                x,
                y,
                w,
                h,
                size,
                fill,
                align,
            )
        }
    }
    pub fn present(&self) -> Result<(), String> {
        if unsafe { native_present() } == 0 {
            Err(Self::error())
        } else {
            Ok(())
        }
    }
    pub fn mirror(&self) -> Result<(), String> {
        if unsafe { native_mirror() } == 0 {
            Err(Self::error())
        } else {
            Ok(())
        }
    }
    pub fn message(text: &str) {
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe { native_message(text.as_ptr()) }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe { native_close() }
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::{HEIGHT, Settings, Ui};
    type Matrix = [[f32; 4]; 3];

    unsafe extern "C" {
        fn check_panel_trigger_capture() -> i32;
        fn check_panel_grip_dwell() -> i32;
        fn check_settle_panel(pose: *mut Matrix) -> bool;
    }

    #[test]
    fn vr_trigger_stays_captured_to_release_only_if_pressed_on_panel() {
        assert_eq!(unsafe { check_panel_trigger_capture() }, 0);
    }

    #[test]
    fn vr_grip_needs_a_continuous_hold_without_delaying_typing() {
        assert_eq!(unsafe { check_panel_grip_dwell() }, 0);
    }

    #[test]
    fn dropping_panel_only_removes_small_angles_and_preserves_position() {
        fn tilted(pitch: f32, roll: f32) -> Matrix {
            let mut p = pose(1.2, pitch.to_radians());
            let (s, c) = roll.to_radians().sin_cos();
            for row in &mut p {
                let (x, y) = (row[0], row[1]);
                row[0] = x * c + y * s;
                row[1] = y * c - x * s;
            }
            p
        }
        for (pitch, roll, expected_pitch, expected_roll, snap) in [
            (4., -5., 0., 0., true),
            (-12., 5., -12., 0., true),
            (-5., -20., 0., -20., true),
            (12., 20., 12., 20., false),
            (90., 3., 90., 3., false),
            (0., 0., 0., 0., false),
        ] {
            let mut panel = tilted(pitch, roll);
            let original = panel;
            let expected = tilted(expected_pitch, expected_roll);
            assert_eq!(unsafe { check_settle_panel(&mut panel) }, snap);
            for (i, row) in panel.iter().enumerate() {
                assert_eq!(row[3], original[i][3]);
                for axis in 0..3 {
                    assert!((row[axis] - expected[i][axis]).abs() < 0.00001);
                }
            }
        }
    }
    unsafe extern "C" {
        fn native_swap_new() -> *mut std::ffi::c_void;
        fn native_swap_delete(p: *mut std::ffi::c_void);
        fn native_swap_step(p: *mut std::ffi::c_void, operation: i32, slot: i32) -> i32;
    }
    #[test]
    fn raw_updates_never_replace_front_or_reuse_it_before_handoff() {
        struct Swap(*mut std::ffi::c_void);
        impl Drop for Swap {
            fn drop(&mut self) {
                unsafe { native_swap_delete(self.0) }
            }
        }
        let swap = Swap(unsafe { native_swap_new() });
        let step = |op, slot| unsafe { native_swap_step(swap.0, op, slot) };
        assert_eq!(step(4, 0), -1);
        for frame in 0..200 {
            let front = step(4, 0);
            let back = step(0, 0);
            assert!(back == 0 || back == 1);
            assert_ne!(back, front);
            assert_eq!(step(0, 0), -1); // A slow load cannot queue another image.
            assert_eq!(step(2, back), 0); // No presentation before ImageLoaded.
            assert_eq!(step(1, 1 - back), 0); // Ignore stale/wrong-slot events.
            assert_eq!(step(4, 0), front);
            if frame % 3 == 0 {
                step(3, back);
                assert_eq!(step(4, 0), front);
                assert_eq!(step(0, 0), back);
            }
            assert_eq!(step(1, back), 1);
            assert_eq!(step(1, back), 0);
            assert_eq!(step(0, 0), -1); // Keep both textures reserved during handoff.
            assert_eq!(step(4, 0), front);
            assert_eq!(step(2, back), 1);
            assert_eq!(step(4, 0), back);
            assert_eq!(step(5, 0), -1);
        }
    }
    unsafe extern "C" {
        fn native_bgra_to_rgba(src: *const u8, stride: u32, width: u32, height: u32, dst: *mut u8);
    }
    #[test]
    fn raw_overlay_colors_and_padded_rows_are_correct() {
        let bgra = [
            3, 2, 1, 255, 30, 20, 10, 128, 99, 99, 99, 99, 6, 5, 4, 0, 60, 50, 40, 255, 88, 88, 88,
            88,
        ];
        let mut rgba = [77u8; 20];
        unsafe { native_bgra_to_rgba(bgra.as_ptr(), 12, 2, 2, rgba.as_mut_ptr()) };
        assert_eq!(
            &rgba[..16],
            &[1, 2, 3, 255, 10, 20, 30, 128, 4, 5, 6, 0, 40, 50, 60, 255]
        );
        assert_eq!(&rgba[16..], &[77; 4]);
        // Exercise the SIMD path, scalar tail, unaligned source and row padding.
        let input: Vec<u8> = (0..72).map(|n| n as u8).collect();
        let mut output = [77u8; 60];
        unsafe { native_bgra_to_rgba(input.as_ptr().add(1), 36, 7, 2, output.as_mut_ptr()) };
        for y in 0..2 {
            for x in 0..7 {
                let p = 1 + y * 36 + x * 4;
                let q = (y * 7 + x) * 4;
                assert_eq!(
                    &output[q..q + 4],
                    &[input[p + 2], input[p + 1], input[p], input[p + 3]]
                );
            }
        }
        assert_eq!(&output[56..], &[77; 4]);
    }
    unsafe extern "C" {
        fn native_frame_remaining(hz: f64, elapsed_ms: f64) -> i64;
    }
    #[test]
    fn late_vr_frame_does_not_sleep_another_frame() {
        assert_eq!(unsafe { native_frame_remaining(72., 14.) }, 0);
        assert_eq!(unsafe { native_frame_remaining(90., 12.) }, 0);
        assert_eq!(unsafe { native_frame_remaining(120., 9.) }, 0);
        // At 72 Hz, 10 ms of polling + runtime wait leaves 3.889 ms, not 14 ms.
        assert_eq!(unsafe { native_frame_remaining(72., 10.) }, 38889);
        assert_eq!(unsafe { native_frame_remaining(90., 0.) }, 111112);
    }
    unsafe extern "C" {
        fn native_feedback_slice(
            panel: *const Matrix,
            width: f32,
            rect: *const f32,
            part: u32,
            pose: *mut Matrix,
            dimensions: *mut f32,
        ) -> i32;
        fn native_feedback_alpha(w: f32, h: f32, x: f32, y: f32) -> u32;
        fn native_feedback_quad(
            panel: *const Matrix,
            width: f32,
            rect: *const f32,
            pose: *mut Matrix,
            dimensions: *mut f32,
        );
        fn native_panel_hit(
            panel: *const Matrix,
            width: f32,
            ray: *const Matrix,
            result: *mut f32,
        ) -> i32;
    }
    #[test]
    fn vr_feedback_slices_cover_actual_keys_without_gaps_at_all_widths() {
        let mut ui = Ui::new(Settings::default(), true, false);
        ui.state.ready = true;
        ui.state.english = true;
        for settings in [false, true] {
            ui.settings_open = settings;
            ui.rebuild();
            for width in [0.45, 0.7, 0.8, 1.4] {
                for (yaw, pitch) in [(0., 0.), (0.7, 0.4), (-1.4, -0.7)] {
                    let panel = pose(yaw, pitch);
                    for key in ui.buttons.iter().filter(|b| b.enabled) {
                        let rect = [key.x, key.y, key.w, key.h];
                        let mut edge = key.x;
                        for part in 0..3 {
                            let mut quad = [[0.; 4]; 3];
                            let mut dims = [0.; 4];
                            assert_eq!(
                                unsafe {
                                    native_feedback_slice(
                                        &panel,
                                        width,
                                        rect.as_ptr(),
                                        part,
                                        &mut quad,
                                        dims.as_mut_ptr(),
                                    )
                                },
                                1,
                                "{}",
                                key.label
                            );
                            let mut hit = [0.; 5];
                            assert_eq!(
                                unsafe { native_panel_hit(&panel, width, &quad, hit.as_mut_ptr()) },
                                1
                            );
                            let actual_width = dims[0] / width * 1024.;
                            // OpenVR derives height from cropped source pixels and texel aspect.
                            let actual_height = actual_width / (dims[2] / dims[3] * dims[1]);
                            assert!(
                                (actual_height - key.h).abs() < 0.002,
                                "{} height",
                                key.label
                            );
                            assert!(
                                (hit[0] - actual_width / 2. - edge).abs() < 0.002,
                                "{} seam",
                                key.label
                            );
                            assert!(
                                (hit[1] - key.y - key.h / 2.).abs() < 0.002,
                                "{} centre",
                                key.label
                            );
                            edge += actual_width;
                        }
                        assert!(
                            (edge - key.x - key.w).abs() < 0.002,
                            "{} right edge",
                            key.label
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn vr_feedback_atlas_keeps_round_corners_and_border_thickness_on_long_keys() {
        let alpha = |w, h, x, y| unsafe { native_feedback_alpha(w, h, x, y) };
        for h in [44., 50., 56., 60., 64.] {
            for w in [52., 64., 84., 108., 187.5, 420., 440., 916.] {
                assert_eq!(alpha(w, h, -0.1, h / 2.), 0);
                assert_eq!(alpha(w, h, w, h / 2.), 0);
                assert_eq!(alpha(w, h, 0.25, 0.25), 0);
                assert!(alpha(w, h, 0.25, h / 2.) > 240);
                assert!(alpha(w, h, w - 0.25, h / 2.) > 240);
                assert!(alpha(w, h, w / 2., 0.25) > 240);
                for x in [9.75, 10.25, w / 2., w - 10.25, w - 9.75] {
                    assert_eq!(alpha(w, h, x, h / 2.), 48, "no vertical seam");
                    assert!(alpha(w, h, x, 0.25) > 240, "unbroken top border");
                }
                for x in [0.25, 0.75, 1.75, 4.25, 8.25, 9.75] {
                    for y in [0.25, 1.75, 4.25, 8.25, 9.75] {
                        assert_eq!(alpha(w, h, x, y), alpha(84., h, x, y), "left cap stretches");
                        assert_eq!(
                            alpha(w, h, w - x, y),
                            alpha(84., h, 84. - x, y),
                            "right cap stretches"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn feedback_quads_match_key_centers_and_sizes_after_rotation() {
        let mut ui = Ui::new(Settings::default(), true, false);
        ui.state.ready = true;
        ui.state.english = true;
        ui.rebuild();
        for width in [0.45, 0.7, 0.8, 1.4] {
            for (yaw, pitch) in [(0., 0.), (0.7, 0.4), (-1.4, -0.7)] {
                let panel = pose(yaw, pitch);
                for key in &ui.buttons {
                    let rect = [key.x, key.y, key.w, key.h];
                    let mut quad = [[0.; 4]; 3];
                    let mut dims = [0.; 2];
                    unsafe {
                        native_feedback_quad(
                            &panel,
                            width,
                            rect.as_ptr(),
                            &mut quad,
                            dims.as_mut_ptr(),
                        )
                    };
                    let mut result = [0.; 5];
                    assert_eq!(
                        unsafe { native_panel_hit(&panel, width, &quad, result.as_mut_ptr()) },
                        1
                    );
                    assert!(
                        (result[0] - key.x - key.w / 2.).abs() < 0.002
                            && (result[1] - key.y - key.h / 2.).abs() < 0.002,
                        "{}",
                        key.label
                    );
                    assert!((dims[0] - key.w / 1024. * width).abs() < 0.00001);
                    assert!((dims[0] / dims[1] - key.h / 1024. * width).abs() < 0.00001);
                }
            }
        }
    }
    fn pose(yaw: f32, pitch: f32) -> Matrix {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        [
            [cy, sy * sp, sy * cp, 1.2],
            [0., cp, -sp, 1.5],
            [-sy, cy * sp, cy * cp, -0.8],
        ]
    }
    fn ray_at(p: Matrix, width: f32, x: f32, y: f32) -> Matrix {
        let local = [
            (x / 1024. - 0.5) * width,
            (0.5 - y / HEIGHT) * width * HEIGHT / 1024.,
            0.6,
        ];
        let mut ray = p;
        for j in 0..3 {
            ray[j][3] = p[j][3] + (0..3).map(|k| p[j][k] * local[k]).sum::<f32>();
        }
        ray
    }
    #[test]
    fn every_key_center_and_cursor_agree_at_all_panel_sizes_and_rotations() {
        let mut ui = Ui::new(Settings::default(), true, false);
        ui.state.ready = true;
        ui.state.english = true;
        ui.rebuild();
        assert_eq!(ui.hit(656., 350.), Some(36)); // N
        assert_eq!(ui.hit(656., 276.), Some(27)); // J
        for width in [0.45, 0.7, 0.8, 1.4] {
            for (yaw, pitch) in [(0., 0.), (0.7, 0.4), (-1.4, -0.7)] {
                let panel = pose(yaw, pitch);
                for key in ui.buttons.iter().filter(|k| k.enabled) {
                    let x = key.x + key.w / 2.;
                    let y = key.y + key.h / 2.;
                    let ray = ray_at(panel, width, x, y);
                    let mut result = [0.; 5];
                    assert_eq!(
                        unsafe { native_panel_hit(&panel, width, &ray, result.as_mut_ptr()) },
                        1,
                        "{}",
                        key.label
                    );
                    assert!(
                        (result[0] - x).abs() < 0.002 && (result[1] - y).abs() < 0.002,
                        "{} {:?}",
                        key.label,
                        result
                    );
                    assert_eq!(ui.hit(result[0], result[1]), Some(key.id));
                    // Project the exact point used by the visible cursor back to this plane.
                    let local_y = (0..3)
                        .map(|j| (result[2 + j] - panel[j][3]) * panel[j][1])
                        .sum::<f32>();
                    assert!(
                        ((0.5 - local_y / (width * HEIGHT / 1024.)) * HEIGHT - y).abs() < 0.002
                    );
                }
            }
        }
    }
    #[test]
    fn ray_outside_behind_parallel_or_back_facing_cannot_type() {
        let panel = pose(0., 0.);
        let mut out = [0.; 5];
        for (x, y) in [(-1., 300.), (1025., 300.), (500., -1.), (500., HEIGHT + 1.)] {
            let ray = ray_at(panel, 0.8, x, y);
            assert_eq!(
                unsafe { native_panel_hit(&panel, 0.8, &ray, out.as_mut_ptr()) },
                0
            );
        }
        let mut ray = ray_at(panel, 0.8, 500., 300.);
        ray[2][3] = panel[2][3] - 0.5;
        assert_eq!(
            unsafe { native_panel_hit(&panel, 0.8, &ray, out.as_mut_ptr()) },
            0
        );
        ray = ray_at(panel, 0.8, 500., 300.);
        ray[2][2] = 0.;
        ray[0][2] = 1.;
        assert_eq!(
            unsafe { native_panel_hit(&panel, 0.8, &ray, out.as_mut_ptr()) },
            0
        );
        ray = ray_at(panel, 0.8, 500., 300.);
        ray[2][2] = -1.;
        assert_eq!(
            unsafe { native_panel_hit(&panel, 0.8, &ray, out.as_mut_ptr()) },
            0
        );
    }
}
