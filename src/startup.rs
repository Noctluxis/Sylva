// SPDX-License-Identifier: MPL-2.0
use crate::native::Native;
use std::os::windows::process::CommandExt;
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

pub const APP_KEY: &str = "local.sylva.keyboard";
fn manifest(exe: &Path, root: &Path, key: &str, check: bool) -> serde_json::Value {
    serde_json::json!({"source":"builtin", "applications":[{
        "app_key":key, "launch_type":"binary", "binary_path_windows":exe,
        "working_directory":root, "arguments":if check {"--self-test"} else {"--from-steamvr"},
        "is_dashboard_overlay":true, "image_path":root.join("assets/branding/sylva.png"),
        "strings":{"en_us":{"name":"Sylva", "description":"Sylva VR Chinese keyboard"},
                   "zh_cn":{"name":"Sylva", "description":"Sylva VR 中文输入法"}}
    }]})
}
pub fn configure(root: &Path, enabled: bool, check: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let key = if check {
        format!("local.sylva.check.{}", std::process::id())
    } else {
        APP_KEY.into()
    };
    let path = root.join(if check {
        "runtime/autostart-check.vrmanifest"
    } else {
        "sylva.vrmanifest"
    });
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    if enabled || check {
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&manifest(&exe, root, &key, check)).unwrap(),
        )
        .map_err(|e| e.to_string())?;
    }
    let result = Native::autostart(&path, &key, enabled, check);
    if check && result.is_ok() {
        let _ = std::fs::remove_file(path);
    }
    result
}
pub struct Task {
    result: Receiver<Result<(), String>>,
    pub enabled: bool,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Task {
    pub fn start(enabled: bool) -> Self {
        let (tx, result) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let outcome = (|| {
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let mut child = Command::new(exe)
                    .args([
                        "--configure-steamvr-autostart",
                        if enabled { "on" } else { "off" },
                    ])
                    .creation_flags(0x08000000)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                let started = Instant::now();
                loop {
                    if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                        break;
                    }
                    if started.elapsed() > Duration::from_secs(20) {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("SteamVR 设置超时，请稍后重试".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let output = child.wait_with_output().map_err(|e| e.to_string())?;
                serde_json::from_slice::<Result<(), String>>(&output.stdout)
                    .map_err(|_| "SteamVR 设置进程未返回有效结果".to_string())?
            })();
            let _ = tx.send(outcome);
        });
        Self {
            result,
            enabled,
            thread: Some(thread),
        }
    }
    pub fn poll(&self) -> Option<Result<(), String>> {
        match self.result.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("SteamVR 设置任务已停止".into())),
        }
    }
    pub fn finish(mut self) -> (bool, Result<(), String>) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        (
            self.enabled,
            self.result
                .recv()
                .unwrap_or_else(|_| Err("SteamVR 设置任务已停止".into())),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_preserves_absolute_unicode_paths_and_launch_policy() {
        let root = Path::new(r"E:\输入法 test\Sylva");
        let exe = root.join("sylva-vr.exe");
        let m = manifest(&exe, root, APP_KEY, false);
        let text = serde_json::to_string(&m).unwrap();
        let restored: serde_json::Value = serde_json::from_str(&text).unwrap();
        let app = &restored["applications"][0];
        assert_eq!(app["binary_path_windows"], exe.to_str().unwrap());
        assert_eq!(app["app_key"], APP_KEY);
        assert_eq!(app["arguments"], "--from-steamvr");
        assert_eq!(app["is_dashboard_overlay"], true);
        assert_eq!(
            manifest(&exe, root, "local.sylva.check", true)["applications"][0]["arguments"],
            "--self-test"
        );
    }
}
