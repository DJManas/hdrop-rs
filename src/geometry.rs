//! Floating window placement: positions, sizes and work-area clamping.

use crate::hyprctl::Monitor;

/// --floating placement; parsed by clap, so an invalid value fails even without -f.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq)]
pub enum Position {
  #[value(alias = "t")]
  Top,
  #[value(alias = "b")]
  Bottom,
  #[value(alias = "l")]
  Left,
  #[value(alias = "r")]
  Right,
}

impl std::fmt::Display for Position {
  /// The CLI spelling (`top`, ...), as clap parses it.
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let value = clap::ValueEnum::to_possible_value(self).expect("no skipped variants");
    formatter.write_str(value.get_name())
  }
}

impl Monitor {
  /// Logical (layout) size: `width`/`height` are the unrotated mode in physical pixels, so odd
  /// transforms (90/270 degrees, flipped or not) swap the axes, and scale divides both.
  pub fn logicalSize(&self) -> (f32, f32) {
    let (width, height) = if self.transform % 2 == 1 { (self.height, self.width) } else { (self.width, self.height) };
    (width as f32 / self.scale, height as f32 / self.scale)
  }

  /// Clamps a window's monitor-local top-left so the window stays inside the work area (the
  /// monitor minus `reserved`), the same way Hyprland 0.57's `fitBoxInWorkArea` treats a
  /// window rule's `move`. Without this the first launch sits below a bar while every later
  /// toggle's explicit reposition would put it underneath.
  pub fn clampToWorkArea(&self, x: i32, y: i32, windowWidth: i32, windowHeight: i32) -> (i32, i32) {
    let (monitorWidth, monitorHeight) = self.logicalSize();
    let [left, top, right, bottom] = self.reserved;
    let clampAxis = |value: i32, size: i32, start: i32, end: i32| value.min(end - size).max(start);
    (
      clampAxis(x, windowWidth, left, monitorWidth.round() as i32 - right),
      clampAxis(y, windowHeight, top, monitorHeight.round() as i32 - bottom),
    )
  }
}

/// Width/height percentages and the Hyprland Lua `move` expression for a --floating window.
///
/// `monitor_w`/`monitor_h` in `expr` are left as literal identifiers: they're resolved by
/// Hyprland's Lua config engine against the target monitor when the window rule fires, so we
/// don't need to query monitor geometry ourselves for this part.
pub struct FloatingPosition {
  pub widthPct: u16,
  pub heightPct: u16,
  pub expr: String,
  pub sizeExpr: String,
}

pub fn floatingPosition(position: Position, width: Option<u16>, height: Option<u16>, gap: u16) -> FloatingPosition {
  let (widthPct, heightPct, expr) = match position {
    Position::Top => {
      let w = width.unwrap_or(100);
      let h = height.unwrap_or(50);
      (w, h, format!("(monitor_w/2*(1-{w}/100)) ({gap})"))
    }
    Position::Bottom => {
      let w = width.unwrap_or(100);
      let h = height.unwrap_or(50);
      (w, h, format!("(monitor_w/2*(1-{w}/100)) (monitor_h*(1-({h}/100))-{gap})"))
    }
    Position::Left => {
      let w = width.unwrap_or(50);
      let h = height.unwrap_or(100);
      (w, h, format!("({gap}) (monitor_h/2*(1-{h}/100))"))
    }
    Position::Right => {
      let w = width.unwrap_or(50);
      let h = height.unwrap_or(100);
      (w, h, format!("(monitor_w*(1-({w}/100))-{gap}) (monitor_h/2*(1-{h}/100))"))
    }
  };
  let sizeExpr = format!("(monitor_w*{widthPct}/100) (monitor_h*{heightPct}/100)");
  FloatingPosition { widthPct, heightPct, expr, sizeExpr }
}

/// Same formulas as `floatingPosition`'s `expr`, but resolved to actual pixels against a given
/// monitor size instead of left as a `monitor_w`/`monitor_h` Lua expression. Needed because
/// `hl.window_rule`'s `move` only re-fires the first time a window becomes floating, not on every
/// subsequent workspace toggle - so re-centering an already-floating window on each toggle needs
/// an explicit `hl.dsp.window.move` call with real numbers (confirmed live: that call has no
/// `monitor_w`/`monitor_h` globals available, unlike the window_rule context).
pub fn floatingPixelPosition(position: Position, widthPct: u16, heightPct: u16, gap: u16, monitorWidth: f32, monitorHeight: f32) -> (i32, i32) {
  let w = widthPct as f32;
  let h = heightPct as f32;
  let gap = gap as f32;
  let (x, y) = match position {
    Position::Top => (monitorWidth / 2.0 * (1.0 - w / 100.0), gap),
    Position::Bottom => (monitorWidth / 2.0 * (1.0 - w / 100.0), monitorHeight * (1.0 - h / 100.0) - gap),
    Position::Left => (gap, monitorHeight / 2.0 * (1.0 - h / 100.0)),
    Position::Right => (monitorWidth * (1.0 - w / 100.0) - gap, monitorHeight / 2.0 * (1.0 - h / 100.0)),
  };
  (x.round() as i32, y.round() as i32)
}

/// A shown floating window's box on `monitor`, in global layout pixels.
#[derive(Debug, PartialEq)]
pub struct WindowBox {
  pub x: i32,
  pub y: i32,
  pub width: i32,
  pub height: i32,
}

/// Where and how big a floating window is put when shown: percentages of the monitor's logical
/// size, positioned per `floatingPixelPosition`, kept inside the work area, then offset by the
/// monitor origin, since absolute `hl.dsp.window.move` coordinates are global (Hyprland 0.57
/// `Actions::move` diffs against the window's global position).
pub fn floatingBox(monitor: &Monitor, position: Position, widthPct: u16, heightPct: u16, gap: u16) -> WindowBox {
  let (monitorWidth, monitorHeight) = monitor.logicalSize();
  let width = (monitorWidth * widthPct as f32 / 100.0).round() as i32;
  let height = (monitorHeight * heightPct as f32 / 100.0).round() as i32;
  let (x, y) = floatingPixelPosition(position, widthPct, heightPct, gap, monitorWidth, monitorHeight);
  let (x, y) = monitor.clampToWorkArea(x, y, width, height);
  WindowBox { x: monitor.x + x, y: monitor.y + y, width, height }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn topPositionPixelsIgnoreReservedAreaAndUseGapAsIs() {
    // A 2256x1504 monitor, matching the machine this was verified against.
    let (x, y) = floatingPixelPosition(Position::Top, 100, 25, 36, 2256.0, 1504.0);
    assert_eq!((x, y), (0, 36));
  }

  #[test]
  fn bottomPositionAnchorsToMonitorHeightMinusGap() {
    let (x, y) = floatingPixelPosition(Position::Bottom, 100, 25, 36, 2256.0, 1504.0);
    assert_eq!((x, y), (0, 1504 - 376 - 36));
  }

  #[test]
  fn leftAndRightPositionsUseGapAsXOffset() {
    let (leftX, _) = floatingPixelPosition(Position::Left, 50, 100, 20, 2256.0, 1504.0);
    assert_eq!(leftX, 20);
    let (rightX, _) = floatingPixelPosition(Position::Right, 50, 100, 20, 2256.0, 1504.0);
    assert_eq!(rightX, 2256 - 1128 - 20);
  }

  #[test]
  fn rotatedMonitorSwapsLogicalAxes() {
    let monitor = Monitor { name: "eDP-1".to_string(), x: 0, y: 0, width: 2560, height: 1440, scale: 2.0, transform: 1, reserved: [0; 4], focused: true };
    assert_eq!(monitor.logicalSize(), (720.0, 1280.0));
    let monitor = Monitor { transform: 2, ..monitor };
    assert_eq!(monitor.logicalSize(), (1280.0, 720.0));
  }

  #[test]
  fn floatingBoxIsOffsetByTheMonitorOrigin() {
    let monitor = Monitor { name: "eDP-1".to_string(), x: 2256, y: 0, width: 1920, height: 1080, scale: 1.0, transform: 0, reserved: [0, 30, 0, 0], focused: true };
    // Top, 50% x 25%, gap 10: x centered, y pushed below the 30px bar, both offset by x = 2256.
    assert_eq!(floatingBox(&monitor, Position::Top, 50, 25, 10), WindowBox { x: 2256 + 480, y: 30, width: 960, height: 270 });
  }

  #[test]
  fn clampToWorkAreaKeepsWindowsOutOfReservedSpace() {
    let monitor = Monitor { name: "eDP-1".to_string(), x: 0, y: 0, width: 2256, height: 1504, scale: 1.0, transform: 0, reserved: [0, 40, 0, 30], focused: true };
    // Top gap smaller than the bar: pushed below the bar.
    assert_eq!(monitor.clampToWorkArea(564, 10, 1128, 451), (564, 40));
    // Bottom placement overlapping a bottom bar: pushed up.
    assert_eq!(monitor.clampToWorkArea(0, 1200, 2256, 300), (0, 1504 - 30 - 300));
    // Already inside: untouched.
    assert_eq!(monitor.clampToWorkArea(100, 100, 200, 200), (100, 100));
  }
}
