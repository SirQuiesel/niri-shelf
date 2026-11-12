# !! Vibe Coded !!
# niri-shelf

`niri-shelf` lets you park up to nine windows on a dedicated Niri workspace and
restore one with a keybind or by clicking it in a graphical overview. The
overview is a small local web app opened in Firefox kiosk mode; it is intended
to live on a secondary monitor and shows the stored windows by title.

## Requirements

- Niri
- Rust and Cargo
- Firefox (or another browser that supports `--new-window --kiosk`)
- Two connected outputs if you want to use the secondary-monitor layout

## Install

From the project directory, install both executables under `~/.local/bin`:

```sh
cargo install --path . --root "$HOME/.local"
```

Make sure `~/.local/bin` is on `PATH` in your Niri session. For example, add
this to your shell profile or session environment:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Check that the commands are available:

```sh
command -v niri-shelf-daemon
command -v niri-shelf-ctl
```

## Update

From the project directory, rebuild and replace the installed executables:

```sh
cargo install --path . --root "$HOME/.local" --force
```

Then restart the running shelf daemon so it uses the newly installed binary.
If Niri starts the daemon and overview with `spawn-at-startup`, log out of
Niri and back in. If you started the daemon manually, stop it with `Ctrl+C`
and start `niri-shelf-daemon` again. The overview process should also be
restarted if it was started manually.

Both commands should print paths under your home directory, for example:

```text
/home/alice/.local/bin/niri-shelf-daemon
/home/alice/.local/bin/niri-shelf-ctl
```

If either command prints nothing and returns to the prompt, continue with
[“command not found”](#command-not-found-for-either-executable) below.

## Configure Niri

Copy the relevant sections from
[`examples/niri-config.kdl`](examples/niri-config.kdl) into your Niri config,
usually `~/.config/niri/config.kdl`. The example is a template, not a complete
replacement config. Change `eDP-1` and `DP-2` to the output names reported by:

```sh
niri msg outputs
```

The `shelf` workspace should be assigned to your main output; `shelf-overview`
and the graphical overview window should be assigned to your secondary output.
The example starts the daemon and overview automatically at login.

The sample keybinds are:

| Keybind | Action |
| --- | --- |
| `Mod+Shift+1` … `Mod+Shift+9` | Store the focused window in that slot |
| `Mod+1` … `Mod+9` | Fetch and focus the window in that slot |
| `Mod+Ctrl+1` … `Mod+Ctrl+9` | Swap the focused window with that slot |

These are examples and may conflict with your existing Niri binds (in
particular, `Mod+1` … `Mod+9` are often used for workspaces). Choose unused
combinations and change the `binds` entries accordingly. Each entry invokes
`niri-shelf-ctl` with the action and slot number, for example:

```kdl
Mod+Alt+1 { spawn "niri-shelf-ctl" "fetch" "1"; }
```

Validate the edited configuration and reload it:

```sh
niri validate
niri msg action load-config-file
```

If Firefox is installed under a different executable name, set
`NIRI_SHELF_BROWSER` to that executable in the Niri session environment.

## Run manually

If you do not want to start the programs from Niri config, run the daemon in
one terminal and start the overview in another:

```sh
niri-shelf-daemon
```

```sh
niri-shelf-ctl overview
```

The overview starts Firefox and serves its UI on a randomly selected
localhost-only port. Closing Firefox does not stop the local server; terminate
the `niri-shelf-ctl overview` process to stop it. The daemon must remain running
for store, fetch, swap, and status commands. The overview receives slot and
title changes over a persistent local event stream rather than polling, and
updates existing cards in place so window previews stay still. At startup, the
overview waits up to 30 seconds for the daemon, so Niri can launch both startup
commands concurrently. Its event stream also reconnects if the daemon restarts.

## Commands

```sh
niri-shelf-ctl store 3
niri-shelf-ctl fetch 3
niri-shelf-ctl swap 3
niri-shelf-ctl status
```

`store` places the currently focused window in an empty slot. It does not
replace a different window already in that slot; use `swap` to replace it.
`fetch` returns the window to the exact workspace it was on when stored, then
focuses it; Niri consequently switches to that workspace and its monitor if
needed. `swap` also remembers the current window's original workspace. If
there is no focused window when `swap` is invoked, it behaves like `fetch` and
leaves the selected slot empty.
Screenshot previews are temporarily disabled while investigating black images.
Storing and restoring windows does not invoke Niri's screenshot action.
Closing a stored window clears its slot.

## Troubleshooting

- **“Is it running?”** Make sure `niri-shelf-daemon` is running and that
  `NIRI_SOCKET` is present in its environment. Starting it with Niri's
  `spawn-at-startup` normally provides the compositor environment.

### “command not found” for either executable

Diagnose installation and `PATH` separately.

1. In the project directory, check Cargo's install root and whether the
   binaries are actually there:

   ```sh
   printf 'HOME=%s\n' "$HOME"
   ls -1 "$HOME/.local/bin"/niri-shelf-*
   ```

   `HOME` should be an absolute path such as `/home/alice`. If `ls` prints
   both files, its output should look like this (with your own home path):

   ```text
   /home/alice/.local/bin/niri-shelf-ctl
   /home/alice/.local/bin/niri-shelf-daemon
   ```

   Go to step 2. If `ls` reports `No such file or directory` (or prints the
   unmatched `niri-shelf-*` pattern), install them into that directory:

   ```sh
   cargo install --path . --root "$HOME/.local" --force
   ```

   Cargo should finish with a line like:

   ```text
   Installed package `niri-shelf v0.1.0` (executables `niri-shelf-ctl`, `niri-shelf-daemon`)
   ```

   Then repeat the `ls` command; it should print both paths shown above. Use
   double quotes exactly as shown around `"$HOME/.local"`. Do not use
   single quotes or escape the `$`: those prevent the shell from expanding
   `HOME` and can install under a literal `$HOME` directory instead.

2. Check whether the installed directory is on this shell's `PATH`:

   ```sh
   printf '%s\n' "$PATH" | tr ':' '\n' | grep -Fx "$HOME/.local/bin"
   ```

   If it prints `$HOME/.local/bin` as an expanded path (for example
   `/home/alice/.local/bin`), it is on `PATH`. If it prints nothing, add it
   to the current shell and check command lookup again:

   ```sh
   export PATH="$HOME/.local/bin:$PATH"
   hash -r
   command -v niri-shelf-daemon
   command -v niri-shelf-ctl
   ```

   The last two commands should print the full paths to the executables,
   for example `/home/alice/.local/bin/niri-shelf-daemon` and
   `/home/alice/.local/bin/niri-shelf-ctl`. This `export` only fixes the
   current shell. Add it to the startup file for your shell and open a new
   terminal to keep it there.

If `command -v` still prints nothing, compare the `HOME` printed in step 1
with the directory where `ls` found the files. They must match. Also check
for typos in the install command and its `--root` argument.
- **The overview opens on the wrong output:** confirm the output names and
  workspace placement in the Niri config, then reload it.
- **Firefox does not start:** install Firefox, check that it is on `PATH`, or
  set `NIRI_SHELF_BROWSER` to the browser executable name.
- **A keybind still switches workspaces:** change the sample bind to a
  combination not already used in your Niri config.
