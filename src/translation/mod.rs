// SPDX-License-Identifier: MPL-2.0
mod platform;
mod providers;
pub use platform::{Account, Field, load_account, save_account};
use serde::{Deserialize, Serialize};
use std::{sync::mpsc, thread};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Edge,
    Memory,
    Bing,
    Google,
    Azure,
    DeepL,
    CloudGoogle,
    Papago,
    Baidu,
    Tencent,
    Alibaba,
    Chat,
    LocalChat,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Free,
    Machine,
    Model,
    Hosted,
}
impl Group {
    pub const ALL: [Self; 4] = [Self::Free, Self::Machine, Self::Model, Self::Hosted];
    pub fn label(self) -> &'static str {
        match self {
            Self::Free => "免 Key 在线",
            Self::Machine => "机器翻译 API",
            Self::Model => "大模型 API",
            Self::Hosted => "本地 / 自建",
        }
    }
}
pub struct Service {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    base: &'static str,
}
macro_rules! service {
    ($id:literal,$name:literal,$kind:ident) => {
        Service {
            id: $id,
            name: $name,
            kind: Kind::$kind,
            base: "",
        }
    };
    ($id:literal,$name:literal,$base:literal) => {
        Service {
            id: $id,
            name: $name,
            kind: Kind::Chat,
            base: $base,
        }
    };
}
pub static SERVICES: &[Service] = &[
    service!("edge", "Microsoft Edge", Edge),
    service!("mymemory", "MyMemory", Memory),
    service!("bing", "Bing", Bing),
    service!("google", "Google 网页", Google),
    service!("azure", "Azure Translator", Azure),
    service!("deepl", "DeepL API", DeepL),
    service!("google-cloud", "Google Cloud", CloudGoogle),
    service!("papago", "NAVER Papago", Papago),
    service!("baidu", "百度翻译", Baidu),
    service!("tencent", "腾讯云机器翻译", Tencent),
    service!("aliyun", "阿里云机器翻译", Alibaba),
    service!("openai", "OpenAI", "https://api.openai.com/v1"),
    service!("claude", "Claude", "https://api.anthropic.com/v1"),
    service!(
        "gemini",
        "Gemini",
        "https://generativelanguage.googleapis.com/v1beta/openai"
    ),
    service!("deepseek", "DeepSeek", "https://api.deepseek.com/v1"),
    service!(
        "qwen",
        "通义千问",
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    ),
    service!("kimi", "Kimi", "https://api.moonshot.cn/v1"),
    service!("glm", "智谱 GLM", "https://open.bigmodel.cn/api/paas/v4"),
    service!(
        "ark",
        "火山方舟",
        "https://ark.cn-beijing.volces.com/api/v3"
    ),
    service!("siliconflow", "硅基流动", "https://api.siliconflow.cn/v1"),
    service!("groq", "Groq", "https://api.groq.com/openai/v1"),
    service!("openrouter", "OpenRouter", "https://openrouter.ai/api/v1"),
    service!("plamo", "PLaMo", "https://api.platform.preferredai.jp/v1"),
    Service {
        id: "ollama",
        name: "Ollama",
        kind: Kind::LocalChat,
        base: "http://127.0.0.1:11434/v1",
    },
    Service {
        id: "lmstudio",
        name: "LM Studio",
        kind: Kind::LocalChat,
        base: "http://127.0.0.1:1234/v1",
    },
    Service {
        id: "llamacpp",
        name: "llama.cpp",
        kind: Kind::LocalChat,
        base: "http://127.0.0.1:8080/v1",
    },
    Service {
        id: "custom",
        name: "自定义兼容接口",
        kind: Kind::LocalChat,
        base: "",
    },
];
pub const LANGS: &[(&str, &str)] = &[
    ("zh-CN", "简体中文"),
    ("en", "英语"),
    ("ja", "日语"),
    ("ko", "韩语"),
    ("zh-TW", "繁体中文"),
    ("fr", "法语"),
    ("de", "德语"),
    ("es", "西班牙语"),
    ("pt", "葡萄牙语"),
    ("it", "意大利语"),
    ("ru", "俄语"),
    ("vi", "越南语"),
    ("th", "泰语"),
    ("id", "印尼语"),
    ("nl", "荷兰语"),
    ("pl", "波兰语"),
    ("tr", "土耳其语"),
    ("ar", "阿拉伯语"),
];
pub fn lang_name(code: &str) -> &'static str {
    LANGS.iter().find(|l| l.0 == code).map_or("无", |l| l.1)
}
pub fn service(id: &str) -> Option<&'static Service> {
    SERVICES.iter().find(|p| p.id == id)
}
impl Service {
    pub fn group(&self) -> Group {
        match self.kind {
            Kind::Edge | Kind::Memory | Kind::Bing | Kind::Google => Group::Free,
            Kind::Chat => Group::Model,
            Kind::LocalChat => Group::Hosted,
            _ => Group::Machine,
        }
    }
    pub fn account(&self) -> bool {
        self.group() != Group::Free
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub provider: String,
    pub source: String,
    pub first: String,
    pub second: Option<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "edge".into(),
            source: "zh-CN".into(),
            first: "en".into(),
            second: None,
        }
    }
}
impl Config {
    fn languages(&self) -> Result<Vec<&str>, String> {
        let targets: Vec<_> = std::iter::once(self.first.as_str())
            .chain(self.second.as_deref())
            .collect();
        if lang_name(&self.source) == "无"
            || targets
                .iter()
                .any(|t| lang_name(t) == "无" || *t == self.source)
            || self.second.as_deref() == Some(&self.first)
        {
            return Err("请选择不同的原文和目标语言".into());
        }
        Ok(targets)
    }
}
type ResultText = Result<String, String>;
fn assemble(source: &str, translations: Vec<String>) -> ResultText {
    let mut lines = vec![single_line(source)];
    for t in translations {
        let line = single_line(t.trim());
        if line.is_empty() || line.contains('\0') {
            return Err("翻译服务返回空白或无效译文".into());
        }
        lines.push(line);
    }
    let message = lines.join("\n");
    if message.encode_utf16().count() > 144 {
        return Err("原文加译文超过 144 字符，请缩短或减少一种译文".into());
    }
    Ok(message)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bilingual_limit_counts_newlines_and_utf16() {
        let source = "👋".repeat(69);
        assert!(assemble(&source, vec!["en".into(), "日本".into()]).is_ok());
        assert!(assemble(&source, vec!["eng".into(), "日本".into()]).is_err());
        assert_eq!(
            assemble("中  文", vec!["hello\r\nworld".into(), "こんにちは".into()]).unwrap(),
            "中  文\nhello world\nこんにちは"
        );
        assert!(assemble("中", vec![" ".into()]).is_err());
    }
    #[test]
    #[ignore = "calls public translators with one synthetic phrase per service"]
    fn live_anonymous_translators() {
        let mut failures = Vec::new();
        for id in ["edge", "mymemory", "bing", "google"] {
            let config = Config {
                provider: id.into(),
                second: Some("ja".into()),
                ..Default::default()
            };
            let t = std::time::Instant::now();
            let r = translate(&config, "你好，你能听到我吗？");
            println!("{id} {:?} {r:?}", t.elapsed());
            if !r.as_ref().is_ok_and(|v| {
                let lines: Vec<_> = v.lines().collect();
                lines.len() == 3
                    && lines[0] == "你好，你能听到我吗？"
                    && lines[1].contains(|c: char| c.is_ascii_alphabetic())
                    && lines[2].contains(|c: char| ('ぁ'..='ヿ').contains(&c))
            }) {
                failures.push(id);
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
    }
}
fn single_line(s: &str) -> String {
    s.replace("\r\n", " ").replace(
        [
            '\r', '\n', '\u{000b}', '\u{000c}', '\u{0085}', '\u{2028}', '\u{2029}',
        ],
        " ",
    )
}
fn translate(config: &Config, source: &str) -> ResultText {
    let targets = config.languages()?;
    let service = service(&config.provider).ok_or("翻译服务不存在")?;
    let account = platform::account(service)?;
    let lines = providers::translate(
        service,
        &account,
        &config.source,
        &targets,
        &single_line(source),
    )?;
    if lines.len() != targets.len() {
        return Err("译文数量不符".into());
    }
    assemble(source, lines)
}
pub struct Task {
    pub original: String,
    pub result: mpsc::Receiver<ResultText>,
}
impl Task {
    pub fn start(
        config: Config,
        original: String,
        notify: mpsc::SyncSender<crate::engine::Action>,
    ) -> Result<Self, String> {
        let (tx, result) = mpsc::channel();
        let input = original.clone();
        thread::Builder::new()
            .name("sylva-translate".into())
            .spawn(move || {
                let _ = tx.send(translate(&config, &input));
                let _ = notify.try_send(crate::engine::Action::SuggestionsReady);
            })
            .map_err(|_| "无法启动翻译任务")?;
        Ok(Self { original, result })
    }
}
