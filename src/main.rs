#![allow(non_snake_case)] // This must be at the very top of main.rs

use std::process;

use hdrop_rs::cli::parseArgs;
use hdrop_rs::inspect::{check, explain, listWindows};
use hdrop_rs::notify::notifyError;
use hdrop_rs::toggle::{Outcome, run};

fn main() {
  let args = parseArgs();

  let result = if args.listWindows {
    listWindows().map(Outcome::from)
  } else if args.check {
    check().map(Outcome::from)
  } else if args.explain && !args.command.is_empty() {
    explain(&args).map(Outcome::from)
  } else {
    run(&args)
  };
  match result {
    Ok(outcome) => process::exit(outcome.exitCode()),
    Err(error) => {
      notifyError(&error.to_string());
      process::exit(Outcome::Failed.exitCode());
    }
  }
}
