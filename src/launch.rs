//! Starting the program: connectivity wait, per-class lock, launch, and waiting for its window.

use std::fs::{self, File, TryLockError};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::HIDDEN_WORKSPACE;
use crate::hyprctl::{getClients, hyprctlEval};
use crate::lua::luaString;

/// Overall budget for --online, including all retries.
const ONLINE_DEADLINE: Duration = Duration::from_secs(20);

/// Default for --launch-timeout: how long a launch waits for its window before reporting that its
/// startup couldn't be confirmed.
pub const DEFAULT_LAUNCH_TIMEOUT_SECS: u64 = 10;

/// What the per-class lock file records about the latest launch, so that presses arriving while a
/// program starts join that startup instead of replaying later (a second press would otherwise
/// hide the window the moment it appears) or launching a duplicate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LaunchState {
  /// The invocation holding the lock is launching right now.
  Starting,
  /// That launch's window appeared at `at` (ms since the epoch).
  Launched { at: u64 },
  /// That launch gave up waiting for its window. Until `until`, or until `pid` (a foreground
  /// launch) has exited, the program counts as still starting: no second launch.
  Pending { until: u64, pid: Option<u32> },
}

impl LaunchState {
  fn encode(&self) -> String {
    match self {
      LaunchState::Starting => "starting".to_string(),
      LaunchState::Launched { at } => format!("launched {at}"),
      LaunchState::Pending { until, pid } => format!("pending {until} {}", pid.map_or("-".to_string(), |pid| pid.to_string())),
    }
  }

  /// `None` for an empty or unreadable file (e.g. caught mid-write): treated as "no state".
  fn decode(text: &str) -> Option<LaunchState> {
    let mut words = text.split_whitespace();
    match (words.next()?, words.next(), words.next()) {
      ("starting", None, None) => Some(LaunchState::Starting),
      ("launched", Some(at), None) => Some(LaunchState::Launched { at: at.parse().ok()? }),
      ("pending", Some(until), Some(pid)) => Some(LaunchState::Pending { until: until.parse().ok()?, pid: if pid == "-" { None } else { Some(pid.parse().ok()?) } }),
      _ => None,
    }
  }

  /// For a still valid `Pending`, how much of it is left; `None` otherwise.
  pub fn pendingRemaining(&self, now: u64) -> Option<Duration> {
    match *self {
      LaunchState::Pending { until, pid } if until > now && pid.is_none_or(processAlive) => Some(Duration::from_millis(until - now)),
      _ => None,
    }
  }
}

/// Exclusive per-class lock, which also carries the class' `LaunchState` as its file content.
/// Released when dropped (or when the process dies).
pub struct ClassLock {
  _file: File,
  path: PathBuf,
}

impl ClassLock {
  pub fn state(&self) -> Option<LaunchState> {
    readLaunchState(&self.path)
  }

  pub fn setState(&self, state: Option<LaunchState>) -> std::io::Result<()> {
    fs::write(&self.path, state.map(|state| state.encode()).unwrap_or_default())
  }
}

/// Blocks for up to 20s in total waiting for connectivity, mirroring bash hdrop's `wait_online`.
/// Only an actual 204 counts as online, so a captive portal answering 200/302 doesn't. Returns
/// whether connectivity was seen; like bash, callers launch anyway either way. Errors only if curl
/// itself can't be run, instead of retrying a missing binary for the whole budget.
pub fn waitOnline() -> Result<bool, Box<dyn std::error::Error>> {
  let deadline = Instant::now() + ONLINE_DEADLINE;
  loop {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
      return Ok(false);
    }
    let output = process::Command::new("curl")
      .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "-m", &format!("{:.1}", remaining.as_secs_f32()), "http://connectivitycheck.gstatic.com/generate_204"])
      .output()
      .map_err(|error| format!("--online needs curl: {error}"))?;
    if output.stdout == b"204" {
      return Ok(true);
    }
    // Failures like "no route" return instantly; don't spin on them.
    thread::sleep(Duration::from_secs(1).min(deadline.saturating_duration_since(Instant::now())));
  }
}

pub fn nowMillis() -> u64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as u64).unwrap_or_default()
}

fn processAlive(pid: u32) -> bool {
  Path::new(&format!("/proc/{pid}")).exists()
}

fn lockPath(class: &str) -> PathBuf {
  let directory = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
  // Collisions after sanitizing only over-serialize, never break anything.
  let fileName: String = class.chars().map(|character| if character.is_ascii_alphanumeric() || "._-".contains(character) { character } else { '_' }).collect();
  directory.join(format!("hdrop-rs-{fileName}.lock"))
}

fn readLaunchState(path: &Path) -> Option<LaunchState> {
  LaunchState::decode(&fs::read_to_string(path).ok()?)
}

fn openLock(class: &str) -> Result<(File, PathBuf), Box<dyn std::error::Error>> {
  let path = lockPath(class);
  let file = File::options().create(true).truncate(false).write(true).open(&path).map_err(|error| format!("can't open lock file {}: {error}", path.display()))?;
  Ok((file, path))
}

/// Serializes hdrop-rs invocations for `class`, so two quick key presses can't both see "not
/// running" and launch twice. Blocks until the lock is free.
pub fn lockClass(class: &str) -> Result<ClassLock, Box<dyn std::error::Error>> {
  let (file, path) = openLock(class)?;
  file.lock()?;
  Ok(ClassLock { _file: file, path })
}

/// Like `lockClass`, but `None` instead of blocking if another invocation holds the lock.
pub fn tryLockClass(class: &str) -> Result<Option<ClassLock>, Box<dyn std::error::Error>> {
  let (file, path) = openLock(class)?;
  match file.try_lock() {
    Ok(()) => Ok(Some(ClassLock { _file: file, path })),
    Err(TryLockError::WouldBlock) => Ok(None),
    Err(TryLockError::Error(error)) => Err(error.into()),
  }
}

/// The class' `LaunchState` read without taking the lock (only meaningful while someone holds it,
/// e.g. to see that the holder is mid-launch).
pub fn peekLaunchState(class: &str) -> Option<LaunchState> {
  readLaunchState(&lockPath(class))
}

/// Starts `command` as a child of hdrop-rs. Only a failure to start it is an error here: how the
/// program eventually exits (closing foot/kitty can yield a non-zero status or a signal) isn't
/// hdrop's business - bash hdrop special-cased foot for exactly that.
pub fn launchForeground(command: &[String]) -> Result<process::Child, Box<dyn std::error::Error>> {
  let (program, programArgs) = command.split_first().ok_or("no command given")?;
  process::Command::new(program).args(programArgs).spawn().map_err(|error| {
    if error.kind() == ErrorKind::NotFound { format!("command not found: '{program}'").into() } else { format!("can't start '{program}': {error}").into() }
  })
}

/// Quotes one argument for POSIX `sh`: single quotes make everything literal, and an embedded `'`
/// is closed, escaped and reopened.
pub fn shellQuote(argument: &str) -> String {
  format!("'{}'", argument.replace('\'', "'\\''"))
}

/// Launches `command` into the hidden `HIDDEN_WORKSPACE` without switching to it, via
/// `hl.exec_cmd` (the working replacement for the classic `exec [workspace ... silent] cmd`,
/// which fails like the classic dispatchers do, see `dispatch::focusWindow`). `silent` has to be a
/// modifier inside the workspace string itself (`"special:hdrop silent"`) - passing it as its
/// own `silent = true` key errors with "unknown effect 'silent'". Confirmed live: a real `kitty`
/// launched this way lands directly in `special:hdrop`, never appearing on the active workspace.
///
/// `hl.exec_cmd` runs its string through `/bin/sh -c` (confirmed live on 0.57), so each argument
/// is shell-quoted to keep the same argument boundaries as `launchForeground`.
pub fn launchInBackground(command: &[String]) -> Result<(), Box<dyn std::error::Error>> {
  let commandLine: Vec<String> = command.iter().map(|argument| shellQuote(argument)).collect();
  let luaCall = format!(
    "hl.exec_cmd({}, {{workspace = {}}})",
    luaString(&commandLine.join(" ")),
    luaString(&format!("{HIDDEN_WORKSPACE} silent"))
  );
  hyprctlEval(&luaCall)
}

/// Waits up to `timeout` for a (mapped) window of `class` to appear after a launch. If `child` (a
/// foreground launch) exits unsuccessfully before that, it's reported as the launch failing; a
/// successful early exit is fine (e.g. a launcher handing off to an already running instance).
pub fn waitForWindow(class: &str, mut child: Option<&mut process::Child>, timeout: Duration) -> Result<bool, Box<dyn std::error::Error>> {
  let deadline = Instant::now() + timeout;
  loop {
    if getClients()?.iter().any(|client| client.mapped && client.class == class) {
      return Ok(true);
    }
    if let Some(child) = child.as_deref_mut()
      && let Some(status) = child.try_wait()?
      && !status.success()
    {
      return Err(format!("the command exited with {status} before opening a window").into());
    }
    if Instant::now() >= deadline {
      return Ok(false);
    }
    thread::sleep(Duration::from_millis(100));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn launchStateRoundTripsThroughItsFileFormat() {
    for state in [LaunchState::Starting, LaunchState::Launched { at: 1_793_600_000_000 }, LaunchState::Pending { until: 5, pid: Some(4242) }, LaunchState::Pending { until: 5, pid: None }] {
      assert_eq!(LaunchState::decode(&state.encode()), Some(state));
    }
    // Empty (fresh lock file, or caught mid-write) and garbage both mean "no state".
    assert_eq!(LaunchState::decode(""), None);
    assert_eq!(LaunchState::decode("pending soon"), None);
  }

  #[test]
  fn pendingExpiresByTimeOrWhenItsProcessIsGone() {
    let ownPid = Some(process::id());
    assert_eq!(LaunchState::Pending { until: 2_000, pid: ownPid }.pendingRemaining(1_500), Some(Duration::from_millis(500)));
    assert_eq!(LaunchState::Pending { until: 2_000, pid: None }.pendingRemaining(1_500), Some(Duration::from_millis(500)));
    assert_eq!(LaunchState::Pending { until: 2_000, pid: ownPid }.pendingRemaining(2_000), None);
    // No such process (pid_max is at most 2^22).
    assert_eq!(LaunchState::Pending { until: 2_000, pid: Some(u32::MAX) }.pendingRemaining(1_500), None);
    assert_eq!(LaunchState::Launched { at: 1 }.pendingRemaining(0), None);
  }

  #[test]
  fn shellQuoteKeepsArgumentBoundariesAndMetacharactersLiteral() {
    let command = ["kitty", "--title", "Work terminal", "", "it's", "$HOME;echo injected"];
    let quoted: Vec<String> = command.iter().map(|argument| shellQuote(argument)).collect();
    let output = process::Command::new("sh").arg("-c").arg(format!("printf '<%s>' {}", quoted[1..].join(" "))).output().unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "<--title><Work terminal><><it's><$HOME;echo injected>");
  }
}
