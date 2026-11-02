//! The read-only modes: --explain, --list-windows and --check.

use std::process;

use crate::HIDDEN_WORKSPACE;
use crate::class::resolveClass;
use crate::cli::CliArgs;
use crate::dispatch::focusWindow;
use crate::geometry::{FloatingPosition, WindowBox, floatingBox, floatingPosition};
use crate::hyprctl::{Client, getActiveWindowAddress, getActiveWorkspace, getClients, getHyprlandVersion, getMonitorOfWorkspace};
use crate::launch::{LaunchState, nowMillis, peekLaunchState};
use crate::toggle::{Action, decideAction};

/// Hyprland versions hdrop-rs has actually been run against (reported by --check).
pub const TESTED_HYPRLAND_VERSIONS: [&str; 2] = ["0.56.2", "0.57.0"];

/// --explain: the same resolution and decision as `run`, printed instead of executed. Read-only:
/// no lock, no window rule changes, no connectivity wait, no launch, no dispatch. Geometry is for
/// the monitor of the current workspace, as a real show would use.
pub fn explain(args: &CliArgs) -> Result<bool, Box<dyn std::error::Error>> {
  let resolved = resolveClass(&args.command, args.class.as_deref(), args.insensitive)?;
  let class = resolved.class.as_str();
  let activeWorkspace = getActiveWorkspace()?;
  let clients = getClients()?;
  let activeWindow = getActiveWindowAddress()?;
  let floatingPos = args.isFloating().then(|| floatingPosition(args.position(), args.width, args.height, args.gap()));
  let describeBox = |position: &FloatingPosition| -> Result<String, Box<dyn std::error::Error>> {
    let WindowBox { x, y, width, height } = floatingBox(&getMonitorOfWorkspace(&activeWorkspace)?, args.position(), position.widthPct, position.heightPct, args.gap());
    Ok(format!("{width}x{height} at ({x}, {y})"))
  };

  println!("command:   {}", args.command.join(" "));
  println!("class:     {class} ({})", resolved.reason);
  // Same criterion as the real selection: unmapped clients aren't usable windows.
  let matches: Vec<&Client> = clients.iter().filter(|client| client.mapped && client.class == class).collect();
  if matches.is_empty() {
    println!("matches:   none");
  }
  for (index, client) in matches.iter().enumerate() {
    let workspace = client.workspace.address();
    let mut notes = Vec::new();
    if workspace == HIDDEN_WORKSPACE {
      notes.push("hidden");
    }
    if workspace == activeWorkspace.address() {
      notes.push("current workspace");
    }
    if activeWindow.as_deref() == Some(client.address.as_str()) {
      notes.push("focused");
    }
    let notes = if notes.is_empty() { String::new() } else { format!(" ({})", notes.join(", ")) };
    let label = if index == 0 { "matches:" } else { "" };
    println!("{label:<10} {} on {workspace}{notes} - {}", client.address, client.title);
  }

  let launchState = peekLaunchState(class);
  let pending = launchState.and_then(|state| state.pendingRemaining(nowMillis()));
  match (launchState, pending) {
    (Some(LaunchState::Starting), _) => println!("startup:   in progress (another invocation is launching it)"),
    (_, Some(remaining)) => println!("startup:   unconfirmed; counts as still starting for another {}s", remaining.as_secs().max(1)),
    _ => {}
  }

  match &floatingPos {
    Some(position) => println!("floating:  rule {}% x {}% at {}, gap {}", position.widthPct, position.heightPct, args.position(), args.gap()),
    None => println!("floating:  no (hdrop-rs' floating rule for this class, if any, gets turned off)"),
  }

  let geometry = match &floatingPos {
    Some(position) => format!(", resize to {}", describeBox(position)?),
    None => String::new(),
  };
  let action = match decideAction(args, &clients, class, activeWorkspace.address(), activeWindow.as_deref()) {
    Action::Show(client) => format!("show {}: move from {} to {}{geometry}, then focus", client.address, client.workspace.address(), activeWorkspace.address()),
    Action::Focus(client) if client.workspace.address() != activeWorkspace.address() => {
      format!("focus {} (switches to workspace {}; -F doesn't move it)", client.address, client.workspace.address())
    }
    Action::Focus(client) if !args.focus => format!("focus {} (--focus-first: it's here but not focused; the next press hides it)", client.address),
    Action::Focus(client) => format!("focus {}", client.address),
    Action::Hide(client) => format!("hide {}: move to {HIDDEN_WORKSPACE}", client.address),
    Action::Launch if launchState == Some(LaunchState::Starting) => "nothing: a press now joins the startup in progress".to_string(),
    Action::Launch if pending.is_some() => "nothing: it's still starting, so a press now wouldn't launch it again".to_string(),
    Action::Launch => {
      let target = if args.background { format!("hidden in {HIDDEN_WORKSPACE}") } else { format!("on {}", activeWorkspace.address()) };
      let opens = match &floatingPos {
        Some(position) => format!(", opening floating {} via the window rule", describeBox(position)?),
        None => String::new(),
      };
      let online = if args.online { "wait up to 20s for connectivity, then " } else { "" };
      format!("{online}launch {target}{opens}, then wait up to {}s for a window of class '{class}'", args.launchTimeout().as_secs())
    }
  };
  println!("action:    {action}");
  Ok(true)
}

/// --list-windows: open (mapped) windows, sorted by class, as an aligned table on stdout.
pub fn listWindows() -> Result<bool, Box<dyn std::error::Error>> {
  let clients = getClients()?;
  let mut windows: Vec<&Client> = clients.iter().filter(|client| client.mapped).collect();
  windows.sort_by(|a, b| (a.class.to_lowercase(), a.workspace.address()).cmp(&(b.class.to_lowercase(), b.workspace.address())));
  let classWidth = windows.iter().map(|client| client.class.chars().count()).chain(["CLASS".len()]).max().unwrap_or_default();
  let workspaceWidth = windows.iter().map(|client| client.workspace.address().chars().count()).chain(["WORKSPACE".len()]).max().unwrap_or_default();
  println!("{:<classWidth$}  {:<workspaceWidth$}  TITLE", "CLASS", "WORKSPACE");
  for client in windows {
    println!("{:<classWidth$}  {:<workspaceWidth$}  {}", client.class, client.workspace.address(), client.title);
  }
  Ok(true)
}

/// Parses `0.57.0`-style versions into comparable numbers; anything unparsable becomes 0.
fn versionNumbers(version: &str) -> Vec<u32> {
  version.split(['.', '-', '+']).take(3).map(|part| part.parse().unwrap_or(0)).collect()
}

/// --check: environment report on stdout. Fails (exit 1) only on what hdrop-rs can't work without:
/// a reachable, new enough Hyprland that understands the Lua dispatchers.
pub fn check() -> Result<bool, Box<dyn std::error::Error>> {
  let mut healthy = true;
  let mut report = |status: &str, what: &str, detail: String| {
    healthy &= status != "FAIL";
    println!("{status:<5} {what}: {detail}");
  };

  match getHyprlandVersion() {
    Err(error) => report("FAIL", "Hyprland", format!("can't query it via hyprctl: {error}")),
    Ok(version) => {
      let tested = TESTED_HYPRLAND_VERSIONS.join(", ");
      if versionNumbers(&version) < vec![0, 55] {
        report("FAIL", "Hyprland", format!("{version} is too old; hdrop-rs needs the Lua dispatch of >=0.55 (tested: {tested})"));
      } else if TESTED_HYPRLAND_VERSIONS.contains(&version.as_str()) {
        report("ok", "Hyprland", format!("{version} (tested)"));
      } else {
        report("warn", "Hyprland", format!("{version} is untested (tested: {tested}); run 'cargo test' against it to verify"));
      }
      // A window that can't exist: "window not found" proves the call itself is understood. Any
      // other answer fails conservatively, but is shown as-is: it may just be new wording.
      match focusWindow("address:0xdeadbeef") {
        Ok(()) => report("ok", "Lua dispatch", "hl.dsp.focus understood".to_string()),
        Err(error) if error.to_string().contains("window not found") => report("ok", "Lua dispatch", "hl.dsp.focus understood".to_string()),
        Err(error) => report("FAIL", "Lua dispatch", format!("compatibility probe failed (expected \"window not found\"), Hyprland answered: {error}")),
      }
    }
  }

  match process::Command::new("curl").arg("--version").output() {
    Ok(output) => {
      let version = String::from_utf8_lossy(&output.stdout).split_whitespace().take(2).collect::<Vec<_>>().join(" ");
      report("ok", "curl", format!("{version} (used by -o/--online)"));
    }
    Err(_) => report("warn", "curl", "not found; -o/--online will launch without waiting".to_string()),
  }

  match notify_rust::get_server_information() {
    Ok(server) => report("ok", "notifications", format!("{} {}", server.name, server.version)),
    Err(_) => report("warn", "notifications", "no notification daemon; errors fall back to 'hyprctl notify'".to_string()),
  }

  match std::env::var_os("XDG_RUNTIME_DIR") {
    Some(directory) => report("ok", "lock directory", format!("{} (XDG_RUNTIME_DIR)", directory.to_string_lossy())),
    None => report("warn", "lock directory", format!("XDG_RUNTIME_DIR unset, using {}", std::env::temp_dir().display())),
  }

  Ok(healthy)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn versionNumbersCompareNumerically() {
    assert!(versionNumbers("0.57.0") > versionNumbers("0.56.2"));
    assert!(versionNumbers("0.100.0") > versionNumbers("0.57.0"));
    assert!(versionNumbers("0.54.9") < vec![0, 55]);
  }
}
