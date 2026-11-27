// SPDX-License-Identifier: MPL-2.0
use crate::{
    engine::{Action, Snapshot},
    native::{Icon, Native},
};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

pub const WIDTH: f32 = 1024.;
pub const HEIGHT: f32 = 480.;
const LEFT: f32 = 16.;
const RIGHT: f32 = 932.;
const GAP: f32 = 8.;
const KEY_TOP: f32 = 170.;
const ROW_STEP: f32 = 74.;
const LETTER_ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const INK: u32 = 0xff263734;
const MUTED: u32 = 0xff657570;
const ACCENT: u32 = 0xff356b59;
const DRAFT_FIELD: usize = 114;
const DRAFT_BOUNDS: [f32; 4] = [30., 43., 716., 40.];

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub osc_address: String,
    #[serde(deserialize_with = "read_key_sound")]
    pub key_sound: bool,
    pub typing_indicator: bool,
    pub haptics: bool,
    pub width_m: f32,
    pub block_input: bool,
    pub dark: bool,
    pub steamvr_startup: bool,
    pub close_to_tray: bool,
    pub silent_start: bool,
    pub translation: crate::translation::Config,
}
// Version 0.1.12 saved a sound name in this field. New saves use a boolean.
fn read_key_sound<'de, D: serde::Deserializer<'de>>(de: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Preference {
        Enabled(bool),
        Legacy(String),
    }
    match Preference::deserialize(de)? {
        Preference::Enabled(enabled) => Ok(enabled),
        Preference::Legacy(name) => match name.as_str() {
            "off" => Ok(false),
            "standard" | "tick" | "space" | "return" | "wood" => Ok(true),
            _ => Err(serde::de::Error::custom("unknown key sound")),
        },
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            osc_address: "127.0.0.1:9000".into(),
            key_sound: true,
            typing_indicator: false,
            haptics: true,
            width_m: 0.7,
            block_input: true,
            dark: false,
            steamvr_startup: false,
            close_to_tray: true,
            silent_start: true,
            translation: Default::default(),
        }
    }
}
impl Settings {
    pub fn plays_key_sound(&self, action: &Action) -> bool {
        self.key_sound
            && matches!(
                action,
                Action::Key(_)
                    | Action::Literal(_)
                    | Action::Space
                    | Action::Backspace
                    | Action::Select(..)
                    | Action::Send
            )
    }
}
#[derive(Clone, Debug)]
pub enum UiAction {
    Input(Action),
    Expand,
    Previous,
    Next,
    Symbols,
    Settings,
    KeySound,
    TypingIndicator,
    Haptics,
    Dark,
    Block,
    Narrow,
    Widen,
    Center,
    Copy,
    SettingsPage(SettingsPage),
    ToggleTranslation,
    Choose(Choice),
    Pick(usize),
    ChoicePage(usize),
    CloseChoices,
    ConfigureService,
    SteamVrStartup,
    CloseToTray,
    SilentStart,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Input,
    Translation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Service,
    Source,
    First,
    Second,
}
impl Choice {
    fn title(self) -> &'static str {
        match self {
            Self::Service => "翻译服务",
            Self::Source => "原文语言",
            Self::First => "第一译文",
            Self::Second => "第二译文",
        }
    }
    fn entries(self) -> Vec<(&'static str, &'static str)> {
        use crate::translation::{LANGS, SERVICES};
        if self == Self::Service {
            return SERVICES.iter().map(|s| (s.id, s.name)).collect();
        }
        let mut list = Vec::new();
        if self == Self::Second {
            list.push(("", "无"));
        }
        list.extend_from_slice(LANGS);
        list
    }
}
#[derive(Clone)]
pub struct Button {
    pub id: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
    pub sub: String,
    pub corner_hint: bool,
    pub action: UiAction,
    pub accent: bool,
    pub enabled: bool,
    pub font: f32,
}
impl Button {
    pub fn hit(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}
pub struct Ui {
    line: DraftLine,
    pub key_audio: Option<crate::key_sound::KeyAudio>,
    pub state: Snapshot,
    pub buttons: Vec<Button>,
    pub expanded: bool,
    pub symbols: bool,
    pub settings_open: bool,
    pub settings_page: SettingsPage,
    pub choices: Option<(Choice, usize)>,
    pub startup_pending: bool,
    pub page: usize,
    pub settings: Settings,
    pub hover: [Option<usize>; 3],
    pub pressed: [Option<usize>; 3],
    pub pending: usize,
    pub vr: bool,
    pub osc_enabled: bool,
    pub notice: String,
    pub runtime_status: String,
    pub copied: bool,
    page_count: usize,
}
#[derive(Default)]
struct DraftLine {
    text: Vec<u16>,
    preedit_start: u32,
    preedit_len: u32,
}
impl DraftLine {
    fn update(&mut self, state: &Snapshot) {
        self.text.clear();
        self.text.extend(state.before_caret().encode_utf16());
        self.preedit_start = self.text.len() as u32;
        self.text.extend(state.preedit.encode_utf16());
        self.preedit_len = self.text.len() as u32 - self.preedit_start;
        self.text.extend(state.draft[state.caret..].encode_utf16());
        // Display line breaks in this single-line viewport without changing OSC text or offsets.
        for c in &mut self.text {
            if matches!(*c, 10..=13 | 0x85 | 0x2028 | 0x2029) {
                *c = '↵' as u16;
            }
        }
    }
    fn draft_offset(&self, draft: &str, displayed: u32) -> usize {
        let offset = displayed
            - displayed
                .saturating_sub(self.preedit_start)
                .min(self.preedit_len);
        let mut units = 0;
        for (start, cluster) in draft.grapheme_indices(true) {
            let length = cluster.encode_utf16().count() as u32;
            if offset <= units + length {
                return if (offset - units) * 2 < length {
                    start
                } else {
                    start + cluster.len()
                };
            }
            units += length;
        }
        draft.len()
    }
}
struct CandidateSlot {
    index: usize,
    x: f32,
    row: usize,
    width: f32,
}
fn candidate_width(text: &str) -> f32 {
    (Native::text_width(text, 23.) + 32.).clamp(72., RIGHT - LEFT)
}
// The collapsed strip stops after one row; expanded pages flow by measured
// width across four rows. Every source index is retained, including long words.
fn candidate_pages(candidates: &[String], expanded: bool) -> Vec<Vec<CandidateSlot>> {
    let rows = if expanded { 4 } else { 1 };
    let mut pages = vec![Vec::new()];
    let mut x = LEFT;
    let mut row = 0;
    for (index, text) in candidates.iter().enumerate() {
        let width = candidate_width(text);
        if x > LEFT && x + width > RIGHT + 0.01 {
            x = LEFT;
            row += 1;
        }
        if row == rows {
            if !expanded {
                break;
            }
            pages.push(Vec::new());
            row = 0;
        }
        pages.last_mut().unwrap().push(CandidateSlot {
            index,
            x,
            row,
            width,
        });
        x += width + GAP;
    }
    pages
}
fn fit_text(text: &str, width: f32, size: f32) -> String {
    if Native::text_width(text, size) <= width {
        return text.into();
    }
    let mut end = text.len();
    while end > 0 {
        end = text[..end]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        let fitted = format!("{}…", &text[..end]);
        if Native::text_width(&fitted, size) <= width {
            return fitted;
        }
    }
    "…".into()
}
impl Ui {
    fn choice_value(&self, choice: Choice) -> &str {
        let c = &self.settings.translation;
        match choice {
            Choice::Service => &c.provider,
            Choice::Source => &c.source,
            Choice::First => &c.first,
            Choice::Second => c.second.as_deref().unwrap_or(""),
        }
    }
    fn available_choice(&self, choice: Choice, id: &str) -> bool {
        let c = &self.settings.translation;
        match choice {
            Choice::Service | Choice::Source => true,
            Choice::First => id != c.source && Some(id) != c.second.as_deref(),
            Choice::Second => id != c.first && id != c.source,
        }
    }
    pub fn pick(&mut self, index: usize) {
        let Some((choice, _)) = self.choices else {
            return;
        };
        let Some(&(id, _)) = choice.entries().get(index) else {
            return;
        };
        if !self.available_choice(choice, id) {
            return;
        }
        let c = &mut self.settings.translation;
        match choice {
            Choice::Service => c.provider = id.into(),
            Choice::First => c.first = id.into(),
            Choice::Second => c.second = (!id.is_empty()).then(|| id.to_owned()),
            Choice::Source => {
                if c.first == id {
                    c.first = c.source.clone();
                }
                if c.second.as_deref() == Some(id) {
                    c.second = Some(c.source.clone());
                }
                c.source = id.into();
            }
        }
        self.choices = None;
    }
    fn translation_controls(&mut self) {
        use crate::translation::{lang_name, service};
        if let Some((choice, page)) = self.choices {
            let entries = choice.entries();
            let last = (entries.len() - 1) / 9;
            let page = page.min(last);
            let start = page * 9;
            for (offset, &(id, name)) in entries.iter().skip(start).take(9).enumerate() {
                self.button(
                    450 + offset,
                    36. + (offset % 3) as f32 * 292.,
                    169. + (offset / 3) as f32 * 63.,
                    280.,
                    54.,
                    name,
                    "",
                    UiAction::Pick(start + offset),
                    self.choice_value(choice) == id,
                    self.available_choice(choice, id),
                    21.,
                );
            }
            for (id, x, label, action, enabled) in [
                (
                    470,
                    36.,
                    "上一页",
                    UiAction::ChoicePage(page.saturating_sub(1)),
                    page > 0,
                ),
                (471, 328., "返回", UiAction::CloseChoices, true),
                (
                    472,
                    620.,
                    "下一页",
                    UiAction::ChoicePage(page + 1),
                    page < last,
                ),
            ] {
                self.button(
                    id, x, 371., 280., 52., label, "", action, false, enabled, 20.,
                );
            }
            return;
        }
        let c = &self.settings.translation;
        let enabled = c.enabled;
        let service = service(&c.provider);
        let rows = [
            (
                320,
                36.,
                170.,
                "发送时翻译".to_owned(),
                UiAction::ToggleTranslation,
            ),
            (
                321,
                480.,
                170.,
                format!("翻译服务　{}", service.map_or("请选择", |s| s.name)),
                UiAction::Choose(Choice::Service),
            ),
            (
                322,
                36.,
                246.,
                format!("第一译文　{}", lang_name(&c.first)),
                UiAction::Choose(Choice::First),
            ),
            (
                323,
                480.,
                246.,
                format!("第二译文　{}", lang_name(c.second.as_deref().unwrap_or(""))),
                UiAction::Choose(Choice::Second),
            ),
            (
                324,
                36.,
                322.,
                format!("原文语言　{}", lang_name(&c.source)),
                UiAction::Choose(Choice::Source),
            ),
        ];
        for (id, x, y, label, action) in rows {
            self.button(
                id,
                x,
                y,
                426.,
                60.,
                &label,
                if id == 320 {
                    if enabled { "开启" } else { "关闭" }
                } else {
                    ""
                },
                action,
                id == 320 && enabled,
                true,
                21.,
            );
        }
        if service.is_some_and(|s| s.account()) {
            self.button(
                325,
                480.,
                322.,
                426.,
                60.,
                "配置服务（桌面）",
                "",
                UiAction::ConfigureService,
                false,
                true,
                21.,
            );
        }
    }
    pub fn new(settings: Settings, vr: bool, osc_enabled: bool) -> Self {
        Self {
            line: DraftLine::default(),
            key_audio: None,
            state: Snapshot {
                status: "正在加载水杉词库…".into(),
                ..Default::default()
            },
            buttons: Vec::new(),
            expanded: false,
            symbols: false,
            settings_open: false,
            settings_page: SettingsPage::General,
            choices: None,
            startup_pending: false,
            page: 0,
            settings,
            hover: [None; 3],
            pressed: [None; 3],
            pending: 0,
            vr,
            osc_enabled,
            notice: String::new(),
            runtime_status: if vr {
                "SteamVR 已连接"
            } else {
                "桌面模式 · 未连接 SteamVR"
            }
            .into(),
            copied: false,
            page_count: 1,
        }
    }
    fn button(
        &mut self,
        id: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        label: &str,
        sub: &str,
        action: UiAction,
        accent: bool,
        enabled: bool,
        font: f32,
    ) {
        self.buttons.push(Button {
            id,
            x,
            y,
            w,
            h,
            label: label.into(),
            sub: sub.into(),
            corner_hint: false,
            action,
            accent,
            enabled,
            font,
        });
    }
    fn key(
        &mut self,
        id: usize,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        sub: &str,
        action: Action,
        accent: bool,
    ) {
        self.button(
            id,
            x,
            y,
            w,
            64.,
            label,
            sub,
            UiAction::Input(action),
            accent,
            self.state.ready,
            26.,
        );
    }
    pub fn rebuild(&mut self) {
        self.line.update(&self.state);
        self.buttons.clear();
        if !self.settings_open {
            let expanded = self.expanded;
            let pages = candidate_pages(&self.state.candidates, expanded);
            self.page_count = pages.len();
            self.page = self.page.min(self.page_count - 1);
            for slot in &pages[if expanded { self.page } else { 0 }] {
                let text = self.state.candidates[slot.index].clone();
                self.button(
                    1000 + slot.index,
                    slot.x,
                    100. + slot.row as f32 * ROW_STEP,
                    slot.width,
                    if slot.row == 0 { 56. } else { 64. },
                    &text,
                    "",
                    UiAction::Input(Action::Select(slot.index, self.state.candidate_revision)),
                    slot.index == 0,
                    self.pending == 0 && self.state.ready,
                    23.,
                );
            }
            self.button(
                110,
                944.,
                100.,
                64.,
                56.,
                if self.expanded {
                    "收起候选"
                } else {
                    "展开候选"
                },
                "",
                UiAction::Expand,
                self.expanded,
                !self.state.candidates.is_empty(),
                30.,
            );
        }
        self.button(
            111,
            832.,
            23.,
            64.,
            64.,
            "复制",
            "",
            UiAction::Copy,
            false,
            self.state.ready && self.pending == 0 && !self.state.draft.is_empty(),
            19.,
        );
        self.button(
            113,
            758.,
            42.,
            52.,
            44.,
            "清空",
            "",
            UiAction::Input(Action::Clear),
            false,
            self.state.ready && (!self.state.draft.is_empty() || !self.state.preedit.is_empty()),
            20.,
        );
        let restore =
            self.state.recoverable && self.state.draft.is_empty() && self.state.preedit.is_empty();
        self.button(
            112,
            908.,
            23.,
            100.,
            64.,
            if restore { "取回上条" } else { "发送" },
            "",
            UiAction::Input(if restore {
                Action::RestoreUnsent
            } else {
                Action::Send
            }),
            true,
            (restore || self.state.can_send()) && self.pending == 0,
            if restore { 18. } else { 23. },
        );
        self.button(
            200,
            944.,
            244.,
            64.,
            64.,
            if self.symbols { "abc" } else { "123" },
            if self.symbols { "字母" } else { "符号" },
            UiAction::Symbols,
            self.symbols,
            true,
            21.,
        );
        self.button(
            201,
            944.,
            318.,
            64.,
            64.,
            "设置",
            "",
            UiAction::Settings,
            self.settings_open,
            true,
            19.,
        );
        self.button(
            202,
            944.,
            392.,
            64.,
            64.,
            "居中",
            "",
            UiAction::Center,
            false,
            self.vr,
            18.,
        );
        if self.settings_open {
            for (id, x, label, page) in [
                (313, 36., "常规", SettingsPage::General),
                (314, 186., "输入", SettingsPage::Input),
                (316, 336., "翻译", SettingsPage::Translation),
            ] {
                self.button(
                    id,
                    x,
                    110.,
                    140.,
                    44.,
                    label,
                    "",
                    UiAction::SettingsPage(page),
                    self.settings_page == page,
                    true,
                    22.,
                );
            }
            if self.settings_page == SettingsPage::Translation {
                self.translation_controls();
                return;
            }
            let general = self.settings_page == SettingsPage::General;
            let controls = if general {
                vec![
                    (
                        310,
                        "跟随 SteamVR 启动",
                        self.settings.steamvr_startup,
                        UiAction::SteamVrStartup,
                    ),
                    (
                        311,
                        "关闭时缩小到托盘区",
                        self.settings.close_to_tray,
                        UiAction::CloseToTray,
                    ),
                    (
                        312,
                        "静默启动",
                        self.settings.silent_start,
                        UiAction::SilentStart,
                    ),
                ]
            } else {
                vec![
                    (315, "按键音", self.settings.key_sound, UiAction::KeySound),
                    (308, "震动反馈", self.settings.haptics, UiAction::Haptics),
                    (
                        307,
                        "Vrchat输入状态表示",
                        self.settings.typing_indicator,
                        UiAction::TypingIndicator,
                    ),
                    (
                        304,
                        "防止游戏误触",
                        self.settings.block_input,
                        UiAction::Block,
                    ),
                    (303, "深色面板", self.settings.dark, UiAction::Dark),
                ]
            };
            for (i, (id, label, on, action)) in controls.into_iter().enumerate() {
                self.button(
                    id,
                    36.,
                    if general {
                        168. + i as f32 * 68.
                    } else {
                        164. + i as f32 * 56.
                    },
                    440.,
                    if general { 60. } else { 50. },
                    label,
                    if id == 310 && self.startup_pending {
                        "处理中"
                    } else if on {
                        "开启"
                    } else {
                        "关闭"
                    },
                    action,
                    on,
                    id != 310 || !self.startup_pending,
                    22.,
                );
            }
            self.button(
                305,
                554.,
                210.,
                120.,
                50.,
                "缩小",
                "",
                UiAction::Narrow,
                false,
                self.vr,
                22.,
            );
            self.button(
                306,
                714.,
                210.,
                120.,
                50.,
                "放大",
                "",
                UiAction::Widen,
                false,
                self.vr,
                22.,
            );
            return;
        }
        if self.expanded {
            self.button(
                120,
                16.,
                398.,
                160.,
                56.,
                "上一页",
                "",
                UiAction::Previous,
                false,
                self.page > 0,
                20.,
            );
            self.button(
                121,
                768.,
                398.,
                160.,
                56.,
                "下一页",
                "",
                UiAction::Next,
                false,
                self.page + 1 < self.page_count,
                20.,
            );
            return;
        }
        let symbols = if self.state.english {
            ["1234567890", "~!@#%\"'*?", "()-_:;/"]
        } else {
            ["1234567890", "~!@#%“”*?", "()-_:;、"]
        };
        for (row, letters) in LETTER_ROWS.iter().enumerate() {
            let inset = [0., 0.5, 1.5][row];
            for (col, (letter, symbol)) in letters.chars().zip(symbols[row].chars()).enumerate() {
                let letter_label = if self.state.english && self.state.caps {
                    letter.to_ascii_uppercase()
                } else {
                    letter
                };
                let (primary, secondary, action) = if self.symbols {
                    (symbol, letter_label, Action::Literal(symbol))
                } else {
                    (letter_label, symbol, Action::Key(letter))
                };
                self.key(
                    11 + row * 10 + col,
                    16. + (col as f32 + inset) * 92.,
                    KEY_TOP + row as f32 * ROW_STEP,
                    84.,
                    &primary.to_string(),
                    &secondary.to_string(),
                    action,
                    false,
                );
                self.buttons.last_mut().unwrap().corner_hint = true;
            }
        }
        if self.state.english && !self.symbols {
            self.key(
                40,
                16.,
                318.,
                130.,
                "Caps",
                if self.state.caps { "ABC" } else { "abc" },
                Action::Caps,
                self.state.caps,
            );
            self.buttons.last_mut().unwrap().font = 22.;
        }
        let labels = [
            if self.state.english { "EN" } else { "中 / EN" },
            "'",
            if !self.state.preedit.is_empty() {
                "空格 · 选首词"
            } else {
                "空格"
            },
            if self.state.english { "," } else { "，" },
            if self.state.english { "." } else { "。" },
            "退格",
        ];
        let actions = [
            Action::Language,
            Action::Key('\''),
            Action::Space,
            Action::Key(','),
            Action::Key('.'),
            Action::Backspace,
        ];
        let mut x = 16.;
        for (i, ((label, action), width)) in labels
            .iter()
            .zip(actions)
            .zip([108., 68., 420., 84., 84., 112.])
            .enumerate()
        {
            self.key(50 + i, x, 392., width, label, "", action, i == 0);
            if i == 2 {
                self.buttons.last_mut().unwrap().font = 20.;
            }
            x += width + GAP;
        }
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if self.pending == 0
            && !self.state.draft.is_empty()
            && (18.0..754.).contains(&x)
            && (40.0..89.).contains(&y)
        {
            return Some(DRAFT_FIELD);
        }
        self.buttons
            .iter()
            .find(|b| b.enabled && b.hit(x, y))
            .map(|b| b.id)
    }
    pub fn candidate_targeted(&self) -> bool {
        self.hover
            .iter()
            .chain(self.pressed.iter())
            .flatten()
            .any(|id| {
                self.buttons
                    .iter()
                    .any(|b| b.id == *id && matches!(b.action, UiAction::Input(Action::Select(..))))
            })
    }
    pub fn action(&self, id: usize, x: f32, y: f32) -> Option<UiAction> {
        if id == DRAFT_FIELD {
            let [left, top, width, height] = DRAFT_BOUNDS;
            let displayed =
                Native::draft_hit((x - left).clamp(0., width), (y - top).clamp(0., height))?;
            return Some(UiAction::Input(Action::MoveCaret {
                offset: self.line.draft_offset(&self.state.draft, displayed),
                sequence: self.state.sequence,
            }));
        }
        self.buttons
            .iter()
            .find(|b| b.id == id && b.enabled)
            .map(|b| b.action.clone())
    }
    pub fn render(&self, n: &Native) {
        if self.vr {
            n.key_regions(&self.buttons);
        }
        let dark = self.settings.dark;
        let bg = if dark { 0xff202a27 } else { 0xffe9eeeb };
        let key = if dark { 0xff36423d } else { 0xfff9fbf9 };
        let ink = if dark { 0xffedf4f0 } else { INK };
        let muted = if dark { 0xffb3c4ba } else { MUTED };
        let hint = if dark { 0xff9eafa4 } else { 0xff8a9a91 };
        let border = if dark { 0xff46554c } else { 0xffd8e0da };
        n.begin(bg);
        n.rect(16., 18., 804., 74., 10., key, border);
        // Only the empty keyboard needs a placeholder. Candidate chips supply
        // their own backgrounds; settings replaces this entire region.
        if !self.settings_open && self.state.candidates.is_empty() {
            n.rect(16., 100., 916., 56., 9., key, border);
        }
        let mode = if self.state.english {
            if self.state.caps { "EN · CAPS" } else { "EN" }
        } else {
            "全拼"
        };
        n.text(mode, 30., 20., 100., 22., 12., muted, 0);
        n.text(
            &format!("{} / 144", self.state.count()),
            715.,
            20.,
            90.,
            22.,
            12.,
            if self.state.count() > 144 {
                0xffbd5353
            } else {
                muted
            },
            2,
        );
        let status = if !self.notice.is_empty() {
            self.notice.as_str()
        } else if self.state.translating {
            "正在翻译，可继续输入下一句"
        } else if !self.state.status.is_empty() && self.state.status != "水杉已就绪" {
            self.state.status.as_str()
        } else {
            if self.state.recoverable {
                "上条未发送，草稿空白时可取回"
            } else {
                ""
            }
        };
        n.text(
            &fit_text(status, 565., 13.),
            134.,
            20.,
            565.,
            22.,
            13.,
            muted,
            0,
        );
        if self.line.text.is_empty() {
            let [x, y, w, h] = DRAFT_BOUNDS;
            n.text("说点什么…", x, y, w, h, 26., muted, 0);
        } else {
            n.draft(
                &self.line.text,
                self.line.preedit_start,
                self.line.preedit_len,
                DRAFT_BOUNDS,
                ink,
            );
        }
        if self.settings_open {
            n.rect(16., 100., 916., 356., 10., key, border);
            if self.settings_page == SettingsPage::General {
                n.text(
                    "静默启动时隐藏窗口和 VR 面板",
                    36.,
                    380.,
                    440.,
                    24.,
                    16.,
                    muted,
                    0,
                );
                n.text(
                    "双击右 B 键呼出 VR 面板",
                    36.,
                    410.,
                    440.,
                    24.,
                    16.,
                    muted,
                    0,
                );
            }
            if self.settings_page != SettingsPage::Translation {
                n.text("悬浮宽度", 554., 114., 300., 28., 18., muted, 0);
                n.text(
                    &format!("{:.0} cm", self.settings.width_m * 100.),
                    554.,
                    144.,
                    280.,
                    44.,
                    30.,
                    ink,
                    0,
                );
                n.text(
                    &format!("OSC  {}", self.settings.osc_address),
                    514.,
                    294.,
                    400.,
                    26.,
                    15.,
                    muted,
                    0,
                );
                n.text(
                    &fit_text(&self.runtime_status, 400., 12.),
                    514.,
                    322.,
                    400.,
                    24.,
                    12.,
                    muted,
                    0,
                );
                n.text(
                    if self.state.neural_status.is_empty() {
                        "智能联想已开启"
                    } else {
                        &self.state.neural_status
                    },
                    514.,
                    343.,
                    400.,
                    22.,
                    12.,
                    muted,
                    0,
                );
                n.text(
                    "防止游戏误触需 SteamVR 实验选项",
                    514.,
                    363.,
                    400.,
                    20.,
                    12.,
                    muted,
                    0,
                );
                n.text(
                    "Experimental overlay input overrides",
                    514.,
                    384.,
                    400.,
                    18.,
                    11.,
                    muted,
                    0,
                );
                n.text(
                    &format!(
                        "Sylva · {}{}",
                        env!("CARGO_PKG_VERSION"),
                        if self.osc_enabled {
                            ""
                        } else {
                            " · OSC 已关闭"
                        }
                    ),
                    514.,
                    429.,
                    400.,
                    18.,
                    11.,
                    muted,
                    0,
                );
            } else if let Some((choice, page)) = self.choices {
                n.text(
                    &format!(
                        "{}　{} / {}",
                        choice.title(),
                        page + 1,
                        choice.entries().len().div_ceil(9)
                    ),
                    514.,
                    114.,
                    380.,
                    30.,
                    18.,
                    muted,
                    0,
                );
            } else {
                n.text(
                    "发送时翻译，原文和译文各占一行",
                    36.,
                    414.,
                    700.,
                    28.,
                    17.,
                    muted,
                    0,
                );
            }
        }
        if self.expanded && !self.settings_open {
            n.text(
                &format!("{} / {}", self.page + 1, self.page_count),
                350.,
                398.,
                300.,
                56.,
                19.,
                muted,
                1,
            );
        }
        for b in &self.buttons {
            let hovered = !self.vr && self.hover.contains(&Some(b.id));
            let pressed = !self.vr && self.pressed.contains(&Some(b.id));
            let fill = if !b.enabled {
                if dark { 0xff29322e } else { 0xffe1e7e2 }
            } else if b.accent || pressed {
                ACCENT
            } else if hovered {
                if dark { 0xff4e6657 } else { 0xffdce9dd }
            } else {
                key
            };
            let text = if !b.enabled {
                muted
            } else if b.accent || pressed {
                0xfff4faf6
            } else {
                ink
            };
            if b.id != 113 || hovered || pressed {
                n.rect(
                    b.x,
                    b.y,
                    b.w,
                    b.h,
                    9.,
                    fill,
                    if hovered { 0xff75a187 } else { border },
                );
            }
            let icon = match b.id {
                110 => Some(if self.expanded {
                    Icon::ChevronUp
                } else {
                    Icon::ChevronDown
                }),
                111 => Some(Icon::Copy),
                113 => Some(Icon::Clear),
                55 => Some(Icon::Backspace),
                _ => None,
            };
            if let Some(icon) = icon {
                let size = match b.id {
                    113 => 24.,
                    55 => 30.,
                    _ => 26.,
                };
                let y = if b.id == 111 {
                    b.y + 8.
                } else {
                    b.y + (b.h - size) * 0.5
                };
                n.icon(icon, b.x + (b.w - size) * 0.5, y, size, text);
                if b.id == 111 {
                    n.text(
                        if self.copied { "已复制" } else { "复制" },
                        b.x + 4.,
                        b.y + 37.,
                        b.w - 8.,
                        21.,
                        12.,
                        text,
                        1,
                    );
                }
                continue;
            }
            let label = if matches!(b.action, UiAction::Input(Action::Select(..))) {
                fit_text(&b.label, b.w - 24., b.font)
            } else {
                b.label.clone()
            };
            if b.corner_hint {
                let small = if b.accent || pressed {
                    0xffc4dfce
                } else {
                    hint
                };
                n.text(&b.sub, b.x + b.w - 31., b.y + 5., 23., 18., 14., small, 2);
                n.text(&label, b.x + 7., b.y + 13., b.w - 14., 46., b.font, text, 1);
            } else if b.sub.is_empty() {
                n.text(&label, b.x + 7., b.y, b.w - 14., b.h, b.font, text, 1);
            } else if b.id >= 300 {
                n.text(&label, b.x + 20., b.y, b.w - 120., b.h, b.font, text, 0);
                n.text(&b.sub, b.x + b.w - 90., b.y, 70., b.h, 16., text, 2);
            } else {
                n.text(&label, b.x + 5., b.y + 4., b.w - 10., 34., b.font, text, 1);
                n.text(
                    &b.sub,
                    b.x + 5.,
                    b.y + 36.,
                    b.w - 10.,
                    20.,
                    12.,
                    if b.accent { 0xffc4dfce } else { hint },
                    1,
                );
            }
        }
        n.rect(
            440.,
            466.,
            144.,
            4.,
            2.,
            if dark { 0xff708777 } else { 0xffb1c1b5 },
            0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_composition_maps_hit_positions_to_committed_graphemes() {
        let state = Snapshot {
            draft: "我👨‍👩‍👧‍👦\n好".into(),
            caret: "我".len(),
            preedit: "pin".into(),
            ..Default::default()
        };
        let mut line = DraftLine::default();
        line.update(&state);
        assert_eq!(String::from_utf16(&line.text).unwrap(), "我pin👨‍👩‍👧‍👦↵好");
        assert_eq!((line.preedit_start, line.preedit_len), (1, 3));
        for pos in 1..=4 {
            assert_eq!(line.draft_offset(&state.draft, pos), state.caret);
        }
        for pos in 4..=15 {
            let offset = line.draft_offset(&state.draft, pos);
            assert!(offset == state.caret || offset == "我👨‍👩‍👧‍👦".len());
        }
        assert_eq!(line.draft_offset(&state.draft, 17), state.draft.len());
    }
    #[test]
    fn new_settings_defaults_and_saved_preferences_survive_upgrade() {
        let defaults: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults.width_m, 0.7);
        assert!(!defaults.typing_indicator);
        assert!(defaults.haptics);
        assert!(!defaults.steamvr_startup);
        assert!(defaults.close_to_tray && defaults.silent_start);
        let legacy: Settings = serde_json::from_str(r#"{"width_m":0.8,"dark":true}"#).unwrap();
        assert_eq!(legacy.width_m, 0.8);
        assert!(legacy.dark && legacy.haptics);
        assert!(!legacy.typing_indicator);
        assert!(!legacy.steamvr_startup && legacy.close_to_tray && legacy.silent_start);
        let persisted: Settings = serde_json::from_str(
            r#"{"steamvr_startup":true,"close_to_tray":false,"silent_start":false}"#,
        )
        .unwrap();
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&persisted).unwrap()).unwrap();
        assert!(restored.steamvr_startup && !restored.close_to_tray && !restored.silent_start);
        let saved: Settings =
            serde_json::from_str(r#"{"width_m":0.9,"typing_indicator":true,"haptics":false}"#)
                .unwrap();
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored.width_m, 0.9);
        assert!(restored.typing_indicator);
        assert!(!restored.haptics);
    }
    #[test]
    fn key_sound_switch_migrates_and_round_trips() {
        let defaults: Settings = serde_json::from_str("{}").unwrap();
        assert!(defaults.key_sound);
        for enabled in [false, true] {
            let saved = serde_json::json!({"sound": true, "key_sound": enabled});
            let settings: Settings = serde_json::from_value(saved).unwrap();
            assert_eq!(settings.plays_key_sound(&Action::Key('e')), enabled);
            assert_eq!(settings.plays_key_sound(&Action::Send), enabled);
            assert!(
                serde_json::to_value(settings)
                    .unwrap()
                    .get("sound")
                    .is_none()
            );
        }
        for name in ["standard", "tick", "space", "return", "wood", "off"] {
            let saved = serde_json::json!({"key_sound": name, "width_m": 0.85});
            let settings: Settings = serde_json::from_value(saved).unwrap();
            let enabled = name != "off";
            assert_eq!(settings.key_sound, enabled);
            let json = serde_json::to_value(settings).unwrap();
            assert_eq!(json["key_sound"].as_bool(), Some(enabled));
            let restored: Settings = serde_json::from_value(json).unwrap();
            assert_eq!(restored.key_sound, enabled);
            assert_eq!(restored.width_m, 0.85);
        }
    }
    #[test]
    fn legacy_prediction_switch_is_ignored_and_not_saved_again() {
        let settings: Settings =
            serde_json::from_str(r#"{"prediction":false,"dark":true}"#).unwrap();
        assert!(settings.dark);
        assert!(
            serde_json::to_value(settings)
                .unwrap()
                .get("prediction")
                .is_none()
        );
    }
    fn ready() -> Ui {
        let mut ui = Ui::new(Settings::default(), true, false);
        ui.state.ready = true;
        ui.rebuild();
        ui
    }
    fn assert_geometry(ui: &Ui) {
        for (i, a) in ui.buttons.iter().enumerate() {
            assert!(
                a.x >= 0. && a.y >= 0. && a.x + a.w <= WIDTH && a.y + a.h <= HEIGHT,
                "outside: {}",
                a.label
            );
            for b in &ui.buttons[i + 1..] {
                assert_ne!(a.id, b.id);
                assert!(
                    !(a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h),
                    "overlap: {} / {}",
                    a.label,
                    b.label
                );
            }
        }
    }
    #[test]
    fn translation_pages_fit_and_remember_distinct_languages() {
        let mut ui = ready();
        ui.settings_open = true;
        ui.settings_page = SettingsPage::Translation;
        assert!(!ui.settings.translation.enabled);
        assert_eq!(ui.settings.translation.provider, "edge");
        for choice in [
            Choice::Service,
            Choice::Source,
            Choice::First,
            Choice::Second,
        ] {
            for page in 0..choice.entries().len().div_ceil(9) {
                ui.choices = Some((choice, page));
                ui.rebuild();
                assert_geometry(&ui);
            }
        }
        ui.choices = Some((Choice::Second, 0));
        let idx = Choice::Second
            .entries()
            .iter()
            .position(|l| l.0 == "ja")
            .unwrap();
        ui.pick(idx);
        assert_eq!(ui.settings.translation.second.as_deref(), Some("ja"));
        ui.choices = Some((Choice::Source, 0));
        ui.pick(2);
        assert_eq!(ui.settings.translation.source, "ja");
        assert_eq!(ui.settings.translation.second.as_deref(), Some("zh-CN"));
        ui.rebuild();
        assert_geometry(&ui);
    }
    #[test]
    fn layers_preserve_positions_and_change_the_actual_key_actions() {
        let mut ui = ready();
        let keys: Vec<_> = ui
            .buttons
            .iter()
            .filter(|b| b.corner_hint)
            .cloned()
            .collect();
        assert_eq!(keys.len(), 26);
        assert_eq!(keys.iter().filter(|b| b.y == KEY_TOP).count(), 10);
        assert_eq!(
            keys.iter()
                .map(|b| b.sub.as_str())
                .take(10)
                .collect::<String>(),
            "1234567890"
        );
        assert_geometry(&ui);
        ui.symbols = true;
        ui.rebuild();
        for letter in &keys {
            let symbol = ui.buttons.iter().find(|b| b.id == letter.id).unwrap();
            assert_eq!(
                (symbol.x, symbol.y, symbol.w, symbol.h),
                (letter.x, letter.y, letter.w, letter.h)
            );
            assert_eq!(symbol.label, letter.sub);
            assert_eq!(symbol.sub, letter.label);
            match symbol.action {
                UiAction::Input(Action::Literal(c)) => assert_eq!(c.to_string(), symbol.label),
                _ => panic!("wrong layer action"),
            }
        }
        assert_geometry(&ui);
        ui.symbols = false;
        ui.state.english = true;
        ui.state.caps = true;
        ui.rebuild();
        assert_eq!(ui.buttons.iter().find(|b| b.id == 11).unwrap().label, "Q");
        assert!(ui.buttons.iter().any(|b| b.id == 40 && b.accent));
        assert_geometry(&ui);
        ui.settings_open = true;
        ui.rebuild();
        assert_geometry(&ui);
        assert!(
            ui.buttons
                .iter()
                .any(|b| matches!(b.action, UiAction::SilentStart) && b.accent)
        );
        assert!(
            ui.buttons
                .iter()
                .any(|b| matches!(b.action, UiAction::SteamVrStartup) && !b.accent)
        );
        ui.settings_page = crate::ui::SettingsPage::Input;
        ui.rebuild();
        assert!(
            ui.buttons
                .iter()
                .any(|b| matches!(b.action, UiAction::TypingIndicator) && !b.accent)
        );
        assert!(
            ui.buttons
                .iter()
                .any(|b| matches!(b.action, UiAction::Haptics) && b.accent)
        );
        assert_geometry(&ui);
    }
    #[test]
    fn candidate_widths_flow_without_losing_or_overlapping_choices() {
        let mut ui = ready();
        ui.state.candidates = (0..73).map(|i| "我".repeat(1 + i % 11)).collect();
        ui.rebuild();
        let candidates: Vec<_> = ui
            .buttons
            .iter()
            .filter(|b| matches!(b.action, UiAction::Input(Action::Select(..))))
            .collect();
        assert!(candidates[0].w < candidates[2].w);
        assert!(candidates.iter().all(|b| b.y == 100.));
        assert_geometry(&ui);
        ui.state.candidates.push("非常长的候选".repeat(32));
        ui.expanded = true;
        ui.rebuild();
        let mut found = Vec::new();
        for page in 0..ui.page_count {
            ui.page = page;
            ui.rebuild();
            assert_geometry(&ui);
            for b in &ui.buttons {
                if let UiAction::Input(Action::Select(i, _)) = b.action {
                    found.push(i);
                }
            }
        }
        assert_eq!(found, (0..ui.state.candidates.len()).collect::<Vec<_>>());
    }
}
