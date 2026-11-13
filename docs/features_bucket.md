# Features bucket

Work that remains after v1, the packaging tools, shell completions (including names from the registry), JSON output for every listing, live sync progress, the first-root offer, registry export and import, and the per-repository sync options (fast-forwarding around local changes, the rebase strategy and submodules). Each entry says what it is, why it matters, where it plugs into the code, roughly how big it is, and what has to be decided first. The order is a suggestion: release work first, then larger changes worth making when a concrete need appears.

[ARCHITECTURE.md](ARCHITECTURE.md) section 13 lists the same items in brief; this file is where they are planned. When an item is done, remove it here and update section 13.

| # | Item | Kind | Size |
| --- | --- | --- | --- |
| 1 | [Release 0.1.0](#1-release-010) | Release | Small |
| 2 | [Other version-control backends](#2-other-version-control-backends) | Feature | Large per backend |

## 1. Release 0.1.0

**What.** Settle the licence, tag `v0.1.0`, build the `.deb` and `.msix` with `cargo xtask`, and optionally publish the crate so `cargo install vuta` works.

**Why.** Gives every later change a fixed baseline, and gives users something to install that is not a source checkout.

**Decisions first.**

- **Licence.** `Cargo.toml` and the licence files say `MIT OR Apache-2.0`, which ARCHITECTURE.md marks as an assumption. Confirm it or change it before anything is published.
- **crates.io.** Publishing needs the `xtask` workspace member excluded from the package (it has `publish = false`, so only the root crate goes up), a check of `cargo package --list`, and a crates.io account.
- **Where packages live.** Releases do not depend on GitHub. Decide where the `.deb` and `.msix` files are offered, and which certificate signs the `.msix` (see [PACKAGING.md](PACKAGING.md)).

**Done when.** A `v0.1.0` tag exists, both packages install on a clean machine, and (if chosen) `cargo install vuta` installs the same version.

## 2. Other version-control backends

**What.** Jujutsu, Mercurial, Subversion or Fossil checkouts, managed like Git ones.

**Why.** Some projects are not on Git; Jujutsu users colocate with Git but have their own working-copy rules.

**Where it plugs in.** As designed in ARCHITECTURE.md section 5:

- a new file in `vcs/` implementing the `Vcs` trait, and a `VcsKind` variant;
- a migration that widens the `repos.vcs` CHECK constraint;
- a `--vcs` option on `vuta add`, and detection of `.hg`, `.svn`, `.jj` and so on in `vuta scan`;
- a test fixture with local "remotes" for that tool, and the tool installed in CI on Linux and Windows.

The sync engine and the per-repository decisions do not change; each backend answers the same questions (checkout, origin, head, dirty, local and incoming changes, divergence, submodules) and performs the same actions (clone, fetch, fast-forward, rebase, submodule update). A backend with no rebase or submodules can refuse those, and the repository is reported as skipped or its submodule step as failed.

**Decisions first.** Which backend is actually needed. Each is a large piece of work, so build one only for a real repository that needs it.
