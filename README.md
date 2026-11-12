# !! Vibe Coded !!
# niri-shelf

`niri-shelf` temporarily parks up to nine Niri windows on a dedicated
workspace. Restore a window with a keybind or from a small graphical overview
that can show a screenshot preview of each stored window.

## Requirements

- Niri 26.04 (uses the Niri 26.4 IPC protocol)
- Rust and Cargo
- Firefox, or a browser that accepts `--new-window --kiosk`
- Two connected outputs for the example's secondary-monitor layout

## Install

From the project directory, install both programs under `~/.local/bin`:

```sh
cargo install --path . --root "$HOME/.local"
```

Ensure that directory is on `PATH` in both your terminal and Niri session:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Check that both programs are installed and discoverable:

```sh
command -v niri-shelf-daemon
command -v niri-shelf-ctl
```

Expected output (with your own home directory in place of `/home/alice`):

```text
/home/alice/.local/bin/niri-shelf-daemon
/home/alice/.local/bin/niri-shelf-ctl
```

If either command is not found, see [Troubleshooting](#troubleshooting).
For subsequent builds, reinstall with `--force` to replace the executables:

```sh
cargo install --path . --root "$HOME/.local" --force
```

Restart the daemon and overview after updating.

## Configure Niri

Use [`examples/niri-config.kdl`](examples/niri-config.kdl) as a template and
merge the relevant parts into your Niri configuration; it is not a complete
configuration. The example:

- reserves a `shelf` workspace on the primary output and a `shelf-overview`
  workspace on the secondary output;
- starts the daemon and graphical overview at login;
- binds keys to store, fetch, and swap windows.

Replace `eDP-1` and `DP-2` in the example with the output names shown by:

```sh
niri msg outputs
```

The daemon must inherit `NIRI_SOCKET` from the Niri session. Starting it with
Niri's `spawn-at-startup` does this. Ensure the session's `PATH` includes
`~/.local/bin` as well, so Niri can find both executables.

The example keybinds are:

| Key | Action |
| --- | --- |
| `Mod+Shift+1` … `Mod+Shift+9` | Store the focused window |
| `Mod+1` … `Mod+9` | Fetch and focus a stored window |
| `Mod+Ctrl+1` … `Mod+Ctrl+9` | Swap the focused window with a stored one |

These are examples and may conflict with your existing binds, especially
`Mod+1` … `Mod+9`. Choose unused combinations. A bind invokes the control
program with an action and slot number, for example:

```kdl
Mod+Alt+1 { spawn "niri-shelf-ctl" "fetch" "1"; }
```

Validate and reload your Niri configuration:

```sh
niri validate
niri msg action load-config-file
```

The overview uses `firefox` by default. Set `NIRI_SHELF_BROWSER` in the Niri
session environment to use a different browser executable.

## Use

The control program supports these commands:

```sh
niri-shelf-ctl store 3
niri-shelf-ctl fetch 3
niri-shelf-ctl swap 3
niri-shelf-ctl status
```

Slots are numbered 1–9. `store` moves the focused window to the shelf and
remembers its original workspace; it will not overwrite an occupied slot.
`fetch` restores that window to its original workspace and focuses it.
`swap` puts the focused window in the slot and restores the previous occupant;
if there is no focused window, it behaves like `fetch`. Closing a stored
window clears its slot.

The graphical overview starts Firefox in kiosk mode on a local-only address
with a randomly chosen port. It waits up to 30 seconds for the daemon to start.
Closing Firefox does not stop the overview server; stop the
`niri-shelf-ctl overview` process to shut it down. The daemon must be running
for all commands.

Slot assignments exist only in daemon memory. Restarting the daemon clears
them, so fetch any stored windows before restarting it. Screenshot previews are
cached under `$XDG_CACHE_HOME/niri-shelf`, or `~/.cache/niri-shelf` if
`XDG_CACHE_HOME` is unset.

## Run without startup entries

Instead of the `spawn-at-startup` entries in the example, start the programs
manually in separate terminals within the Niri session:

```sh
niri-shelf-daemon
```

```sh
niri-shelf-ctl overview
```

## Troubleshooting

### `command not found`

First check that Cargo installed the executables:

```sh
ls -l "$HOME/.local/bin/niri-shelf-daemon" "$HOME/.local/bin/niri-shelf-ctl"
```

If either file is missing, run the install command from the project directory:

```sh
cargo install --path . --root "$HOME/.local" --force
```

If the files exist but `command -v` cannot find them, add their directory to
the current shell and retry:

```sh
export PATH="$HOME/.local/bin:$PATH"
hash -r
command -v niri-shelf-daemon
command -v niri-shelf-ctl
```

Both `command -v` commands should print paths under `$HOME/.local/bin`.
Persist the `PATH` change in your shell startup configuration. Niri also needs
to inherit a `PATH` containing this directory; after changing the session
environment, log out and back in. Use double quotes around `"$HOME/.local"`
in the Cargo command so the shell expands your home directory.

### The daemon cannot connect to Niri

Run it from a Niri session, where `NIRI_SOCKET` is set. Niri's
`spawn-at-startup` passes the session environment to the daemon.

### The overview does not open

Check that Firefox (or the executable named by `NIRI_SHELF_BROWSER`) is
installed and on the Niri session's `PATH`. Check the overview process output
for launch errors.

### A keybind switches workspaces instead

Another Niri bind may use the same key combination. Change the example bind to
an unused combination and reload the configuration.

### The overview appears on the wrong output

Check the output names and workspace assignments in your Niri configuration,
then reload it.
