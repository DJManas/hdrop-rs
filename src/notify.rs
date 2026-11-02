//! Reporting errors and diagnostics: stderr plus desktop notifications.

use std::process;

use notify_rust::{Notification, Urgency};

/// Prints `message` to stderr and shows it as a desktop notification, mirroring bash hdrop's
/// `notify()`: try a D-Bus notification first, fall back to `hyprctl notify` if there's no
/// notification daemon. Since hdrop is normally launched from a keybind with no visible terminal,
/// the notification is the only way most errors are ever seen; stderr covers terminal use and the
/// case where neither notification path works.
pub fn notifyError(message: &str) {
  eprintln!("hdrop: {message}");
  notifyDesktop(message);
}

/// Same delivery as `notifyError`, for things the user should see that aren't failures (e.g. a
/// press ignored because the program is still starting).
pub fn notifyInfo(message: &str) {
  eprintln!("hdrop: {message}");
  notifyDesktop(message);
}

/// The notification half of `notifyError`, for messages already printed elsewhere (clap errors).
pub fn notifyDesktop(message: &str) {
  if Notification::new().summary("hdrop").body(message).show().is_ok() {
    return;
  }
  let _ = process::Command::new("hyprctl").args(["notify", "0", "5000", "0", &format!("hdrop: {message}")]).output();
}

/// Low-urgency counterpart to `notifyError`, for --verbose diagnostics (mirrors bash's
/// `notify_low`). Always printed to stderr; the notification is best-effort only.
pub fn notifyVerbose(message: &str) {
  eprintln!("hdrop: {message}");
  let _ = Notification::new().summary("hdrop").body(message).urgency(Urgency::Low).show();
}
