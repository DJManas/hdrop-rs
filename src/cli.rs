//! The command line.

use clap::error::ErrorKind;
use clap::{ArgAction, Parser};

use crate::geometry::Position;
use crate::launch::DEFAULT_LAUNCH_TIMEOUT_SECS;
use crate::notify::notifyDesktop;

/// Usage: hdrop [OPTIONS] [COMMAND]
#[derive(Parser, Debug)]
#[command(author, version, about = "A way to drop down applications (Rust port of hdrop 0.7.10)", long_about = None)]
#[command(disable_help_flag = true)] // 1. Disable the default -h and --help
pub struct CliArgs {
  /// Print this help message
  #[arg(short = 'H', long, action = ArgAction::Help)]
  pub help: Option<bool>,

  /// Launch hidden (into the special:hdrop workspace) instead of on the current workspace.
  #[arg(short, long)]
  pub background: bool,

  /// Match windows of this class instead of the one inferred from COMMAND. Doesn't change the class the program gets - use its own flag for that (e.g. 'kitty --class X').
  #[arg(short, long)]
  pub class: Option<String>,

  /// Spawn as a floating window. Implied by -g/-h/-p/-w.
  #[arg(short, long)]
  pub floating: bool,

  /// Focus the window (switching to its workspace if needed) instead of moving or hiding it.
  #[arg(short='F', long)]
  pub focus: bool,

  /// If the window is on the current workspace but not focused, focus it instead of hiding it (the next press hides it).
  #[arg(long)]
  pub focusFirst: bool,

  /// Case insensitive partial matching of class names.
  #[arg(short, long)]
  pub insensitive: bool,

  /// Floating: gap to the screen edge in pixels [default: 0]
  #[arg(short, long)]
  pub gap: Option<u16>,

  /// Floating: height of the window, in percent (1-100) [default: 50, or 100 for left/right]
  #[arg(short, long, value_parser = clap::value_parser!(u16).range(1..=100))]
  pub height: Option<u16>,

  /// Floating: position of the window [default: top]
  #[arg(short, long, value_enum)]
  pub position: Option<Position>,

  /// Floating: width of the window, in percent (1-100) [default: 100, or 50 for left/right]
  #[arg(short, long, value_parser = clap::value_parser!(u16).range(1..=100))]
  pub width: Option<u16>,

  /// Delay initial launch for up to 20 seconds until internet connectivity is established.
  #[arg(short = 'o', long)]
  pub online: bool,

  /// Seconds a launch waits for its window (1-600). Until then, and as long again afterwards if none appeared, further presses don't launch it again.
  #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_LAUNCH_TIMEOUT_SECS, value_parser = clap::value_parser!(u64).range(1..=600))]
  pub launchTimeout: u64,

  /// Explain the matching and the chosen action (stderr and notifications).
  #[arg(short, long)]
  pub verbose: bool,

  /// Print what this invocation would do (class, matching windows, action, geometry) without doing anything.
  #[arg(long)]
  pub explain: bool,

  /// List open windows (class, workspace, title), to find the class to use.
  #[arg(long, conflicts_with_all = ["command", "explain", "check"])]
  pub listWindows: bool,

  /// Check the setup: Hyprland reachable and tested, Lua dispatch working, curl, notifications.
  #[arg(long, conflicts_with_all = ["command", "explain"])]
  pub check: bool,

  /// The usual command you would run to start the desired program
  #[arg(trailing_var_arg = true)]
  pub command: Vec<String>,
}

impl CliArgs {
  /// Geometry flags only mean something for a floating window, so giving any of them is taken as
  /// asking for one. That's why -g/-p have no clap defaults: a default must not count as given.
  pub fn isFloating(&self) -> bool {
    self.floating || self.gap.is_some() || self.height.is_some() || self.position.is_some() || self.width.is_some()
  }

  pub fn gap(&self) -> u16 {
    self.gap.unwrap_or(0)
  }

  pub fn position(&self) -> Position {
    self.position.unwrap_or(Position::Top)
  }

  pub fn launchTimeout(&self) -> std::time::Duration {
    std::time::Duration::from_secs(self.launchTimeout)
  }
}

/// Like `CliArgs::parse`, but a usage error also becomes a notification: from a keybind, clap's
/// stderr output is invisible and the key would just seem dead.
pub fn parseArgs() -> CliArgs {
  CliArgs::try_parse().unwrap_or_else(|error| {
    if !matches!(error.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
      let rendered = error.to_string();
      let summary = rendered.lines().next().unwrap_or_default().trim_start_matches("error: ");
      notifyDesktop(&format!("{summary}\nRun 'hdrop-rs -H' for more information"));
    }
    error.exit()
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse(arguments: &[&str]) -> CliArgs {
    CliArgs::try_parse_from(std::iter::once("hdrop-rs").chain(arguments.iter().copied())).unwrap()
  }

  #[test]
  fn geometryFlagsImplyFloating() {
    assert!(!parse(&["kitty"]).isFloating());
    assert!(parse(&["-f", "kitty"]).isFloating());
    for flag in [["-w", "80"], ["-h", "30"], ["-g", "10"], ["-p", "bottom"]] {
      assert!(parse(&[flag[0], flag[1], "kitty"]).isFloating(), "{flag:?} must imply -f");
    }
  }

  #[test]
  fn flagsAfterTheCommandBelongToTheCommand() {
    let args = parse(&["kitty", "-w", "80", "--class", "Guake"]);
    assert!(!args.isFloating());
    assert_eq!(args.class, None);
    assert_eq!(args.command, ["kitty", "-w", "80", "--class", "Guake"]);
  }

  #[test]
  fn listWindowsAndCheckDontTakeACommand() {
    let attempt = |arguments: &[&str]| CliArgs::try_parse_from(std::iter::once("hdrop-rs").chain(arguments.iter().copied()));
    assert!(attempt(&["--list-windows"]).is_ok());
    assert!(attempt(&["--check"]).is_ok());
    assert!(attempt(&["--list-windows", "kitty"]).is_err());
    assert!(attempt(&["--check", "kitty"]).is_err());
    assert!(attempt(&["--explain", "kitty"]).is_ok());
  }

  #[test]
  fn floatingDefaultsAreTopWithNoGap() {
    let args = parse(&["-f", "kitty"]);
    assert_eq!((args.position(), args.gap()), (Position::Top, 0));
  }
}
