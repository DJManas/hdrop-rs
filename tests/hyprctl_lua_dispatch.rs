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
//! The default tests only prove each call is *accepted*: they target a window address that can
//! never exist, so nothing on the desktop is touched. The ignored ones (run with
//! `cargo test -- --ignored`) open disposable kitty windows and assert actual effects:
//! `realWindowEffects` drives the library functions (placement, initial size/position, rule
//! switching, argument boundaries, resize/reposition), `cliToggleFlow` runs the real binary through
//! launch -> show (+ focus) -> --focus-first -> hide, and the `cli*` startup tests cover presses
//! during a launch (joining it, not replaying later), an unconfirmed startup staying "pending", a
//! failed launch being retried, and a launcher that hands off to another process.
//! Everything is skipped if there's no live Hyprland (HYPRLAND_INSTANCE_SIGNATURE unset), e.g. in
//! CI or a plain TTY - a skip is printed, and is not evidence of compatibility.

use std::process;
use std::thread;
use std::time::{Duration, Instant};

use hdrop_rs::HIDDEN_WORKSPACE;
use hdrop_rs::dispatch::{focusWindow, moveWindowToWorkspace, repositionWindow, resizeWindow};
use hdrop_rs::geometry::{Position, floatingPosition};
use hdrop_rs::hyprctl::{getActiveMonitor, getActiveWorkspace};
use hdrop_rs::launch::{LaunchState, launchInBackground, peekLaunchState};
use hdrop_rs::rules::{applyFloatingRule, disableFloatingRule};

const NONEXISTENT_WINDOW: &str = "address:0xdeadbeef";
const TEST_CLASS: &str = "__hdrop_rs_integration_test_nonexistent__";

/// Kills every test window of `class` and disables its rule even if an assertion panics halfway.
struct RealWindowCleanup {
  class: String,
}

impl Drop for RealWindowCleanup {
  fn drop(&mut self) {
    for client in clientsJson().iter().filter(|client| client["class"] == self.class.as_str()) {
      if let Some(pid) = client["pid"].as_i64() {
        let _ = process::Command::new("kill").arg(pid.to_string()).output();
      }
    }
    let _ = disableFloatingRule(&self.class);
    // Wait until the windows are really gone: closing a focused window moves focus asynchronously,
    // which would otherwise race the next test's focus assertions.
    let deadline = Instant::now() + Duration::from_secs(3);
    while clientByClass(&self.class).is_some() && Instant::now() < deadline {
      thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(200));
  }
}

fn hasLiveHyprland() -> bool {
  let live = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok();
  if !live {
    eprintln!("skipping: no live Hyprland (HYPRLAND_INSTANCE_SIGNATURE unset)");
  }
  live
}

/// Targeting a window that doesn't exist, "window not found" is the one expected outcome besides
/// success. Anything else - a Lua error, a socket/permission failure, new error wording - fails,
/// so it gets looked at instead of silently passing.
fn assertAcceptedOrWindowNotFound<E: std::fmt::Display>(result: Result<(), E>, label: &str) {
  let Err(error) = result else { return };
  let message = error.to_string();
  assert!(message.contains("window not found"), "{label}: unexpected failure: {message}");
}

fn clientsJson() -> Vec<serde_json::Value> {
  let output = process::Command::new("hyprctl").args(["-j", "clients"]).output().unwrap();
  serde_json::from_slice(&output.stdout).unwrap()
}

fn clientByClass(class: &str) -> Option<serde_json::Value> {
  clientsJson().into_iter().find(|client| client["class"] == class)
}

fn clientByAddress(address: &str) -> serde_json::Value {
  clientsJson().into_iter().find(|client| client["address"] == address).expect("test window vanished")
}

/// Polls until a window of `class` exists, then lets the window rule and initial configure settle.
fn waitForClient(class: &str) -> serde_json::Value {
  let deadline = Instant::now() + Duration::from_secs(10);
  let client = loop {
    if let Some(client) = clientByClass(class) {
      break client;
    }
    assert!(Instant::now() < deadline, "kitty window with class '{class}' never appeared");
    thread::sleep(Duration::from_millis(100));
  };
  thread::sleep(Duration::from_millis(500));
  client
}

fn activeWindowAddress() -> String {
  let output = process::Command::new("hyprctl").args(["-j", "activewindow"]).output().unwrap();
  let window: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
  window["address"].as_str().unwrap_or_default().to_string()
}

fn workspaceOf(client: &serde_json::Value) -> String {
  let workspace = &client["workspace"];
  workspace["address"].as_str().or(workspace["name"].as_str()).unwrap().to_string()
}

fn pair(value: &serde_json::Value) -> (i64, i64) {
  (value[0].as_i64().unwrap(), value[1].as_i64().unwrap())
}

fn assertNear(actual: (i64, i64), expected: (i64, i64), label: &str) {
  let close = (actual.0 - expected.0).abs() <= 2 && (actual.1 - expected.1).abs() <= 2;
  assert!(close, "{label}: got {actual:?}, expected {expected:?}");
}

#[test]
fn windowRuleLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    return;
  }
  let position = floatingPosition(Position::Top, Some(50), Some(25), 0);
  let result = applyFloatingRule(TEST_CLASS, &position);
  assert!(result.is_ok(), "hl.window_rule call failed: {:?}", result.err());
}

#[test]
fn windowMoveLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    return;
  }
  assertAcceptedOrWindowNotFound(moveWindowToWorkspace(NONEXISTENT_WINDOW, HIDDEN_WORKSPACE), "hl.dsp.window.move");
}

#[test]
fn focusLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    return;
  }
  assertAcceptedOrWindowNotFound(focusWindow(NONEXISTENT_WINDOW), "hl.dsp.focus");
}

#[test]
fn resizeLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    return;
  }
  assertAcceptedOrWindowNotFound(resizeWindow(NONEXISTENT_WINDOW, &getActiveMonitor().unwrap(), 60, 40), "hl.dsp.window.resize");
}

#[test]
fn repositionLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    return;
  }
  assertAcceptedOrWindowNotFound(repositionWindow(NONEXISTENT_WINDOW, &getActiveMonitor().unwrap(), Position::Top, 100, 25, 36), "hl.dsp.window.move (reposition)");
}

#[test]
fn launchInBackgroundLuaCallIsAccepted() {
  if !hasLiveHyprland() {
    return;
  }
  // "true" exits immediately with no window, so this can't leave anything behind on the desktop.
  let result = launchInBackground(&["true".to_string()]);
  assert!(result.is_ok(), "hl.exec_cmd call failed: {:?}", result.err());
}

#[test]
#[ignore = "opens a real kitty window (briefly visible on the active workspace); run with --ignored"]
fn realWindowEffects() {
  if !hasLiveHyprland() {
    return;
  }
  let class = format!("hdrop-rs.test-{}", process::id());
  let title = "hdrop rs; $HOME 'quoted' \"double\"";
  let _cleanup = RealWindowCleanup { class: class.clone() };

  // A -> B -> A: B's rule is created after A's, so if switching back didn't really disable B and
  // re-enable A, B's size (last effect wins) would show up below instead of A's.
  let position = floatingPosition(Position::Top, Some(50), Some(30), 10);
  applyFloatingRule(&class, &position).unwrap();
  applyFloatingRule(&class, &floatingPosition(Position::Bottom, Some(80), Some(60), 0)).unwrap();
  applyFloatingRule(&class, &position).unwrap();
  let command = ["kitty", "--class", &class, "--title", title, "sleep", "60"].map(String::from);
  launchInBackground(&command).unwrap();

  let client = waitForClient(&class);
  let address = client["address"].as_str().unwrap().to_string();
  let window = format!("address:{address}");
  let client = clientByAddress(&address);

  assert_eq!(client["title"], title, "argument boundaries/quoting lost on background launch");
  assert_eq!(workspaceOf(&client), HIDDEN_WORKSPACE, "background launch must land in the hidden workspace");
  assert_eq!(client["floating"], true);

  let monitor = getActiveMonitor().unwrap();
  let (monitorWidth, monitorHeight) = monitor.logicalSize();
  let expectedSize = ((monitorWidth * 0.5).round() as i64, (monitorHeight * 0.3).round() as i64);
  assertNear(pair(&client["size"]), expectedSize, "initial size from the window rule");
  let (localX, localY) = monitor.clampToWorkArea((monitorWidth * 0.25).round() as i32, 10, expectedSize.0 as i32, expectedSize.1 as i32);
  let initialAt = pair(&client["at"]);
  assertNear(initialAt, ((monitor.x + localX) as i64, (monitor.y + localY) as i64), "initial position from the window rule");

  let activeWorkspace = getActiveWorkspace().unwrap();
  moveWindowToWorkspace(&window, activeWorkspace.address()).unwrap();
  // Same geometry as the rule: an explicit reposition must land exactly where the first launch did.
  repositionWindow(&window, &monitor, Position::Top, 50, 30, 10).unwrap();
  thread::sleep(Duration::from_millis(300));
  assertNear(pair(&clientByAddress(&address)["at"]), initialAt, "toggle reposition vs. first-launch position");
  resizeWindow(&window, &monitor, 40, 20).unwrap();
  repositionWindow(&window, &monitor, Position::Bottom, 40, 20, 5).unwrap();
  thread::sleep(Duration::from_millis(300));
  let client = clientByAddress(&address);
  assert_eq!(workspaceOf(&client), activeWorkspace.address(), "show must move to the active workspace");
  let expectedSize = ((monitorWidth * 0.4).round() as i64, (monitorHeight * 0.2).round() as i64);
  assertNear(pair(&client["size"]), expectedSize, "resize");
  let (localX, localY) = monitor.clampToWorkArea((monitorWidth * 0.3).round() as i32, (monitorHeight * 0.8).round() as i32 - 5, expectedSize.0 as i32, expectedSize.1 as i32);
  let expectedAt = ((monitor.x + localX) as i64, (monitor.y + localY) as i64);
  assertNear(pair(&client["at"]), expectedAt, "reposition (global coordinates)");

  moveWindowToWorkspace(&window, HIDDEN_WORKSPACE).unwrap();
  thread::sleep(Duration::from_millis(300));
  assert_eq!(workspaceOf(&clientByAddress(&address)), HIDDEN_WORKSPACE, "hide must use the same hidden workspace as launch");
}

/// Runs the real binary for a floating, background-launched kitty of `class`.
fn hdropCommand(class: &str, extraArgs: &[&str]) -> process::Command {
  let mut command = process::Command::new(env!("CARGO_BIN_EXE_hdrop-rs"));
  command.args(extraArgs).args(["-b", "-f", "-w", "50", "-h", "30", "-g", "10", "kitty", "--class", class, "sleep", "60"]);
  command
}

/// The real binary with exactly `arguments`.
fn hdropRaw(arguments: &[&str]) -> process::Command {
  let mut command = process::Command::new(env!("CARGO_BIN_EXE_hdrop-rs"));
  command.args(arguments);
  command
}

/// A kitty of `class` that only maps its window after `delay` seconds (same pid via `exec`, so
/// Hyprland's exec rules still apply), for testing presses during a slow startup.
fn delayedKitty(class: &str, delay: &str) -> String {
  format!("sleep {delay}; exec kitty --class {class} sleep 60")
}

fn windowsOf(class: &str) -> Vec<serde_json::Value> {
  clientsJson().into_iter().filter(|client| client["class"] == class).collect()
}

fn runHdrop(class: &str, extraArgs: &[&str]) {
  let status = hdropCommand(class, extraArgs).status().unwrap();
  assert!(status.success(), "hdrop-rs {extraArgs:?} exited with {status}");
}

/// Polls `current` for up to 1s until it equals `expected`. Checking right away rather than after a
/// fixed sleep matters for focus: with `misc.focus_on_activate`, any app requesting activation
/// (e.g. the editor running these tests) can take focus back a moment after hdrop-rs gave it away.
fn assertEventually(current: impl Fn() -> String, expected: &str, label: &str) {
  let deadline = Instant::now() + Duration::from_secs(1);
  loop {
    let value = current();
    if value == expected {
      return;
    }
    assert!(Instant::now() < deadline, "{label}: got {value:?}, expected {expected:?}");
    thread::sleep(Duration::from_millis(20));
  }
}

#[test]
#[ignore = "opens a real kitty window via the hdrop-rs binary (briefly visible and focused); run with --ignored"]
fn cliToggleFlow() {
  if !hasLiveHyprland() {
    return;
  }
  let class = format!("hdrop-rs.cli-{}", process::id());
  let _cleanup = RealWindowCleanup { class: class.clone() };

  // hdrop-rs itself now waits for the launched window before exiting.
  runHdrop(&class, &[]);
  let client = clientByClass(&class).expect("first run must have waited for its window");
  let address = client["address"].as_str().unwrap().to_string();
  assert_eq!(workspaceOf(&client), HIDDEN_WORKSPACE, "first run (-b) must launch hidden");

  let activeWorkspace = getActiveWorkspace().unwrap();
  runHdrop(&class, &[]);
  assertEventually(activeWindowAddress, &address, "second run must focus it");
  assertEventually(|| workspaceOf(&clientByAddress(&address)), activeWorkspace.address(), "second run must show it on the active workspace");
  assert_eq!(clientsJson().iter().filter(|client| client["class"] == class.as_str()).count(), 1, "toggling must not launch a second instance");

  // --focus-first: visible but unfocused -> focus; focused -> hide.
  // Focus another window on the *same* workspace: one elsewhere would switch workspaces, and then
  // --focus-first rightly shows the dropdown there instead.
  let neighbour = clientsJson().into_iter().find(|client| client["address"] != address.as_str() && client["mapped"] == true && workspaceOf(client) == activeWorkspace.address());
  let neighbour = neighbour.and_then(|client| client["address"].as_str().map(str::to_string));
  if let Some(neighbour) = neighbour
    && focusWindow(&format!("address:{neighbour}")).is_ok()
    && activeWindowAddress() == neighbour
    && workspaceOf(&clientByAddress(&address)) == activeWorkspace.address()
  {
    runHdrop(&class, &["--focus-first"]);
    assertEventually(activeWindowAddress, &address, "--focus-first must focus it");
    assert_eq!(workspaceOf(&clientByAddress(&address)), activeWorkspace.address(), "--focus-first must not hide an unfocused window");
  } else {
    eprintln!("skipping the unfocused --focus-first step: no other window on this workspace to focus");
  }
  // Only meaningful while it's focused; re-focus in case something took focus in the meantime.
  focusWindow(&format!("address:{address}")).unwrap();
  assertEventually(activeWindowAddress, &address, "test window must be focused before the hiding press");
  runHdrop(&class, &["--focus-first"]);
  assertEventually(|| workspaceOf(&clientByAddress(&address)), HIDDEN_WORKSPACE, "--focus-first must hide a focused window");
}

#[test]
#[ignore = "opens a real kitty window via the hdrop-rs binary; run with --ignored"]
fn cliDoublePressLaunchesOnce() {
  if !hasLiveHyprland() {
    return;
  }
  let class = format!("hdrop-rs.double-{}", process::id());
  let _cleanup = RealWindowCleanup { class: class.clone() };

  let mut first = hdropCommand(&class, &[]).spawn().unwrap();
  let mut second = hdropCommand(&class, &[]).spawn().unwrap();
  assert!(first.wait().unwrap().success());
  assert!(second.wait().unwrap().success());

  let windows = windowsOf(&class);
  assert_eq!(windows.len(), 1, "two presses during startup must launch only one window");
  // The second press joined the startup instead of toggling the new window right after (-b, so it
  // stays hidden).
  assert_eq!(workspaceOf(&windows[0]), HIDDEN_WORKSPACE, "a press during startup must not replay as a toggle afterwards");
}

#[test]
#[ignore = "opens a real kitty window via the hdrop-rs binary; run with --ignored"]
fn cliPressesDuringASlowStartupJoinIt() {
  if !hasLiveHyprland() {
    return;
  }
  let class = format!("hdrop-rs.slow-{}", process::id());
  let _cleanup = RealWindowCleanup { class: class.clone() };
  let script = delayedKitty(&class, "1.5");
  let press = || hdropRaw(&["-b", "--class", &class, "sh", "-c", &script]).spawn().unwrap();

  let started = Instant::now();
  let mut first = press();
  thread::sleep(Duration::from_millis(300));
  let (mut second, mut third) = (press(), press());
  assert!(second.wait().unwrap().success() && third.wait().unwrap().success());
  assert!(started.elapsed() < Duration::from_millis(1200), "presses during startup must return right away, not wait for it");
  assert!(first.wait().unwrap().success());

  let windows = windowsOf(&class);
  assert_eq!(windows.len(), 1, "presses during a slow startup must not launch again");
  assert_eq!(workspaceOf(&windows[0]), HIDDEN_WORKSPACE, "presses during startup must not replay as toggles");
  assert_eq!(peekLaunchState(&class).map(|state| matches!(state, LaunchState::Launched { .. })), Some(true));
}

#[test]
#[ignore = "opens a real kitty window via the hdrop-rs binary; run with --ignored"]
fn cliUnconfirmedStartupStaysPending() {
  if !hasLiveHyprland() {
    return;
  }
  let class = format!("hdrop-rs.pending-{}", process::id());
  let _cleanup = RealWindowCleanup { class: class.clone() };
  let script = delayedKitty(&class, "2.5");
  let press = || hdropRaw(&["-b", "--launch-timeout", "1", "--class", &class, "sh", "-c", &script]).status().unwrap();

  assert_eq!(press().code(), Some(3), "a launch whose window doesn't appear in time is 'unconfirmed' (exit 3)");
  assert!(matches!(peekLaunchState(&class), Some(LaunchState::Pending { .. })));
  assert_eq!(press().code(), Some(0), "a press while still pending is not an error");

  let deadline = Instant::now() + Duration::from_secs(6);
  while windowsOf(&class).is_empty() {
    assert!(Instant::now() < deadline, "the slow kitty never appeared");
    thread::sleep(Duration::from_millis(100));
  }
  thread::sleep(Duration::from_millis(500));
  assert_eq!(windowsOf(&class).len(), 1, "a press during the pending period must not launch a duplicate");

  // Now the window exists: a press toggles it as usual and the pending record is gone.
  assert_eq!(press().code(), Some(0));
  assertEventually(|| workspaceOf(&windowsOf(&class)[0]), getActiveWorkspace().unwrap().address(), "once the window exists, a press shows it");
  assert_eq!(peekLaunchState(&class), None);
}

#[test]
#[ignore = "pops error notifications on the desktop; run with --ignored"]
fn cliFailedLaunchIsReportedAndRetried() {
  if !hasLiveHyprland() {
    return;
  }
  let class = format!("hdrop-rs.fail-{}", process::id());
  for attempt in ["first", "second"] {
    let started = Instant::now();
    let status = hdropRaw(&["--class", &class, "false"]).status().unwrap();
    assert_eq!(status.code(), Some(1), "{attempt} press: a command exiting unsuccessfully before any window is a failure");
    assert!(started.elapsed() < Duration::from_secs(3), "{attempt} press: a known failure must not wait for the timeout");
    // No pending record, so the next press really tries again instead of saying "still starting".
    assert_eq!(peekLaunchState(&class), None, "{attempt} press left a launch state behind");
  }
}

#[test]
#[ignore = "opens a real kitty window via the hdrop-rs binary (visible on the active workspace); run with --ignored"]
fn cliLauncherHandoffIsConfirmed() {
  if !hasLiveHyprland() {
    return;
  }
  let class = format!("hdrop-rs.handoff-{}", process::id());
  let _cleanup = RealWindowCleanup { class: class.clone() };
  // The launched process exits successfully right away; the window comes from its background child.
  let script = format!("({}) &", delayedKitty(&class, "1"));
  let status = hdropRaw(&["--class", &class, "sh", "-c", &script]).status().unwrap();
  assert_eq!(status.code(), Some(0), "a launcher handing off to another process must still be confirmed by its window");
  assert_eq!(windowsOf(&class).len(), 1);
}
