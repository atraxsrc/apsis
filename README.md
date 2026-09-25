# Apsis

Timeshift-style system snapshots for the COSMIC™ desktop.

A panel applet with a small, keyboard-and-mouse terminal-style popup to list, create and delete
system snapshots. It follows your COSMIC theme.

> Status: early development. Not ready for use.

## Requirements

- COSMIC desktop
- [Timeshift](https://github.com/linuxmint/timeshift), configured (for now Apsis drives its CLI)

## Build

```sh
just            # build release
just run        # run for development
sudo just install
```

## License

GPL-3.0-only - see [LICENSE](LICENSE)
