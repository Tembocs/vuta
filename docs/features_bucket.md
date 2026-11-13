# Features bucket

Work that remains after v1, the packaging tools, shell completions (including names from the registry), JSON output for every listing, live sync progress, the first-root offer, and registry export and import. Each entry says what it is, why it matters, where it plugs into the code, roughly how big it is, and what has to be decided first. The order is a suggestion: release work first, then larger changes worth making when a concrete need appears.

[ARCHITECTURE.md](ARCHITECTURE.md) section 13 lists the same items in brief; this file is where they are planned. When an item is done, remove it here and update section 13.

| # | Item | Kind | Size |
| --- | --- | --- | --- |
| 1 | [Release 0.1.0](#1-release-010) | Release | Small |
| 2 | [Fast-forward around local changes](#2-fast-forward-around-local-changes) | Behaviour | Medium |
| 3 | [Submodules](#3-submodules) | Feature | Small to medium |
| 4 | [Other update strategies](#4-other-update-strategies) | Feature | Medium |
| 5 | [Other version-control backends](#5-other-version-control-backends) | Feature | Large per backend |

## 1. Release 0.1.0

**What.** Settle the licence, tag `v0.1.0`, build the `.deb` and `.msix` with `cargo xtask`, and optionally publish the crate so `cargo install vuta` works.

**Why.** Gives every later change a fixed baseline, and gives users something to install that is not a source checkout.

**Decisions first.**

- **Licence.** `Cargo.toml` and the licence files say `MIT OR Apache-2.0`, which ARCHITECTURE.md marks as an assumption. Confirm it or change it before anything is published.
- **crates.io.** Publishing needs the `xtask` workspace member excluded from the package (it has `publish = false`, so only the root crate goes up), a check of `cargo package --list`, and a crates.io account.
- **Where packages live.** Releases do not depend on GitHub. Decide where the `.deb` and `.msix` files are offered, and which certificate signs the `.msix` (see [PACKAGING.md](PACKAGING.md)).

**Done when.** A `v0.1.0` tag exists, both packages install on a clean machine, and (if chosen) `cargo install vuta` installs the same version.

## 2. Fast-forward around local changes

**What.** An opt-in, per repository, to fast-forward a checkout whose tracked files are modified when the upstream changes do not touch those files.

**Why.** Some checkouts always carry a small local edit (a config tweak) and are skipped on every sync today.

**Where it plugs in.** A new `repos` column through a migration (`0002_...sql`), `vuta set <name> --allow-dirty`, and check 7 in `sync/worker.rs`. Git's `merge --ff-only` already refuses when local changes would be overwritten, so the worker can try it and report a skip if git refuses.

**Constraints.** Must keep the first goal: never damage a checkout. A refused merge must leave the working tree exactly as it was, with a skip reason that says why. Needs integration tests for both the allowed and the refused case.

## 3. Submodules

**What.** An opt-in per repository to run `git submodule update --init --recursive` after a clone or fast-forward.

**Why.** Some projects do not build without their submodules.

**Where it plugs in.** A `repos` column (migration), `vuta add`/`set --submodules`, a new `Vcs` method in `vcs/mod.rs` and `vcs/git.rs`, and one step in `sync/worker.rs` after a successful clone or update.

**Decisions first.** How a submodule failure is reported: the repository itself updated, so either a new failure reason or a note on the result line, similar to how a failed hook leaves the outcome `updated`.

## 4. Other update strategies

**What.** A per-repository strategy beside the default `ff-only`, such as `rebase` for checkouts with local commits that should follow upstream.

**Why.** Diverged checkouts are skipped forever today; someone who keeps a small local patch has to rebase by hand.

**Where it plugs in.** A `strategy` column (migration), `vuta set --strategy`, and the diverged check in `sync/worker.rs`.

**Constraints.** This weakens "Vuta never merges, rebases, stashes or resets". If done, a failed rebase must be aborted so the checkout is left as it was, and the skip or failure must say so. Do it only with a concrete need.

## 5. Other version-control backends

**What.** Jujutsu, Mercurial, Subversion or Fossil checkouts, managed like Git ones.

**Why.** Some projects are not on Git; Jujutsu users colocate with Git but have their own working-copy rules.

**Where it plugs in.** As designed in ARCHITECTURE.md section 5:

- a new file in `vcs/` implementing the `Vcs` trait, and a `VcsKind` variant;
- a migration that widens the `repos.vcs` CHECK constraint;
- a `--vcs` option on `vuta add`, and detection of `.hg`, `.svn`, `.jj` and so on in `vuta scan`;
- a test fixture with local "remotes" for that tool, and the tool installed in CI on Linux and Windows.

The sync engine and the per-repository decisions do not change; each backend answers the same questions (checkout, origin, head, dirty, divergence) and performs the same actions (clone, fetch, fast-forward).

**Decisions first.** Which backend is actually needed. Each is a large piece of work, so build one only for a real repository that needs it.
