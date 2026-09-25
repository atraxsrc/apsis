# Timeshift CLI contract

Written from memory of Timeshift's help text. **Verify every flag against `timeshift --help` on the
user's machine** (ask the user to paste it) and correct this file before Phase 1 code relies on it.

## Commands we use

| purpose | argv |
|---|---|
| list | `timeshift --list --scripted` |
| create | `timeshift --create --comments <text> --tags O --scripted` |
| delete | `timeshift --delete --snapshot <name> --scripted` |
| check config | `timeshift --check --scripted` (creates a scheduled snapshot if due — **don't** use for probing) |
| devices | `timeshift --list-devices --scripted` |

Global options worth knowing: `--snapshot-device <dev>`, `--btrfs`, `--rsync`, `--yes`, `--quiet`,
`--verbose`, `--debug`.

Tags: `O` on-demand, `B` boot, `H` hourly, `D` daily, `W` weekly, `M` monthly.

## Not in scope until Phase 6

`--restore`, `--target-device`, `--grub-device`, `--skip-grub`, `--delete-all`.

## Notes

- Every command needs root. Timeshift refuses to run otherwise.
- `--list` prints a few header lines (device, mode, status) then a table roughly like:

  ```
  Num     Name                 Tags  Description
  ------------------------------------------------------------------------------
  0    >  2026-09-18_12-41-00  O     before kernel update
  1    >  2026-09-24_03-00-01  D
  ```

  The exact header and spacing must come from real output (fixtures). Parse by columns defensively:
  name is the `YYYY-MM-DD_HH-MM-SS` token; tags follow; the rest of the line is the comment.
- Config lives at `/etc/timeshift/timeshift.json` (root-owned). Apsis reads nothing from it in
  Phases 1–3.
- Known COSMIC issues with the Timeshift GTK UI (e.g. pop-os/cosmic-epoch#485, #1558) are part of why
  Apsis exists; the CLI is unaffected.
