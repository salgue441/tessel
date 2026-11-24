//! The TOML config file: schema, defaults, validation and window rules.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::anim::Curve;
use crate::command::Command;
use crate::keys;

/// Written to disk on first run. A test keeps it in sync with `Config::default()`.
pub const DEFAULT_CONFIG: &str = include_str!("../config/default.toml");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub decoration: Decoration,
    pub animations: Animations,
    pub rules: Vec<Rule>,
    pub binds: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    pub gaps_in: i32,
    pub gaps_out: i32,
    pub split_ratio: f32,
    pub workspaces: usize,
    pub focus_follows_mouse: bool,
    pub cursor_follows_focus: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Decoration {
    pub active_border: String,
    pub inactive_border: String,
    pub rounding: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Animations {
    pub enabled: bool,
    pub duration_ms: u64,
    pub curve: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rule {
    pub exe: Option<String>,
    pub class: Option<String>,
    pub title: Option<String>,
    pub float: bool,
    pub ignore: bool,
    pub workspace: Option<usize>,
}

pub struct WindowInfo<'a> {
    pub exe: &'a str,
    pub class: &'a str,
    pub title: &'a str,
}

/// The combined effect of every rule that matches a window.
#[derive(Debug, Default, PartialEq)]
pub struct RuleOutcome {
    pub float: bool,
    pub ignore: bool,
    /// One-based workspace number.
    pub workspace: Option<usize>,
}

pub struct Binding {
    pub keys: String,
    pub mods: u32,
    pub vk: u32,
    pub command: Command,
}

const DEFAULT_BINDS: &[(&str, &str)] = &[
    ("alt+h", "focus left"),
    ("alt+j", "focus down"),
    ("alt+k", "focus up"),
    ("alt+l", "focus right"),
    ("alt+shift+h", "move left"),
    ("alt+shift+j", "move down"),
    ("alt+shift+k", "move up"),
    ("alt+shift+l", "move right"),
    ("alt+1", "workspace 1"),
    ("alt+2", "workspace 2"),
    ("alt+3", "workspace 3"),
    ("alt+4", "workspace 4"),
    ("alt+5", "workspace 5"),
    ("alt+6", "workspace 6"),
    ("alt+7", "workspace 7"),
    ("alt+8", "workspace 8"),
    ("alt+9", "workspace 9"),
    ("alt+shift+1", "move-to-workspace 1"),
    ("alt+shift+2", "move-to-workspace 2"),
    ("alt+shift+3", "move-to-workspace 3"),
    ("alt+shift+4", "move-to-workspace 4"),
    ("alt+shift+5", "move-to-workspace 5"),
    ("alt+shift+6", "move-to-workspace 6"),
    ("alt+shift+7", "move-to-workspace 7"),
    ("alt+shift+8", "move-to-workspace 8"),
    ("alt+shift+9", "move-to-workspace 9"),
    ("alt+shift+space", "toggle-float"),
    ("alt+shift+f", "toggle-fullscreen"),
    ("alt+shift+m", "toggle-maximize"),
    ("alt+shift+enter", "promote"),
    ("alt+equal", "resize +0.05"),
    ("alt+minus", "resize -0.05"),
    ("alt+shift+q", "close"),
    ("alt+enter", "exec wt"),
    ("alt+shift+r", "reload"),
    ("alt+shift+p", "toggle-pause"),
    ("alt+shift+e", "quit"),
];

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            decoration: Decoration::default(),
            animations: Animations::default(),
            rules: vec![
                Rule { class: Some("#32770".into()), float: true, ..Default::default() },
                Rule { title: Some("Picture-in-Picture".into()), float: true, ..Default::default() },
                Rule { exe: Some("Taskmgr".into()), float: true, ..Default::default() },
            ],
            binds: DEFAULT_BINDS.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }
}

impl Default for General {
    fn default() -> Self {
        Self {
            gaps_in: 5,
            gaps_out: 20,
            split_ratio: 0.5,
            workspaces: 9,
            focus_follows_mouse: false,
            cursor_follows_focus: false,
        }
    }
}

impl Default for Decoration {
    fn default() -> Self {
        Self { active_border: "#33ccff".into(), inactive_border: "#595959".into(), rounding: "round".into() }
    }
}

impl Default for Animations {
    fn default() -> Self {
        Self { enabled: true, duration_ms: 180, curve: "overshoot".into() }
    }
}

impl Rule {
    pub fn matches(&self, info: &WindowInfo) -> bool {
        let exact = |pat: &Option<String>, val: &str| pat.as_ref().is_none_or(|p| p.eq_ignore_ascii_case(val));
        let substring = |pat: &Option<String>, val: &str| {
            pat.as_ref().is_none_or(|p| val.to_lowercase().contains(&p.to_lowercase()))
        };
        exact(&self.exe, info.exe) && exact(&self.class, info.class) && substring(&self.title, info.title)
    }
}

/// DWM colour values for "default" and "none" borders.
pub const BORDER_DEFAULT: u32 = 0xFFFF_FFFF;
pub const BORDER_NONE: u32 = 0xFFFF_FFFE;

/// Parses "#rrggbb" into a COLORREF (0x00bbggrr).
pub fn parse_color(s: &str) -> Result<u32, String> {
    match s {
        "default" => return Ok(BORDER_DEFAULT),
        "none" => return Ok(BORDER_NONE),
        _ => {}
    }
    let hex = s.strip_prefix('#').filter(|h| h.len() == 6);
    let rgb = hex.and_then(|h| u32::from_str_radix(h, 16).ok()).ok_or_else(|| format!("invalid colour `{s}`"))?;
    let (r, g, b) = (rgb >> 16 & 0xFF, rgb >> 8 & 0xFF, rgb & 0xFF);
    Ok(b << 16 | g << 8 | r)
}

/// Maps a rounding name to a DWM_WINDOW_CORNER_PREFERENCE value.
fn parse_rounding(s: &str) -> Result<i32, String> {
    match s {
        "default" => Ok(0),
        "square" => Ok(1),
        "round" => Ok(2),
        "small" => Ok(3),
        _ => Err(format!("invalid rounding `{s}` (expected round, small, square or default)")),
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        let g = &self.general;
        if g.gaps_in < 0 || g.gaps_out < 0 {
            errors.push("gaps must not be negative".to_string());
        }
        if !(0.1..=0.9).contains(&g.split_ratio) {
            errors.push(format!("split_ratio must be between 0.1 and 0.9, got {}", g.split_ratio));
        }
        if !(1..=20).contains(&g.workspaces) {
            errors.push(format!("workspaces must be between 1 and 20, got {}", g.workspaces));
        }
        for c in [&self.decoration.active_border, &self.decoration.inactive_border] {
            if let Err(e) = parse_color(c) {
                errors.push(e);
            }
        }
        if let Err(e) = parse_rounding(&self.decoration.rounding) {
            errors.push(e);
        }
        if Curve::parse(&self.animations.curve).is_none() {
            errors.push(format!("invalid animation curve `{}`", self.animations.curve));
        }
        for (i, r) in self.rules.iter().enumerate() {
            if r.exe.is_none() && r.class.is_none() && r.title.is_none() {
                errors.push(format!("rule {} must set exe, class or title", i + 1));
            }
            if r.workspace.is_some_and(|w| w == 0 || w > g.workspaces) {
                errors.push(format!("rule {} has a workspace outside 1-{}", i + 1, g.workspaces));
            }
        }
        if let Err(e) = self.bindings() {
            errors.push(e);
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
    }

    pub fn bindings(&self) -> Result<Vec<Binding>, String> {
        let mut errors = Vec::new();
        let mut out = Vec::new();
        for (keys, cmd) in &self.binds {
            match (keys::parse(keys), Command::parse(cmd)) {
                (Ok((mods, vk)), Ok(command)) => out.push(Binding { keys: keys.clone(), mods, vk, command }),
                (Err(e), _) | (_, Err(e)) => errors.push(format!("bind `{keys}`: {e}")),
            }
        }
        if errors.is_empty() { Ok(out) } else { Err(errors.join("\n")) }
    }

    pub fn match_rules(&self, info: &WindowInfo) -> RuleOutcome {
        let mut out = RuleOutcome::default();
        for r in self.rules.iter().filter(|r| r.matches(info)) {
            out.float |= r.float;
            out.ignore |= r.ignore;
            out.workspace = r.workspace.or(out.workspace);
        }
        out
    }

    pub fn active_border(&self) -> u32 {
        parse_color(&self.decoration.active_border).unwrap_or(BORDER_DEFAULT)
    }

    pub fn inactive_border(&self) -> u32 {
        parse_color(&self.decoration.inactive_border).unwrap_or(BORDER_DEFAULT)
    }

    pub fn corner_preference(&self) -> i32 {
        parse_rounding(&self.decoration.rounding).unwrap_or(0)
    }

    pub fn curve(&self) -> Curve {
        Curve::parse(&self.animations.curve).unwrap_or(Curve::EaseOut)
    }
}

pub fn dir() -> PathBuf {
    std::env::var_os("APPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from).join("tessel")
}

pub fn path() -> PathBuf {
    dir().join("tessel.toml")
}

pub fn ensure_exists(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, DEFAULT_CONFIG)?;
    }
    Ok(())
}

pub fn parse(text: &str) -> Result<Config, String> {
    let config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
    config.validate()?;
    Ok(config)
}

pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_matches_default_struct() {
        assert_eq!(parse(DEFAULT_CONFIG).unwrap(), Config::default());
    }

    #[test]
    fn missing_sections_use_defaults() {
        let c = parse("[general]\ngaps_in = 2").unwrap();
        assert_eq!(c.general.gaps_in, 2);
        assert_eq!(c.general.gaps_out, 20);
        assert_eq!(c.binds, Config::default().binds);
    }

    #[test]
    fn reports_errors() {
        assert!(parse("[general]\ngapz = 2").unwrap_err().contains("gapz"));
        assert!(parse("[decoration]\nactive_border = \"blue\"").unwrap_err().contains("blue"));
        assert!(parse("[binds]\n\"alt+h\" = \"fly\"").unwrap_err().contains("fly"));
        assert!(parse("[[rules]]\nfloat = true").unwrap_err().contains("rule 1"));
    }

    #[test]
    fn colors_are_bgr() {
        assert_eq!(parse_color("#33ccff"), Ok(0x00ffcc33));
        assert_eq!(parse_color("none"), Ok(BORDER_NONE));
        assert!(parse_color("#12345").is_err());
    }

    #[test]
    fn rules_match_and_combine() {
        let c =
            parse("[[rules]]\nexe = \"Code\"\nworkspace = 2\n[[rules]]\ntitle = \"settings\"\nfloat = true").unwrap();
        let info = WindowInfo { exe: "code", class: "Chrome_WidgetWin_1", title: "Settings - VS Code" };
        assert_eq!(c.match_rules(&info), RuleOutcome { float: true, ignore: false, workspace: Some(2) });
        let other = WindowInfo { exe: "vscode", class: "x", title: "main.rs" };
        assert_eq!(c.match_rules(&other), RuleOutcome::default());
    }
}
