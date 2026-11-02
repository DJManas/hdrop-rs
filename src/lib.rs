#![allow(non_snake_case)] // This must be at the very top of lib.rs

//! hdrop-rs: toggle a program's window between hidden and shown on Hyprland.
//!
//! - `cli`: the command line; `toggle`: one key press (decide, then act); `inspect`: the
//!   read-only modes (--explain, --list-windows, --check).
//! - `class`: which window class a command means; `launch`: starting the program;
//!   `rules`: the floating window rule; `dispatch`: acting on one window; `geometry`: floating
//!   placement.
//! - `hyprctl`: reading Hyprland state and running Lua; `lua`: string encoding; `notify`: errors
//!   and diagnostics.

pub mod class;
pub mod cli;
pub mod dispatch;
pub mod geometry;
pub mod hyprctl;
pub mod inspect;
pub mod launch;
pub mod lua;
pub mod notify;
pub mod rules;
pub mod toggle;

/// The hidden workspace hdrop parks windows in, both when hiding and when launching with
/// --background. Must be the same named special workspace in both places, as in bash hdrop.
pub const HIDDEN_WORKSPACE: &str = "special:hdrop";
