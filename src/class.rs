//! Working out which window class an invocation is about.

use std::path::Path;

use crate::hyprctl::getClients;

/// The command's executable name without its directory, for class inference only (the original,
/// possibly absolute, path is what gets launched): `/run/current-system/sw/bin/foot` is `foot`.
pub fn programName(command: &[String]) -> &str {
  let program = command.first().map(String::as_str).unwrap_or_default();
  Path::new(program).file_name().and_then(|name| name.to_str()).unwrap_or(program)
}

/// Extracts the class the spawned program will register under, from a flag embedded in its own
/// arguments: `-a`/`--app-id` for `foot`, `--class` for everything else. Accepts both
/// `--class value` and `--class=value` forms.
///
/// Scanning stops at `--` and `-e`, the usual "the rest is the child command" separators of
/// terminals, so `kitty -- sh -c ... --class Child` stays `kitty`. Without a separator a generic
/// scanner can't know where an arbitrary program's own options end.
pub fn extractEmbeddedClass(command: &[String]) -> Option<String> {
  let isFoot = programName(command) == "foot";
  let longFlag = if isFoot { "--app-id" } else { "--class" };
  let shortFlag = if isFoot { Some("-a") } else { None };

  let mut args = command.iter().skip(1).take_while(|arg| *arg != "--" && *arg != "-e");
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

/// Picks the open window class `pattern` refers to, from most to least specific: exact, then
/// case-insensitive equality, then a case-insensitive substring - but only if exactly one distinct
/// class contains it. Several candidates are an error listing them, rather than an arbitrary pick
/// that could change with window order.
pub fn matchClassInsensitive(pattern: &str, classes: &[&str]) -> Result<Option<String>, String> {
  if classes.contains(&pattern) {
    return Ok(Some(pattern.to_string()));
  }
  let lowerPattern = pattern.to_lowercase();
  if let Some(equal) = classes.iter().find(|class| class.to_lowercase() == lowerPattern) {
    return Ok(Some(equal.to_string()));
  }
  let mut candidates: Vec<&str> = classes.iter().copied().filter(|class| caseInsensitiveContains(class, pattern)).collect();
  candidates.sort();
  candidates.dedup();
  match candidates.as_slice() {
    [] => Ok(None),
    [only] => Ok(Some(only.to_string())),
    several => Err(format!("'{pattern}' matches several window classes: {}. Pass the exact one with --class.", several.join(", "))),
  }
}

/// `matchClassInsensitive` against the currently open (mapped) windows.
pub fn partialMatchClass(pattern: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
  let clients = getClients()?;
  let classes: Vec<&str> = clients.iter().filter(|client| client.mapped).map(|client| client.class.as_str()).collect();
  Ok(matchClassInsensitive(pattern, &classes)?)
}

/// The class an invocation matches windows by, and how it was arrived at (for -v and --explain).
#[derive(Debug)]
pub struct ResolvedClass {
  pub class: String,
  pub reason: String,
}

/// Resolves the window class to match/apply rules against, mirroring hdrop's precedence:
/// hardcoded per-app overrides, then a class/app-id flag embedded in the command itself,
/// then (if --insensitive) a live partial match, then the top-level --class override (wins always).
pub fn resolveClass(command: &[String], classOverride: Option<&str>, insensitive: bool) -> Result<ResolvedClass, Box<dyn std::error::Error>> {
  // The override wins anyway, so skip the inference steps (and their live client queries).
  if let Some(overrideClass) = classOverride {
    return Ok(ResolvedClass { class: overrideClass.to_string(), reason: "given with --class".to_string() });
  }

  let commandName = programName(command);

  // An embedded flag is checked first only so godot4's live query is skipped when it's present;
  // precedence-wise it still sits after the hardcoded default.
  let (mut class, mut reason) = match extractEmbeddedClass(command) {
    Some(embedded) => {
      let flag = if commandName == "foot" { "-a/--app-id" } else { "--class" };
      (embedded, format!("from the {flag} flag in the command"))
    }
    None => match commandName {
      "epiphany" => ("org.gnome.Epiphany".to_string(), format!("hardcoded for {commandName}")),
      "logseq" => ("Logseq".to_string(), format!("hardcoded for {commandName}")),
      "telegram-desktop" => ("org.telegram.desktop".to_string(), format!("hardcoded for {commandName}")),
      "tor-browser" => ("Tor Browser".to_string(), format!("hardcoded for {commandName}")),
      "godot4" => match partialMatchClass("org.godotengine.")? {
        Some(matched) => (matched, "open window matching 'org.godotengine.' (godot4)".to_string()),
        None => (commandName.to_string(), "the command's name (no open org.godotengine.* window)".to_string()),
      },
      _ => (commandName.to_string(), "the command's name".to_string()),
    },
  };

  if insensitive && let Some(matched) = partialMatchClass(&class)? {
    if matched != class {
      reason = format!("-i matched open class '{matched}' for '{class}' ({reason})");
    }
    class = matched;
  }

  Ok(ResolvedClass { class, reason })
}

#[cfg(test)]
mod tests {
  use super::*;

  // Pure logic only: godot4/--insensitive call partialMatchClass, which needs a real compositor,
  // so only its pure core (matchClassInsensitive) is covered here.

  fn resolve(command: &[&str], classOverride: Option<&str>) -> String {
    let command: Vec<String> = command.iter().map(|s| s.to_string()).collect();
    resolveClass(&command, classOverride, false).unwrap().class
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
  fn embeddedClassScanStopsAtChildCommandSeparators() {
    assert_eq!(resolve(&["kitty", "--", "sh", "-c", "sleep 60", "--class", "Child"], None), "kitty");
    assert_eq!(resolve(&["alacritty", "-e", "app", "--class", "Child"], None), "alacritty");
    assert_eq!(resolve(&["foot", "-a", "drop", "--", "app", "-a", "Child"], None), "drop");
  }

  #[test]
  fn embeddedClassAppliesToHardcodedAppsToo() {
    assert_eq!(resolve(&["epiphany", "--class", "Work"], None), "Work");
    assert_eq!(resolve(&["tor-browser", "--class=Private"], None), "Private");
  }

  #[test]
  fn topLevelOverrideWinsOverEverythingElse() {
    assert_eq!(resolve(&["epiphany"], Some("Forced")), "Forced");
    assert_eq!(resolve(&["firefox", "--class", "Profile"], Some("Forced")), "Forced");
  }

  #[test]
  fn insensitiveMatchPrefersExactThenEqualThenUniquePartial() {
    let classes = ["kitty-work", "kitty", "Firefox", "firefox-dev", "org.telegram.desktop", "org.telegram.desktop"];
    assert_eq!(matchClassInsensitive("kitty", &classes).unwrap().as_deref(), Some("kitty"));
    assert_eq!(matchClassInsensitive("firefox", &classes).unwrap().as_deref(), Some("Firefox"));
    // Duplicate windows of one class aren't ambiguity.
    assert_eq!(matchClassInsensitive("telegram", &classes).unwrap().as_deref(), Some("org.telegram.desktop"));
    assert_eq!(matchClassInsensitive("nothing", &classes).unwrap(), None);
    let error = matchClassInsensitive("fox", &classes).unwrap_err();
    assert!(error.contains("Firefox, firefox-dev"), "{error}");
  }

  #[test]
  fn absolutePathsInferTheSameClassAsBareNames() {
    assert_eq!(resolve(&["/run/current-system/sw/bin/foot", "-a", "drop"], None), "drop");
    assert_eq!(resolve(&["/usr/bin/epiphany"], None), "org.gnome.Epiphany");
    assert_eq!(resolve(&["/usr/bin/kitty"], None), "kitty");
  }
}
