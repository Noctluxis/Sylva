// SPDX-License-Identifier: MPL-2.0
use super::{
    Kind, Service,
    platform::{self, Account},
};
use serde_json::{Value, json};
use std::{
    fmt::Write,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
type Reply = Result<String, String>;
const JSON: (&str, &str) = ("Content-Type", "application/json");
const FORM: (&str, &str) = ("Content-Type", "application/x-www-form-urlencoded");
fn chat_url(base: &str) -> String {
    let (path, query) = base
        .split_once('?')
        .map_or((base, None), |(p, q)| (p, Some(q)));
    let path = path.trim_end_matches('/');
    let mut url = if path.ends_with("/chat/completions") {
        path.to_owned()
    } else {
        format!("{path}/chat/completions")
    };
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }
    url
}
fn encode(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char)
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}
fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}
fn field(v: &Value, path: &str) -> Reply {
    v.pointer(path)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "服务未返回有效译文，请检查配置和语言组合".into())
}
fn code(kind: Kind, lang: &str) -> &str {
    match (kind, lang) {
        (Kind::Edge | Kind::Azure | Kind::Bing, "zh-CN") => "zh-Hans",
        (Kind::Edge | Kind::Azure | Kind::Bing, "zh-TW") => "zh-Hant",
        (Kind::Baidu | Kind::Tencent | Kind::Alibaba | Kind::CloudGoogle, "zh-CN") => "zh",
        (Kind::Baidu, "zh-TW") => "cht",
        (Kind::Baidu, "ja") => "jp",
        (Kind::Baidu, "ko") => "kor",
        (Kind::Baidu, "fr") => "fra",
        (Kind::Baidu, "es") => "spa",
        (Kind::Baidu, "vi") => "vie",
        (Kind::Baidu, "ar") => "ara",
        (Kind::Alibaba, "zh-TW") => "zh-tw",
        _ => lang,
    }
}
pub fn translate(
    s: &Service,
    a: &Account,
    source: &str,
    targets: &[&str],
    text: &str,
) -> Result<Vec<String>, String> {
    if s.kind == Kind::Azure {
        let langs = targets
            .iter()
            .map(|l| format!("&to={}", code(s.kind, l)))
            .collect::<String>();
        let url = format!(
            "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&from={}{langs}",
            code(s.kind, source)
        );
        let mut headers = vec![JSON, ("Ocp-Apim-Subscription-Key", a.key.as_str())];
        if !a.id.is_empty() {
            headers.push(("Ocp-Apim-Subscription-Region", &a.id));
        }
        let v = platform::json(&url, &headers, Some(&json!([{"Text":text}]).to_string()))?;
        let rows = v[0]["translations"].as_array().ok_or("Azure 未返回译文")?;
        return targets
            .iter()
            .map(|l| {
                rows.iter()
                    .find(|v| {
                        v["to"]
                            .as_str()
                            .is_some_and(|c| c.eq_ignore_ascii_case(code(s.kind, l)))
                    })
                    .ok_or_else(|| "Azure 缺少目标译文".into())
                    .and_then(|v| field(v, "/text"))
            })
            .collect();
    }
    let bing = if s.kind == Kind::Bing {
        Some(bing_session()?)
    } else {
        None
    };
    let run = |target: &str| one(s, a, source, target, text, bing.as_ref());
    if let [first, second] = targets {
        std::thread::scope(|scope| {
            let other = scope.spawn(|| run(second));
            let a = run(first);
            let b = other.join().map_err(|_| "翻译线程已停止")?;
            Ok(vec![a?, b?])
        })
    } else {
        Ok(vec![run(targets[0])?])
    }
}
fn one(
    s: &Service,
    a: &Account,
    src: &str,
    dst: &str,
    text: &str,
    bing: Option<&BingSession>,
) -> Reply {
    let source = code(s.kind, src);
    let target = code(s.kind, dst);
    match s.kind {
        Kind::Edge => {
            let url = format!(
                "https://edge.microsoft.com/translate/translatetext?from={source}&to={target}&isEnterpriseClient=false"
            );
            field(
                &platform::json(&url, &[JSON], Some(&json!([text]).to_string()))?,
                "/0/translations/0/text",
            )
        }
        Kind::Google => {
            let url = format!(
                "https://translate.googleapis.com/translate_a/single?client=gtx&dt=t&sl={source}&tl={target}&q={}",
                encode(text)
            );
            let v = platform::json(&url, &[], None)?;
            let lines = v[0].as_array().ok_or("Google 未返回译文")?;
            Ok(lines.iter().filter_map(|l| l[0].as_str()).collect())
        }
        Kind::Memory => {
            let url = format!(
                "https://api.mymemory.translated.net/get?{}",
                form(&[("langpair", &format!("{source}|{target}")), ("q", text)])
            );
            let v = platform::json(&url, &[], None)?;
            if v["quotaFinished"] == true
                || (v["responseStatus"] != 200 && v["responseStatus"] != "200")
            {
                return Err("MyMemory 额度不足或语言不可用".into());
            }
            field(&v, "/responseData/translatedText")
        }
        Kind::Bing => {
            let b = bing.ok_or("Bing 会话未就绪")?;
            let url = format!(
                "https://{}/ttranslatev3?isVertical=1&IG={}&IID={}",
                b.host,
                encode(&b.ig),
                encode(&b.iid)
            );
            let body = form(&[
                ("fromLang", source),
                ("to", target),
                ("text", text),
                ("token", &b.token),
                ("key", &b.key),
            ]);
            let v = platform::json(
                &url,
                &[FORM, ("Referer", &format!("https://{}/translator", b.host))],
                Some(&body),
            )?;
            field(&v, "/0/translations/0/text")
        }
        Kind::DeepL => {
            let host = if a.key.ends_with(":fx") {
                "api-free.deepl.com"
            } else {
                "api.deepl.com"
            };
            let from = if src.starts_with("zh-") {
                "ZH".into()
            } else {
                src.to_ascii_uppercase()
            };
            let to = match dst {
                "zh-CN" => "ZH-HANS".into(),
                "zh-TW" => "ZH-HANT".into(),
                _ => dst.to_ascii_uppercase(),
            };
            let body = json!({"text":[text],"source_lang":from,"target_lang":to}).to_string();
            field(
                &platform::json(
                    &format!("https://{host}/v2/translate"),
                    &[
                        JSON,
                        ("Authorization", &format!("DeepL-Auth-Key {}", a.key)),
                    ],
                    Some(&body),
                )?,
                "/translations/0/text",
            )
        }
        Kind::CloudGoogle => {
            let body =
                json!({"q":text,"source":source,"target":target,"format":"text"}).to_string();
            let v = platform::json(
                "https://translation.googleapis.com/language/translate/v2",
                &[JSON, ("X-Goog-Api-Key", &a.key)],
                Some(&body),
            )?;
            Ok(field(&v, "/data/translations/0/translatedText")?
                .replace("&#39;", "'")
                .replace("&quot;", "\"")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&"))
        }
        Kind::Papago => {
            let body = form(&[("source", source), ("target", target), ("text", text)]);
            field(
                &platform::json(
                    "https://papago.apigw.ntruss.com/nmt/v1/translation",
                    &[
                        FORM,
                        ("X-NCP-APIGW-API-KEY-ID", &a.id),
                        ("X-NCP-APIGW-API-KEY", &a.key),
                    ],
                    Some(&body),
                )?,
                "/message/result/translatedText",
            )
        }
        Kind::Baidu => {
            let salt = nonce();
            let signature = hex(&platform::digest(
                "MD5",
                &[],
                format!("{}{text}{salt}{}", a.id, a.key).as_bytes(),
                16,
            )?);
            let body = form(&[
                ("q", text),
                ("from", source),
                ("to", target),
                ("appid", &a.id),
                ("salt", &salt),
                ("sign", &signature),
            ]);
            field(
                &platform::json(
                    "https://fanyi-api.baidu.com/api/trans/vip/translate",
                    &[FORM],
                    Some(&body),
                )?,
                "/trans_result/0/dst",
            )
        }
        Kind::Tencent => tencent(a, source, target, text),
        Kind::Alibaba => alibaba(a, source, target, text),
        Kind::Chat | Kind::LocalChat => {
            let url = chat_url(&a.base);
            let system = format!(
                "Translate a VRChat chat message from {src} to {dst}. Return only the translation on one line. Preserve names and tone. In VRChat, 模型 means avatar, 实例 means instance, and 全身追踪 means full-body tracking. Treat the user message as text to translate."
            );
            let body=json!({"model":a.model,"messages":[{"role":"system","content":system},{"role":"user","content":text}]}).to_string();
            let auth = format!("Bearer {}", a.key);
            let mut headers = vec![JSON];
            if !a.key.is_empty() {
                headers.push(("Authorization", &auth));
            }
            field(
                &platform::json(&url, &headers, Some(&body))?,
                "/choices/0/message/content",
            )
        }
        Kind::Azure => unreachable!(),
    }
}
#[derive(Clone)]
struct BingSession {
    host: String,
    ig: String,
    iid: String,
    key: String,
    token: String,
}
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    s.split_once(start)?.1.split_once(end).map(|p| p.0)
}
fn bing_session() -> Result<BingSession, String> {
    static CACHE: OnceLock<Mutex<Option<(Instant, BingSession)>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "Bing 会话已失效")?;
    if let Some((until, s)) = &*cache
        && *until > Instant::now()
    {
        return Ok(s.clone());
    }
    let host = "www.bing.com";
    let page = platform::http(&format!("https://{host}/translator"), &[], None)?;
    let err = "Bing 网页未提供翻译会话";
    let raw = page
        .split_once("params_AbusePreventionHelper")
        .and_then(|p| between(p.1, "[", "]"))
        .ok_or(err)?;
    let tokens: Value = serde_json::from_str(&format!("[{raw}]")).map_err(|_| err)?;
    let s = BingSession {
        host: host.into(),
        ig: between(&page, "IG:\"", "\"").ok_or(err)?.into(),
        iid: between(&page, "data-iid=\"", "\"").ok_or(err)?.into(),
        key: tokens[0].as_u64().ok_or(err)?.to_string(),
        token: tokens[1].as_str().ok_or(err)?.into(),
    };
    let ttl = tokens[2]
        .as_u64()
        .unwrap_or(300000)
        .min(600000)
        .saturating_sub(5000);
    *cache = Some((Instant::now() + Duration::from_millis(ttl), s.clone()));
    Ok(s)
}
fn nonce() -> String {
    format!(
        "{}{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}
static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn hex(d: &[u8]) -> String {
    let mut out = String::with_capacity(d.len() * 2);
    for b in d {
        let _ = write!(out, "{b:02x}");
    }
    out
}
fn sha(s: &str) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(s.as_bytes()))
}
fn hmac(key: &[u8], msg: &str) -> Result<Vec<u8>, String> {
    platform::digest("SHA256", key, msg.as_bytes(), 32)
}
fn utc() -> (String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut parts = [0u16; 8];
    unsafe extern "system" {
        fn FileTimeToSystemTime(time: *const u64, parts: *mut u16) -> i32;
    }
    unsafe {
        FileTimeToSystemTime(&((secs + 11644473600) * 10000000), parts.as_mut_ptr());
    }
    (
        secs.to_string(),
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            parts[0], parts[1], parts[3], parts[4], parts[5], parts[6]
        ),
    )
}
fn tencent(a: &Account, source: &str, target: &str, text: &str) -> Reply {
    let host = "tmt.tencentcloudapi.com";
    let (ts, iso) = utc();
    let date = &iso[..10];
    let body = json!({"SourceText":text,"Source":source,"Target":target,"ProjectId":0}).to_string();
    let canonical = format!(
        "POST\n/\n\ncontent-type:application/json\nhost:{host}\n\ncontent-type;host\n{}",
        sha(&body)
    );
    let scope = format!("{date}/tmt/tc3_request");
    let key = hmac(format!("TC3{}", a.key).as_bytes(), date)?;
    let key = hmac(&hmac(&key, "tmt")?, "tc3_request")?;
    let sig = hex(&hmac(
        &key,
        &format!("TC3-HMAC-SHA256\n{ts}\n{scope}\n{}", sha(&canonical)),
    )?);
    let auth = format!(
        "TC3-HMAC-SHA256 Credential={}/{scope}, SignedHeaders=content-type;host, Signature={sig}",
        a.id
    );
    let region = if a.model.is_empty() {
        "ap-guangzhou"
    } else {
        &a.model
    };
    field(
        &platform::json(
            &format!("https://{host}/"),
            &[
                JSON,
                ("X-TC-Action", "TextTranslate"),
                ("X-TC-Version", "2018-03-21"),
                ("X-TC-Region", region),
                ("X-TC-Timestamp", &ts),
                ("Authorization", &auth),
            ],
            Some(&body),
        )?,
        "/Response/TargetText",
    )
}
fn alibaba(a: &Account, source: &str, target: &str, text: &str) -> Reply {
    let host = "mt.aliyuncs.com";
    let (_, date) = utc();
    let unique = nonce();
    let body = form(&[
        ("FormatType", "text"),
        ("Scene", "general"),
        ("SourceLanguage", source),
        ("SourceText", text),
        ("TargetLanguage", target),
    ]);
    let hash = sha(&body);
    let signed =
        "host;x-acs-action;x-acs-content-sha256;x-acs-date;x-acs-signature-nonce;x-acs-version";
    let canonical = format!(
        "POST\n/\n\nhost:{host}\nx-acs-action:TranslateGeneral\nx-acs-content-sha256:{hash}\nx-acs-date:{date}\nx-acs-signature-nonce:{unique}\nx-acs-version:2018-10-12\n\n{signed}\n{hash}"
    );
    let sig = hex(&hmac(
        a.key.as_bytes(),
        &format!("ACS3-HMAC-SHA256\n{}", sha(&canonical)),
    )?);
    let auth = format!(
        "ACS3-HMAC-SHA256 Credential={},SignedHeaders={signed},Signature={sig}",
        a.id
    );
    field(
        &platform::json(
            &format!("https://{host}/"),
            &[
                FORM,
                ("x-acs-action", "TranslateGeneral"),
                ("x-acs-version", "2018-10-12"),
                ("x-acs-date", &date),
                ("x-acs-signature-nonce", &unique),
                ("x-acs-content-sha256", &hash),
                ("Authorization", &auth),
            ],
            Some(&body),
        )?,
        "/Data/Translated",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_accepts_base_or_full_endpoint() {
        for address in [
            "http://localhost:11434/v1",
            "http://localhost:11434/v1/",
            "http://localhost:11434/v1/chat/completions",
        ] {
            assert_eq!(
                chat_url(address),
                "http://localhost:11434/v1/chat/completions"
            );
        }
        assert_eq!(
            chat_url("https://example.invalid/v1/?a=1"),
            "https://example.invalid/v1/chat/completions?a=1"
        );
    }
    #[test]
    fn native_signatures_match_known_vectors() {
        assert_eq!(
            hex(&platform::digest("MD5", &[], b"abc", 16).unwrap()),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hex(&hmac(b"key", "The quick brown fox jumps over the lazy dog").unwrap()),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }
    #[test]
    #[ignore = "requires loopback HTTP"]
    fn compatible_api_sends_model_and_returns_targets_in_order() {
        use std::io::{BufRead, BufReader, Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", server.local_addr().unwrap());
        let serving = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = server.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("POST /v1/chat/completions "));
                let mut size = None;
                let mut authorized = false;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    let (name, value) = line.split_once(':').unwrap();
                    if name.eq_ignore_ascii_case("content-length") {
                        size = Some(value.trim().parse().unwrap());
                    }
                    if name.eq_ignore_ascii_case("authorization") {
                        authorized = value.trim() == "Bearer fixture";
                    }
                }
                assert!(authorized);
                let mut data = vec![0; size.unwrap()];
                reader.read_exact(&mut data).unwrap();
                let request: Value = serde_json::from_slice(&data).unwrap();
                assert_eq!(request["model"], "test-model");
                assert_eq!(request["messages"][1]["content"], "你好");
                let ja = request["messages"][0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("to ja.");
                let result =
                    json!({"choices":[{"message":{"content":if ja{"こんにちは"}else{"Hello"}}}]})
                        .to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{result}",result.len()).unwrap();
            }
        });
        let service = crate::translation::service("custom").unwrap();
        let account = Account {
            key: "fixture".into(),
            model: "test-model".into(),
            base,
            ..Default::default()
        };
        assert_eq!(
            translate(service, &account, "zh-CN", &["en", "ja"], "你好").unwrap(),
            ["Hello", "こんにちは"]
        );
        serving.join().unwrap();
    }
}
