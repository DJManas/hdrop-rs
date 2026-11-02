//! The per-class floating window rule (`hl.window_rule`).

use crate::geometry::FloatingPosition;
use crate::hyprctl::hyprctlEval;
use crate::lua::{luaString, regexEscape};

/// Installs the float/size/move rule for windows of `class` (matched literally, not as a regex).
///
/// Unnamed `hl.window_rule`s are registered anew on every call, and a named one is reused but gets
/// its effects appended (Hyprland 0.57, `hlWindowRule`/`addParsedEffect`; the last effect wins).
/// So `hl.window_rule` is called at most once per distinct class+geometry in a compositor session:
/// the returned handles live in a Lua global keyed by the rule text, and switching geometry only
/// disables the class's current rule and enables (or first creates) the wanted one. Alternating
/// bindings (`-h 25`/`-h 50`) thus keep one fixed rule each instead of growing anything. A config
/// reload wipes both the rules and the global, so they can't drift apart.
pub fn applyFloatingRule(class: &str, position: &FloatingPosition) -> Result<(), Box<dyn std::error::Error>> {
  let classKey = luaString(class);
  let classPattern = luaString(&format!("^{}$", regexEscape(class)));
  let size = luaString(&position.sizeExpr);
  let movePos = luaString(&position.expr);
  // Unique per geometry, so hl.window_rule never reuses (and appends to) an existing named rule.
  let ruleName = luaString(&format!("hdrop-rs:{class}:{}:{}", position.sizeExpr, position.expr));
  let luaCall = format!(
    "local s = __hdrop_rs or {{ active = {{}}, rules = {{}} }}; __hdrop_rs = s; \
     local key = {ruleName}; local current = s.active[{classKey}]; \
     if current ~= key then \
       if current then s.rules[current]:set_enabled(false) end; \
       if s.rules[key] then s.rules[key]:set_enabled(true) \
       else s.rules[key] = hl.window_rule({{ name = key, match = {{ class = {classPattern} }}, float = true, size = {size}, move = {movePos} }}) end; \
       s.active[{classKey}] = key \
     end"
  );
  hyprctlEval(&luaCall)
}

/// Disables whichever hdrop-rs floating rule is active for `class` (e.g. to clean up after tests).
pub fn disableFloatingRule(class: &str) -> Result<(), Box<dyn std::error::Error>> {
  let classKey = luaString(class);
  hyprctlEval(&format!(
    "local s = __hdrop_rs; local current = s and s.active[{classKey}]; \
     if current then s.rules[current]:set_enabled(false); s.active[{classKey}] = nil end"
  ))
}
