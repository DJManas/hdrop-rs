//! Acting on one window (by `address:` selector) through Hyprland's Lua dispatchers.

use hyprland::dispatch::*;

use crate::geometry::{Position, WindowBox, floatingBox};
use crate::hyprctl::Monitor;
use crate::lua::luaString;

/// Moves the window `selector` to `workspace` via the `hl.dsp.window.move` Lua dispatcher, which
/// replaced `movetoworkspacesilent` for this purpose in Hyprland >=0.55.
pub fn moveWindowToWorkspace(selector: &str, workspace: &str) -> hyprland::Result<()> {
  let luaCall = format!("hl.dsp.window.move({{workspace={}, follow=false, window={}}})", luaString(workspace), luaString(selector));
  Dispatch::call(DispatchType::Custom(&luaCall, ""))
}

/// Focuses the window `selector`.
///
/// Uses `hl.dsp.focus` (a top-level dispatcher, *not* `hl.dsp.window.focus` despite the
/// namespacing of other window dispatchers) since the classic `focuswindow` dispatcher no longer
/// works: as of Hyprland 0.56.2, every `hyprctl dispatch <name> <args>` call is run through a Lua
/// shim (`hl.dispatch(...)`), and classic bareword syntax isn't valid Lua. Confirmed live against
/// this machine's compositor.
pub fn focusWindow(selector: &str) -> hyprland::Result<()> {
  let luaCall = format!("hl.dsp.focus({{window = {}}})", luaString(selector));
  Dispatch::call(DispatchType::Custom(&luaCall, ""))
}

/// Resizes the window `selector` to the given width/height percentages of `monitor`.
///
/// `hl.dsp.window.resize` (the working replacement for the classic `resizewindowpixel`, which
/// fails the same way as `focuswindow`, see `focusWindow`) takes numeric pixel `x`/`y`, not percentages, and
/// doesn't have `monitor_w`/`monitor_h` available like `hl.window_rule`'s `move` string does — so
/// unlike the floating-rule position, this needs the active monitor's real geometry.
pub fn resizeWindow(selector: &str, monitor: &Monitor, widthPct: u16, heightPct: u16) -> Result<(), Box<dyn std::error::Error>> {
  // Position doesn't affect the size.
  let WindowBox { width: widthPx, height: heightPx, .. } = floatingBox(monitor, Position::Top, widthPct, heightPct, 0);
  let luaCall = format!("hl.dsp.window.resize({{window = {}, x = {widthPx}, y = {heightPx}}})", luaString(selector));
  Dispatch::call(DispatchType::Custom(&luaCall, ""))?;
  Ok(())
}

/// Explicitly moves the (already floating) window `selector` to its `floatingBox` position on
/// `monitor`. See `floatingPixelPosition` for why this can't just reuse the `hl.window_rule` move
/// string. Takes the monitor as a parameter so a toggle sizes and positions against one snapshot.
pub fn repositionWindow(selector: &str, monitor: &Monitor, position: Position, widthPct: u16, heightPct: u16, gap: u16) -> Result<(), Box<dyn std::error::Error>> {
  let WindowBox { x: xPx, y: yPx, .. } = floatingBox(monitor, position, widthPct, heightPct, gap);
  let luaCall = format!("hl.dsp.window.move({{window = {}, x = {xPx}, y = {yPx}}})", luaString(selector));
  Dispatch::call(DispatchType::Custom(&luaCall, ""))?;
  Ok(())
}
