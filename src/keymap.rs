use anyhow::{Result, bail, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Tasks,
    Review,
    Project,
    Knowledge,
    NextPanel,
    PreviousPanel,
    Up,
    Down,
    PreviousItem,
    NextItem,
    PreviousNode,
    NextNode,
    Open,
    Explain,
    Fullscreen,
    Help,
    Palette,
    Search,
    Theme,
    MarkRead,
    ApproveNode,
    Cancel,
    Pause,
    Reply,
    Quit,
}

pub const ACTIONS: &[(Action, &str, &[&str])] = &[
    (Action::Tasks, "任务视图", &["1"]),
    (Action::Review, "审核视图", &["2"]),
    (Action::Project, "项目视图", &["3"]),
    (Action::Knowledge, "知识视图", &["4"]),
    (Action::NextPanel, "下一面板", &["tab"]),
    (Action::PreviousPanel, "上一面板", &["shift+tab"]),
    (Action::Up, "向上", &["up", "k"]),
    (Action::Down, "向下", &["down", "j"]),
    (Action::PreviousItem, "上一阅读项", &["["]),
    (Action::NextItem, "下一阅读项", &["]"]),
    (Action::PreviousNode, "上一节点", &["{"]),
    (Action::NextNode, "下一节点", &["}"]),
    (Action::Open, "打开 / 切换内容页", &["enter"]),
    (Action::Explain, "显示 / 隐藏解释", &["e"]),
    (Action::Fullscreen, "内容全屏", &["f"]),
    (Action::Help, "快捷键帮助", &["?"]),
    (Action::Palette, "命令输入", &[":"]),
    (Action::Search, "搜索内容", &["/"]),
    (Action::Theme, "切换深浅主题", &["t"]),
    (Action::MarkRead, "标记已读（不代表批准）", &["m"]),
    (Action::ApproveNode, "批准当前节点版本（需确认）", &["a"]),
    (Action::Pause, "暂停 / 继续编码", &["p"]),
    (Action::Reply, "纠正 / 追问 AI", &["i"]),
    (Action::Cancel, "取消编码", &["ctrl+x"]),
    (Action::Quit, "退出", &["q", "ctrl+c"]),
];

#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<(KeyCode, KeyModifiers), Action>,
    pub help: Vec<(Action, String, String)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::from_overrides(BTreeMap::new()).expect("valid defaults")
    }
}
impl Keymap {
    pub fn load(path: &Path) -> Result<Self> {
        let config: BTreeMap<String, Vec<String>> =
            serde_json::from_str(&fs::read_to_string(path)?)?;
        Self::from_overrides(config)
    }

    pub fn from_overrides(mut config: BTreeMap<String, Vec<String>>) -> Result<Self> {
        let mut bindings = HashMap::new();
        let mut help = Vec::new();
        for (action, label, defaults) in ACTIONS {
            let id = serde_json::to_value(action)?.as_str().unwrap().to_owned();
            let keys = config
                .remove(&id)
                .unwrap_or_else(|| defaults.iter().map(|s| s.to_string()).collect());
            for key in &keys {
                let parsed = parse_key(key)?;
                ensure!(
                    bindings.insert(parsed, *action).is_none(),
                    "快捷键冲突：{key}"
                );
            }
            help.push((*action, keys.join(" / "), (*label).into()));
        }
        ensure!(
            config.is_empty(),
            "未知动作：{}",
            config.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        Ok(Self { bindings, help })
    }

    pub fn resolve(&self, event: KeyEvent) -> Option<Action> {
        // Printable punctuation is often reported with SHIFT by enhanced protocols.
        let mut modifiers = event.modifiers;
        if matches!(event.code, KeyCode::Char(_))
            && !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            modifiers.remove(KeyModifiers::SHIFT);
        }
        self.bindings.get(&(event.code, modifiers)).copied()
    }

    pub fn hint(&self, action: Action) -> &str {
        self.help
            .iter()
            .find(|h| h.0 == action)
            .map(|h| h.1.as_str())
            .unwrap_or("")
    }
}

fn parse_key(value: &str) -> Result<(KeyCode, KeyModifiers)> {
    let mut modifiers = KeyModifiers::empty();
    let parts: Vec<_> = value.split('+').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        match *part {
            "ctrl" => modifiers |= KeyModifiers::CONTROL,
            "alt" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            _ => bail!("未知修饰键：{part}"),
        }
    }
    let name = parts.last().copied().unwrap_or("");
    let code = match name {
        "tab" if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        "tab" => KeyCode::Tab,
        "enter" => KeyCode::Enter,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "esc" => KeyCode::Esc,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        _ if name.chars().count() == 1 => KeyCode::Char(name.chars().next().unwrap()),
        _ => bail!("不支持的快捷键：{value}"),
    };
    Ok((code, modifiers))
}
