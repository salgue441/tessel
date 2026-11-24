//! Commands that hotkeys, the tray menu and the CLI dispatch to the window manager.

use crate::layout::Direction;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Focus(Direction),
    Move(Direction),
    Workspace(WsTarget),
    MoveToWorkspace(WsTarget),
    ToggleFloat,
    ToggleFullscreen,
    ToggleMaximize,
    Close,
    Promote,
    Resize(f32),
    Retile,
    Reload,
    TogglePause,
    Quit,
    Exec(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsTarget {
    /// Zero-based workspace index.
    Index(usize),
    Next,
    Prev,
}

pub const HELP: &str = "\
  focus <left|right|up|down>          focus the neighbouring window (or monitor)
  move <left|right|up|down>           swap with the neighbour, or move to the next monitor
  workspace <1-20|next|prev>          switch workspace on the focused monitor
  move-to-workspace <1-20|next|prev>  send the focused window to another workspace
  toggle-float                        float / tile the focused window
  toggle-fullscreen                   cover the whole monitor
  toggle-maximize                     fill the work area, keeping gaps and taskbar
  promote                             swap the focused window with the master
  resize <delta>                      grow (+0.05) or shrink (-0.05) the focused split
  close                               close the focused window
  retile                              re-scan windows and re-apply the layout
  reload                              reload the config file
  toggle-pause                        stop / resume tiling
  quit                                restore all windows and exit
  exec <command line>                 launch a program";

impl Command {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let (name, arg) = s.split_once(char::is_whitespace).map_or((s, ""), |(a, b)| (a, b.trim()));
        let bare = |c: Command| if arg.is_empty() { Ok(c) } else { Err(format!("`{name}` takes no argument")) };
        match name {
            "focus" => Ok(Self::Focus(direction(arg)?)),
            "move" => Ok(Self::Move(direction(arg)?)),
            "workspace" => Ok(Self::Workspace(workspace(arg)?)),
            "move-to-workspace" => Ok(Self::MoveToWorkspace(workspace(arg)?)),
            "toggle-float" => bare(Self::ToggleFloat),
            "toggle-fullscreen" => bare(Self::ToggleFullscreen),
            "toggle-maximize" => bare(Self::ToggleMaximize),
            "close" => bare(Self::Close),
            "promote" => bare(Self::Promote),
            "retile" => bare(Self::Retile),
            "reload" => bare(Self::Reload),
            "toggle-pause" => bare(Self::TogglePause),
            "quit" => bare(Self::Quit),
            "resize" => arg
                .parse::<f32>()
                .ok()
                .filter(|d| d.abs() < 1.0)
                .map(Self::Resize)
                .ok_or_else(|| format!("`resize` needs a delta like +0.05 or -0.05, got `{arg}`")),
            "exec" if !arg.is_empty() => Ok(Self::Exec(arg.to_string())),
            "exec" => Err("`exec` needs a command line".into()),
            "" => Err("empty command".into()),
            _ => Err(format!("unknown command `{name}`")),
        }
    }
}

fn direction(arg: &str) -> Result<Direction, String> {
    match arg {
        "left" => Ok(Direction::Left),
        "right" => Ok(Direction::Right),
        "up" => Ok(Direction::Up),
        "down" => Ok(Direction::Down),
        _ => Err(format!("expected left, right, up or down, got `{arg}`")),
    }
}

fn workspace(arg: &str) -> Result<WsTarget, String> {
    match arg {
        "next" => Ok(WsTarget::Next),
        "prev" => Ok(WsTarget::Prev),
        n => match n.parse::<usize>() {
            Ok(n @ 1..=20) => Ok(WsTarget::Index(n - 1)),
            _ => Err(format!("expected a workspace number 1-20, next or prev, got `{arg}`")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(Command::parse("focus left"), Ok(Command::Focus(Direction::Left)));
        assert_eq!(Command::parse("  workspace 3 "), Ok(Command::Workspace(WsTarget::Index(2))));
        assert_eq!(Command::parse("move-to-workspace next"), Ok(Command::MoveToWorkspace(WsTarget::Next)));
        assert_eq!(Command::parse("resize +0.05"), Ok(Command::Resize(0.05)));
        assert_eq!(Command::parse("resize -0.1"), Ok(Command::Resize(-0.1)));
        assert_eq!(Command::parse("exec wt -d ."), Ok(Command::Exec("wt -d .".into())));
        assert_eq!(Command::parse("toggle-float"), Ok(Command::ToggleFloat));
    }

    #[test]
    fn rejects_bad_commands() {
        assert!(Command::parse("focus sideways").is_err());
        assert!(Command::parse("workspace 0").is_err());
        assert!(Command::parse("workspace 21").is_err());
        assert!(Command::parse("close now").is_err());
        assert!(Command::parse("resize big").is_err());
        assert!(Command::parse("exec").is_err());
        assert!(Command::parse("dance").is_err());
        assert!(Command::parse("").is_err());
    }
}
