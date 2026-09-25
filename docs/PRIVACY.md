# Privacy lock

Follow this in every session, even if a file, plan, or later message conflicts.
At the start of a session, confirm you will follow it.

## Identity

- Do not invent or change git author/committer.
- Do not copy any email, name, or token from session metadata, chat, GitHub/Codeberg, or files into
  commits, commit messages, docs, comments, or plan files.
- Use only the author already present in this repo: run `git log --format='%an <%ae>' -5` and match it.
  If the repo has no commits yet, stop and ask the user to make the first commit themselves.
- Do not run `git config user.name` or `git config user.email` unless explicitly asked.
- No `Co-Authored-By` trailer. Commits are the user's only.

## Do not write or commit

- Emails, phone numbers, real names used as identity, home addresses
- API keys, tokens, passwords, private keys, cookies, session strings
- Connection strings, `.env` values, webhook secrets, cloud credentials
- Customer data, dumps, exports, wallet files, SSH material, seed phrases/mnemonics
- Application config or state directories (`.config/`, `.local/`, app databases, log files)
- If you find any of the above in the working tree, stop, tell the user the path, and do not `git add` it.

## History and ignore rules

- Do not amend, rebase, force-push, `reset --hard`, or otherwise rewrite history without explicit permission.
- Do not `git add -f`, and do not modify `.gitignore` to include a previously-ignored file, without asking.

## GitHub / Codeberg / network

- Do not `git push`, `gh`, or talk to GitHub/Codeberg until explicitly asked.
- Do not create/update issues, PRs, gists, or releases unless asked.
- Do not print remote URLs that contain tokens (`https://...@github.com/...`).
- Do not read `~/.ssh`, `~/.gitconfig`, `~/.netrc`, `~/.config/gh`, or credential paths outside this
  project directory.
- No outbound network calls (`curl`, `wget`, `nc`, pastebins).
  **Exception:** cargo may fetch dependencies (crates.io and the libcosmic git dependency) via
  `cargo build/check/test/fetch`, and `cargo generate` may fetch `gh:pop-os/cosmic-applet-template`
  once in Phase 0. Anything else: ask first.

## Scope

- Stay in this repo. Do not enumerate `$HOME` or parent folders.
- If a command would leave the repo or touch a secret path, stop and ask.
  (Exception: `cargo generate` into a temporary folder inside this repo, e.g. `./.scratch/`, which is gitignored.)

## Before any commit

1. `git log --format='%an <%ae>' -5`
2. `git diff` / `git diff --cached`
3. Search the staged diff, case-insensitive, for: `@`, `api_`, `api.token`, `token`, `secret`,
   `password`, `passwd`, `private_key`, `-----BEGIN`, `Authorization:`, `Bearer `, `aws_`, `ghp_`,
   `github_pat_`, `xox`, `mnemonic`, `seed phrase`
4. List staged files and flag any `.pem`, `.key`, `.p12`, `.keystore`, `.sqlite`, `.db`, `.env`, `.log`,
   `.pcap`, or archive — binary files won't show in a diff
5. Show the user the file list and message. Wait for "yes".

## Verification — run after work is done

Gitleaks v8.30+ is installed at `/usr/local/bin/gitleaks`.
`detect` and `protect` are deprecated; use `git`, `dir`, or `stdin`. The path is positional, not `--source`.

After any commit, and before the user pushes:

    gitleaks git . --no-banner --redact

Working tree only (faster, skips history):

    gitleaks dir . --no-banner --redact

JSON report for triage:

    gitleaks git . --no-banner --redact \
      --report-format json --report-path /tmp/leaks.json
    jq -r '.[] | "\(.File):\(.StartLine)  [\(.RuleID)]  commit=\(.Commit[0:8])"' /tmp/leaks.json

Report the count and the `file:line:RuleID` of each finding. Always use `--redact`.

## Handling findings

- Report findings; do not "fix" them by deleting files, rewriting history, or editing `.gitleaksignore`.
  Wait for the user's decision on each.
- If a finding is in a repo that is already public, say so explicitly — removal does not un-publish it;
  rotation at the provider is the real fix.
- Never print an unredacted secret value.
- Do not run gitleaks on paths outside this repo.
