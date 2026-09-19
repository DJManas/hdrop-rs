#![allow(non_snake_case, warnings)] // This must be at the very top of main.rs

use std::process;

use clap::{CommandFactory, Parser, ArgAction};

use hyprland::data::{Clients, Workspace};
use hyprland::prelude::*;
use hyprland::shared::{HyprData, HyprDataActive};

use hdrop_rs::{applyFloatingRule, floatingPosition, focusWindow, launchForeground, launchInBackground, moveWindowToWorkspace, notifyError, notifyVerbose, repositionWindow, resizeWindow, resolveClass, waitOnline};

/// Usage: hdrop [OPTIONS] [COMMAND]
#[derive(Parser, Debug)]
#[command(author, version = "0.7.10", about = "A way to drop down applications", long_about = None)]
#[command(disable_help_flag = true)] // 1. Disable the default -h and --help
struct CliArgs {
  /// Print this help message
  #[arg(short = 'H', long, action = ArgAction::Help)]
  help: Option<bool>,

  /// Launch in the background instead of foreground
  #[arg(short, long)]
  background: bool,

  /// Set classname of the program to be run.
  #[arg(short, long)]
  class: Option<String>,

  /// Spawn as a floating window
  #[arg(short, long)]
  floating: bool,

  /// Focus the program.
  #[arg(short='F', long)]
  focus: bool,

  /// Case insensitive partial matching of class names.
  #[arg(short, long)]
  insensitive: bool,

  /// If using --floating: specify gap to the screen edge in pixels
  #[arg(short, long, default_value_t = 0)]
  gap: u16,

  /// If using --floating: set the height of the window.
  #[arg(short, long)]
  height: Option<u16>,

  /// If using --floating: set the position of the window. One of '[t]op' '[b]ottom' '[l]eft' '[r]ight'.
  #[arg(short, long, default_value = "top")]
  position: String,

  /// If using --floating: set the width of the window.
  #[arg(short, long)]
  width: Option<u16>,

  /// Delay initial launch for up to 20 seconds until internet connectivity is established.
  #[arg(short = 'o', long)]
  online: bool,

  /// Show notifications regarding the matching process.
  #[arg(short, long)]
  verbose: bool,

  /// The usual command you would run to start the desired program
  #[arg(trailing_var_arg = true)]
  command: Vec<String>,
}

/// Notifies on failure and keeps going, mirroring bash hdrop's `<dispatch> || notify ...`: a
/// failed resize or focus shouldn't stop the rest of the toggle from being attempted.
fn notifyOnError<T, E: std::fmt::Display>(result: Result<T, E>, context: &str) {
  if let Err(error) = result {
    notifyError(&format!("{context}: {error}"));
  }
}

fn run(args: &CliArgs) -> Result<(), Box<dyn std::error::Error>> {
  if args.command.is_empty() {
    notifyError("Missing Argument\nRun 'hdrop-rs -H' for more information");
    CliArgs::command().print_help()?;
    process::exit(1);
  }

  let class = resolveClass(&args.command, args.class.as_deref(), args.insensitive, args.verbose)?;

  let floatingPos = if args.floating {
    let position = floatingPosition(&args.position, args.width, args.height, args.gap)?;
    applyFloatingRule(&class, &position.expr)?;
    Some(position)
  } else {
    None
  };

  let activeWorkspace = Workspace::get_active()?;
  let clients = Clients::get()?;
  let matchedClient = clients.iter().find(|client| client.class == class);

  match matchedClient {
    Some(client) if client.workspace.name != activeWorkspace.name => {
      if !args.focus {
        notifyOnError(moveWindowToWorkspace(&class, &activeWorkspace.name), "Error moving window to current workspace");
        if let Some(position) = &floatingPos {
          notifyOnError(resizeWindow(&class, position.widthPct, position.heightPct), "Error resizing window");
          notifyOnError(repositionWindow(&class, &args.position, position.widthPct, position.heightPct, args.gap), "Error repositioning window");
        }
        if args.verbose {
          notifyVerbose(&format!("Matched class '{class}' on another workspace and moved it to current workspace"));
        }
      }
      notifyOnError(focusWindow(&class), "Error focusing window");
    }
    Some(_) => {
      if !args.focus {
        notifyOnError(moveWindowToWorkspace(&class, "special"), "Error hiding window");
        if let Some(position) = &floatingPos {
          notifyOnError(resizeWindow(&class, position.widthPct, position.heightPct), "Error resizing window");
          notifyOnError(repositionWindow(&class, &args.position, position.widthPct, position.heightPct, args.gap), "Error repositioning window");
        }
        if args.verbose {
          notifyVerbose(&format!("Matched class '{class}' on current workspace and moved it to workspace 'special:hdrop'"));
        }
      } else {
        notifyOnError(focusWindow(&class), "Error focusing window");
      }
    }
    None => {
      if args.online {
        waitOnline();
      }
      if args.background {
        launchInBackground(&args.command)?;
      } else {
        launchForeground(&args.command)?;
      }
      if args.verbose {
        let mut activeClasses: Vec<&str> = clients.iter().filter(|client| client.mapped).map(|client| client.class.as_str()).collect();
        activeClasses.sort();
        notifyVerbose(&format!(
          "No running program matches class '{class}'. Currently active classes are '{}'. Executed '{}' in case it was not running already.",
          activeClasses.join(" "),
          args.command.join(" ")
        ));
      }
    }
  }

  Ok(())
}

fn main() {
  let args = CliArgs::parse();

  if let Err(error) = run(&args) {
    let message = error.to_string();
    notifyError(&message);
    eprintln!("hdrop: {message}");
    process::exit(1);
  }
}
