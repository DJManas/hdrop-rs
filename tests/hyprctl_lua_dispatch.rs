//! Live-compositor checks for the `hyprctl eval`/`hyprctl dispatch` Lua calls hdrop-rs sends.
//!
//! Hyprland >=0.55 routes every `hyprctl dispatch <string>` through a Lua interpreter
//! (`hl.dispatch(...)`), so the classic dispatcher syntax hyprland-rs still generates
//! (`focuswindow class:^x$`, `resizewindowpixel exact ...`) fails outright, and the Lua
//! replacements (`hl.dsp.focus`, `hl.dsp.window.resize`, ...) aren't part of any stable
//! public API - they were reverse engineered against a live Hyprland 0.56.2. Since Hyprland
//! could rename/reshape these again at any point, these tests exist to catch that immediately
//! via `cargo test` rather than as a silent runtime failure next time hdrop-rs is actually used.
//!
//! Run against a class that can never match a real window, so nothing on the desktop is touched.
//! Skipped entirely if there's no live Hyprland to connect to (HYPRLAND_INSTANCE_SIGNATURE unset),
//! e.g. in CI or a plain TTY.

use hdrop_rs::{applyFloatingRule, focusWindow, launchInBackground, moveWindowToWorkspace, repositionWindow, resizeWindow};

const TEST_CLASS: &str = "__hdrop_rs_integration_test_nonexistent__";

fn hasLiveHyprland() -> bool {
  std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
}

/// Some of these dispatchers "fail" with a benign warning (e.g. "window not found") when
/// nothing matches `TEST_CLASS`, which is expected and fine. What we actually want to catch is
/// Hyprland no longer understanding the Lua call *at all* - a syntax error, an unknown field
/// on the dsp table, or a nil global - which means the call needs to be updated.
fn assertNotASyntaxError(result: hyprland::Result<()>, label: &str) {
  let Err(error) = result else { return };
  let message = error.to_string();
  let syntaxErrorMarkers = ["expected a dispatcher", "attempt to call a nil value", "expected near", "attempt to perform arithmetic on a nil value", "unknown effect"];
  for marker in syntaxErrorMarkers {
    assert!(
      !message.contains(marker),
      "{label}: hyprctl no longer understands this Lua call (got: {message})"
    );
  }
}

#[test]
fn windowRuleLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    eprintln!("skipping: no live Hyprland (HYPRLAND_INSTANCE_SIGNATURE unset)");
    return;
  }
  let result = applyFloatingRule(TEST_CLASS, "(monitor_w/2) (0)");
  assert!(result.is_ok(), "hl.window_rule call failed: {:?}", result.err());
}

#[test]
fn windowMoveLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    eprintln!("skipping: no live Hyprland (HYPRLAND_INSTANCE_SIGNATURE unset)");
    return;
  }
  let result = moveWindowToWorkspace(TEST_CLASS, "special");
  assertNotASyntaxError(result, "hl.dsp.window.move");
}

#[test]
fn focusLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    eprintln!("skipping: no live Hyprland (HYPRLAND_INSTANCE_SIGNATURE unset)");
    return;
  }
  let result = focusWindow(TEST_CLASS);
  assertNotASyntaxError(result, "hl.dsp.focus");
}

#[test]
fn resizeLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    eprintln!("skipping: no live Hyprland (HYPRLAND_INSTANCE_SIGNATURE unset)");
    return;
  }
  let result = resizeWindow(TEST_CLASS, 60, 40);
  assert!(result.is_ok(), "hl.dsp.window.resize call failed: {:?}", result.err());
}

#[test]
fn repositionLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    eprintln!("skipping: no live Hyprland (HYPRLAND_INSTANCE_SIGNATURE unset)");
    return;
  }
  let result = repositionWindow(TEST_CLASS, "top", 100, 25, 36);
  assert!(result.is_ok(), "hl.dsp.window.move (reposition) call failed: {:?}", result.err());
}

#[test]
fn launchInBackgroundLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    eprintln!("skipping: no live Hyprland (HYPRLAND_INSTANCE_SIGNATURE unset)");
    return;
  }
  // "true" exits immediately with no window, so this can't leave anything behind on the desktop.
  let result = launchInBackground(&["true".to_string()]);
  assert!(result.is_ok(), "hl.exec_cmd call failed: {:?}", result.err());
}
