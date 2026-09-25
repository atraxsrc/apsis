<p align="center">
  <img src="resources/icons/hicolor/scalable/apps/io.github.atraxsrc.Apsis.svg" width="128" alt="Apsis logo">
</p>

<h1 align="center">Apsis</h1>

<p align="center">Timeshift-style system snapshots for the COSMIC™ desktop.</p>

A panel applet with a small terminal-style popup to list, create and delete system snapshots.
It works with keyboard or mouse and follows your COSMIC theme.

> **Status:** early development. Not ready for use.

<p align="center"><img src="docs/screenshot.png" width="560" alt="Apsis popup"></p>

## Name

Apsis is not an acronym.

In orbital mechanics an **apsis** (plural *apsides*, pronounced *AP-sis* / *AP-sih-deez*) is a
turning point on a body's path: **periapsis** at the nearest point, **apoapsis** at the farthest.
A system snapshot is the same idea: a fixed point on the machine's timeline that you can return to.

The logo shows exactly that: an orbit with its two apsides, the glowing one being the point you
come back to.

## Requirements

- COSMIC desktop
- [Timeshift](https://github.com/linuxmint/timeshift), configured. For now Apsis drives its CLI.

## Build

```sh
just            # build release
just run        # run for development
sudo just install    # the applet and apsis-helper (see below)
sudo just uninstall
```

`just install` also installs `apsis-helper`: a small root D-Bus service that runs Timeshift for
the applet, with polkit deciding who may do what. Listing needs no password; creating or
deleting asks once ("Apsis needs your password…") and polkit remembers it for a few minutes.
Without the helper the applet falls back to `pkexec`, which asks every time. Files and design:
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## License

GPL-3.0-only. See [LICENSE](LICENSE).
