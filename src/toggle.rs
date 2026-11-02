//! One key press: decide what to do with the matching window, then do it.

use std::process;

use clap::CommandFactory;

use crate::HIDDEN_WORKSPACE;
use crate::class::resolveClass;
use crate::cli::CliArgs;
use crate::dispatch::{focusWindow, moveWindowToWorkspace, repositionWindow, resizeWindow};
use crate::geometry::{FloatingPosition, floatingPosition};
use crate::hyprctl::{Client, Monitor, getActiveWindowAddress, getActiveWorkspace, getClients, getMonitorOfWorkspace};
use crate::launch::{ClassLock, LaunchState, launchForeground, launchInBackground, lockClass, nowMillis, peekLaunchState, tryLockClass, waitForWindow, waitOnline};
use crate::notify::{notifyError, notifyInfo, notifyVerbose};
use crate::rules::{applyFloatingRule, disableFloatingRule};

/// How an invocation ended, and its exit code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
  /// Everything requested was done (or a press joined a startup already in progress).
  Done,
  /// Something failed and was reported.
  Failed,
  /// The program was launched, but its window didn't appear within --launch-timeout. Not proof of
  /// failure (slow start, or a class mismatch), hence its own code.
  Unconfirmed,
}

impl Outcome {
  pub fn exitCode(self) -> i32 {
    match self {
      Outcome::Done => 0,
      Outcome::Failed => 1,
      Outcome::Unconfirmed => 3,
    }
  }
}

impl From<bool> for Outcome {
  fn from(succeeded: bool) -> Outcome {
    if succeeded { Outcome::Done } else { Outcome::Failed }
  }
}

/// What an invocation does, decided from one snapshot of the windows. Shared by the real run and
/// --explain, so the explanation can't drift from the behavior.
#[derive(Debug, PartialEq)]
pub(crate) enum Action<'a> {
  /// Bring the window to the current workspace (resizing/positioning it if floating) and focus it.
  Show(&'a Client),
  /// Only focus it, switching to its workspace if needed (-F, or --focus-first on an unfocused one).
  Focus(&'a Client),
  /// Move it to the hidden workspace.
  Hide(&'a Client),
  /// Nothing matches: start the command.
  Launch,
}

/// Picks the window an invocation acts on when several share `class`. Ties go to the first in
/// `hyprctl clients` order. Unmapped clients never count: they aren't a usable window.
///
/// Toggling mirrors bash hdrop's order: a window elsewhere (preferring one hidden in
/// `HIDDEN_WORKSPACE`) is brought here before one already on the active workspace is hidden.
/// With `preferVisible` (--focus) it's the reverse, since focusing a window that's already here
/// beats switching workspaces or opening the hidden one: active workspace, then any other
/// regular workspace, then hidden.
pub fn selectClient<'a>(clients: &'a [Client], class: &str, activeWorkspace: &str, preferVisible: bool) -> Option<&'a Client> {
  let matching = || clients.iter().filter(move |client| client.mapped && client.class == class);
  let (first, notSecond) = if preferVisible { (activeWorkspace, HIDDEN_WORKSPACE) } else { (HIDDEN_WORKSPACE, activeWorkspace) };
  matching()
    .find(|client| client.workspace.address() == first)
    .or_else(|| matching().find(|client| client.workspace.address() != notSecond))
    .or_else(|| matching().next())
}

/// `activeWindow` is only consulted for --focus-first (the caller may skip querying it otherwise).
pub(crate) fn decideAction<'a>(args: &CliArgs, clients: &'a [Client], class: &str, activeWorkspace: &str, activeWindow: Option<&str>) -> Action<'a> {
  match selectClient(clients, class, activeWorkspace, args.focus) {
    Some(client) if client.workspace.address() != activeWorkspace => {
      if args.focus { Action::Focus(client) } else { Action::Show(client) }
    }
    Some(client) => {
      let unfocused = activeWindow != Some(client.address.as_str());
      if args.focus || (args.focusFirst && unfocused) { Action::Focus(client) } else { Action::Hide(client) }
    }
    None => Action::Launch,
  }
}

/// Notifies on failure and keeps going, mirroring bash hdrop's `<dispatch> || notify ...`: a
/// failed resize or focus shouldn't stop the rest of the toggle from being attempted. Returns
/// whether it succeeded, so the process can still exit non-zero afterwards.
fn notifyOnError<T, E: std::fmt::Display>(result: Result<T, E>, context: &str) -> bool {
  match result {
    Ok(_) => true,
    Err(error) => {
      notifyError(&format!("{context}: {error}"));
      false
    }
  }
}

/// Brings the window here: move, then (floating) size and position for `monitor` - the one the
/// destination workspace is on, resolved together with it - then focus. Nothing after a failed
/// move, since it would act on the window where it still is.
fn showWindow(args: &CliArgs, window: &str, workspace: &str, monitor: impl FnOnce() -> Result<Monitor, Box<dyn std::error::Error>>, floatingPos: Option<&FloatingPosition>) -> bool {
  if !notifyOnError(moveWindowToWorkspace(window, workspace), "Error moving window to current workspace") {
    return false;
  }
  let mut succeeded = true;
  if let Some(position) = floatingPos {
    match monitor() {
      Ok(monitor) => {
        succeeded &= notifyOnError(resizeWindow(window, &monitor, position.widthPct, position.heightPct), "Error resizing window");
        succeeded &= notifyOnError(repositionWindow(window, &monitor, args.position(), position.widthPct, position.heightPct, args.gap()), "Error repositioning window");
      }
      Err(error) => succeeded &= notifyOnError(Err::<(), _>(error), "Error reading monitor geometry"),
    }
  }
  succeeded & notifyOnError(focusWindow(window), "Error focusing window")
}

/// Installs (with -f) or turns off hdrop-rs' floating rule for `class`, which affects windows of
/// the class opened from now on.
fn updateFloatingRule(class: &str, floatingPos: Option<&FloatingPosition>) -> Result<(), Box<dyn std::error::Error>> {
  match floatingPos {
    Some(position) => applyFloatingRule(class, position),
    None => disableFloatingRule(class),
  }
}

/// Takes the class lock, or returns `None` when this press should just join a startup that's
/// already in progress: either the holder is mid-launch right now, or a launch finished while
/// this press was waiting for the lock. Short ordinary toggles still serialize normally.
fn lockOrJoinStartup(args: &CliArgs, class: &str) -> Result<Option<ClassLock>, Box<dyn std::error::Error>> {
  let waitStart = nowMillis();
  let lock = match tryLockClass(class)? {
    Some(lock) => lock,
    None if peekLaunchState(class) == Some(LaunchState::Starting) => {
      if args.verbose {
        notifyVerbose(&format!("'{}' is already starting; this press joins that startup", args.command.join(" ")));
      }
      return Ok(None);
    }
    None => lockClass(class)?,
  };
  match lock.state() {
    Some(LaunchState::Launched { at }) if at >= waitStart => {
      if args.verbose {
        notifyVerbose(&format!("'{}' finished starting while this press waited; nothing more to do", args.command.join(" ")));
      }
      return Ok(None);
    }
    // We hold the lock, so whoever wrote this is gone without cleaning up.
    Some(LaunchState::Starting) => lock.setState(None)?,
    _ => {}
  }
  Ok(Some(lock))
}

/// One key press: resolve the class, decide, act. Failed window actions are reported as they
/// happen and turn the outcome into `Failed`, but the remaining useful steps are still attempted.
pub fn run(args: &CliArgs) -> Result<Outcome, Box<dyn std::error::Error>> {
  if args.command.is_empty() {
    notifyError("Missing Argument\nRun 'hdrop-rs -H' for more information");
    CliArgs::command().print_help()?;
    // A usage error, like clap's own.
    process::exit(2);
  }

  let resolved = resolveClass(&args.command, args.class.as_deref(), args.insensitive)?;
  if args.verbose {
    notifyVerbose(&format!("Matching windows of class '{}' ({})", resolved.class, resolved.reason));
  }
  let class = resolved.class;
  let Some(lock) = lockOrJoinStartup(args, &class)? else {
    return Ok(Outcome::Done);
  };

  let floatingPos = args.isFloating().then(|| floatingPosition(args.position(), args.width, args.height, args.gap()));
  let activeWorkspace = getActiveWorkspace()?;
  let clients = getClients()?;
  let activeWindow = if args.focusFirst { getActiveWindowAddress()? } else { None };
  let action = decideAction(args, &clients, &class, activeWorkspace.address(), activeWindow.as_deref());

  if action == Action::Launch {
    if let Some(remaining) = lock.state().and_then(|state| state.pendingRemaining(nowMillis())) {
      notifyInfo(&format!(
        "'{}' is still starting (no window of class '{class}' yet); not launching it again for another {}s",
        args.command.join(" "),
        remaining.as_secs().max(1)
      ));
      return Ok(Outcome::Done);
    }
    // A new launch needs the rule in place (or gone) before its window appears.
    updateFloatingRule(&class, floatingPos.as_ref())?;
    let outcome = launch(args, &class, &lock, &clients);
    if outcome.is_err() {
      let _ = lock.setState(None);
    }
    return outcome;
  }

  // A matching window exists, so any recorded pending startup is over.
  if matches!(lock.state(), Some(LaunchState::Pending { .. })) {
    let _ = lock.setState(None);
  }
  let mut succeeded = true;
  match action {
    Action::Show(client) => {
      let monitor = || getMonitorOfWorkspace(&activeWorkspace);
      succeeded &= showWindow(args, &client.selector(), activeWorkspace.address(), monitor, floatingPos.as_ref());
      if args.verbose && succeeded {
        notifyVerbose(&format!("Matched class '{class}' on workspace '{}' and moved it to the current workspace", client.workspace.address()));
      }
    }
    Action::Focus(client) => {
      succeeded &= notifyOnError(focusWindow(&client.selector()), "Error focusing window");
      if args.verbose && succeeded {
        notifyVerbose(&format!("Matched class '{class}' on workspace '{}' and focused it", client.workspace.address()));
      }
    }
    Action::Hide(client) => {
      // Only a move: geometry is applied when it's shown again, against that moment's monitor.
      succeeded &= notifyOnError(moveWindowToWorkspace(&client.selector(), HIDDEN_WORKSPACE), "Error hiding window");
      if args.verbose && succeeded {
        notifyVerbose(&format!("Matched class '{class}' on current workspace and moved it to workspace '{HIDDEN_WORKSPACE}'"));
      }
    }
    Action::Launch => unreachable!("handled above"),
  }
  // Only affects future windows of the class, so a failure here mustn't undo or block the action.
  if let Err(error) = updateFloatingRule(&class, floatingPos.as_ref()) {
    notifyError(&format!("Couldn't update the floating rule for new '{class}' windows: {error}"));
  }
  Ok(succeeded.into())
}

/// Starts the program (lock held, state `Starting` while it runs) and waits for its window:
/// confirmed -> `Launched`, so presses that waited meanwhile don't act again; not confirmed in time
/// -> `Pending` for another --launch-timeout, so further presses don't launch a duplicate. A known
/// failure (spawn error, early unsuccessful exit) is an `Err`, and the caller clears the state.
fn launch(args: &CliArgs, class: &str, lock: &ClassLock, clients: &[Client]) -> Result<Outcome, Box<dyn std::error::Error>> {
  lock.setState(Some(LaunchState::Starting))?;
  if args.online {
    if args.verbose {
      notifyVerbose("Waiting up to 20s for internet connectivity");
    }
    match waitOnline() {
      Ok(true) => {}
      Ok(false) => notifyVerbose("No internet connectivity after 20s, launching anyway"),
      Err(error) => notifyError(&format!("{error}; launching without waiting")),
    }
  }
  let mut child = if args.background {
    launchInBackground(&args.command)?;
    None
  } else {
    Some(launchForeground(&args.command)?)
  };
  if args.verbose {
    let mut activeClasses: Vec<&str> = clients.iter().filter(|client| client.mapped).map(|client| client.class.as_str()).collect();
    activeClasses.sort();
    activeClasses.dedup();
    notifyVerbose(&format!(
      "No running program matches class '{class}'. Currently active classes are '{}'. Executed '{}'.",
      activeClasses.join(" "),
      args.command.join(" ")
    ));
  }
  let timeout = args.launchTimeout();
  let appeared = waitForWindow(class, child.as_mut(), timeout).map_err(|error| format!("'{}': {error}", args.command.join(" ")))?;
  // A foreground launch keeps running on its own; hdrop doesn't wait for it to exit.
  let pid = child.as_ref().map(|child| child.id());
  if appeared {
    lock.setState(Some(LaunchState::Launched { at: nowMillis() }))?;
    return Ok(Outcome::Done);
  }
  lock.setState(Some(LaunchState::Pending { until: nowMillis() + timeout.as_millis() as u64, pid }))?;
  notifyError(&format!(
    "Started '{}', but its startup couldn't be confirmed: no window of class '{class}' within {}s. Presses in the next {}s won't launch it again. If the program registers a different class, set it with the program's own flag (e.g. 'kitty --class {class}') or match it with --class; --explain shows what a press would do. A slow program may need a longer --launch-timeout.",
    args.command.join(" "),
    timeout.as_secs(),
    timeout.as_secs()
  ));
  Ok(Outcome::Unconfirmed)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn client(address: &str, class: &str, workspace: &str) -> Client {
    serde_json::from_value(serde_json::json!({
      "address": address, "class": class, "title": "", "mapped": true,
      "workspace": { "address": workspace, "name": workspace }
    }))
    .unwrap()
  }

  fn parse(arguments: &[&str]) -> CliArgs {
    use clap::Parser;
    CliArgs::try_parse_from(std::iter::once("hdrop-rs").chain(arguments.iter().copied())).unwrap()
  }

  #[test]
  fn selectClientPrefersHiddenThenElsewhereThenActive() {
    let clients = [client("0x1", "kitty", "1"), client("0x2", "kitty", "3"), client("0x3", "kitty", HIDDEN_WORKSPACE), client("0x4", "foot", "3")];
    assert_eq!(selectClient(&clients, "kitty", "1", false).unwrap().address, "0x3");
    assert_eq!(selectClient(&clients[..2], "kitty", "1", false).unwrap().address, "0x2");
    assert_eq!(selectClient(&clients[..1], "kitty", "1", false).unwrap().address, "0x1");
    assert!(selectClient(&clients, "alacritty", "1", false).is_none());
  }

  #[test]
  fn unmappedClientsAreNeverSelected() {
    let mut unmapped = client("0x9", "kitty", HIDDEN_WORKSPACE);
    unmapped.mapped = false;
    let clients = [unmapped, client("0x2", "kitty", "3")];
    assert_eq!(selectClient(&clients, "kitty", "1", false).unwrap().address, "0x2");
    assert!(selectClient(&clients[..1], "kitty", "1", false).is_none());
    assert_eq!(decideAction(&parse(&["kitty"]), &clients[..1], "kitty", "1", None), Action::Launch);
  }

  #[test]
  fn selectClientForFocusPrefersActiveThenRegularThenHidden() {
    let clients = [client("0x3", "kitty", HIDDEN_WORKSPACE), client("0x2", "kitty", "3"), client("0x1", "kitty", "1")];
    assert_eq!(selectClient(&clients, "kitty", "1", true).unwrap().address, "0x1");
    assert_eq!(selectClient(&clients[..2], "kitty", "1", true).unwrap().address, "0x2");
    assert_eq!(selectClient(&clients[..1], "kitty", "1", true).unwrap().address, "0x3");
  }

  #[test]
  fn decideActionCoversShowHideFocusAndLaunch() {
    let here = [client("0x1", "kitty", "1")];
    let elsewhere = [client("0x2", "kitty", "3")];
    let plain = parse(&["kitty"]);
    assert_eq!(decideAction(&plain, &elsewhere, "kitty", "1", None), Action::Show(&elsewhere[0]));
    assert_eq!(decideAction(&plain, &here, "kitty", "1", None), Action::Hide(&here[0]));
    assert_eq!(decideAction(&plain, &here, "foot", "1", None), Action::Launch);

    let focus = parse(&["-F", "kitty"]);
    assert_eq!(decideAction(&focus, &elsewhere, "kitty", "1", None), Action::Focus(&elsewhere[0]));
    assert_eq!(decideAction(&focus, &here, "kitty", "1", Some("0x1")), Action::Focus(&here[0]));

    let focusFirst = parse(&["--focus-first", "kitty"]);
    assert_eq!(decideAction(&focusFirst, &here, "kitty", "1", Some("0x9")), Action::Focus(&here[0]));
    assert_eq!(decideAction(&focusFirst, &here, "kitty", "1", None), Action::Focus(&here[0]));
    assert_eq!(decideAction(&focusFirst, &here, "kitty", "1", Some("0x1")), Action::Hide(&here[0]));
    assert_eq!(decideAction(&focusFirst, &elsewhere, "kitty", "1", Some("0x9")), Action::Show(&elsewhere[0]));
  }
}
