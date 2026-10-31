# hdrop-rs

A Rust port of [hdrop](https://github.com/hyprland-community/hdrop), a dropdown-window helper for
Hyprland (think Guake/Yakuake, but for any application). Given a command, it toggles a window of
that program between hidden (moved to a special workspace) and shown (moved to your current
workspace, optionally floating at a specific position/size) - launching it if it isn't running yet.

This targets Hyprland >=0.55 only, with no backward compatibility for older versions.

## Why a rewrite

The original bash script works, but Hyprland >=0.55 replaced several of the classic `hyprctl
dispatch` commands it depends on with a new Lua-based config/dispatch system, and the bash script's
handling of that transition has real, silent bugs (see [Notes on Hyprland's Lua dispatch
protocol](#notes-on-hyprlands-lua-dispatch-protocol) below) - most notably, on Hyprland 0.56.2, its
per-toggle window resize and focus calls fail on every single invocation. This port fixes those,
adds desktop notifications so failures aren't silently lost when launched from a keybind, and is
covered by tests that catch it early if Hyprland changes this protocol again.

## Usage

```
hdrop-rs [OPTIONS] [COMMAND]
```

`COMMAND` is the program you'd normally run to start the application (e.g. `kitty`, `firefox
--class Work`).

| Flag | Description |
|---|---|
| `-b`, `--background` | Launch in the background instead of foreground (via a Hyprland exec rule into the hidden special workspace), if the program isn't running yet. |
| `-c`, `--class <CLASS>` | Override the classname to match against. Takes precedence over everything below. |
| `-f`, `--floating` | Spawn as a floating window. Default is top half, full width, no gap; adjust with `-p`/`-g`/`-h`/`-w`. |
| `-F`, `--focus` | Focus the window and switch to its workspace if necessary, instead of hiding it when it's already on the current workspace. |
| `-g`, `--gap <GAP>` | If using `--floating`: gap to the screen edge, in pixels. Default `0`. |
| `-h`, `--height <HEIGHT>` | If using `--floating`: window height, as a percentage (e.g. `25`). |
| `-H`, `--help` | Print the help message. |
| `-i`, `--insensitive` | Case-insensitive partial matching of class names, for when a running program isn't recognized by its exact class. |
| `-o`, `--online` | Delay the initial launch for up to ~20s until internet connectivity is established. |
| `-p`, `--position <POSITION>` | If using `--floating`: `top`/`bottom`/`left`/`right` (or `t`/`b`/`l`/`r`). Default `top`. |
| `-v`, `--verbose` | Show notifications explaining how the class was resolved and matched - useful when a running program isn't being recognized. |
| `-V`, `--version` | Print the version. |
| `-w`, `--width <WIDTH>` | If using `--floating`: window width, as a percentage (e.g. `100`). |

### Multiple instances

To run several instances of the same program with different classes, `hdrop-rs` looks for a
class/app-id flag embedded in `COMMAND` itself:

- `foot`: `-a`/`--app-id` (e.g. `hdrop-rs foot -a work-term`)
- everything else: `--class` (e.g. `hdrop-rs firefox --class Work`)

### Hardcoded class overrides

A handful of programs report a window class that doesn't match their command name, so it's
special-cased: `epiphany` → `org.gnome.Epiphany`, `logseq` → `Logseq`, `telegram-desktop` →
`org.telegram.desktop`, `tor-browser` → `Tor Browser`, `godot4` → matched by live partial match
against `org.godotengine.*`.

### Example: a Guake-style dropdown terminal

Bind this to a key in your Hyprland config:

```
bind = , F12, exec, hdrop-rs -f -w 100 -h 25 -g 36 --class Guake kitty
```

And, optionally, autostart it hidden at login so the first press is instant:

```
exec-once = hdrop-rs -Fbf -g 36 kitty --class Guake
```

## Installing

### NixOS / home-manager (flake)

```nix
{
  inputs.hdrop-rs.url = "path:/path/to/hdrop-rs"; # or a git URL once pushed
}
```

Then add `inputs.hdrop-rs.packages.${system}.default` to `environment.systemPackages` or a
home-manager `home.packages` list.

Packaging note: `hyprland-rs` is pinned to `git branch = "master"` in `Cargo.toml` (no published
release supports Hyprland's post-0.55 Lua dispatch protocol yet), so `flake.nix` carries an
explicit `cargoLock.outputHashes` entry for it. Whenever `cargo update -p hyprland` moves that pin,
the hash needs updating too:

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
```

The integration tests in `tests/hyprctl_lua_dispatch.rs` talk to a real, running Hyprland instance
and skip cleanly if `HYPRLAND_INSTANCE_SIGNATURE` isn't set (e.g. in CI, or when building the Nix
package in its sandbox).

## Notes on Hyprland's Lua dispatch protocol

As of Hyprland 0.56.2, every `hyprctl dispatch <name> <args>` call is run through a Lua
interpreter (`hl.dispatch(...)`) rather than parsed as classic dispatcher syntax - so calls like
`focuswindow class:^x$` or `resizewindowpixel exact 100% 25%,class:^x$` fail outright, and this
isn't limited to the couple of dispatchers that obviously changed shape. `hyprland-rs` (checked
against its git `master` branch) has no support at all for this - its dispatch-sending code still
only emits the classic wire format, and nothing in its changelog, README, or commit history
mentions the change.

The working replacements below were reverse engineered against a live Hyprland 0.56.2 (partly from
its bundled Lua API stub at `/nix/store/.../hyprland-<version>/share/hypr/stubs/hl.meta.lua`,
partly by trial and error against the running compositor) - they are not documented, stable API,
and could change on any Hyprland update:

| Purpose | Call |
|---|---|
| Float + initial position for a class | `hyprctl eval "hl.window_rule({match={class='^X$'}, float=true, move='<expr>'})"` - only fires the *first* time a window becomes floating, not on later toggles |
| Move between workspaces | `hyprctl dispatch "hl.dsp.window.move({workspace=\"name\"\|\"special\", follow=false, window=\"class:^X$\"})"` |
| Reposition an already-floating window | Same dispatcher, different keys: `{window=\"class:^X$\", x=<px>, y=<px>}` - numeric pixels only, and `monitor_w`/`monitor_h` aren't available here like they are in `hl.window_rule`'s `move` string |
| Focus a window | `hl.dsp.focus({window=\"class:^X$\"})` - top-level, *not* `hl.dsp.window.focus` |
| Resize a window | `hl.dsp.window.resize({window=\"class:^X$\", x=<px>, y=<px>})` - numeric pixels only |
| Background-launch into a hidden workspace | `hyprctl eval "hl.exec_cmd(\"cmd\", {workspace=\"special:hdrop silent\"})"` - `silent` must be a modifier inside the workspace string, not its own key |

Since these are undocumented and could shift, `tests/hyprctl_lua_dispatch.rs` exercises each of
them against a live compositor (using a class that can never match a real window) and fails loudly
if Hyprland stops understanding the call, rather than letting it fail silently at runtime.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
