// SPDX-License-Identifier: MPL-2.0
use super::{Kind, Service};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ptr;
unsafe extern "C" {
    fn translation_http(
        method: *const u16,
        url: *const u16,
        headers: *const u16,
        body: *const u8,
        size: u32,
        out: *mut *mut u8,
        len: *mut u32,
        status: *mut u32,
    ) -> u32;
    fn translation_free(data: *mut u8);
    fn translation_credential(name: *const u16, buf: *mut u8, len: *mut u32, write: i32) -> u32;
    fn translation_edit(
        title: *const u16,
        labels: *const *const u16,
        values: *const *mut u16,
    ) -> i32;
    fn translation_dialog_error(text: *const u16);
    fn translation_digest(
        algorithm: *const u16,
        key: *const u8,
        keylen: u32,
        data: *const u8,
        size: u32,
        out: *mut u8,
        len: u32,
    ) -> i32;
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
#[derive(Default, Serialize, Deserialize)]
pub struct Account {
    pub key: String,
    pub id: String,
    pub model: String,
    pub base: String,
}
fn target(s: &Service) -> Vec<u16> {
    wide(&format!("Sylva/Translation/{}", s.id))
}
fn load(s: &Service) -> Result<Account, String> {
    let mut buf = [0u8; 2560];
    let mut n = buf.len() as u32;
    match unsafe { translation_credential(target(s).as_ptr(), buf.as_mut_ptr(), &mut n, 0) } {
        0 => serde_json::from_slice(&buf[..n as usize])
            .map_err(|_| "服务配置损坏，请重新配置".into()),
        1168 => Ok(Account::default()),
        e => Err(format!("读取 Windows 凭据失败 ({e})")),
    }
}
pub fn account(s: &Service) -> Result<Account, String> {
    if !s.account() {
        return Ok(Account::default());
    }
    let mut a = load(s)?;
    if a.base.is_empty() {
        a.base = s.base.into();
    }
    if a.key.is_empty() && s.id != "custom" {
        return Err(format!("请先配置 {} 的凭据", s.name));
    }
    if s.kind == Kind::Chat && (a.model.is_empty() || a.base.is_empty()) {
        return Err("请先配置模型 ID 和服务地址".into());
    }
    if matches!(
        s.kind,
        Kind::Papago | Kind::Baidu | Kind::Tencent | Kind::Alibaba
    ) && a.id.is_empty()
    {
        return Err("请先配置应用 ID 或 Access Key ID".into());
    }
    Ok(a)
}
pub fn edit_account(s: &Service) -> Result<(), String> {
    let mut a = load(s)?;
    if a.base.is_empty() {
        a.base = s.base.into();
    }
    let labels = match s.kind {
        Kind::Chat => ["API Key", "", "模型 / 接入点 ID", "API 基础地址"],
        Kind::Azure => ["订阅密钥", "区域（按资源填写）", "", ""],
        Kind::Papago => ["Client Secret", "Client ID", "", ""],
        Kind::Baidu => ["密钥", "APP ID", "", ""],
        Kind::Tencent => ["SecretKey", "SecretId", "地域（默认广州）", ""],
        Kind::Alibaba => ["AccessKey Secret", "AccessKey ID", "", ""],
        _ => ["API Key", "", "", ""],
    }
    .map(wide);
    let labels = labels.each_ref().map(|s| s.as_ptr());
    let mut values = [&a.key, &a.id, &a.model, &a.base].map(|s| {
        let mut out = [0u16; 1024];
        for (dest, c) in out[..1023].iter_mut().zip(s.encode_utf16()) {
            *dest = c;
        }
        out
    });
    let pointers = values.each_mut().map(|s| s.as_mut_ptr());
    let code = unsafe {
        translation_edit(
            wide(&format!("Sylva · {}", s.name)).as_ptr(),
            labels.as_ptr(),
            pointers.as_ptr(),
        )
    };
    if code < 0 {
        return Err("无法打开配置窗口".into());
    }
    if code == 1 {
        let [key, id, model, base] = values.map(|b| {
            String::from_utf16_lossy(&b[..b.iter().position(|x| *x == 0).unwrap_or(b.len())])
                .trim()
                .to_owned()
        });
        let mut data = serde_json::to_vec(&Account {
            key,
            id,
            model,
            base,
        })
        .unwrap();
        let mut size = data.len() as u32;
        let err =
            unsafe { translation_credential(target(s).as_ptr(), data.as_mut_ptr(), &mut size, 1) };
        if err != 0 {
            let msg = format!("保存 Windows 凭据失败 ({err})");
            unsafe { translation_dialog_error(wide(&msg).as_ptr()) };
            return Err(msg);
        }
    }
    Ok(())
}
pub fn http(url: &str, headers: &[(&str, &str)], body: Option<&str>) -> Result<String, String> {
    if url.chars().any(char::is_control)
        || headers.iter().any(|(_, v)| v.chars().any(char::is_control))
    {
        return Err("服务配置含无效字符".into());
    }
    let h = headers
        .iter()
        .map(|(k, v)| format!("{k}: {v}\r\n"))
        .collect::<String>();
    let mut out = ptr::null_mut();
    let mut size = 0;
    let mut status = 0;
    let verb = if body.is_some() { "POST" } else { "GET" };
    let data = body.unwrap_or("");
    let err = unsafe {
        translation_http(
            wide(verb).as_ptr(),
            wide(url).as_ptr(),
            wide(&h).as_ptr(),
            data.as_ptr(),
            data.len() as u32,
            &mut out,
            &mut size,
            &mut status,
        )
    };
    if err != 0 {
        return Err(format!("网络请求失败 ({err})"));
    }
    if !(200..300).contains(&status) {
        return Err(match status {
            401 | 403 => format!("服务拒绝请求 ({status})"),
            429 => "服务限流或额度不足 (429)".into(),
            _ => format!("翻译服务 HTTP {status}"),
        });
    }
    if out.is_null() {
        return Err("服务返回空响应".into());
    }
    let bytes = unsafe { std::slice::from_raw_parts(out, size as usize) };
    let result = std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| "服务返回无效文本".into());
    unsafe { translation_free(out) };
    result
}
pub fn json(url: &str, headers: &[(&str, &str)], body: Option<&str>) -> Result<Value, String> {
    serde_json::from_str(&http(url, headers, body)?).map_err(|_| "服务返回了非 JSON 响应".into())
}
pub fn digest(alg: &str, key: &[u8], data: &[u8], len: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0; len];
    if unsafe {
        translation_digest(
            wide(alg).as_ptr(),
            if key.is_empty() {
                ptr::null()
            } else {
                key.as_ptr()
            },
            key.len() as u32,
            data.as_ptr(),
            data.len() as u32,
            out.as_mut_ptr(),
            len as u32,
        )
    } == 0
    {
        return Err("请求签名失败".into());
    }
    Ok(out)
}
