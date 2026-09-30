# Apsis

System snapshot and restore for the COSMIC™ desktop.
A panel applet and a window laid out like Timeshift's, in standard COSMIC widgets that follow the live
theme (see `docs/APSIS-UI-PROMPT.md`).

- Display name: **Apsis**. Repo, crate and binary: **`apsis`**.
- Listing line (cosmic-utils): *System snapshot and restore* (no tagline; owner, 2026-09-30)
- Target: the community collection at https://cosmic-utils.github.io/ (PR to
  `cosmic-utils/cosmic-project-collection`, file `applets.ron`). Only the user opens that PR.

## Rules (always loaded — these win over anything else)

@docs/PRIVACY.md

Project-specific additions:

- **Never run anything as root.** No `sudo`, no `pkexec`, no `timeshift` commands that create, delete or
  restore. If a step needs root (installing the applet, running a real snapshot), write the exact
  command in chat and let the user run it.
- **App ID:** don't invent one and don't derive it from anything in the environment. Ask the user for
  it once (expected form `io.github.<github-user>.Apsis`), then record it in `docs/DECISIONS.md`.
- **Fixtures:** real `timeshift` output saved under `crates/apsis-core/tests/fixtures/` must have
  hostnames, usernames, device serials, and disk UUIDs replaced with placeholders before saving.
- Match libcosmic style: `cargo fmt`, `cargo clippy -- -D warnings`. No hard-coded colours; every
  colour comes from the COSMIC theme.

## Read these before starting work

1. `docs/PLAN.md` — phases, what "done" means for each. **Work one phase at a time.**
2. `docs/ARCHITECTURE.md` — crates, backend trait, privilege model.
3. `docs/UI.md` — the terminal-style popup, keybindings, states.
4. `docs/TIMESHIFT-CLI.md` — the CLI contract we wrap (verify against `timeshift --help` locally).
5. `docs/DECISIONS.md` — decisions made and open questions. Append; don't rewrite history.

## Layout (target)

```
apsis/
├── CLAUDE.md
├── README.md
├── Cargo.toml              ← workspace
├── justfile                ← adapted from cosmic-applet-template
├── crates/
│   ├── apsis-core/         ← snapshot model, Backend trait, Timeshift CLI backend + parser (no UI)
│   └── apsis/              ← the libcosmic panel applet (binary `apsis`)
│   (phase 2: apsis-helper/ ← privileged D-Bus service)
├── resources/              ← .desktop, icons, metainfo, (later) polkit + dbus + systemd files
├── i18n/en/                ← fluent strings (from template)
├── docs/
└── .claude/settings.json
```

## Commands

```sh
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
just run          # run the applet in a window for development (no root needed)
just check        # template's clippy recipe
# user only (needs root):  just deb-install
```

Testing on the user's machine: `just deb-install` (builds the .deb and reinstalls it with apt).
Never suggest `sudo just install`.

## Working style

- Start each session by reading `docs/PLAN.md` and saying which phase and task you're on.
- Small, reviewable changes. After each task: build, test, clippy, then stop and summarise.
- Record anything non-obvious (what you tried, what broke, why) in `docs/DECISIONS.md`.
- Before any commit, follow the checklist in `docs/PRIVACY.md` exactly and wait for "yes".
