//! Encoding values for the Lua and regex strings hdrop-rs sends to Hyprland.

/// Encodes `value` as a complete double-quoted Lua string literal (quotes included). Every byte
/// that could end the literal or isn't printable ASCII is written as a decimal escape, so
/// newlines/control characters can't break the surrounding Lua and non-ASCII UTF-8 round-trips.
pub fn luaString(value: &str) -> String {
  let mut literal = String::with_capacity(value.len() + 2);
  literal.push('"');
  for byte in value.bytes() {
    match byte {
      b'"' => literal.push_str("\\\""),
      b'\\' => literal.push_str("\\\\"),
      0x20..=0x7e => literal.push(byte as char),
      // Always 3 digits, so a following literal digit can't be absorbed into the escape.
      _ => literal.push_str(&format!("\\{byte:03}")),
    }
  }
  literal.push('"');
  literal
}

/// Escapes a literal string for Hyprland's (RE2) regex matching, e.g. `term.work` must not also
/// match `termXwork`.
pub fn regexEscape(value: &str) -> String {
  let mut escaped = String::with_capacity(value.len());
  for character in value.chars() {
    if "\\.+*?()|[]{}^$".contains(character) {
      escaped.push('\\');
    }
    escaped.push(character);
  }
  escaped
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn luaStringEscapesQuotesBackslashesAndControlCharacters() {
    assert_eq!(luaString("plain"), r#""plain""#);
    assert_eq!(luaString(r#"a"b\c"#), r#""a\"b\\c""#);
    assert_eq!(luaString("line\nbreak"), r#""line\010break""#);
    // A digit after an escape must not be swallowed into it.
    assert_eq!(luaString("\u{1}2"), r#""\0012""#);
    assert_eq!(luaString("ž"), r#""\197\190""#);
  }

  #[test]
  fn regexEscapeMakesClassesLiteral() {
    assert_eq!(regexEscape("org.gnome.Epiphany"), r"org\.gnome\.Epiphany");
    assert_eq!(regexEscape("term[1]|x"), r"term\[1\]\|x");
    assert_eq!(regexEscape("Tor Browser"), "Tor Browser");
  }
}
