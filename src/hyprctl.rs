//! Reading Hyprland state through `hyprctl -j`, and running Lua through `hyprctl eval`.

use std::process;

use serde::Deserialize;
use serde::de::DeserializeOwned;

/// Workspace reference as embedded in `hyprctl -j clients` and returned by `activeworkspace`.
/// `id` is never read: since Hyprland 0.57 it's present for numbered workspaces only (missing for
/// special/named ones, which broke hyprland-rs' structs). 0.57 also added `address`, the name to
/// dispatch with, as `name` became a display name that can differ after a rename; 0.56 has no
/// `address` and its `name` is the addressable one, hence the fallback. `monitor` (the owning
/// monitor's name) is only in `activeworkspace`, not in the copy embedded in clients.
#[derive(Deserialize, Debug, PartialEq)]
pub struct WorkspaceRef {
  address: Option<String>,
  name: String,
  monitor: Option<String>,
}

impl WorkspaceRef {
  pub fn address(&self) -> &str {
    self.address.as_deref().unwrap_or(&self.name)
  }
}

/// The subset of `hyprctl -j clients` fields hdrop needs.
#[derive(Deserialize, Debug, PartialEq)]
pub struct Client {
  pub address: String,
  pub class: String,
  pub title: String,
  pub mapped: bool,
  pub workspace: WorkspaceRef,
}

impl Client {
  /// Window selector for Lua dispatchers. Dispatching by address, not by class, guarantees every
  /// action of one toggle hits the window that was inspected, even with several windows of the
  /// same class or a class containing regex metacharacters.
  pub fn selector(&self) -> String {
    format!("address:{}", self.address)
  }
}

/// The subset of `hyprctl -j monitors` fields hdrop needs.
#[derive(Deserialize, Debug)]
pub struct Monitor {
  pub name: String,
  pub x: i32,
  pub y: i32,
  pub width: u32,
  pub height: u32,
  pub scale: f32,
  pub transform: u8,
  /// Space taken by bars etc., in logical pixels: `[left, top, right, bottom]`.
  pub reserved: [i32; 4],
  pub focused: bool,
}

/// Runs `hyprctl -j <command>` and deserializes only the fields `T` declares, so unrelated
/// changes to Hyprland's JSON output don't break hdrop (hyprland-rs' full structs did, on 0.57).
pub(crate) fn hyprctlJson<T: DeserializeOwned>(command: &str) -> Result<T, Box<dyn std::error::Error>> {
  let output = process::Command::new("hyprctl").args(["-j", command]).output()?;
  if !output.status.success() {
    return Err(format!("hyprctl -j {command} failed: {}", String::from_utf8_lossy(&output.stderr)).into());
  }
  serde_json::from_slice(&output.stdout).map_err(|error| format!("hyprctl -j {command}: {error}").into())
}

pub(crate) fn hyprctlEval(luaCall: &str) -> Result<(), Box<dyn std::error::Error>> {
  let output = process::Command::new("hyprctl").arg("eval").arg(luaCall).output()?;
  if !output.status.success() {
    return Err(format!("hyprctl eval failed: {}", String::from_utf8_lossy(&output.stderr)).into());
  }
  Ok(())
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

/// The monitor `workspace` is on (an `activeworkspace` result), so a shown window is sized for the
/// workspace it was moved to even if focus changed monitors in the meantime. Falls back to the
/// focused monitor if the workspace doesn't name one.
pub fn getMonitorOfWorkspace(workspace: &WorkspaceRef) -> Result<Monitor, Box<dyn std::error::Error>> {
  let monitors: Vec<Monitor> = hyprctlJson("monitors")?;
  let owner = |monitor: &Monitor| match &workspace.monitor {
    Some(name) => &monitor.name == name,
    None => monitor.focused,
  };
  monitors.into_iter().find(owner).ok_or_else(|| format!("no monitor for workspace '{}'", workspace.address()).into())
}

/// Address of the focused window, if any.
pub fn getActiveWindowAddress() -> Result<Option<String>, Box<dyn std::error::Error>> {
  #[derive(Deserialize)]
  struct ActiveWindow {
    address: Option<String>,
  }
  // With nothing focused, hyprctl prints `{}`.
  let window: ActiveWindow = hyprctlJson("activewindow")?;
  Ok(window.address)
}

/// Running Hyprland's version, e.g. `0.57.0` (`version` since 0.57, else the `v`-prefixed `tag`).
pub fn getHyprlandVersion() -> Result<String, Box<dyn std::error::Error>> {
  #[derive(Deserialize)]
  struct Version {
    version: Option<String>,
    tag: Option<String>,
  }
  let version: Version = hyprctlJson("version")?;
  version.version.or_else(|| version.tag.map(|tag| tag.trim_start_matches('v').to_string())).ok_or_else(|| "hyprctl version reports no version".into())
}

#[cfg(test)]
mod tests {
  use super::*;

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
      "class": "kitty",
      "title": "~"
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
