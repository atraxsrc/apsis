# Contributing

How to report a bug, suggest an improvement, or send a change to Apsis.

## Bugs and questions

[Open an issue](https://github.com/atraxsrc/apsis/issues). Say which version (the window's
About page shows it), what you did, what you expected and what the status line or the dialog
said, with the log lines around it:

- `journalctl -u apsis-helper -e` is the helper's log.
- After a restore, `journalctl -b -1 -u apsis-restore` is the log of the restore's own boot.

Logs hold disk UUIDs, your user name and your computer's name: take out what you don't want
public. [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) may already have the answer.

Restore is new: a report of how it went on your hardware, good or bad, helps.

## Ideas and improvements

Open an issue first and say what you want to do and why, before you write the code. Some
things are left out on purpose: scheduled snapshots, automatic deletion, btrfs snapshots,
single-file restore and encrypted backup disks.

## Security problems

Don't open an issue. Report them privately, as [SECURITY.md](SECURITY.md) describes. It also
says what runs as root and how the helper checks its callers.

## Code

Build requirements: Rust (stable), [`just`](https://github.com/casey/just) and libcosmic's
build dependencies. On Pop!_OS / Ubuntu / Debian:

```sh
sudo apt install cargo just pkgconf libexpat1-dev libfontconfig-dev libfreetype-dev \
  libxkbcommon-dev libwayland-dev
```

```sh
just run             # run in a window, no root needed
cargo test --workspace
just check           # clippy
just deb             # build the .deb into target/debian/ (needs cargo-deb)
just ext4-image      # an ext4 image for the rsync tests (then mount it, just test-ext4)
APSIS_LAYOUT_TEST=1 cargo test -p apsis fit                     # layouts fit the smallest window (and every restore state)
APSIS_SCREENSHOTS=/tmp/shots cargo test -p apsis screenshots    # renders the views to .rgba files
```

Before a pull request, run what CI runs: `cargo fmt --all --check`, `cargo clippy --locked
--workspace --all-targets -- -D warnings` and `cargo test --locked --workspace`. The tests
need no root.

- Keep changes small, and say how you tested them. A change to Restore needs more than unit
  tests: say what ran on real hardware and what didn't.
- No hard-coded colours: every colour comes from the COSMIC theme.
- The window's texts are in `i18n/en/apsis.ftl` (Fluent). English is the only language so
  far.

## License

Apsis is GPL-3.0-only. By sending a change you agree it is published under that license.
