# hdrop-rs

A Rust port of [hdrop](https://github.com/Schweber/hdrop), a dropdown-window helper for Hyprland (think Guake/Yakuake, but for any application). Given a command, it toggles a window of that program between hidden (moved to a special workspace) and shown (moved to your current workspace, optionally floating at a specific position/size) - launching it if it isn't running yet.

This targets Hyprland's Lua-based config/dispatch (>=0.55), with no backward compatibility for older versions. Actually tested on Hyprland 0.56.2 and 0.57.0.

## Why a rewrite

The original bash script works, but Hyprland >=0.55 replaced several of the classic `hyprctl dispatch` commands it depends on with a new Lua-based config/dispatch system, and the bash script's handling of that transition has real, silent bugs (see [Notes on Hyprland's Lua dispatch protocol](#notes-on-hyprlands-lua-dispatch-protocol) below) - most notably, on Hyprland 0.56.2, its per-toggle window resize and focus calls fail on every single invocation. This port fixes those, adds desktop notifications so failures aren't silently lost when launched from a keybind, and is covered by tests that catch it early if Hyprland changes this protocol again.

## Usage

```
hdrop-rs [OPTIONS] COMMAND...
hdrop-rs --explain [OPTIONS] COMMAND...
hdrop-rs --list-windows
hdrop-rs --check
```

`COMMAND` is the program you'd normally run to start the application (e.g. `kitty`, `firefox --class Work`). hdrop-rs' own options must come *before* it: everything from `COMMAND` on is passed to the program (so in `hdrop-rs kitty -w 80`, the `-w 80` goes to kitty).

Each press does one of four things with the matching window: **show** it (move it to the current workspace, resize/position it if floating, focus it), **hide** it (move it to `special:hdrop`), **focus** it (with `-F` or `--focus-first`), or **launch** the program if nothing matches.

### Toggle options

| Flag | Description |
|---|---|
| `-b`, `--background` | If the program isn't running yet, launch it hidden (into the `special:hdrop` workspace) instead of on the current workspace. |
| `-c`, `--class <CLASS>` | Match windows of this class instead of the one inferred from `COMMAND`. Takes precedence over everything below. It does **not** change the class the program gets - pass the program's own flag for that (e.g. `kitty --class X`). |
| `-f`, `--floating` | Spawn as a floating window. Default is top half, full width, no gap; adjust with `-p`/`-g`/`-h`/`-w`, any of which also implies `-f`. |
| `-F`, `--focus` | Focus the window (switching to its workspace if necessary) instead of moving or hiding it. With several matching windows, one on the current workspace is preferred. |
| `--focus-first` | If the window is on the current workspace but not focused, focus it instead of hiding it; the next press hides it. Without it, every press on a visible window hides it. |
| `-g`, `--gap <GAP>` | Floating (implies `-f`): gap to the screen edge, in pixels. Default `0`. |
| `-h`, `--height <HEIGHT>` | Floating (implies `-f`): window height, as a percentage `1`-`100`. Default `50`, or `100` for left/right. |
| `-H`, `--help` | Print the help message. |
| `--launch-timeout <SECONDS>` | How long a launch waits for its window (`1`-`600`, default `10`); see "Behavior worth knowing". Raise it for slow-starting programs. |
| `-i`, `--insensitive` | Case-insensitive partial matching of class names, for when a running program isn't recognized by its exact class. |
| `-o`, `--online` | Delay the initial launch for up to 20s (total, including retries) until internet connectivity is established (an HTTP 204 from a connectivity check). |
| `-p`, `--position <POSITION>` | Floating (implies `-f`): `top`/`bottom`/`left`/`right` (or `t`/`b`/`l`/`r`). Default `top`. |
| `-v`, `--verbose` | Explain how the class was resolved and what was done (stderr and notifications) - useful when a running program isn't being recognized. |
| `-V`, `--version` | Print the hdrop-rs package version (the upstream hdrop it ports is 0.7.10). |
| `-w`, `--width <WIDTH>` | Floating (implies `-f`): window width, as a percentage `1`-`100`. Default `100`, or `50` for left/right. |

### Inspection options

These only read state and never change anything on the desktop.

| Flag | Description |
|---|---|
| `--explain` | Print what this invocation would do - resolved class and why, matching windows, the chosen action and geometry - without doing anything. Takes the same options and `COMMAND` as a real press. |
| `--list-windows` | List open windows (class, workspace, title), to find the class to use. Takes no `COMMAND`. |
| `--check` | Check the setup, one `ok`/`warn`/`FAIL` line per item: Hyprland reachable and a tested version (too old = `FAIL`), Lua dispatch understood, curl (for `-o`), notification daemon, lock directory. Takes no `COMMAND`. |

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Done (or, for the inspection options, report printed and nothing failed). |
| `1` | Something failed and was reported: a window action, a launch that failed to start or exited before opening a window, a `FAIL` in `--check`, or a Hyprland/hyprctl error. |
| `2` | Invalid command line (also shown as a notification). |
| `3` | The program was launched, but its window didn't appear within `--launch-timeout` - not necessarily a failure (slow start, or a class mismatch). |

### Multiple instances

To run several instances of the same program with different classes, `hdrop-rs` looks for a class/app-id flag embedded in `COMMAND` itself:

- `foot`: `-a`/`--app-id` (e.g. `hdrop-rs foot -a work-term`)
- everything else: `--class` (e.g. `hdrop-rs firefox --class Work`)

Scanning stops at `--` or `-e`, so a flag meant for the child command of a terminal (`hdrop-rs kitty -- app --class X`) isn't mistaken for the terminal's own class.

### Hardcoded class overrides

A handful of programs report a window class that doesn't match their command name, so it's special-cased: `epiphany` → `org.gnome.Epiphany`, `logseq` → `Logseq`, `telegram-desktop` → `org.telegram.desktop`, `tor-browser` → `Tor Browser`, `godot4` → matched by live partial match against `org.godotengine.*`. Only the executable's name counts, so `/usr/bin/epiphany` or a Nix store path to `foot` are recognized the same as the bare names.

### Debugging a binding

`--explain` runs the same class resolution and decision as a real press (the same code path, so it can't disagree with what a press does), prints it, and changes nothing - no launch, no window rules, no moves:

```
$ hdrop-rs --explain -w 100 -h 25 -g 36 kitty --class Guake
command:   kitty --class Guake
class:     Guake (from the --class flag in the command)
matches:   0x636d89df31d0 on special:hdrop (hidden) - djmanas@Rincewind:~
floating:  rule 100% x 25% at top, gap 36
action:    show 0x636d89df31d0: move from special:hdrop to 4, resize to 2256x376 at (0, 40), then focus
```

`--list-windows` shows which classes are open:

```
$ hdrop-rs --list-windows
CLASS                WORKSPACE      TITLE
dev.zed.Zed          4              hdrop-rs — README.md
Guake                special:hdrop  djmanas@Rincewind:~
```

`--check` reports whether the environment is usable:

```
$ hdrop-rs --check
ok    Hyprland: 0.57.0 (tested)
ok    Lua dispatch: hl.dsp.focus understood
ok    curl: curl 8.23.0 (used by -o/--online)
ok    notifications: dunst 1.13.2
ok    lock directory: /run/user/1000 (XDG_RUNTIME_DIR)
```

`-v` is the live counterpart: during a real press it reports the resolved class and the action taken, on stderr and as notifications.

### Behavior worth knowing

- **Launching waits for the window, not for the program.** When the program isn't running, hdrop-rs launches it and waits up to `--launch-timeout` (default 10s) for its window, then exits. A foreground launch keeps running on its own - unlike upstream hdrop, which blocked until the program exited (relevant if a script relied on that).
  - A program that can't be started, or exits unsuccessfully before opening a window, is an error (exit 1), and the next press tries again.
  - A launcher that exits successfully and hands off to another process is fine: its window still confirms the launch.
  - If no window appears in time, you get a notification naming the class it waited for - usually a class mismatch (see `--class`) or a slow program - and exit code 3. For another `--launch-timeout` after that, the program counts as still starting: presses don't launch a duplicate, they say it's still starting. That ends early once its window appears, or once the launched process exits (foreground launches).
- **Presses during a startup join it.** Invocations for one class are serialized with a lock file in `$XDG_RUNTIME_DIR`, which also records the launch state. A press while the program is starting returns immediately instead of queueing, so repeated presses on a slow program open it once and leave it as launched (shown, or hidden with `-b`) - not toggled away the moment it appears. Ordinary toggles of an existing window still run one after another.
- **Only mapped windows count**, for matching, `-i`, and confirming a launch.
- **The floating rule is per class, and the latest invocation decides it.** `-f` installs a Hyprland window rule for the whole class, so every new window of that class opens floating; an invocation without `-f` turns hdrop-rs' rule for that class off again. So with two bindings for one class, one with `-f` and one without, whichever ran last decides how the *next new* window opens. Turning the rule off doesn't re-tile a window that's already floating. Use a dedicated class (e.g. `kitty --class Guake`) for dropdowns.
  - The rule is updated before a launch (it must be in place for the new window). For show/hide/focus it's updated afterwards, and a failure there is reported without undoing or blocking the action.
  - Each distinct geometry (`-w`/`-h`/`-p`/`-g` combination) gets its own rule, kept until Hyprland's config is reloaded. That's a handful for fixed bindings; something generating many different geometries would accumulate rules until the next reload.
- **Hidden windows live in `special:hdrop`**, both when hidden by a press and when launched with `-b`. They keep running while hidden. To see all hidden dropdowns at once, bind `togglespecialworkspace hdrop`. (Versions before 0.5.0 hid into the default `special:special` workspace but launched into `special:hdrop`; such a window is simply shown from wherever it is on the next press.)
- **Hiding only moves the window**; size and position are applied when it's shown, for the monitor of the workspace it's shown on.
- **With several windows of one class**, a press shows a hidden one first, then one on another workspace, and only hides when the matches are all on the current workspace. `-F` prefers the opposite: one already on the current workspace.
- **Errors are always visible**: on stderr and as a desktop notification (falling back to `hyprctl notify`), including command-line mistakes, which would otherwise be silent from a bind.
- **`-i` refuses to guess**: an exact or case-insensitive equal class wins; otherwise a partial match is only used if exactly one class contains it. With several candidates it's an error that lists them - pass the one you mean with `--class` (upstream hdrop silently took the first match).

### Example: a Guake-style dropdown terminal

Bind this to a key in your Hyprland config. `--class Guake` comes *after* `kitty`, so it's kitty's own flag: kitty's window gets class `Guake`, and hdrop-rs infers that class from it.

```
bind = , F12, exec, hdrop-rs -f -w 100 -h 25 -g 36 kitty --class Guake
```

And, optionally, autostart it hidden at login so the first press is instant. Use the same geometry flags as the bind, so the first show doesn't jump between two sizes:

```
exec-once = hdrop-rs -bf -w 100 -h 25 -g 36 kitty --class Guake
```

Putting `--class Guake` *before* `kitty` would only tell hdrop-rs to look for a `Guake` window, while kitty would still start as `kitty` - so each press would launch another terminal.

## Installing

### NixOS / home-manager (flake)

```nix
{
  inputs.hdrop-rs.url = "path:/path/to/hdrop-rs"; # or a git URL once pushed
}
```

Then add `inputs.hdrop-rs.packages.${system}.default` to `environment.systemPackages` or a home-manager `home.packages` list.

Packaging note: `hyprland-rs` is pinned to `git branch = "master"` in `Cargo.toml` (no published release supports Hyprland's post-0.55 Lua dispatch protocol yet), so `flake.nix` carries an explicit `cargoLock.outputHashes` entry for it. Whenever `cargo update -p hyprland` moves that pin, the hash needs updating too:

```
nix run nixpkgs#nix-prefetch-git -- --url https://github.com/hyprland-community/hyprland-rs --rev <new-rev>
```

### From source

```
cargo build --release
```

## Development

```
nix develop        # or: use shell.nix
cargo test         # unit tests + live-compositor integration tests
cargo test -- --ignored --test-threads=1   # live effect tests: open and kill real kitty windows, move focus
cargo clippy --all-targets -- -D warnings
```

The live effect tests need `kitty` and run one at a time because they share focus. Their focus checks poll briefly instead of checking once, since with `misc.focus_on_activate` another app (e.g. the editor running the tests) may take focus back a moment later.

The integration tests in `tests/hyprctl_lua_dispatch.rs` talk to a real, running Hyprland instance and skip cleanly if `HYPRLAND_INSTANCE_SIGNATURE` isn't set (e.g. in CI, or when building the Nix package in its sandbox).

## Notes on Hyprland's Lua dispatch protocol

As of Hyprland 0.56.2, every `hyprctl dispatch <name> <args>` call is run through a Lua interpreter (`hl.dispatch(...)`) rather than parsed as classic dispatcher syntax - so calls like `focuswindow class:^x$` or `resizewindowpixel exact 100% 25%,class:^x$` fail outright, and this isn't limited to the couple of dispatchers that obviously changed shape. `hyprland-rs` (checked against its git `master` branch) has no support at all for this - its dispatch-sending code still only emits the classic wire format, and nothing in its changelog, README, or commit history mentions the change.

The working replacements below were reverse engineered against a live Hyprland 0.56.2 (partly from its bundled Lua API stub at `/nix/store/.../hyprland-<version>/share/hypr/stubs/hl.meta.lua`, partly by trial and error against the running compositor) - they are not documented, stable API, and could change on any Hyprland update:

| Purpose | Call |
|---|---|
| Float + initial size/position for a class | `hyprctl eval "hl.window_rule({name='hdrop-rs:X:<size>:<move>', match={class='^X$'}, float=true, size='<expr>', move='<expr>'})"` - only fires the *first* time a window becomes floating, not on later toggles. The class is regex-escaped. Unnamed rules pile up on every call, and a named one gets its effects appended, so hdrop-rs creates each distinct class+geometry rule once, keeps the returned handles in a Lua global, and switches geometry with `rule:set_enabled(false/true)` (a config reload wipes the global along with the rules). `move` is monitor-local, then clamped into the work area (below bars) |
| Move between workspaces | `hyprctl dispatch "hl.dsp.window.move({workspace=\"name\"\|\"special:hdrop\", follow=false, window=\"address:0x...\"})"` |
| Reposition an already-floating window | Same dispatcher, different keys: `{window=\"address:0x...\", x=<px>, y=<px>}` - numeric pixels only, in *global* layout coordinates (add the monitor origin), and `monitor_w`/`monitor_h` aren't available here like they are in `hl.window_rule`'s `move` string. Not clamped to the work area, so hdrop-rs clamps it itself to match the rule |
| Focus a window | `hl.dsp.focus({window=\"address:0x...\"})` - top-level, *not* `hl.dsp.window.focus` |
| Resize a window | `hl.dsp.window.resize({window=\"address:0x...\", x=<px>, y=<px>})` - numeric pixels only |
| Background-launch into a hidden workspace | `hyprctl eval "hl.exec_cmd(\"cmd\", {workspace=\"special:hdrop silent\"})"` - `silent` must be a modifier inside the workspace string, not its own key. `cmd` goes through `/bin/sh -c`, so every argument is shell-quoted |

Every action of a toggle targets the selected window by `address:`, never by class, so with several windows of one class all actions hit the window that was inspected (see "Behavior worth knowing" for which one is picked). All strings are written as full Lua string literals (control characters escaped).

Since these are undocumented and could shift, `tests/hyprctl_lua_dispatch.rs` exercises each of them against a live compositor (using a window address that can never exist) and fails on anything but success or "window not found", rather than letting it fail silently at runtime. That only proves the calls are accepted; `cargo test -- --ignored` additionally opens disposable kitty windows and checks the actual effects (hidden-workspace placement, initial size/position, rule switching, quoted arguments, show/hide, resize, reposition), the real binary's launch -> show + focus -> `--focus-first` -> hide flow, and startup handling (simultaneous presses, presses during a slow startup, an unconfirmed startup staying pending, a failed launch being retried, a launcher handing off to another process), then kills them.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
