#![allow(non_snake_case, warnings)] // This must be at the very top of lib.rs

use std::process;

use hyprland::dispatch::*;
use hyprland::prelude::*;
use notify_rust::{Notification, Urgency};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// Workspace reference as embedded in `hyprctl -j clients` and returned by `activeworkspace`.
/// `id` is never read: since Hyprland 0.57 it's present for numbered workspaces only (missing for
/// special/named ones, which broke hyprland-rs' structs). 0.57 also added `address`, the name to
/// dispatch with, as `name` became a display name that can differ after a rename; 0.56 has no
/// `address` and its `name` is the addressable one, hence the fallback.
#[derive(Deserialize, Debug)]
pub struct WorkspaceRef {
  address: Option<String>,
  name: String,
}

impl WorkspaceRef {
  pub fn address(&self) -> &str {
    self.address.as_deref().unwrap_or(&self.name)
  }
}

/// The subset of `hyprctl -j clients` fields hdrop needs.
#[derive(Deserialize, Debug)]
pub struct Client {
  pub class: String,
  pub mapped: bool,
  pub workspace: WorkspaceRef,
}

/// The subset of `hyprctl -j monitors` fields hdrop needs.
#[derive(Deserialize, Debug)]
pub struct Monitor {
  pub width: u32,
  pub height: u32,
  pub scale: f32,
  pub focused: bool,
}

/// Runs `hyprctl -j <command>` and deserializes only the fields `T` declares, so unrelated
/// changes to Hyprland's JSON output don't break hdrop (hyprland-rs' full structs did, on 0.57).
fn hyprctlJson<T: DeserializeOwned>(command: &str) -> Result<T, Box<dyn std::error::Error>> {
  let output = process::Command::new("hyprctl").args(["-j", command]).output()?;
  if !output.status.success() {
    return Err(format!("hyprctl -j {command} failed: {}", String::from_utf8_lossy(&output.stderr)).into());
  }
  serde_json::from_slice(&output.stdout).map_err(|error| format!("hyprctl -j {command}: {error}").into())
}

pub fn getClients() -> Result<Vec<Client>, Box<dyn std::error::Error>> {
  hyprctlJson("clients")
}

pub fn getActiveWorkspace() -> Result<WorkspaceRef, Box<dyn std::error::Error>> {
  hyprctlJson("activeworkspace")
}

pub fn getActiveMonitor() -> Result<Monitor, Box<dyn std::error::Error>> {
  let monitors: Vec<Monitor> = hyprctlJson("monitors")?;
  monitors.into_iter().find(|monitor| monitor.focused).ok_or_else(|| "no focused monitor".into())
}

/// Shows a desktop notification for `message`, mirroring bash hdrop's `notify()`: try a D-Bus
/// notification first, fall back to `hyprctl notify` if there's no notification daemon. Since
/// hdrop is normally launched from a keybind with no visible terminal, this is the only way most
/// errors are ever seen - `eprintln!` alone would be silently lost.
pub fn notifyError(message: &str) {
  if Notification::new().summary("hdrop").body(message).show().is_ok() {
    return;
  }
  let _ = process::Command::new("hyprctl").args(["notify", "0", "5000", "0", &format!("hdrop: {message}")]).output();
}

/// Low-urgency counterpart to `notifyError`, for --verbose diagnostics (mirrors bash's
/// `notify_low`). Best-effort only: unlike errors, a missed diagnostic isn't worth a fallback.
pub fn notifyVerbose(message: &str) {
  let _ = Notification::new().summary("hdrop").body(message).urgency(Urgency::Low).show();
}

/// Blocks for up to ~20s waiting for connectivity, mirroring bash hdrop's `wait_online`. The
/// result is intentionally discarded, same as bash: this only delays the initial launch, it never
/// gates it - if connectivity never comes, hdrop launches the command anyway.
pub fn waitOnline() {
  let _ = process::Command::new("curl")
    .args(["-fIs", "-m", "15", "--retry", "5", "--retry-connrefused", "--retry-all-errors", "http://connectivitycheck.gstatic.com/generate_204"])
    .output();
}

/// Width/height percentages and the Hyprland Lua `move` expression for a --floating window.
///
/// `monitor_w`/`monitor_h` in `expr` are left as literal identifiers: they're resolved by
/// Hyprland's Lua config engine against the target monitor when the window rule fires, so we
/// don't need to query monitor geometry ourselves for this part.
pub struct FloatingPosition {
  pub widthPct: u16,
  pub heightPct: u16,
  pub expr: String,
}

pub fn floatingPosition(position: &str, width: Option<u16>, height: Option<u16>, gap: u16) -> Result<FloatingPosition, String> {
  let (widthPct, heightPct, expr) = match position {
    "t" | "top" => {
      let w = width.unwrap_or(100);
      let h = height.unwrap_or(50);
      (w, h, format!("(monitor_w/2*(1-{w}/100)) ({gap})"))
    }
    "b" | "bottom" => {
      let w = width.unwrap_or(100);
      let h = height.unwrap_or(50);
      (w, h, format!("(monitor_w/2*(1-{w}/100)) (monitor_h*(1-({h}/100))-{gap})"))
    }
    "l" | "left" => {
      let w = width.unwrap_or(50);
      let h = height.unwrap_or(100);
      (w, h, format!("({gap}) (monitor_h/2*(1-{h}/100))"))
    }
    "r" | "right" => {
      let w = width.unwrap_or(50);
      let h = height.unwrap_or(100);
      (w, h, format!("(monitor_w*(1-({w}/100))-{gap}) (monitor_h/2*(1-{h}/100))"))
    }
    other => return Err(format!("invalid --position value '{other}'")),
  };
  Ok(FloatingPosition { widthPct, heightPct, expr })
}

/// Same formulas as `floatingPosition`'s `expr`, but resolved to actual pixels against a given
/// monitor size instead of left as a `monitor_w`/`monitor_h` Lua expression. Needed because
/// `hl.window_rule`'s `move` only re-fires the first time a window becomes floating, not on every
/// subsequent workspace toggle - so re-centering an already-floating window on each toggle needs
/// an explicit `hl.dsp.window.move` call with real numbers (confirmed live: that call has no
/// `monitor_w`/`monitor_h` globals available, unlike the window_rule context).
pub fn floatingPixelPosition(position: &str, widthPct: u16, heightPct: u16, gap: u16, monitorWidth: f32, monitorHeight: f32) -> Result<(i32, i32), String> {
  let w = widthPct as f32;
  let h = heightPct as f32;
  let gap = gap as f32;
  let (x, y) = match position {
    "t" | "top" => (monitorWidth / 2.0 * (1.0 - w / 100.0), gap),
    "b" | "bottom" => (monitorWidth / 2.0 * (1.0 - w / 100.0), monitorHeight * (1.0 - h / 100.0) - gap),
    "l" | "left" => (gap, monitorHeight / 2.0 * (1.0 - h / 100.0)),
    "r" | "right" => (monitorWidth * (1.0 - w / 100.0) - gap, monitorHeight / 2.0 * (1.0 - h / 100.0)),
    other => return Err(format!("invalid --position value '{other}'")),
  };
  Ok((x.round() as i32, y.round() as i32))
}

/// Explicitly moves the (already floating) window matching `class` to its position, computed from
/// the active monitor's real geometry. See `floatingPixelPosition` for why this can't just reuse
/// the `hl.window_rule` move string.
pub fn repositionWindow(class: &str, position: &str, widthPct: u16, heightPct: u16, gap: u16) -> Result<(), Box<dyn std::error::Error>> {
  let activeMonitor = getActiveMonitor()?;
  let monitorWidth = activeMonitor.width as f32 / activeMonitor.scale;
  let monitorHeight = activeMonitor.height as f32 / activeMonitor.scale;
  let (xPx, yPx) = floatingPixelPosition(position, widthPct, heightPct, gap, monitorWidth, monitorHeight)?;
  let classPattern = escapeLuaDoubleQuoted(class);
  let luaCall = format!("hl.dsp.window.move({{window = \"class:^{classPattern}$\", x = {xPx}, y = {yPx}}})");
  Dispatch::call(DispatchType::Custom(&luaCall, ""))?;
  Ok(())
}

/// Escapes a value for embedding in a single-quoted Lua string literal passed to `hyprctl eval`.
pub fn escapeLuaString(value: &str) -> String {
  value.replace('\\', "\\\\").replace('\'', "\\'")
}

pub fn applyFloatingRule(class: &str, positionExpr: &str) -> Result<(), Box<dyn std::error::Error>> {
  let classPattern = escapeLuaString(class);
  let luaCall = format!(
    "hl.window_rule({{ match = {{ class = '^{classPattern}$' }}, float = true, move = '{positionExpr}' }})"
  );
  let output = process::Command::new("hyprctl").arg("eval").arg(&luaCall).output()?;
  if !output.status.success() {
    return Err(format!("hyprctl eval failed: {}", String::from_utf8_lossy(&output.stderr)).into());
  }
  Ok(())
}

/// Extracts the class the spawned program will register under, from a flag embedded in its own
/// arguments: `-a`/`--app-id` for `foot`, `--class` for everything else. Accepts both
/// `--class value` and `--class=value` forms.
pub fn extractEmbeddedClass(command: &[String]) -> Option<String> {
  let isFoot = command.first().map(String::as_str) == Some("foot");
  let longFlag = if isFoot { "--app-id" } else { "--class" };
  let shortFlag = if isFoot { Some("-a") } else { None };

  let mut args = command.iter().skip(1);
  while let Some(arg) = args.next() {
    if arg == longFlag || Some(arg.as_str()) == shortFlag {
      return args.next().cloned();
    }
    if let Some(value) = arg.strip_prefix(&format!("{longFlag}=")) {
      return Some(value.to_string());
    }
  }
  None
}

pub fn caseInsensitiveContains(haystack: &str, needle: &str) -> bool {
  haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Finds the first currently open window whose class contains `pattern` (case insensitive).
pub fn partialMatchClass(pattern: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
  let clients = getClients()?;
  Ok(clients.into_iter().find(|client| caseInsensitiveContains(&client.class, pattern)).map(|client| client.class))
}

/// Resolves the window class to match/apply rules against, mirroring hdrop's precedence:
/// hardcoded per-app overrides, then a class/app-id flag embedded in the command itself,
/// then (if --insensitive) a live partial match, then the top-level --class override (wins always).
pub fn resolveClass(command: &[String], classOverride: Option<&str>, insensitive: bool, verbose: bool) -> Result<String, Box<dyn std::error::Error>> {
  let commandName = command.first().map(String::as_str).unwrap_or_default();

  let mut class = match commandName {
    "epiphany" => "org.gnome.Epiphany".to_string(),
    "logseq" => "Logseq".to_string(),
    "telegram-desktop" => "org.telegram.desktop".to_string(),
    "tor-browser" => "Tor Browser".to_string(),
    "godot4" => partialMatchClass("org.godotengine.")?.unwrap_or_else(|| commandName.to_string()),
    _ => extractEmbeddedClass(command).unwrap_or_else(|| commandName.to_string()),
  };

  if verbose && class != commandName {
    notifyVerbose(&format!("Using '{class}' as classname of '{commandName}' for matching"));
  }

  if insensitive {
    if let Some(matched) = partialMatchClass(&class)? {
      if verbose {
        notifyVerbose(&format!("--insensitive -> using class '{matched}' after partial match of '{class}'"));
      }
      class = matched;
    }
  }

  if let Some(overrideClass) = classOverride {
    if verbose {
      notifyVerbose(&format!("--class -> using given class '{overrideClass}' instead of '{class}' for matching"));
    }
    class = overrideClass.to_string();
  }

  Ok(class)
}

/// Escapes a value for embedding in a double-quoted Lua string literal passed to `hyprctl dispatch`.
pub fn escapeLuaDoubleQuoted(value: &str) -> String {
  value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Moves the window matching `class` to `workspace` (a workspace name, or the literal
/// "special") via the `hl.dsp.window.move` Lua dispatcher, which replaced `movetoworkspacesilent`
/// for this purpose in Hyprland >=0.55.
pub fn moveWindowToWorkspace(class: &str, workspace: &str) -> hyprland::Result<()> {
  let classPattern = escapeLuaDoubleQuoted(class);
  let workspaceName = escapeLuaDoubleQuoted(workspace);
  let luaCall = format!(
    "hl.dsp.window.move({{workspace=\"{workspaceName}\", follow=false, window=\"class:^{classPattern}$\"}})"
  );
  Dispatch::call(DispatchType::Custom(&luaCall, ""))
}

/// Focuses the window matching `class`.
///
/// Uses `hl.dsp.focus` (a top-level dispatcher, *not* `hl.dsp.window.focus` despite the
/// namespacing of other window dispatchers) since the classic `focuswindow` dispatcher no longer
/// works: as of Hyprland 0.56.2, every `hyprctl dispatch <name> <args>` call is run through a Lua
/// shim (`hl.dispatch(...)`), and classic bareword syntax isn't valid Lua. Confirmed live against
/// this machine's compositor.
pub fn focusWindow(class: &str) -> hyprland::Result<()> {
  let classPattern = escapeLuaDoubleQuoted(class);
  let luaCall = format!("hl.dsp.focus({{window = \"class:^{classPattern}$\"}})");
  Dispatch::call(DispatchType::Custom(&luaCall, ""))
}

/// Resizes the window matching `class` to the given width/height percentages of the active
/// monitor.
///
/// `hl.dsp.window.resize` (the working replacement for the classic `resizewindowpixel`, which
/// fails the same way as `focuswindow` above) takes numeric pixel `x`/`y`, not percentages, and
/// doesn't have `monitor_w`/`monitor_h` available like `hl.window_rule`'s `move` string does — so
/// unlike the floating-rule position, this needs the active monitor's real geometry.
pub fn resizeWindow(class: &str, widthPct: u16, heightPct: u16) -> Result<(), Box<dyn std::error::Error>> {
  let activeMonitor = getActiveMonitor()?;
  let widthPx = (activeMonitor.width as f32 / activeMonitor.scale * widthPct as f32 / 100.0).round() as i32;
  let heightPx = (activeMonitor.height as f32 / activeMonitor.scale * heightPct as f32 / 100.0).round() as i32;
  let classPattern = escapeLuaDoubleQuoted(class);
  let luaCall = format!("hl.dsp.window.resize({{window = \"class:^{classPattern}$\", x = {widthPx}, y = {heightPx}}})");
  Dispatch::call(DispatchType::Custom(&luaCall, ""))?;
  Ok(())
}

pub fn launchForeground(command: &[String]) -> Result<(), Box<dyn std::error::Error>> {
  let (program, programArgs) = command.split_first().ok_or("hdrop: no command given")?;
  let status = process::Command::new(program).args(programArgs).status()?;
  if !status.success() {
    return Err(format!("hdrop: '{}' exited with {status}", command.join(" ")).into());
  }
  Ok(())
}

/// Launches `command` into the hidden `special:hdrop` workspace without switching to it, via
/// `hl.exec_cmd` (the working replacement for the classic `exec [workspace ... silent] cmd`,
/// which fails the same way as the other classic dispatchers above). `silent` has to be a
/// modifier inside the workspace string itself (`"special:hdrop silent"`) - passing it as its
/// own `silent = true` key errors with "unknown effect 'silent'". Confirmed live: a real `kitty`
/// launched this way lands directly in `special:hdrop`, never appearing on the active workspace.
pub fn launchInBackground(command: &[String]) -> Result<(), Box<dyn std::error::Error>> {
  let commandLine = escapeLuaDoubleQuoted(&command.join(" "));
  let luaCall = format!("hl.exec_cmd(\"{commandLine}\", {{workspace = \"special:hdrop silent\"}})");
  let output = process::Command::new("hyprctl").arg("eval").arg(&luaCall).output()?;
  if !output.status.success() {
    return Err(format!("hyprctl eval failed: {}", String::from_utf8_lossy(&output.stderr)).into());
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  // Pure logic only, no live Hyprland connection: godot4/--insensitive both call
  // partialMatchClass, which needs a real compositor, so they're not covered here.
  // See tests/hyprctl_lua_dispatch.rs for the live-compositor checks.

  #[test]
  fn topPositionPixelsIgnoreReservedAreaAndUseGapAsIs() {
    // A 2256x1504 monitor, matching the machine this was verified against.
    let (x, y) = floatingPixelPosition("top", 100, 25, 36, 2256.0, 1504.0).unwrap();
    assert_eq!((x, y), (0, 36));
  }

  #[test]
  fn bottomPositionAnchorsToMonitorHeightMinusGap() {
    let (x, y) = floatingPixelPosition("bottom", 100, 25, 36, 2256.0, 1504.0).unwrap();
    assert_eq!((x, y), (0, 1504 - 376 - 36));
  }

  #[test]
  fn leftAndRightPositionsUseGapAsXOffset() {
    let (leftX, _) = floatingPixelPosition("left", 50, 100, 20, 2256.0, 1504.0).unwrap();
    assert_eq!(leftX, 20);
    let (rightX, _) = floatingPixelPosition("right", 50, 100, 20, 2256.0, 1504.0).unwrap();
    assert_eq!(rightX, 2256 - 1128 - 20);
  }

  #[test]
  fn invalidPositionIsRejected() {
    assert!(floatingPixelPosition("diagonal", 100, 50, 0, 2256.0, 1504.0).is_err());
  }

  fn resolve(command: &[&str], classOverride: Option<&str>) -> String {
    let command: Vec<String> = command.iter().map(|s| s.to_string()).collect();
    resolveClass(&command, classOverride, false, false).unwrap()
  }

  #[test]
  fn hardcodedClassesOverrideCommandName() {
    assert_eq!(resolve(&["epiphany"], None), "org.gnome.Epiphany");
    assert_eq!(resolve(&["logseq"], None), "Logseq");
    assert_eq!(resolve(&["telegram-desktop"], None), "org.telegram.desktop");
    assert_eq!(resolve(&["tor-browser"], None), "Tor Browser");
  }

  #[test]
  fn unknownCommandFallsBackToItsOwnName() {
    assert_eq!(resolve(&["kitty"], None), "kitty");
  }

  #[test]
  fn footUsesShortAppIdFlag() {
    assert_eq!(resolve(&["foot", "-a", "myapp"], None), "myapp");
  }

  #[test]
  fn footUsesLongAppIdFlag() {
    assert_eq!(resolve(&["foot", "--app-id", "myapp"], None), "myapp");
  }

  #[test]
  fn genericCommandUsesEmbeddedClassFlag() {
    assert_eq!(resolve(&["firefox", "--class", "Profile"], None), "Profile");
  }

  #[test]
  fn genericCommandUsesEmbeddedClassFlagWithEquals() {
    assert_eq!(resolve(&["firefox", "--class=Profile"], None), "Profile");
  }

  #[test]
  fn footsOwnClassFlagIsIgnoredSinceItOnlyLooksForAppId() {
    // foot doesn't take --class, only -a/--app-id, so this must fall through to the command name.
    assert_eq!(resolve(&["foot", "--class", "myapp"], None), "foot");
  }

  #[test]
  fn topLevelOverrideWinsOverEverythingElse() {
    assert_eq!(resolve(&["epiphany"], Some("Forced")), "Forced");
    assert_eq!(resolve(&["firefox", "--class", "Profile"], Some("Forced")), "Forced");
  }

  #[test]
  fn clientOnSpecialWorkspaceWithoutIdDeserializes() {
    // Shape of Hyprland 0.57 `hyprctl -j clients`: no "id" for non-numbered workspaces.
    let json = r#"[{
      "address": "0x55d0c1a2b3c0",
      "mapped": true,
      "workspace": {
        "address": "special:hdrop",
        "type": "special",
        "name": "special:hdrop"
      },
      "class": "kitty"
    }]"#;
    let clients: Vec<Client> = serde_json::from_str(json).unwrap();
    assert_eq!(clients[0].class, "kitty");
    assert_eq!(clients[0].workspace.address(), "special:hdrop");
  }

  #[test]
  fn renamedWorkspaceUsesAddressNotDisplayName() {
    let json = r#"{"address": "3", "id": 3, "type": "numbered", "name": "mail"}"#;
    let workspace: WorkspaceRef = serde_json::from_str(json).unwrap();
    assert_eq!(workspace.address(), "3");
  }

  #[test]
  fn hyprland056WorkspaceWithoutAddressFallsBackToName() {
    // Shape of Hyprland 0.56 `hyprctl -j activeworkspace`: no "address" field yet.
    let json = r#"{"id": -98, "name": "special:hdrop", "monitor": "DP-1"}"#;
    let workspace: WorkspaceRef = serde_json::from_str(json).unwrap();
    assert_eq!(workspace.address(), "special:hdrop");
  }
}
