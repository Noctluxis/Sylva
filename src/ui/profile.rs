// SPDX-License-Identifier: MPL-2.0
use super::{Button, Native, Ui, UiAction};
use crate::translation::{self, Account, Field, Service};
use std::{sync::mpsc, thread};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const VALUE_BUTTON: usize = 620;
const VALUE: [f32; 4] = [38., 64., 656., 41.];
#[derive(Clone, Debug)]
pub enum ProfileAction {
    Focus(Field),
    Back,
    Save,
    Done,
    Next(bool),
    Paste,
    PasteKey,
    SelectAll,
    Caps,
    Symbols,
    Type(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Caret(u32),
}
pub(super) struct Profile {
    service: &'static Service,
    account: Account,
    edit: Option<Edit>,
    job: Option<mpsc::Receiver<Result<Completion, String>>>,
    loaded: bool,
    error: String,
}
enum Completion {
    Loaded(Account),
    Saved,
}
struct Edit {
    field: Field,
    text: String,
    cursor: usize,
    selected: bool,
    caps: bool,
    symbols: bool,
}
impl Edit {
    fn new(field: Field, text: &str) -> Self {
        Self {
            field,
            text: text.into(),
            cursor: text.len(),
            selected: !text.is_empty(),
            caps: false,
            symbols: false,
        }
    }
    fn left(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn right(&self) -> usize {
        self.cursor
            + self.text[self.cursor..]
                .graphemes(true)
                .next()
                .map_or(0, str::len)
    }
    fn set_cursor(&mut self, offset: usize) {
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|&i| i >= offset)
            .unwrap_or(self.text.len());
        self.selected = false;
    }
    fn replace(&mut self, text: &str) -> Result<(), String> {
        if text.chars().any(char::is_control) {
            return Err("该字段只接受单行文字".into());
        }
        let range = if self.selected {
            0..self.text.len()
        } else {
            self.cursor..self.cursor
        };
        if self.text.len() - range.len() + text.len() > 2560 {
            return Err("字段过长，请检查粘贴内容".into());
        }
        let end = range.start + text.len();
        self.text.replace_range(range, text);
        self.set_cursor(end);
        Ok(())
    }
    fn erase(&mut self, back: bool) {
        let range = if self.selected {
            0..self.text.len()
        } else if back {
            self.left()..self.cursor
        } else {
            self.cursor..self.right()
        };
        let pos = range.start;
        self.text.replace_range(range, "");
        self.set_cursor(pos);
    }
    fn display(&self) -> (Vec<u16>, u32) {
        if self.field == Field::Key {
            let n = self.text.graphemes(true).count();
            return (
                vec!['•' as u16; n],
                self.text[..self.cursor].graphemes(true).count() as u32,
            );
        }
        (
            self.text.encode_utf16().collect(),
            self.text[..self.cursor].encode_utf16().count() as u32,
        )
    }
    fn hit(&mut self, offset: u32) {
        let mut units = 0;
        let pos = self
            .text
            .grapheme_indices(true)
            .find_map(|(i, g)| {
                let len = if self.field == Field::Key {
                    1
                } else {
                    g.encode_utf16().count() as u32
                };
                let start = units;
                units += len;
                (offset <= units).then(|| {
                    if (offset - start) * 2 < len {
                        i
                    } else {
                        i + g.len()
                    }
                })
            })
            .unwrap_or(self.text.len());
        self.set_cursor(pos);
    }
}
impl Profile {
    fn new(service: &'static Service, account: Account) -> Self {
        Self {
            service,
            account,
            edit: None,
            job: None,
            loaded: true,
            error: String::new(),
        }
    }
    fn run(&mut self, op: impl FnOnce() -> Result<Completion, String> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name("sylva-credentials".into())
            .spawn(move || {
                let _ = tx.send(op());
            }) {
            Ok(_) => {
                self.error.clear();
                self.job = Some(rx);
            }
            Err(_) => self.error = "无法访问 Windows 凭据管理器".into(),
        }
    }
    fn finish_field(&mut self) {
        if let Some(e) = self.edit.take() {
            e.field.set(&mut self.account, e.text.trim().to_owned());
        }
    }
    fn label(&self, field: Field) -> &'static str {
        self.service
            .fields()
            .iter()
            .find(|f| f.0 == field)
            .map_or("", |f| f.1)
    }
}
impl Ui {
    pub fn open_profile(&mut self) {
        if let Some(s) =
            translation::service(&self.settings.translation.provider).filter(|s| s.account())
        {
            let mut p = Profile::new(s, Account::default());
            p.loaded = false;
            p.run(move || translation::load_account(s).map(Completion::Loaded));
            self.profile = Some(p);
            self.notice.clear();
            self.hover = [None; 3];
            self.pressed = [None; 3];
        }
    }
    pub fn in_profile(&self) -> bool {
        self.profile.is_some()
    }
    pub fn finish_profile_save(&mut self) {
        if let Some(p) = &mut self.profile
            && p.loaded
            && let Some(rx) = p.job.take()
        {
            let _ = rx.recv();
        }
    }
    pub fn poll_profile(&mut self) -> bool {
        let Some(p) = self.profile.as_mut() else {
            return false;
        };
        let Some(rx) = &p.job else { return false };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(_) => Err("凭据操作未完成".into()),
        };
        p.job = None;
        match result {
            Ok(Completion::Saved) => {
                self.notice = format!("已保存 {} 配置", p.service.name);
                self.profile = None;
            }
            Ok(Completion::Loaded(a)) => {
                p.account = a;
                p.loaded = true;
            }
            Err(e) => p.error = e,
        }
        self.rebuild();
        true
    }
    pub fn profile_action(&mut self, action: ProfileAction) {
        use ProfileAction::*;
        let Some(p) = &mut self.profile else { return };
        if matches!(action, Back) {
            if p.job.is_some() && p.loaded {
                return;
            }
            if p.edit.take().is_none() {
                self.profile = None;
            } else {
                p.error.clear();
            }
            return;
        }
        if p.job.is_some() || !p.loaded {
            return;
        }
        p.error.clear();
        match action {
            Focus(f) if p.edit.is_none() && !p.label(f).is_empty() => {
                p.edit = Some(Edit::new(f, f.get(&p.account)))
            }
            Save if p.edit.is_none() => {
                let s = p.service;
                let a = p.account.clone();
                p.run(move || translation::save_account(s, &a).map(|()| Completion::Saved));
            }
            PasteKey if p.edit.is_none() => {
                let mut input = Edit::new(Field::Key, &p.account.key);
                match Native::clipboard_text().and_then(|s| input.replace(s.trim())) {
                    Ok(()) => p.account.key = input.text,
                    Err(e) => p.error = e,
                }
            }
            Done => p.finish_field(),
            Next(back) => {
                let fields = p.service.fields();
                let next = if let Some(ed) = &p.edit {
                    let i = fields.iter().position(|f| f.0 == ed.field).unwrap();
                    (i + if back { fields.len() - 1 } else { 1 }) % fields.len()
                } else {
                    0
                };
                p.finish_field();
                let f = fields[next].0;
                p.edit = Some(Edit::new(f, f.get(&p.account)));
            }
            _ => {
                let Some(e) = &mut p.edit else { return };
                match action {
                    Paste => match Native::clipboard_text()
                        .and_then(|s| e.replace(s.trim_end_matches(['\r', '\n'])))
                    {
                        Ok(()) => {}
                        Err(err) => p.error = err,
                    },
                    SelectAll => e.selected = !e.text.is_empty(),
                    Caps => e.caps = !e.caps,
                    Symbols => e.symbols = !e.symbols,
                    Type(c) => {
                        if let Err(err) = e.replace(c.encode_utf8(&mut [0; 4])) {
                            p.error = err;
                        }
                    }
                    Backspace => e.erase(true),
                    Delete => e.erase(false),
                    Left => e.set_cursor(if e.selected { 0 } else { e.left() }),
                    Right => e.set_cursor(if e.selected { e.text.len() } else { e.right() }),
                    Home => e.set_cursor(0),
                    End => e.set_cursor(e.text.len()),
                    Caret(offset) => e.hit(offset),
                    _ => {}
                }
            }
        }
    }
    pub fn profile_keyboard(&mut self, code: u32, special: bool) {
        use ProfileAction::*;
        let action = if special {
            match code {
                1 => SelectAll,
                22 => Paste,
                9 => Next(false),
                0x10009 => Next(true),
                35 => End,
                36 => Home,
                37 => Left,
                39 => Right,
                46 => Delete,
                _ => return,
            }
        } else {
            match code {
                8 => Backspace,
                13 => Done,
                27 => Back,
                0x110000 => return,
                c => match char::from_u32(c) {
                    Some(c) if !c.is_control() => Type(c),
                    _ => return,
                },
            }
        };
        self.profile_action(action);
    }
    fn profile_key(
        &mut self,
        id: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        label: &str,
        action: ProfileAction,
        accent: bool,
        enabled: bool,
    ) {
        self.button(
            id,
            x,
            y,
            w,
            h,
            label,
            "",
            UiAction::Profile(action),
            accent,
            enabled,
            20.,
        );
    }
    pub(super) fn profile_controls(&mut self) {
        use ProfileAction::*;
        let p = self.profile.as_ref().unwrap();
        if let Some(e) = &p.edit {
            let caps = e.caps;
            let symbols = e.symbols;
            self.profile_key(619, 862., 12., 138., 37., "取消编辑", Back, false, true);
            self.profile_key(VALUE_BUTTON, 22., 55., 688., 60., "", Caret(0), false, true);
            self.profile_key(621, 720., 55., 124., 60., "粘贴", Paste, false, true);
            self.profile_key(622, 854., 55., 146., 60., "完成", Done, true, true);
            let rows = if symbols {
                ["!@#$%^&*()", "[]{}<>/\\|~", "-_=+:;\"'?", ".,`"]
            } else {
                ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"]
            };
            for (row, keys) in rows.iter().enumerate() {
                let left = (1024. - (keys.chars().count() as f32 * 96. - 8.)) / 2.;
                for (col, c) in keys.chars().enumerate() {
                    let k = if caps { c.to_ascii_uppercase() } else { c };
                    self.profile_key(
                        640 + row * 10 + col,
                        left + col as f32 * 96.,
                        129. + row as f32 * 63.,
                        88.,
                        54.,
                        &k.to_string(),
                        Type(k),
                        false,
                        true,
                    );
                }
            }
            self.profile_key(690, 36., 318., 119., 54., "Caps", Caps, caps, !symbols);
            self.profile_key(691, 869., 318., 119., 54., "退格", Backspace, false, true);
            let mut x = 34.;
            for (i, (label, w, act, on)) in [
                (if symbols { "ABC" } else { "符号" }, 115., Symbols, symbols),
                ("全选", 100., SelectAll, false),
                ("←", 74., Left, false),
                ("→", 74., Right, false),
                ("空格", 223., Type(' '), false),
                (".", 80., Type('.'), false),
                ("-", 80., Type('-'), false),
                ("_", 80., Type('_'), false),
                ("/", 68., Type('/'), false),
            ]
            .into_iter()
            .enumerate()
            {
                self.profile_key(702 + i, x, 389., w, 60., label, act, on, true);
                x += w + 8.;
            }
        } else {
            let fields = p.service.fields();
            let enabled = p.loaded && p.job.is_none();
            let cancel = p.job.is_none() || !p.loaded;
            self.profile_key(610, 706., 28., 136., 58., "取消", Back, false, cancel);
            self.profile_key(611, 854., 28., 146., 58., "保存", Save, true, enabled);
            for (idx, (f, _, _)) in fields.iter().enumerate() {
                let y = 131. + idx as f32 * 85.;
                self.profile_key(
                    612 + idx,
                    259.,
                    y,
                    if *f == Field::Key { 574. } else { 710. },
                    64.,
                    "",
                    Focus(*f),
                    false,
                    enabled,
                );
                if *f == Field::Key {
                    self.profile_key(617, 844., y, 125., 64., "粘贴", PasteKey, false, enabled);
                }
            }
        }
    }
    pub(super) fn profile_caret(&self, x: f32, y: f32) -> Option<UiAction> {
        self.profile.as_ref()?.edit.as_ref()?;
        let [left, top, width, height] = VALUE;
        let offset = Native::draft_hit((x - left).clamp(0., width), (y - top).clamp(0., height))
            .unwrap_or(0);
        Some(UiAction::Profile(ProfileAction::Caret(offset)))
    }
    pub(super) fn profile_header(&self, n: &Native, ink: u32, muted: u32, key: u32, border: u32) {
        let p = self.profile.as_ref().unwrap();
        if let Some(e) = &p.edit {
            n.text(
                &format!("{}  /  {}", p.service.name, p.label(e.field)),
                30.,
                12.,
                550.,
                37.,
                22.,
                ink,
                0,
            );
            let message = if !p.error.is_empty() {
                p.error.as_str()
            } else if e.selected {
                "已全选"
            } else {
                ""
            };
            n.text(message, 475., 14., 370., 35., 16., muted, 2);
        } else {
            n.text(
                p.service.group().label(),
                35.,
                12.,
                600.,
                28.,
                16.,
                muted,
                0,
            );
            n.text(p.service.name, 34., 38., 652., 52., 31., ink, 0);
            let fields = p.service.fields();
            let height = fields.len() as f32 * 85. + 24.;
            n.rect(18., 108., 988., height, 12., key, border);
            for (i, (_, label, _)) in fields.iter().enumerate() {
                n.text(label, 45., 131. + i as f32 * 85., 210., 64., 22., ink, 0);
            }
            let info = if !p.error.is_empty() {
                &p.error
            } else if p.job.is_some() {
                if p.loaded {
                    "正在保存…"
                } else {
                    "正在读取配置…"
                }
            } else {
                "凭据保存在当前 Windows 账户中"
            };
            n.text(info, 35., 130. + height, 940., 37., 17., muted, 0);
        }
    }
    pub(super) fn profile_button_text(
        &self,
        n: &Native,
        b: &Button,
        text: u32,
        muted: u32,
    ) -> bool {
        let Some(p) = &self.profile else { return false };
        if let UiAction::Profile(ProfileAction::Focus(field)) = b.action {
            let raw = field.get(&p.account);
            let hint = (field == Field::Model)
                .then(|| p.service.model_hint())
                .flatten()
                .unwrap_or_else(|| {
                    p.service
                        .fields()
                        .iter()
                        .find(|f| f.0 == field)
                        .map_or("", |f| f.2)
                });
            let display = if raw.is_empty() {
                hint.to_owned()
            } else if field == Field::Key {
                "••••••••".into()
            } else {
                let mut chars = raw.chars();
                let mut start: String = chars.by_ref().take(56).collect();
                if chars.next().is_some() {
                    start.push('…');
                }
                start
            };
            n.text(
                &display,
                b.x + 20.,
                b.y,
                b.w - 65.,
                b.h,
                20.,
                if raw.is_empty() { muted } else { text },
                0,
            );
            n.text("›", b.x + b.w - 45., b.y, 30., b.h, 25., muted, 1);
            return true;
        }
        if b.id == VALUE_BUTTON
            && let Some(e) = &p.edit
        {
            let (display, caret) = e.display();
            if e.selected {
                n.rect(
                    b.x + 5.,
                    b.y + 5.,
                    b.w - 10.,
                    b.h - 10.,
                    6.,
                    if self.settings.dark {
                        0xff455f55
                    } else {
                        0xffdceadf
                    },
                    0,
                );
            }
            n.draft(&display, caret, 0, VALUE, text);
            return true;
        }
        false
    }
}

impl Ui {
    // Used by the offscreen panel preview. Never read the user's saved credentials.
    pub fn preview_profile(&mut self) {
        let service = translation::service("openai").unwrap();
        self.profile = Some(Profile::new(
            service,
            Account {
                base: "https://api.openai.com/v1".into(),
                ..Default::default()
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Choice, Settings, SettingsPage, tests::assert_geometry};
    use super::*;
    use crate::translation::{Group, SERVICES};
    #[test]
    fn each_service_appears_once_and_group_browsing_preserves_the_selection() {
        let mut ui = Ui::new(Settings::default(), true, false);
        ui.settings_open = true;
        ui.settings_page = SettingsPage::Translation;
        let mut ids = std::collections::HashSet::new();
        for group in Group::ALL {
            ui.choices = Some((Choice::Service(group), 0));
            ui.rebuild();
            assert_geometry(&ui);
            for b in &ui.buttons {
                if let UiAction::PickService(id) = b.action {
                    assert_eq!(translation::service(id).unwrap().group(), group);
                    assert!(ids.insert(id));
                }
            }
        }
        assert_eq!(ids.len(), SERVICES.len());
        assert_eq!(ui.settings.translation.provider, "edge");
    }
    #[test]
    fn every_form_and_keyboard_fit_and_mask_key_input() {
        let mut ui = Ui::new(Settings::default(), true, false);
        for s in SERVICES.iter().filter(|s| s.account()) {
            ui.profile = Some(Profile::new(
                s,
                Account {
                    key: "private-fixture".into(),
                    ..Default::default()
                },
            ));
            ui.rebuild();
            assert_geometry(&ui);
            for &(f, _, _) in s.fields() {
                ui.profile_action(ProfileAction::Focus(f));
                ui.rebuild();
                assert_geometry(&ui);
                if f == Field::Key {
                    let e = ui.profile.as_ref().unwrap().edit.as_ref().unwrap();
                    assert!(e.display().0.iter().all(|&c| c == '•' as u16));
                    assert!(e.selected);
                }
                ui.profile_action(ProfileAction::Symbols);
                ui.rebuild();
                assert_geometry(&ui);
                assert!(
                    ui.buttons
                        .iter()
                        .all(|b| !matches!(b.action, UiAction::Input(_))
                            && !b.label.contains("private-fixture"))
                );
                ui.profile_action(ProfileAction::Back);
            }
        }
    }
    #[test]
    fn draft_and_secret_survive_field_navigation_and_cancel() {
        let mut ui = Ui::new(Settings::default(), false, false);
        ui.state.draft = "聊天草稿".into();
        ui.state.caret = ui.state.draft.len();
        ui.state.preedit = "pinyin".into();
        let s = translation::service("openai").unwrap();
        ui.profile = Some(Profile::new(
            s,
            Account {
                key: "untouched".into(),
                ..Default::default()
            },
        ));
        ui.profile_action(ProfileAction::Focus(Field::Key));
        ui.profile_keyboard('z' as u32, false);
        ui.profile_keyboard(27, false);
        assert_eq!(ui.profile.as_ref().unwrap().account.key, "untouched");
        ui.profile_action(ProfileAction::Focus(Field::Model));
        for c in "abc".chars() {
            ui.profile_keyboard(c as u32, false);
        }
        ui.profile_keyboard(37, true);
        ui.profile_keyboard('X' as u32, false);
        ui.profile_keyboard(13, false);
        assert_eq!(ui.profile.as_ref().unwrap().account.model, "abXc");
        ui.profile_keyboard(27, false);
        assert!(!ui.in_profile());
        assert_eq!(ui.state.draft, "聊天草稿");
        assert_eq!(ui.state.preedit, "pinyin");
    }
    #[test]
    fn graphemes_and_long_or_multiline_paste_are_handled_atomically() {
        let mut e = Edit::new(Field::Model, "old");
        e.replace("a👨‍👩‍👧b").unwrap();
        e.set_cursor(e.left());
        e.erase(true);
        assert_eq!(e.text, "ab");
        e.selected = true;
        assert!(e.replace("bad\nvalue").is_err());
        assert!(e.replace(&"x".repeat(2561)).is_err());
        assert_eq!(e.text, "ab");
        e.replace("new").unwrap();
        assert_eq!(e.text, "new");
    }
}
