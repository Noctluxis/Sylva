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
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Account {
    pub key: String,
    pub id: String,
    pub model: String,
    pub base: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Key,
    Id,
    Model,
    Base,
}
impl Field {
    pub fn get(self, a: &Account) -> &str {
        match self {
            Self::Key => &a.key,
            Self::Id => &a.id,
            Self::Model => &a.model,
            Self::Base => &a.base,
        }
    }
    pub fn set(self, a: &mut Account, value: String) {
        *match self {
            Self::Key => &mut a.key,
            Self::Id => &mut a.id,
            Self::Model => &mut a.model,
            Self::Base => &mut a.base,
        } = value;
    }
}
impl Service {
    pub fn model_hint(&self) -> Option<&'static str> {
        match self.id {
            "ollama" => Some("填写 ollama list 中的模型名，含标签"),
            "lmstudio" => Some("填写 LM Studio 中已加载的模型 ID"),
            "llamacpp" => Some("填写服务使用的模型名或 --alias"),
            "ark" => Some("填写模型 ID 或 ep- 开头的接入点 ID"),
            "openrouter" => Some("填写供应商/模型名"),
            _ => None,
        }
    }
    pub fn fields(&self) -> &'static [(Field, &'static str, &'static str)] {
        use Field::*;
        match self.kind {
            Kind::Chat => &[
                (Key, "API Key", "输入或粘贴密钥"),
                (Model, "模型 ID", "填写账号可用的模型或接入点 ID"),
                (Base, "API 地址", "兼容接口的基础地址"),
            ],
            Kind::LocalChat => &[
                (Base, "API 地址", "例如 http://127.0.0.1:8080/v1"),
                (Model, "模型 ID", "填写服务中已加载的模型名"),
                (Key, "API Key", "可选"),
            ],
            Kind::Azure => &[
                (Key, "订阅密钥", "输入或粘贴密钥"),
                (Id, "资源区域", "按 Azure 资源填写，例如 eastasia"),
            ],
            Kind::Papago => &[
                (Id, "Client ID", "应用客户端 ID"),
                (Key, "Client Secret", "输入或粘贴密钥"),
            ],
            Kind::Baidu => &[
                (Id, "APP ID", "百度翻译开放平台的应用 ID"),
                (Key, "密钥", "输入或粘贴密钥"),
            ],
            Kind::Tencent => &[
                (Id, "SecretId", "腾讯云访问密钥 ID"),
                (Key, "SecretKey", "输入或粘贴密钥"),
                (Model, "地域", "默认 ap-guangzhou"),
            ],
            Kind::Alibaba => &[
                (Id, "AccessKey ID", "阿里云访问密钥 ID"),
                (Key, "AccessKey Secret", "输入或粘贴密钥"),
            ],
            _ => &[(Key, "API Key", "输入或粘贴密钥")],
        }
    }
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
    let a = load_account(s)?;
    if a.key.is_empty() && s.kind != Kind::LocalChat {
        return Err(format!("请先配置 {} 的凭据", s.name));
    }
    if matches!(s.kind, Kind::Chat | Kind::LocalChat) && (a.model.is_empty() || a.base.is_empty()) {
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
pub fn load_account(s: &Service) -> Result<Account, String> {
    let mut a = load(s)?;
    if a.base.is_empty() {
        a.base = s.base.into();
    }
    Ok(a)
}
pub fn save_account(s: &Service, a: &Account) -> Result<(), String> {
    let mut data = serde_json::to_vec(a).map_err(|_| "无法编码服务配置")?;
    if data.len() > 2560 {
        return Err("配置过长，请缩短密钥、模型或地址".into());
    }
    let mut size = data.len() as u32;
    let err =
        unsafe { translation_credential(target(s).as_ptr(), data.as_mut_ptr(), &mut size, 1) };
    if err == 0 {
        Ok(())
    } else {
        Err(format!("保存 Windows 凭据失败 ({err})"))
    }
}
#[cfg(test)]
mod credential_tests {
    use super::*;
    #[test]
    #[ignore = "writes and removes a temporary test-only Windows credential"]
    fn edited_profile_uses_the_existing_credential_format() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let id = format!(
            "fixture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let s = Service {
            id: Box::leak(id.into_boxed_str()),
            name: "Test",
            kind: Kind::Chat,
            base: "https://example.invalid/v1",
        };
        let mut a = load_account(&s).unwrap();
        assert_eq!(a.base, s.base);
        assert!(a.key.is_empty());
        a.key = "synthetic".into();
        a.model = "local-fixture".into();
        save_account(&s, &a).unwrap();
        struct Cleanup(Vec<u16>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe extern "system" {
                    fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
                }
                assert_ne!(unsafe { CredDeleteW(self.0.as_ptr(), 1, 0) }, 0);
            }
        }
        let _cleanup = Cleanup(target(&s));
        let stored = account(&s).unwrap();
        assert_eq!(stored.model, "local-fixture");
        assert_eq!(stored.key, "synthetic");
    }
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
