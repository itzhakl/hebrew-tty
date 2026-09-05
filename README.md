# hebrew-tty

`hebrew-tty` is a Linux terminal proxy that repairs Hebrew and mixed-direction
rows produced by terminal coding agents. It runs the child on a PTY, models its
VT screen, applies at most one verified Unicode BiDi transformation per row,
right-aligns RTL content inside its pane, and restores the caret to the edited
grapheme.

Nothing inside Claude Code, Pi, or Codex is patched.

```sh
cargo build --release
bin/hebrew-tty claude
bin/hebrew-tty pi
bin/hebrew-tty codex
```

The first release targets Linux and Ptyxis/VTE. macOS, Windows, mouse coordinate
remapping, selection reconstruction, and clipboard reconstruction are not yet
supported.

## Always on

`hebrew-tty --install` puts a guarded block first in `~/.zshrc` or `~/.bashrc`.
Every interactive shell then execs into the proxy before it finishes starting,
so nothing has to be launched through `hebrew-tty`, and the integrated terminal
of an IDE gets the same repair as a terminal window. The proxy classifies
whatever the shell brings to the foreground: an agent it has a recording for is
repaired from the moment its version answers, including the rows it painted
while being asked, and everything else passes through byte for byte.
`hebrew-tty --uninstall` removes the block.

```sh
hebrew-tty --install      # once
hebrew-tty                # the same for one shell, without the rc block
```

The child runs with `HEBREW_TTY=1`, which is what stops the inner shell from
wrapping itself again and what a script can test for. A proxy that cannot start
execs the plain shell instead of closing the terminal. The process name follows
the foreground, `claude` while Claude Code runs and `zsh` after, unless `--as`
pinned it.

## Safe modes

The default mode is `auto`. It transforms only an agent version and host path
backed by the recorded measurement fixtures. Unknown or contradictory paths
remain byte-for-byte pass-through.

```sh
hebrew-tty --mode auto claude
hebrew-tty --mode logical command   # child emits logical Unicode text
hebrew-tty --mode visual command    # child emits pre-reordered visual text
hebrew-tty --mode passthrough command
```

CLI mode overrides take precedence over
`$XDG_CONFIG_HOME/hebrew-tty/config.toml` (normally
`~/.config/hebrew-tty/config.toml`):

```toml
version = 1
default_mode = "auto"

[commands.claude]
mode = "visual"

[commands.pi]
mode = "visual"

[commands.codex]
mode = "logical"
```

Use forced modes only when you know the command's output contract. A wrong
forced mode can apply BiDi twice.

## Diagnostics

`--diagnostics` appends one JSON record describing the command, host,
classification evidence, selected mode, row disposition, and safe-mode reason.
Diagnostics never share stdout or stderr with the terminal stream.

```sh
hebrew-tty --diagnostics ~/.local/state/hebrew-tty/events.jsonl claude
```

## Herdr

Herdr 0.8.2 can link the repository as a local plugin and expose three workspace
actions. Each opens a background split in the invoking pane's working directory
through the same Rust executable. The Linux plugin launcher uses `python3` to
read Herdr's JSON invocation context.

```sh
herdr plugin link "$PWD" --enabled
herdr plugin action list --plugin hebrew-tty.terminal-proxy
herdr plugin action invoke claude --plugin hebrew-tty.terminal-proxy
herdr plugin action invoke pi --plugin hebrew-tty.terminal-proxy
herdr plugin action invoke codex --plugin hebrew-tty.terminal-proxy
```

Unlink it with:

```sh
herdr plugin unlink hebrew-tty.terminal-proxy
```

## Packaging

`bin/hebrew-tty` resolves `HEBREW_TTY_BIN`, a repository release/debug build, a
packaged `dist/hebrew-tty-linux-x86_64`, or an installed Rust binary in that
order. `npm pack` builds and stages the Linux x86_64 release binary. Node remains
required for dictation and the retained JavaScript regression suite, not for the
Rust terminal proxy.

## Dictation

Hebrew dictation moved to its own service:
[hebrew-voice](https://github.com/itzhakl/hebrew-voice). It shares no code with
the proxy and is installed separately.

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
npm test
python3 tools/terminal_proxy_probe.py verify test/fixtures/terminal-proxy/measurements
tools/verify-herdr-plugin.sh
tools/smoke-ptyxis.sh
```

The JavaScript suite remains regression evidence for the predecessor caret
engine while the Rust proxy is the shipped terminal path.

## Rollback

Use `--mode passthrough` for one launch, set `default_mode = "passthrough"` for
a global rollback, or unlink the Herdr plugin. These preserve the PTY transport
without transforming rows.

MIT.
