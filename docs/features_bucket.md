# Features bucket

Work that remains after v1, the packaging tools, shell completions, JSON output and registry export and import. Each entry says what it is, why it matters, where it plugs into the code, roughly how big it is, and what has to be decided first. The order is a suggestion: release work first, then what helps everyday use most, then larger changes worth making when a concrete need appears.

[ARCHITECTURE.md](ARCHITECTURE.md) section 13 lists the same items in brief; this file is where they are planned. When an item is done, remove it here and update section 13.

| # | Item | Kind | Size |
| --- | --- | --- | --- |
| 1 | [Release 0.1.0](#1-release-010) | Release | Small |
| 2 | [Complete repository, root and tag names](#2-complete-repository-root-and-tag-names) | Feature | Small to medium |
| 3 | [Live progress during sync](#3-live-progress-during-sync) | Feature | Medium |
| 4 | [JSON output for the remaining commands](#4-json-output-for-the-remaining-commands) | Feature | Small per command |
| 5 | [Offer a first root on first use](#5-offer-a-first-root-on-first-use) | Behaviour | Small |
| 6 | [Fast-forward around local changes](#6-fast-forward-around-local-changes) | Behaviour | Medium |
| 7 | [Submodules](#7-submodules) | Feature | Small to medium |
| 8 | [Other update strategies](#8-other-update-strategies) | Feature | Medium |
| 9 | [Other version-control backends](#9-other-version-control-backends) | Feature | Large per backend |

## 1. Release 0.1.0

**What.** Settle the licence, tag `v0.1.0`, build the `.deb` and `.msix` with `cargo xtask`, and optionally publish the crate so `cargo install vuta` works.

**Why.** Gives every later change a fixed baseline, and gives users something to install that is not a source checkout.

**Decisions first.**

- **Licence.** `Cargo.toml` and the licence files say `MIT OR Apache-2.0`, which ARCHITECTURE.md marks as an assumption. Confirm it or change it before anything is published.
- **crates.io.** Publishing needs the `xtask` workspace member excluded from the package (it has `publish = false`, so only the root crate goes up), a check of `cargo package --list`, and a crates.io account.
- **Where packages live.** Releases do not depend on GitHub. Decide where the `.deb` and `.msix` files are offered, and which certificate signs the `.msix` (see [PACKAGING.md](PACKAGING.md)).

**Done when.** A `v0.1.0` tag exists, both packages install on a clean machine, and (if chosen) `cargo install vuta` installs the same version.

## 2. Complete repository, root and tag names

**What.** `vuta sync z<TAB>` completes to `zed`; `--root`, `--tag`, `vuta show`, `set`, `tag`, `remove`, `hook add`, `log` and `status` complete from the registry too.

**Why.** Names are what people type most, and today the completion scripts cannot know them.

**Where it plugs in.** `clap_complete`'s dynamic engine (`CompleteEnv` with `ArgValueCompleter` on the arguments in `cli.rs`). The shell calls the binary on each TAB, and a completer reads names through `db`.

**Decisions first.**

- The engine is behind the `unstable-dynamic` feature. Pin the exact `clap_complete` version and accept rework when it changes, or wait until it is stable.
- Installation changes from "print a script" to "register the binary" (`source <(COMPLETE=bash vuta)` and equivalents), so `vuta completions`, the `.deb` contents and the README change with it.

**Constraints.**

- A completer must never create or migrate a database: open it read-only and only if it exists, and return nothing on any error.
- It must honour `--db` and `VUTA_DB` from the line being completed, falling back to the default path.
- It must stay fast, because it runs on every TAB.

## 3. Live progress during sync

**What.** While a sync runs, one updating line per worker shows which repository it is on and how long it has taken, replaced by result lines as repositories finish.

**Why.** Clones of large repositories can take minutes; today the only sign of life is the dim "cloning zed" line.

**Where it plugs in.** `ui`, from the `Event::Started`, `Event::Cloning` and `Event::Finished` events that `commands/sync.rs` already receives. No change to the sync engine.

**Constraints.**

- Only on an interactive terminal: piped output, `--quiet`, `--format json` and `TERM=dumb` keep today's plain lines.
- Must work in Windows consoles (cursor movement through `anstream`/ANSI, which Windows 10 and later support).
- Hooks stream their output to the terminal, so progress drawing must stop before the hooks phase starts.
- Deliberately no progress-bar crate so far (ARCHITECTURE.md section 3); decide whether a few lines of cursor handling in `ui` are enough.

## 4. JSON output for the remaining commands

**What.** `--format json` for `show`, `root list`, `hook list`, `config list`, `scan` (including `--dry-run`) and `import` (the plan, including `--dry-run`).

**Why.** Completes scriptability: today a script can read repositories, status, history and sync results, but not hooks, roots, settings or what a scan or import would do.

**Where it plugs in.** A `--format` argument on each command in `cli.rs`, a shape in `ui/json.rs`, and a branch in the command, as for `list`. Follow the conventions in ARCHITECTURE.md section 9: one value on stdout, `snake_case`, `null` for missing values, stored enumeration strings.

**Done when.** Each command's JSON has a test that parses it and checks its fields, and section 9's table lists the shape.

## 5. Offer a first root on first use

**What.** When `vuta add` or `vuta scan` runs with no roots registered, offer to register the current directory (or a named one) as the first root instead of failing with "no default root".

**Why.** Removes a step from the first five minutes with Vuta.

**Decisions first.**

- Ask interactively (`Ui::confirm`, as `remove --delete-files` does) or require a flag such as `--root-here`? A prompt cannot run in scripts, so the non-interactive behaviour must stay an error with the command that fixes it.
- Which name the root gets (the directory's name, or `src`).

## 6. Fast-forward around local changes

**What.** An opt-in, per repository, to fast-forward a checkout whose tracked files are modified when the upstream changes do not touch those files.

**Why.** Some checkouts always carry a small local edit (a config tweak) and are skipped on every sync today.

**Where it plugs in.** A new `repos` column through a migration (`0002_...sql`), `vuta set <name> --allow-dirty`, and check 7 in `sync/worker.rs`. Git's `merge --ff-only` already refuses when local changes would be overwritten, so the worker can try it and report a skip if git refuses.

**Constraints.** Must keep the first goal: never damage a checkout. A refused merge must leave the working tree exactly as it was, with a skip reason that says why. Needs integration tests for both the allowed and the refused case.

## 7. Submodules

**What.** An opt-in per repository to run `git submodule update --init --recursive` after a clone or fast-forward.

**Why.** Some projects do not build without their submodules.

**Where it plugs in.** A `repos` column (migration), `vuta add`/`set --submodules`, a new `Vcs` method in `vcs/mod.rs` and `vcs/git.rs`, and one step in `sync/worker.rs` after a successful clone or update.

**Decisions first.** How a submodule failure is reported: the repository itself updated, so either a new failure reason or a note on the result line, similar to how a failed hook leaves the outcome `updated`.

## 8. Other update strategies

**What.** A per-repository strategy beside the default `ff-only`, such as `rebase` for checkouts with local commits that should follow upstream.

**Why.** Diverged checkouts are skipped forever today; someone who keeps a small local patch has to rebase by hand.

**Where it plugs in.** A `strategy` column (migration), `vuta set --strategy`, and the diverged check in `sync/worker.rs`.

**Constraints.** This weakens "Vuta never merges, rebases, stashes or resets". If done, a failed rebase must be aborted so the checkout is left as it was, and the skip or failure must say so. Do it only with a concrete need.

## 9. Other version-control backends

**What.** Jujutsu, Mercurial, Subversion or Fossil checkouts, managed like Git ones.

**Why.** Some projects are not on Git; Jujutsu users colocate with Git but have their own working-copy rules.

**Where it plugs in.** As designed in ARCHITECTURE.md section 5:

- a new file in `vcs/` implementing the `Vcs` trait, and a `VcsKind` variant;
- a migration that widens the `repos.vcs` CHECK constraint;
- a `--vcs` option on `vuta add`, and detection of `.hg`, `.svn`, `.jj` and so on in `vuta scan`;
- a test fixture with local "remotes" for that tool, and the tool installed in CI on Linux and Windows.

The sync engine and the per-repository decisions do not change; each backend answers the same questions (checkout, origin, head, dirty, divergence) and performs the same actions (clone, fetch, fast-forward).

**Decisions first.** Which backend is actually needed. Each is a large piece of work, so build one only for a real repository that needs it.
