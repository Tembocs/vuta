# Vuta — Architecture

Nov 12, 2026 · @Doc

## 1. Overview

Vuta ("pull" in Swahili) is a single-binary Rust CLI that keeps a set of source checkouts cloned and up to date. Version 1 supports Git only, and one SQLite database is the only source of truth: there is no config file.

| Item | Value |
| --- | --- |
| Crate and binary | `vuta` |
| Repository | `github.com/Tembocs/vuta` |
| Platforms | Linux and Windows, tested in CI. macOS needs no extra code. |
| Language | Rust, edition 2024, minimum Rust 1.89 |
| Licence | `MIT OR Apache-2.0` (assumed; change it in `Cargo.toml` if you prefer another) |

### Goals

- **Works from any directory.** Checkout locations come from the database, never from the current directory.
- **Never damages a checkout.** Updates are fast-forward only unless a repository opts into more (section 7). Vuta never merges, stashes or resets. It rebases only a clean checkout whose repository asks for it, and aborts a rebase that stops. Whatever git refuses or stops part-way leaves the checkout as it was.
- **One command syncs everything.** Repositories sync in parallel, each gets one coloured result line, and the exit code says whether anything failed.
- **All state in one file.** The registry, tags, hooks, settings and run history live in SQLite.
- **Another VCS is a new backend, not a redesign.** Everything Git-specific sits behind one trait.

### Out of scope for v1

- Mercurial, Subversion and other VCS backends.
- Config files. (`vuta export` and `vuta import` move the registry between machines, but the database stays the source of truth.)
- Anything that writes to a remote: commit, push, tag.

Section 13 says where each of these plugs in later.

## 2. Repository layout

One Cargo package with a thin `main.rs` and a library crate, so integration tests and unit tests reach the same code. The workspace has one more member, `xtask`, which holds packaging tools and is never published or installed.

```text
vuta/
├── Cargo.toml                 the vuta package, and a workspace with xtask
├── README.md
├── .cargo/config.toml         the `cargo xtask` alias
├── docs/
│   ├── ARCHITECTURE.md        this document, exported as Markdown
│   ├── PACKAGING.md           building .deb and .msix packages
│   └── features_bucket.md     remaining work, planned
├── LICENSE-MIT
├── LICENSE-APACHE
├── .gitignore                 /target
├── .github/workflows/ci.yml
├── migrations/
│   ├── 0001_init.sql          the schema in section 4
│   └── 0002_sync_options.sql  per-repository sync options (section 4)
├── src/
│   ├── main.rs                parse args, set colour, open DB, dispatch, map errors to exit codes
│   ├── lib.rs                 module declarations
│   ├── cli.rs                 clap definitions only, no logic
│   ├── error.rs               Error enum and exit codes
│   ├── model.rs               Root, Repo, Hook, Run, Outcome, SkipReason
│   ├── paths.rs               data directory, path normalisation, root + rel_path joins
│   ├── proc.rs                spawn a child process, capture output, shared environment
│   ├── transfer.rs            the registry file of vuta export and vuta import
│   ├── commands/              one file per subcommand: read args, call core, render
│   │   ├── mod.rs
│   │   ├── root.rs  add.rs  scan.rs  sync.rs  list.rs  status.rs
│   │   ├── show.rs  set.rs  tag.rs   remove.rs  hook.rs  log.rs
│   │   └── config.rs  completions.rs  db.rs  doctor.rs  export.rs  import.rs
│   ├── db/
│   │   ├── mod.rs             open(), pragmas, migrate()
│   │   └── roots.rs  repos.rs  hooks.rs  runs.rs  settings.rs    typed queries
│   ├── vcs/
│   │   ├── mod.rs             Vcs trait, VcsKind, backend lookup
│   │   ├── git.rs             Git backend
│   │   └── remote.rs          remote URL normalisation and name inference
│   ├── sync/
│   │   ├── mod.rs             select repos, start workers, collect results
│   │   ├── worker.rs          per-repository decisions (section 7)
│   │   └── hooks.rs           hook runner
│   └── ui/
│       ├── mod.rs             Ui handle: stdout, stderr, verbosity
│       ├── theme.rs           semantic styles and glyphs
│       ├── table.rs           column alignment
│       ├── report.rs          sync result lines, live progress lines and the summary
│       ├── live.rs            live progress during a sync, redrawn in place
│       └── json.rs            the shapes of --format json
└── tests/
    ├── common/mod.rs          temp dirs, local bare repos, vuta command builder
    ├── cli.rs                 global flags, db path, doctor, exit codes
    ├── registry.rs
    ├── sync.rs
    ├── hooks.rs
    └── options.rs             allow-dirty, the rebase strategy, submodules
xtask/                         cargo xtask deb | msix (docs/PACKAGING.md)
├── Cargo.toml                 clap and anyhow only
└── src/
    ├── main.rs                task definitions
    ├── meta.rs                package metadata, target triple, release build
    ├── deb.rs                 Debian package with dpkg-deb
    ├── msix.rs                MSIX package with makeappx and signtool
    └── png.rs                 the MSIX logos, drawn and encoded without dependencies
```

### Three rules that keep the modules honest

1. **Only `db` runs SQL.** Commands call typed functions such as `db::repos::by_name`.
2. **Only `vcs::git` (through `proc`) spawns `git`.** The sync engine sees the `Vcs` trait and nothing else.
3. **Only `ui` writes to the terminal.** No `println!` outside it, which is what makes colour and quiet mode consistent.

&#91;embedded content: module map · 6 modules, 3 outside resources\]

Arrows point from caller to callee. `sync` reaches Git only through `vcs`, and it reports results back to its caller instead of writing to the database or the terminal itself. `model.rs`, `paths.rs` and `error.rs` are plain types and helpers used by every module.

## 3. Dependencies

Twelve runtime crates, all checked against crates.io on 12 Nov 2026. The minimum Rust version is 1.89 because `rusqlite` 0.42 requires it and the sync lock uses `File::try_lock`.

```toml
[package]
name = "vuta"
version = "0.1.0"
edition = "2024"
rust-version = "1.89"
description = "Keep many source checkouts cloned and up to date"
license = "MIT OR Apache-2.0"
repository = "https://github.com/Tembocs/vuta"

[dependencies]
clap = { version = "4.6", features = ["derive", "env"] }
# The dynamic engine is unstable, so the version is pinned exactly.
clap_complete = { version = "=4.6.11", features = ["unstable-dynamic"] }
rusqlite = { version = "0.42", features = ["bundled"] }
directories = "6"
anstream = "1"
anstyle = "1"
anstyle-query = "1"
thiserror = "2"
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
ctrlc = { version = "3.5", features = ["termination"] }

[dev-dependencies]
assert_cmd = "2"
tempfile = "3"

[profile.release]
lto = "thin"
strip = true
```

| Crate | Why it is here |
| --- | --- |
| `clap` | Argument parsing with derive macros. The `env` feature backs `VUTA_DB` and `VUTA_COLOR`. |
| `clap_complete` | Shell completion: the registration scripts of `vuta completions`, and the dynamic engine that answers each TAB from `cli.rs` and the registry. The engine is behind the `unstable-dynamic` feature, so the version is pinned with `=` and an upgrade is a deliberate change (section 6). |
| `rusqlite` | SQLite access. `bundled` compiles SQLite in, so Windows needs no system library and both platforms get the same SQLite version. |
| `directories` | Finds the per-user data directory on each OS. |
| `anstream`, `anstyle` | Colour output. clap already depends on both, so they add nothing to the build. Section 9 covers their use. |
| `anstyle-query` | Turns on the Windows console's handling of escape sequences, so live progress can move the cursor. anstream already depends on it. |
| `thiserror` | The typed `Error` enum that maps to exit codes. |
| `anyhow` | Error context inside commands, where the exact type does not matter. |
| `serde`, `serde_json` | `--format json` output, and the registry file of `vuta export` and `vuta import`. |
| `ctrlc` | Ctrl-C handling on both platforms. `termination` adds SIGTERM. |
| `assert_cmd`, `tempfile` | Integration tests that run the real binary against throwaway directories. |

### Deliberately not used

- **`git2` / libgit2.** Vuta shells out to `git`, which reuses your SSH config and credential helpers and matches how every later backend will work.
- **An async runtime.** The work is a handful of child processes. Scoped threads from the standard library are enough.
- **A migration crate.** The runner is about 15 lines (section 4) and avoids pinning to another crate's `rusqlite` version range.
- **A date-time crate.** Timestamps are Unix seconds from SQLite's `unixepoch()`. SQLite formats them for display.
- **A table crate.** `ui/table.rs` pads columns itself, which keeps padding correct when cells carry colour codes.
- **A progress-bar crate.** Live progress is a few lines that `ui/live.rs` erases and redraws with three escape sequences (section 9).

## 4. Data storage

Everything Vuta knows is in one SQLite file in the per-user local data directory.

### Location

| OS | Database file |
| --- | --- |
| Linux | `$XDG_DATA_HOME/vuta/vuta.db`, normally `~/.local/share/vuta/vuta.db` |
| Windows | `%LOCALAPPDATA%\vuta\data\vuta.db` |
| macOS | `~/Library/Application Support/vuta/vuta.db` |

The path is resolved in this order: the `--db <PATH>` flag, the `VUTA_DB` environment variable, then the platform default. `vuta db path` prints the result.

Windows uses the Local folder, not Roaming, because the database holds absolute paths that are only valid on that machine.

```rust
// paths.rs
pub fn default_db_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "vuta")?;
    Some(dirs.data_local_dir().join("vuta.db"))
}
```

### Opening and migrating

Every command opens the database the same way. Migrations are SQL files compiled into the binary, and `PRAGMA user_version` records how many have run.

```rust
// db/mod.rs
const MIGRATIONS: &[&str] = &[include_str!("../../migrations/0001_init.sql")];

pub fn open(path: &Path) -> Result<Connection, DbError> {
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    let known = MIGRATIONS.len() as u32;
    let found: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if found > known {
        return Err(DbError::SchemaTooNew { found, known });
    }
    for version in found..known {
        let tx = conn.transaction()?;
        tx.execute_batch(MIGRATIONS[version as usize])?;
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }
    Ok(())
}
```

- Each migration and its version bump commit together, so a crash never leaves a half-applied schema.
- A database newer than the binary is refused with a message to upgrade Vuta. It is never downgraded.
- A shipped migration file is never edited. Schema changes are new numbered files.

### Schema (`migrations/0001_init.sql`)

```sql
CREATE TABLE roots (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    path        TEXT    NOT NULL UNIQUE,
    is_default  INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE UNIQUE INDEX roots_one_default ON roots (is_default) WHERE is_default = 1;

CREATE TABLE repos (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    root_id         INTEGER NOT NULL REFERENCES roots (id) ON DELETE RESTRICT,
    rel_path        TEXT    NOT NULL,
    vcs             TEXT    NOT NULL DEFAULT 'git' CHECK (vcs IN ('git')),
    url             TEXT    NOT NULL,
    branch          TEXT,
    clone_filter    TEXT,
    clone_depth     INTEGER CHECK (clone_depth IS NULL OR clone_depth > 0),
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    last_rev        TEXT,
    last_outcome    TEXT,
    last_synced_at  INTEGER,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE (root_id, rel_path)
) STRICT;

CREATE TABLE repo_tags (
    repo_id  INTEGER NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    tag      TEXT    NOT NULL COLLATE NOCASE,
    PRIMARY KEY (repo_id, tag)
) STRICT, WITHOUT ROWID;

CREATE INDEX repo_tags_by_tag ON repo_tags (tag);

CREATE TABLE hooks (
    id        INTEGER PRIMARY KEY,
    repo_id   INTEGER NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    position  INTEGER NOT NULL DEFAULT 0,
    platform  TEXT    NOT NULL DEFAULT 'any' CHECK (platform IN ('any', 'unix', 'windows')),
    run_when  TEXT    NOT NULL DEFAULT 'changed' CHECK (run_when IN ('changed', 'always')),
    command   TEXT    NOT NULL,
    enabled   INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
) STRICT;

CREATE TABLE runs (
    id            INTEGER PRIMARY KEY,
    started_at    INTEGER NOT NULL DEFAULT (unixepoch()),
    finished_at   INTEGER,
    vuta_version  TEXT    NOT NULL,
    args          TEXT    NOT NULL,
    exit_code     INTEGER
) STRICT;

CREATE TABLE run_results (
    id            INTEGER PRIMARY KEY,
    run_id        INTEGER NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    repo_id       INTEGER NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    outcome       TEXT    NOT NULL CHECK (outcome IN ('cloned', 'updated', 'up_to_date', 'skipped', 'failed')),
    reason        TEXT,
    old_rev       TEXT,
    new_rev       TEXT,
    commits       INTEGER,
    duration_ms   INTEGER NOT NULL,
    hook_outcome  TEXT CHECK (hook_outcome IN ('ok', 'failed', 'not_run')),
    message       TEXT
) STRICT;

CREATE INDEX run_results_by_repo ON run_results (repo_id, run_id);

CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
) STRICT, WITHOUT ROWID;
```

### Per-repository sync options (`migrations/0002_sync_options.sql`)

```sql
ALTER TABLE repos ADD COLUMN allow_dirty INTEGER NOT NULL DEFAULT 0
    CHECK (allow_dirty IN (0, 1));
ALTER TABLE repos ADD COLUMN submodules INTEGER NOT NULL DEFAULT 0
    CHECK (submodules IN (0, 1));
ALTER TABLE repos ADD COLUMN strategy TEXT NOT NULL DEFAULT 'ff-only'
    CHECK (strategy IN ('ff-only', 'rebase'));
ALTER TABLE run_results ADD COLUMN rebased INTEGER;
ALTER TABLE run_results ADD COLUMN submodules TEXT
    CHECK (submodules IN ('ok', 'failed'));
```

Every option defaults to the v1 behaviour, so a database upgraded from schema 1 syncs exactly as before. `run_results.rebased` counts the local commits an update replayed (NULL when none were). `run_results.submodules` is NULL when the submodule step did not run; when it failed, the repository's own outcome stands and git's output goes in `message`.

### What each table is for

| Table | Holds | Notes |
| --- | --- | --- |
| `roots` | Named base directories | At most one row is the default. |
| `repos` | One row per managed checkout | Location is `roots.path` joined with `rel_path`. `last_*` columns cache the latest result so `vuta list` needs no join. `allow_dirty`, `submodules` and `strategy` are the sync options of section 7. |
| `repo_tags` | Free-form labels | Group repositories, for example by language. Drives `vuta sync --tag rust`. |
| `hooks` | Commands to run after a sync | Ordered by `position` within a repository. |
| `runs` | One row per `vuta sync`, and per `vuta add` that clones | `finished_at` stays NULL if the process was killed. `args` is the command line without the program name. |
| `run_results` | One row per repository per run | Feeds `vuta log` and `vuta show`. `hook_outcome` stays NULL for a repository with no hooks; it is `not_run` when hooks exist but none ran (`--no-hooks`, nothing eligible, or Ctrl-C). `submodules` works the same way for the submodule step. |
| `settings` | Key-value preferences | `sync.jobs` (default 4), `history.keep_runs` (default 200), `ui.glyphs` (`auto`, `unicode`, `ascii`). |

### Conventions

- **Paths.** `roots.path` is absolute, made with `std::path::absolute`. `fs::canonicalize` is avoided because on Windows it returns `\\?\` paths that many tools reject. `rel_path` always uses `/` and is converted on use.
- **Moving a partition.** `vuta root move <name> <new-path>` updates one row and every repository under it follows.
- **Names.** Repository, root and tag names compare case-insensitively, so `vuta sync Zed` and `vuta sync zed` are the same.
- **Time.** All timestamps are Unix seconds in UTC.
- **Atomic units.** `db::atomic` runs a closure inside a SAVEPOINT. Functions that write several rows (`roots::add`, `repos::insert`, `runs::record_result`) use it, and because savepoints nest, a caller can wrap several of them in one more `atomic` to make the whole sequence all-or-nothing.
- **History size.** After each sync, runs older than the newest `history.keep_runs` are deleted. Their results go with them by cascade.

## 5. Domain model and the VCS abstraction

The sync engine speaks to a `Vcs` trait. Git is its only implementation in v1, and it works by running the `git` executable.

### Core types (`model.rs`)

```rust
pub struct Root { pub id: i64, pub name: String, pub path: PathBuf, pub is_default: bool }

pub struct Repo {
    pub id: i64,
    pub name: String,
    pub root_id: i64,
    pub rel_path: String,          // always '/' separators
    pub vcs: VcsKind,
    pub url: String,
    pub branch: Option<String>,    // None = the remote's default branch
    pub clone_filter: Option<String>,
    pub clone_depth: Option<u32>,
    pub enabled: bool,
    pub allow_dirty: bool,         // fast-forward around untouched local changes
    pub submodules: bool,          // update submodules after a clone or update
    pub strategy: Strategy,        // FfOnly (the default) or Rebase
}

pub enum VcsKind { Git }

pub enum Outcome {
    Cloned     { rev: String, submodules: Option<Submodules> },
    Updated    { old: String, new: String, commits: u32, rebased: u32,
                 submodules: Option<Submodules> },
    UpToDate   { rev: String, ahead: u32, submodules: Option<Submodules> },
    Skipped    { reason: SkipReason, behind: u32 },
    Failed     { reason: FailReason, message: String },
}

pub enum Submodules { Ok, Failed { message: String } }   // None: the step did not run

pub enum SkipReason {
    Dirty          { overlap: Vec<String> },  // overlap: only with allow_dirty
    Diverged       { ahead: u32 },
    Detached       { rev: String },
    NoUpstream     { branch: String },
    RemoteMismatch { found: String, expected: String },
    Disabled,
    FetchOnly,     // --fetch-only fetched but left the tree alone
    RebaseStopped  { ahead: u32, why: String },  // aborted; the checkout is as it was
}
pub enum FailReason { NotACheckout, Clone, Fetch, FastForward, Rebase, Interrupted }
```

`Outcome` is the one value a worker returns per repository. The database row, the coloured result line and the exit code are all derived from it. Skip reasons carry what their result line needs (section 7); `SkipReason::describe` gives the plain text stored in `run_results.message`. The submodule step's result rides on the outcome it follows instead of replacing it, the way a hook's does.

`model.rs` also holds the row types the other layers exchange: `Hook`, `Run`, `RunResult`, `RepoEntry` (a repository with its root name, tags and cached last result), and `Tally` (outcome counts, failed hooks and failed submodule steps, for the summary line and `vuta log`). Every enum stored as text (`OutcomeKind`, `SkipKind`, `FailReason`, `HookOutcome`, `SubmoduleOutcome`, `Strategy`, `Platform`, `RunWhen`, `GlyphMode`) parses case-insensitively from the same strings the schema's CHECK constraints allow.

### The trait (`vcs/mod.rs`)

```rust
pub struct CloneSpec<'a> {
    pub url: &'a str,
    pub branch: Option<&'a str>,
    pub filter: Option<&'a str>,
    pub depth: Option<u32>,
}

pub enum Head {
    Branch { name: String, rev: String },
    Detached { rev: String },
    Unborn,
}

pub struct Divergence { pub ahead: u32, pub behind: u32 }

pub enum Rebase { Done, Stopped { why: String } }   // Stopped: aborted, nothing changed

pub enum SubmoduleStatus { Current, Uninitialized, Moved, Conflict }
pub struct Submodule { pub path: String, pub rev: String, pub status: SubmoduleStatus }

pub trait Vcs: Send + Sync {
    fn is_checkout(&self, path: &Path) -> bool;
    fn remote_url(&self, path: &Path) -> anyhow::Result<Option<String>>;
    fn head(&self, path: &Path) -> anyhow::Result<Head>;
    fn is_dirty(&self, path: &Path, ignore_submodules: bool) -> anyhow::Result<bool>;
    fn local_changes(&self, path: &Path, ignore_submodules: bool) -> anyhow::Result<Vec<String>>;
    fn incoming_changes(&self, path: &Path) -> anyhow::Result<Vec<String>>;
    fn clone_into(&self, spec: &CloneSpec, dest: &Path) -> anyhow::Result<()>;
    fn fetch(&self, path: &Path) -> anyhow::Result<()>;
    /// `None` when the current branch has no upstream.
    fn divergence(&self, path: &Path) -> anyhow::Result<Option<Divergence>>;
    fn fast_forward(&self, path: &Path) -> anyhow::Result<()>;
    /// Never leaves a rebase in progress; an error means the abort failed too.
    fn rebase(&self, path: &Path) -> anyhow::Result<Rebase>;
    fn submodules(&self, path: &Path) -> anyhow::Result<Vec<Submodule>>;
    fn head_is_referenced(&self, path: &Path) -> anyhow::Result<bool>;
    fn update_submodules(&self, path: &Path) -> anyhow::Result<()>;
}
```

`vcs::backend(kind, tracer)` returns the implementation for a `VcsKind`. The optional tracer sees every command and its output, which is how `--verbose` reaches `ui` without the backend printing anything.

The methods are small questions and single actions. All policy (what to do when dirty, diverged or detached, whether local changes overlap upstream, whether a submodule may move) lives in `sync/worker.rs`, so a second backend inherits the same safety rules. The one exception is `rebase`, whose abort belongs to the action itself: no caller ever sees a rebase left in progress.

### Git backend (`vcs/git.rs`)

| Method | Command |
| --- | --- |
| `is_checkout` | `git -C <p> rev-parse --show-toplevel`, and the result must equal `<p>` |
| `remote_url` | `git -C <p> remote get-url origin` |
| `head` | `git -C <p> symbolic-ref --quiet --short HEAD` (exit 1 means detached), then `git -C <p> rev-parse HEAD` |
| `is_dirty` | `git -C <p> status --porcelain --untracked-files=no [--ignore-submodules=all]` prints anything |
| `local_changes` | `git -C <p> status --porcelain -z --untracked-files=no --no-renames [--ignore-submodules=all]`, staged and unstaged paths |
| `incoming_changes` | `git -C <p> diff --name-only --no-renames -z HEAD @{upstream}` |
| `clone_into` | `git clone --origin origin [--branch B] [--filter F] [--depth N] <url> <dest>` |
| `fetch` | `git -C <p> fetch --prune --no-recurse-submodules origin` |
| `divergence` | `git -C <p> rev-list --left-right --count HEAD...@{upstream}` prints `ahead<TAB>behind` |
| `fast_forward` | `git -C <p> -c merge.autoStash=false merge --ff-only @{upstream}` |
| `rebase` | `git -C <p> -c rebase.autoStash=false -c rebase.autoSquash=false -c rebase.updateRefs=false rebase @{upstream}`; if it fails with `rebase-merge` or `rebase-apply` present, `git rebase --abort` |
| `submodules` | `git -C <p> submodule status --recursive`: ` `, `-`, `+` or `U`, the commit, the path |
| `head_is_referenced` | `git -C <p> symbolic-ref --quiet HEAD` succeeds, or `git for-each-ref --count=1 --contains HEAD` prints a ref |
| `update_submodules` | `git -C <p> submodule update --init --recursive --checkout` |

This sequence was run against a local repository with Git 2.43 on 12 Nov 2026. Each command behaved as the table says, including the diverged, detached and no-upstream cases.

- **`is_checkout` compares the top level** because a plain folder inside another repository also answers "yes" to simpler checks.
- **Untracked files do not count as dirty.** Build outputs such as V's compiled binary would otherwise block every update.
- **Nothing stashes.** `merge.autoStash` and `rebase.autoStash` are forced off for the one command, so a user's config can never make Vuta stash local changes and pop them back with conflicts. `rebase.autoSquash` and `rebase.updateRefs` are off too, so a rebase only replays the branch's own commits and moves no other branch. Settings git does not know are ignored, so older versions accept them.
- **Fetch leaves submodules alone.** Git's default `fetch.recurseSubmodules=on-demand` would fail the whole fetch when a submodule's remote is unreachable. The submodule step fetches what submodules need.
- **Submodules check out, nothing else.** `--checkout` overrides an `update = merge` or `rebase` mode from `.gitmodules`, which could stop part-way inside a submodule.
- **Clones land in `<dest>.vuta-partial`** and are renamed on success. An interrupted clone never leaves a half-made checkout at the real path. Stale partial directories are removed before a new clone.

### Process environment (`proc.rs`)

Every `git` call goes through one function that sets the same environment and captures stdout and stderr.

| Setting | Purpose |
| --- | --- |
| `GIT_TERMINAL_PROMPT=0` | Fail instead of waiting for a username or password |
| `GCM_INTERACTIVE=never` | The same, for Git Credential Manager on Windows |
| `LC_ALL=C` | Stable, English output to parse and to store |
| `-c http.lowSpeedLimit=1000 -c http.lowSpeedTime=30` | Abort an HTTP transfer stalled below 1 kB/s for 30 seconds |
| `-c core.longpaths=true` (Windows only) | Allow paths longer than 260 characters |

Credentials themselves are untouched: SSH keys, `~/.ssh/config` and credential helpers work exactly as they do when you run `git` by hand.

### Remote URLs (`vcs/remote.rs`)

- **Name inference.** The last path segment without `.git`: `https://github.com/zed-industries/zed.git` gives `zed`. A clash with an existing name is an error that asks for `--name`.
- **Comparison.** Two URLs match when host and path match after lowercasing the host and dropping the scheme, user, `.git` suffix and trailing slash. `git@github.com:vlang/v.git` and `https://github.com/vlang/v` are the same repository.
- **No URL crate.** The `git@host:path` form is not a URL, so a small hand-written parser handles all three shapes.

## 6. Command reference

The command is `vuta <subcommand>`. Eighteen subcommands cover the registry, syncing, inspection, moving between machines and shell completion; section 12 says in which milestone each one lands.

### Global options

| Option | Environment | Meaning |
| --- | --- | --- |
| `--db <PATH>` | `VUTA_DB` | Use this database file instead of the platform default |
| `--color <auto\|always\|never>` | `VUTA_COLOR` | When to colour output (section 9) |
| `-q`, `--quiet` |  | Print only warnings, errors and the summary |
| `-v`, `--verbose` |  | Also print each `git` command and its output |
| `-h`, `--help`, `-V`, `--version` |  | Provided by clap |

### Subcommands

| Command | What it does |
| --- | --- |
| `vuta root add <name> <path> [--default]` | Register a base directory. The first root becomes the default. |
| `vuta root list` | Show roots and how many repositories each holds |
| `vuta root default <name>` | Change the default root |
| `vuta root move <name> <new-path>` | Point a root at a new location. No files are moved. |
| `vuta root remove <name>` | Remove a root that has no repositories |
| `vuta add <url> [path]` | Register a repository and clone it |
| `vuta scan [dir]` | Register checkouts that already exist on disk |
| `vuta sync [name]...` | Clone what is missing and update the rest |
| `vuta list` (alias `ls`) | One line per repository |
| `vuta status [name]...` | Local state of each checkout, without touching the network |
| `vuta show <name>` | Everything about one repository: location, tags, hooks, recent results |
| `vuta set <name> [options]` | Change name, URL, branch, path, enabled state, or how it syncs |
| `vuta tag <name> <+tag\|-tag>...` | Add or remove tags, for example `vuta tag zed +rust -editor` |
| `vuta remove <name>` (alias `rm`) | Forget a repository. `--delete-files` also deletes the checkout after a confirmation. |
| `vuta hook add\|list\|remove` | Manage post-sync commands (section 8) |
| `vuta log [name]` | Recent sync runs and their results |
| `vuta export [-o FILE]` | Write roots, repositories, tags, hooks and changed settings as JSON |
| `vuta import <FILE> [--map-root NAME=PATH]...` | Add what an exported file holds, with roots remapped |
| `vuta config get\|set\|unset\|list` | Read and write rows in `settings`. `unset` returns a key to its default. |
| `vuta completions <shell>` | Print the script that registers completion for `bash`, `zsh`, `fish`, `powershell` or `elvish`. Names come from the registry. |
| `vuta db path` | Print the database location |
| `vuta doctor` | Check that `git` is on PATH, the database is sound, and every root exists |

### `vuta add`

```text
vuta add <url> [path] [--name N] [--root R] [--tag T]... [--branch B]
                      [--filter F] [--depth N] [--submodules] [--no-clone]
```

- `path` is relative to the root and defaults to the inferred name. `vuta add https://github.com/zed-industries/zed.git rust/zed --tag rust` places Zed under `rust/zed` in the default root.
- `--filter blob:none` makes a partial clone, which suits large repositories you only build, such as Zed or Modular.
- Without `--no-clone`, the clone is an ordinary one-repository run: it takes the sync lock before registering, prints one result line, is recorded in `runs`, and sets the exit code like `vuta sync`. A failed clone leaves the repository registered for the next sync.
- `--no-clone` only records the repository. The next `vuta sync` clones it.
- `--submodules` turns on the submodule step (section 7), so the clone comes with its submodules. It is the only sync option `add` takes, because it is the only one that matters to a fresh clone; `vuta set` sets the others.
- **First use.** With no roots registered and no `--root`, a person at a terminal is asked whether the current directory should become the first root, named after the directory (`src` when that name is not valid). Yes registers it as the default and the add goes on; no, or no terminal on stdin and stderr, exits 3 with the command that adds it, such as `vuta root add code /home/me/code`. Scripts never see a question. The question comes after the arguments are checked, so a typo creates no root.
- A remote given as a local path that exists, such as `../mirror/zed.git`, is stored as an absolute path, so the repository syncs from any directory. Other remotes are stored as typed.
- The name must not clash with another repository's, compared case-insensitively. Names, root names and tags must not be empty, contain whitespace or start with `+` or `-`.

### Registry commands

| Command | Details |
| --- | --- |
| `vuta root add <name> <path> [--default]` | A relative path is made absolute against the current directory. A path that does not exist yet is accepted with a warning; the first clone creates it. |
| `vuta root list` | Name, repository count, path (marked `(missing)` when it does not exist) and which root is the default. |
| `vuta root remove <name>` | Refused while repositories use the root. Removing the default root leaves no default, with a warning. |
| `vuta list [--tag T]... [--root R]` | Includes disabled repositories, dimmed and marked `(disabled)`. A ROOT column appears when more than one root is registered. |
| `vuta show <name>` | Root, path, URL, branch, clone options, sync options (as `sync  rebase, allow-dirty, submodules`), enabled state, tags, the latest result, hooks, the last ten results, and the full git output of the newest failure, of the repository or of its submodules. |
| `vuta set <name> ...` | `--name`, `--url`, `--branch` or `--default-branch`, `--path`, `--root`, `--filter` or `--no-filter`, `--depth` or `--no-depth`, `--enable` or `--disable`, and the sync options: `--allow-dirty` or `--no-allow-dirty`, `--submodules` or `--no-submodules`, `--strategy ff-only\|rebase`. At least one is required. Only the record changes: if the location changes, Vuta warns that no files were moved. |
| `vuta tag <name> [+tag\|-tag\|tag]...` | A bare tag is added. Removals apply before additions. With no tags it prints the repository's tags. |
| `vuta remove <name> [--delete-files [-y]]` | Without `--delete-files` the checkout stays on disk. With it, Vuta deletes the path only if it is a Git checkout, after a `[y/N]` question on stderr (answered on stdin) that mentions local changes. Any answer but `y` or `yes` cancels everything and exits 1. `-y` skips the question. |

### `vuta scan`

```text
vuta scan [dir] [--root R] [--tag-from-parent] [--max-depth N] [--dry-run]
```

- Walks `dir` (default: the path of `--root`, else of the default root; 4 levels deep) and registers every Git checkout that has an `origin` remote. A directory is a candidate when it contains `.git`, and `Vcs::is_checkout` confirms it. Symbolic links are not followed.
- Does not descend into a checkout once it finds one, so submodules and vendored repositories are not registered separately.
- Each checkout is registered under the root that contains it, the deepest one if roots nest. `--root R` restricts registration to that root.
- The name is inferred from the `origin` URL, the same way as for `vuta add`. The branch is not recorded, so a later clone takes the remote's default branch.
- `--tag-from-parent` tags each repository with its parent directory name: `rust/zed` gets the tag `rust`. A checkout directly under its root gets no tag.
- A checkout with no `origin`, outside every root (or outside `--root`), or whose inferred name is already taken, is listed as skipped with the reason; a taken name comes with the `vuta add ... --name` command that registers it. An unreadable directory is skipped the same way.
- A checkout already registered at that location is counted as "already registered" and not listed, so scanning again is safe.
- `--dry-run` lists what would be added and registers nothing.
- **First use.** With no roots registered and no `--root`, scan offers the directory it was given (else the current directory) as the first root, exactly as `vuta add` does. `--dry-run` never registers one; it only names the command.

### `vuta status`

```text
vuta status [name]... [--tag T]... [--root R]
```

- One row per repository: name, branch, short revision and state. Disabled repositories are included and shown as `disabled`.
- The state follows the same checks as a sync, without fetching: `not cloned`, `not a Git checkout`, `origin is <url>`, `no origin remote`, `HEAD is detached`, `no commits yet`, or a combination of `local changes`, `N local commits`, `N commits behind` and `no upstream`, else `clean`. Dirty, behind and missing states are yellow.
- Behind counts compare with the remote-tracking branch as of the last fetch; `vuta sync --fetch-only` refreshes them.
- It exits 0 whatever the states are.

### `vuta export` and `vuta import`

```text
vuta export [-o FILE]
vuta import <FILE|-> [--map-root NAME=PATH]... [--no-hooks] [--dry-run]
```

They set up a second machine from the first: the same repositories, tags and hooks, under roots that may live elsewhere.

- **The file** is JSON with `"format": "vuta-registry"` and `"version": 1` (`src/transfer.rs` documents it). It holds roots with their paths and default flag, repositories with root, relative path, URL, branch, clone options, enabled state, sync options (`allow_dirty`, `submodules`, `strategy`), tags and hooks, and settings that differ from their defaults. Run history and cached revisions stay behind, since they describe one machine.
- **Sync options** were added to version 1 files with defaults (`false`, `false`, `"ff-only"`), so older files still import. An older Vuta ignores them and syncs fast-forward only. An unknown strategy is refused when the file is validated, and the plan lists a repository's options when any is not the default.
- **Writing.** `-o FILE` writes UTF-8 directly. Prefer it on Windows, where PowerShell 5.1's `>` writes UTF-16; `import` reads UTF-16 and UTF-8 with or without a byte-order mark anyway.
- **Roots** are resolved in order: `--map-root NAME=PATH` puts the file's root NAME at PATH (joining a local root that already has that path, else creating NAME there); without a mapping, a local root of the same name, then one with the same path, is used; otherwise the root is created at its exported path, which must be absolute here. A Windows path is not absolute on Linux, and the error names the `--map-root` to add.
- **Only adds.** A repository whose name is registered already is skipped (the line says when its URL differs), as is one whose location is taken. A setting is applied only where the local value is still the default. A default root is set only if there is none. Nothing is cloned: `vuta sync` does that next.
- **All or nothing.** The file is validated first (names, paths, tags, hook platforms), and the changes run inside one `db::atomic` unit, so an error leaves the database as it was. Importing the same file twice adds nothing the second time.
- **Hooks** of added repositories are imported and each is listed with its platform and command, since hooks run on every sync. `--no-hooks` leaves them out. Disabled hooks are not imported.
- **`--dry-run`** prints the same plan and changes nothing.

### `vuta completions`

```text
vuta completions <bash|zsh|fish|powershell|elvish>
```

- Prints a short script that registers `vuta` with the shell. On each TAB the shell runs `COMPLETE=<shell> vuta -- <words of the line>`, and `main` answers that through clap_complete's dynamic engine (`CompleteEnv`) before parsing anything. `COMPLETE=bash vuta` prints the same kind of script, calling the binary by the path it was run as.
- It completes subcommands, flags, fixed values (`--color`, `--platform`, `--format`, the shell name), paths where `cli.rs` marks an argument with a `ValueHint`, and from the registry: repository names (`sync`, `status`, `show`, `set`, `tag`, `remove`, `hook add`, `hook list`, `log`), root names (`--root`, `root default`, `root move`, `root remove`), tags (`--tag`, and `+tag`/`-tag` for `vuta tag`), hook ids (`hook remove`) and setting keys (`config get`, `set`, `unset`). In zsh, fish and PowerShell each name comes with a description: a repository's `root:path`, a root's path, how many repositories have a tag, a hook's repository and command.
- **The completers in `commands/completions.rs` must never get in the way.** They use the database the line being completed names (`--db PATH` or `--db=PATH`, then `VUTA_DB`, then the platform default), open it read-only and only if the file exists, so a TAB never creates, migrates or locks a database, and they return nothing on any error. A completion takes about 2 ms.
- Printing the script reads no database and resolves no database path, so a shell's init file can run it anywhere.
- The zsh script, when zsh autoloads it from `fpath` as `_vuta` (as the `.deb` installs it), also calls the function it defines. clap_complete's script only registers the function, so without this the first TAB in each new shell would offer nothing.
- **Pinned engine.** The engine is behind clap_complete's `unstable-dynamic` feature. `Cargo.toml` pins the exact version; upgrading it means checking the protocol between the script and the binary again. A script written to a file by an older vuta may stop working after an upgrade, which is why the README sources it at shell startup. The `.deb` regenerates its scripts with each build, so they always match the binary they ship with (docs/PACKAGING.md).

### `vuta log`

```text
vuta log [name] [-n N]
```

- Without a name: the newest N runs (default 10), newest first. Each shows its id, age, exit code (or `unfinished` if the process was killed), command line, the summary line, and one line per repository whose result was not "up to date" or whose hook failed.
- With a name: that repository's newest N results with run id, age, result, hook outcome and detail.

### `vuta sync`

```text
vuta sync [name]... [--tag T]... [--root R] [-j N] [--fetch-only]
          [--no-hooks] [--force-hooks] [--strict] [--dry-run]
```

- With no names or filters it syncs every enabled repository. Names, `--tag` and `--root` combine: each one given must match, and repeated tags match any of them.
- A disabled repository named on the command line is included and reported as skipped with reason `disabled`; filters alone never select one.
- An unknown name or root exits 3 before anything runs. A selection that matches nothing prints "nothing to sync", records no run and exits 0.
- `-j N` (1 to 64) overrides `sync.jobs` for this run.
- `--fetch-only` fetches and reports how far behind each checkout is (as a `fetch_only` skip), without changing any working tree. A missing path is still cloned.
- `--strict` makes a skipped repository count as a failure for the exit code.
- `--dry-run` prints, from the disk alone, whether each repository would be cloned, fetched or skipped as disabled, and exits. It takes no lock and records no run.

### Adopting an existing tree

```text
vuta root add src D:\                     # or /mnt/src on Linux
vuta scan --tag-from-parent               # registers what is already cloned
vuta add https://github.com/tursodatabase/turso.git rust/turso --tag rust   # anything not yet cloned
vuta hook add v "make"     --platform unix
vuta hook add v "make.bat" --platform windows
vuta sync
```

Scan infers names from URLs, so Modular's repository registers as `modular`. `vuta set modular --name mojo` renames it.

## 7. Sync engine

A sync run fetches every selected repository in parallel and changes a working tree only when a clean fast-forward is possible, unless the repository's options (below) allow a little more.

### Phases of a run

1. **Select.** Load enabled repositories that match the names, tags and root given, plus disabled ones named explicitly.
2. **Lock.** Take an exclusive lock on `vuta.lock` beside the database with `File::try_lock`. If another sync holds it, exit with code 3 and say so.
3. **Record.** Insert a `runs` row.
4. **Sync.** Start the worker pool. Each worker takes a repository, decides what to do (below) and reports one `Outcome`.
5. **Hooks.** Run hooks one repository at a time, in name order, for repositories whose revision changed.
6. **Finish.** Update `runs`, prune old history, print the summary, and exit with the code from section 10.

`commands/sync.rs` runs phases 3 to 6 in one function, `execute`, which `vuta add` also calls for its clone after taking the lock itself.

### Decisions for one repository

&#91;embedded content: per-repository sync · 7 checks, 2 actions, 9 outcomes\]

Checks run top to bottom and the first one that fails decides the outcome. Green outcomes exit 0, amber ones are skips, red ones are failures. A clone or fast-forward that git rejects becomes `failed` with reason `clone` or `fast_forward`. The diagram shows the defaults; the per-repository options below change checks 5, 6 and 7, and add a step after a clone or update.

| Result | Reason | What the line says |
| --- | --- | --- |
| skipped | `remote_mismatch` | origin is `<url>`, expected `<url>` |
| skipped | `detached` | HEAD is detached at `<rev>` |
| skipped | `no_upstream` | branch `<name>` tracks no remote branch |
| skipped | `diverged` | 2 local commits, 5 behind |
| skipped | `dirty` | local changes; 3 commits behind |
| skipped | `dirty` | local changes overlap upstream in `file.txt`; 3 commits behind (with `allow_dirty`) |
| skipped | `rebase_stopped` | rebase stopped (conflict in `file.txt`) and was undone; 1 local commit, 3 behind |
| skipped | `fetch_only` | 3 commits behind (only with `--fetch-only`) |
| failed | `not_a_checkout` | path exists but is not a Git checkout |
| failed | `clone`, `fetch`, `fast_forward` | the last line of git's error output |
| failed | `rebase` | the abort of a stopped rebase failed too; the line says how to finish it by hand |

A checkout that is ahead of its upstream and not behind counts as up to date, with "2 local commits" noted on the line.

### Per-repository options

Three options, all off by default, are set with `vuta set` and stored in `repos` (section 4). With all three off, a sync behaves exactly as described above.

| Option | Set with | Changes |
| --- | --- | --- |
| Fast-forward around local changes | `--allow-dirty` | Check 7: a checkout with modified tracked files is fast-forwarded when none of them is a file upstream changes. |
| Strategy | `--strategy ff-only\|rebase` | Check 6: with `rebase`, a diverged checkout replays its local commits onto upstream instead of being skipped. |
| Submodules | `--submodules` (also on `vuta add`) | After a clone or update, `git submodule update --init --recursive --checkout`. On an up-to-date checkout too, when a submodule is missing or not at its recorded commit. |

**Fast-forward around local changes.** Vuta first compares the paths `git status` reports as changed (staged or not) with the paths `git diff HEAD @{upstream}` changes. Any overlap is a `dirty` skip that names the file, or the number of files, and nothing is run. Otherwise `git merge --ff-only` updates the tree. Git also refuses, changing nothing, when local changes would be overwritten, so a case the comparison misses fails as `fast_forward` with git's reason rather than touching a file. The local edits stay as they were, staged or not.

**Rebase.** A rebase needs a clean tree: a diverged checkout with local changes is skipped as `dirty` even with `allow_dirty`, which covers fast-forwards only. When git stops part-way (a conflict, an untracked file in the way, a hook that refuses), the backend runs `git rebase --abort`. The worker then checks that HEAD is back on the same branch at the same commit and reports a `rebase_stopped` skip, with git's reason, such as the conflicting file. If the abort fails, or HEAD is not back, the result is a `rebase` failure whose message names the command that finishes the job; it is never mistaken for an interruption. A successful rebase is `updated`, with "rebased 2 local commits" on the line and in `run_results.rebased`. This weakens "Vuta never rebases" only for repositories that ask for it.

**Submodules.**

- **Reporting.** The step's result is a note on the outcome it follows, as a failed hook is. A failure keeps `cloned`, `updated` or `up to date`, adds "submodules failed: <git's last line>" to the line, stores `run_results.submodules = 'failed'` with git's output in `message`, counts in the summary as "1 submodule update failed" and makes the run exit 1. Hooks still run, since the repository itself changed. Success adds "submodules ok".
- **Healing.** An up-to-date checkout whose submodules are missing or moved (a failed step earlier, or the option just turned on) runs the step again. One whose submodules are in place runs nothing.
- **No lost commits.** `git submodule update` moves a submodule's HEAD to the recorded commit, which would orphan a commit made on a detached HEAD inside it. Before the step, every submodule that is not where its parent recorded it, and was not there before the update, must have its HEAD on a branch or held by a branch, tag or remote-tracking branch. Otherwise the step does not run and the failure names the submodule. A submodule with local changes is left to git, which refuses to overwrite them.
- **Not dirty.** With the option on, a submodule at another commit than recorded is not a local change of the superproject. Otherwise one failed step would leave the checkout skipped as `dirty` from then on.
- **Not under `--fetch-only`.** A clone made under `--fetch-only` still gets its submodules, since that changes no existing tree, but an up-to-date checkout is not healed.

### Concurrency

Workers are scoped threads that pull jobs from a shared queue and send events over a channel. The calling thread is the only one that writes to SQLite and to the terminal, so there is no locking around either.

```rust
// sync/mod.rs
pub fn run(jobs: Vec<Job>, workers: usize, stop: &AtomicBool, mut on_event: impl FnMut(Event)) {
    let queue = Mutex::new(VecDeque::from(jobs));
    let (tx, rx) = mpsc::channel::<Event>();

    thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let queue = &queue;
            scope.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Some(job) = queue.lock().unwrap().pop_front() else { break };
                    let _ = tx.send(Event::Started(job.repo_id));
                    let _ = tx.send(Event::Finished(worker::sync_one(&job, stop)));
                }
            });
        }
        drop(tx);
        // The calling thread is the only one that writes to SQLite and the terminal.
        for event in rx {
            on_event(event);
        }
    });
}
```

- **Ending the loop.** With `--verbose`, the Git backend's tracer also holds a sender, so the channel never closes by itself. Each worker sends a final `WorkerDone` message, and the loop ends when every worker has sent one.
- **Worker count.** `sync.jobs`, default 4, capped at the number of repositories. The work is network-bound, so the count is not tied to the number of CPU cores.
- **Output order.** Result lines print as repositories finish, each with a `[3/9]` counter. On an interactive terminal a live line below them shows what each worker is on and for how long (section 9); otherwise a dim "cloning zed" line prints when a clone starts, because clones can run for minutes.
- **Ticks.** For live progress the engine also sends `Event::Tick` every 500 ms (`Options::tick`), from the same receiving loop, so the calling thread stays the only one that writes to the terminal.
- **Hooks stay serial.** Builds are CPU-heavy and their output is streamed live, so running two at once would interleave it.

### Interruption

- `ctrlc` sets an `AtomicBool`. Workers check it before taking the next job and between steps.
- On both Linux and Windows the terminal delivers Ctrl-C to the `git` children as well, so they stop on their own.
- In-flight repositories are recorded as failed with reason `interrupted`. Repositories not yet started get no row.
- The `runs` row is closed and Vuta exits with 130. A partial clone is cleaned up on the next run.

## 8. Hooks

A hook is a shell command attached to one repository that runs after that repository syncs, such as rebuilding V after it updates.

```text
vuta hook add <repo> <command> [--platform any|unix|windows] [--always]
vuta hook list [repo]
vuta hook remove <id>
```

| Aspect | Behaviour |
| --- | --- |
| When it runs | After a clone, or after an update that moved the revision. `--always` runs it on every sync. Never after a skip or a failure. A clone made under `--fetch-only` counts as a change. |
| Platform | A hook marked `unix` or `windows` is ignored on the other. The V build is two hooks: `make` and `make.bat`. |
| Shell | `sh -c <command>` on Unix, `cmd /C <command>` on Windows |
| Working directory | The repository's checkout |
| Environment | `VUTA_REPO_NAME`, `VUTA_REPO_PATH`, `VUTA_OLD_REV` (empty after a clone), `VUTA_NEW_REV` |
| Output | Streamed straight to the terminal under a header line, not captured |
| Order | By `position` within a repository. The first failure stops that repository's remaining hooks. |
| Failure | A non-zero exit sets `hook_outcome = failed` and makes the run exit 1. The repository's own outcome stays `updated`. |
| Overrides | `vuta sync --no-hooks` skips all hooks. `--force-hooks` runs them even when nothing changed. |

Hooks are arbitrary commands, so they only ever enter the database through `vuta hook add`, or through `vuta import` of a file you exported, which lists every hook it adds and takes `--no-hooks`. `vuta scan` and `vuta add` never read hooks from a repository's contents.

- `vuta hook add` refuses an empty command and warns when `--platform` excludes the current OS.
- `vuta hook list` shows id, repository, platform, when and command. Hooks that cannot run on this machine are dim. `vuta show` lists a repository's hooks too.
- `vuta hook remove` takes the id from `vuta hook list`. An unknown id exits 3. Removing a repository removes its hooks.
- Hooks run after all result lines, under a `Hooks` heading that appears only when at least one hook is eligible. On Ctrl-C the hook that is running finishes (or dies with the terminal's signal), no further hook starts, and the run exits 130.

## 9. Terminal output and colour

Colour in Vuta is semantic: each colour means one thing in every command. Styles are defined once in `ui/theme.rs`, and the rest of the code names a role, never a colour.

### Palette

| Role | Style | Meaning | Where it appears |
| --- | --- | --- | --- |
| `ok` | green | Changed successfully | `updated`, a finished hook, passed `doctor` checks |
| `new` | cyan | Created | `cloned`, repositories registered by `add` and `scan` |
| `muted` | dim | Nothing to act on | `up to date`, durations, timestamps, paths, git commands in `--verbose` |
| `warn` | yellow | Needs attention, nothing broken | `skipped`, warnings, dirty or behind in `status` |
| `err` | bold red | Failed | `failed`, the `error:` prefix |
| `name` | bold | Identifiers | Repository and root names, table headers |
| `rev` | magenta | Revisions | Short hashes and commit counts |

- **Only the 16 standard ANSI colours.** The terminal's own theme picks the shades, so output stays readable on light and dark backgrounds.
- **Never colour alone.** Every status also carries a glyph and a word, so piped output, logs and colour-blind readers lose nothing.
- **Blue is unused.** Dark blue on a dark background is hard to read in several default terminal themes.

```rust
// ui/theme.rs
use anstyle::{AnsiColor, Style};

pub struct Theme {
    pub ok: Style,
    pub new: Style,
    pub muted: Style,
    pub warn: Style,
    pub err: Style,
    pub name: Style,
    pub rev: Style,
}

pub const THEME: Theme = Theme {
    ok: AnsiColor::Green.on_default(),
    new: AnsiColor::Cyan.on_default(),
    muted: Style::new().dimmed(),
    warn: AnsiColor::Yellow.on_default(),
    err: AnsiColor::Red.on_default().bold(),
    name: Style::new().bold(),
    rev: AnsiColor::Magenta.on_default(),
};

// `{}` turns a style on and `{:#}` resets it.
anstream::println!("{}updated{:#}  {}zed{:#}", THEME.ok, THEME.ok, THEME.name, THEME.name);
```

### When colour is on

All output goes through `anstream`, which strips colour codes whenever colour is off. Call sites always write styled text and never check.

| Order | Condition | Result |
| --- | --- | --- |
| 1 | `--color always` or `--color never` (or `VUTA_COLOR`) | Wins outright |
| 2 | `NO_COLOR` is set | Off |
| 3 | `CLICOLOR_FORCE` is set | On, even when piped |
| 4 | Otherwise | On only when the stream is a terminal and `TERM` is not `dumb` |

```rust
// ui/mod.rs
pub fn apply_color(when: ColorWhen) {
    let choice = match when {
        ColorWhen::Auto => anstream::ColorChoice::Auto,
        ColorWhen::Always => anstream::ColorChoice::Always,
        ColorWhen::Never => anstream::ColorChoice::Never,
    };
    choice.write_global();
}
```

- stdout and stderr are decided separately, so `vuta list > repos.txt` writes a clean file while warnings stay coloured on screen.
- On Windows, `anstream` handles consoles that do not understand ANSI codes, so no Windows-specific colour code is needed.
- A scratch build on Linux confirmed rows 1 to 4: piped output had no escape codes, `--color always` overrode `NO_COLOR`, and `CLICOLOR_FORCE` forced colour through a pipe.

### Help text

clap's help uses the same palette through `Styles`, set once on the top-level command with `#[command(styles = HELP_STYLES)]`.

```rust
// cli.rs
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().bold())
    .usage(AnsiColor::Yellow.on_default().bold())
    .literal(AnsiColor::Green.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default())
    .error(AnsiColor::Red.on_default().bold())
    .valid(AnsiColor::Green.on_default())
    .invalid(AnsiColor::Yellow.on_default());
```

### Glyphs

| Status | Unicode | ASCII |
| --- | --- | --- |
| updated | `✓` | `ok` |
| cloned | `+` | `+` |
| up to date | `=` | `=` |
| skipped | `!` | `!` |
| failed | `✗` | `x` |
| hook running | `▸` | `>` |

`ui.glyphs = auto` uses Unicode on Linux, macOS and Windows Terminal, and ASCII in other Windows consoles, where the default fonts may lack these characters. Set it to `unicode` or `ascii` to force one.

### What each command prints

```text
$ vuta sync
Syncing 9 repositories, 4 at a time

[1/9] = up to date  surrealdb   a91c0de                              1.2s
[2/9] ✓ updated     zed         3f9a2c1 → 8be04d7   42 commits       6.1s
[3/9] ! skipped     edit        local changes; 3 commits behind      0.9s
[4/9] ✗ failed      openvpn     fetch: could not resolve host        0.4s
[5/9] + cloned      turso       5d2e7aa                             38.4s
...

Hooks
  ▸ v  make
    (build output, streamed live)
  ✓ v  hook finished                                               48.2s

1 cloned · 1 updated · 5 up to date · 1 skipped · 1 failed   in 41.0s
```

- The glyph and status word take the status colour. Names are bold, hashes and counts magenta, durations dim.
- In the summary each count takes its status colour, and zero counts are dim.

While a sync runs on a terminal, a live block below the result lines shows each busy worker, in the columns of a result line:

```text
[2/9] ✓ updated     zed         3f9a2c1 → 8be04d7   42 commits       6.1s
      ▸ cloning     turso       38s
      ▸ syncing     edit        2s
```

- `ui/live.rs` draws it on stderr from the engine's `Started`, `Cloning`, `Finished` and `Tick` events. Before each result line it erases the block (cursor up, then clear to the end of the screen) and draws it again after, so result lines scroll up exactly as they do without it. When the last repository finishes the block is empty, so it is gone before hooks stream their output. Dropping it clears the screen too, so an error mid-run leaves nothing behind.
- Erasing counts lines, so no line may wrap. Auto-wrap is off while the block is drawn (`ESC[?7l`, then `ESC[?7h`), so the terminal clips a long line, and when `COLUMNS` gives the width the name is cut first so the time stays visible. At most ten lines are drawn; more busy workers show as `and N more`.
- Only when `Ui::can_redraw` holds: normal verbosity, stdout and stderr both terminals, `TERM` not `dumb`, and on Windows a console that accepts escape sequences (Windows 10 and later, turned on through `anstyle-query`). Piped output, `--quiet`, `--verbose` (whose git traces would interleave), `--format json` and `TERM=dumb` keep the plain lines and the dim "cloning" line. The escape sequences bypass anstream, so `--color never` still gets live progress, without colour.

```text
$ vuta list
NAME       TAGS   PATH         REV       SYNCED
edit       rust   rust/edit    7c1d2ab   2 hours ago
v          v      v/v          e4f5a6b   3 days ago
zed        rust   rust/zed     8be04d7   2 hours ago
```

- Headers and names are bold, paths and ages dim, hashes magenta.
- A repository whose last result was skipped or failed shows its SYNCED cell in yellow or red.
- A disabled repository is dim across the whole row.

### JSON output

`vuta list`, `status`, `show`, `log`, `sync`, `scan`, `import`, `root list`, `hook list` and `config list` take `--format json`. Each then prints exactly one JSON value on stdout and no text there. Progress, warnings and errors stay on stderr. During `sync`, hooks' stdout is sent to stderr as well, so stdout always parses. `ui/json.rs` defines every shape in one place, built explicitly so that changing a model type cannot change the output by accident.

- **Conventions.** Keys are `snake_case`. A field without a value is `null`, never missing. Revisions are full hashes. Times are Unix seconds and durations milliseconds. Enumerations use the strings stored in the database (`up_to_date`, `fast_forward`). New fields may be added; existing ones keep their name and meaning.
- **A command that fails to start** (exit 3) prints only its `error:` line on stderr, and nothing on stdout.

| Command | stdout |
| --- | --- |
| `list` | An array of repositories: `name`, `root`, `rel_path`, `path` (absolute), `vcs`, `url`, `branch`, `clone_filter`, `clone_depth`, `enabled`, `allow_dirty`, `submodules`, `strategy` (`ff-only` or `rebase`), `tags`, `last_rev`, `last_outcome`, `last_synced_at` |
| `status` | An array: `name`, `enabled`, `path`, `state` (`not_cloned`, `not_a_checkout`, `no_origin`, `remote_mismatch`, `unborn`, `detached`, `branch` or `error`), `branch`, `rev`, `dirty`, `ahead`, `behind`, `has_upstream`, `origin` (the URL found, for `remote_mismatch`) and `message` (for `error`) |
| `log` | An array of runs, newest first: `id`, `started_at`, `finished_at`, `exit_code`, `args`, `vuta_version`, `tally` and every `results` entry |
| `log <name>` | An array of that repository's results, newest first |
| `sync` | An object: `run` (as in `log`, without results), `interrupted`, `elapsed_ms`, `tally` and `results`. `run` is `null` when nothing was selected. |
| `sync --dry-run` | An array: `name`, `action` (`clone`, `fetch` or `skip`), `path`, `url` |
| `show <name>` | An object: the fields of `list`, then `hooks` (as in `hook list`) and `results`, that repository's ten newest results, newest first |
| `root list` | An array: `name`, `path`, `default`, `repos` (how many repositories it holds), `exists` (whether its directory is there) |
| `hook list [repo]` | An array: `id`, `repo`, `command`, `platform` (`any`, `unix` or `windows`), `when` (`changed` or `always`), `enabled` |
| `config list` | An array: `key`, `value`, `default`, `is_default`, `description` |
| `scan` | An object: `dir`, `dry_run`, `added` (each `name`, `root`, `rel_path`, `path`, `url`, `tags`; with `--dry-run`, what would be added), `already_registered` (a count) and `skipped` (each `path` and `reason`) |
| `import` | An object: `dry_run`, `roots` (each `name`, `path`, `action` (`use` or `create`), `becomes_default`), `repos` (each `name`, `root`, `rel_path`, `url`, `branch`, `clone_filter`, `clone_depth`, `enabled`, `allow_dirty`, `submodules`, `strategy`, `tags`, `action` (`add` or `skip`), `reason` (for a skip) and `hooks`, each `command`, `platform`, `when`), `settings` (each `key`, `value`, `action` (`set`, `keep` or `skip`), `local_value` (for `keep`)) and `disabled_hooks`, how many disabled hooks were left out. The plan is the same with and without `--dry-run`. |

A result is `run_id`, `repo`, `outcome`, `reason`, `old_rev`, `new_rev`, `commits`, `duration_ms`, `hook_outcome`, `rebased`, `submodules` (`ok`, `failed`, or `null` when the step did not run) and `message`, exactly as stored in `run_results`. `sync` reads its results back from the database after recording them, so it and `log` cannot disagree. A tally is `cloned`, `updated`, `up_to_date`, `skipped`, `failed`, `hooks_failed` and `submodules_failed`.

```sh
vuta sync --format json | jq -r '.results[] | select(.outcome == "failed") | .repo'
vuta status --format json | jq -r '.[] | select(.behind > 0) | .name'
vuta import registry.json --dry-run --format json | jq -r '.repos[].hooks[].command'
```

### Rules for `ui`

- **Results go to stdout; progress, warnings and errors go to stderr.** `vuta list | grep rust` sees only the table.
- **Pad first, then style.** `ui/table.rs` measures plain text and wraps the padded cell in a style, so escape codes never shift a column.
- **`--quiet`** prints failures, skips and the summary. **`--verbose`** adds each `git` command and its output in dim text.

## 10. Errors and exit codes

The exit code tells a script or scheduled task what happened without parsing output.

| Code | Meaning |
| --- | --- |
| 0 | Everything requested succeeded. Skipped repositories are allowed unless `--strict` is set. |
| 1 | At least one repository, hook or submodule step failed |
| 2 | Usage error: unknown flag, missing argument. This is clap's own code. |
| 3 | Vuta could not start the work: `git` not found, database unreadable or newer than the binary, unknown repository, root or hook, an invalid name, path or setting, another sync or clone holds the lock |
| 130 | Interrupted with Ctrl-C |

### Two kinds of failure

- **A repository failing is data, not an error.** It becomes an `Outcome::Failed`, is stored and printed, and the run continues with the next repository. One unreachable host never stops the other eight.
- **A failure of Vuta itself is an `Error`.** It ends the command, prints one `error:` line in the `err` style on stderr, and maps to code 3.

```rust
// error.rs
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("git was not found on PATH")]
    GitMissing,
    #[error("no repository named '{0}'")]
    UnknownRepo(String),
    #[error("no root named '{0}'")]
    UnknownRoot(String),
    #[error("another vuta sync is already running")]
    Locked,
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

### Messages

- Every error names the thing it concerns and, where one exists, the command that fixes it: `no root named 'src'; add one with: vuta root add src <path>`.
- A failed repository shows the last line of git's stderr on its result line. The full text is stored in `run_results.message` and shown by `vuta show <name>`.
- Vuta never panics on bad input. `unwrap` is reserved for lock poisoning and other states that mean a bug.

## 11. Testing and CI

Tests never touch the network: every "remote" is a bare Git repository in a temporary directory, and every test uses its own database through `--db`.

### Unit tests (beside the code)

| Module | What is tested |
| --- | --- |
| `commands/completions.rs` | Every shell's script calls back into `vuta`, and `--db` is read from the line being completed |
| `vcs/remote.rs` | Name inference and URL comparison across `https://`, `ssh://` and `git@host:path` forms |
| `paths.rs` | Joining a root with `rel_path`, `/` to `\` conversion, rejecting `..` and absolute `rel_path` values |
| `db` | Migrations apply to an in-memory database, a second `open` changes nothing, a schema 1 database upgrades with every option off, and a too-new `user_version` is refused |
| `ui/table.rs` | Column widths are identical with colour on and off |
| `transfer.rs` | Defaults for missing fields, a round trip, UTF-8 with a BOM and UTF-16, and refusing other formats or newer versions |
| `db` (`atomic`) | Nested units commit together, and a failing unit rolls back only its own writes |
| `ui/json.rs` | Missing values are `null`, and enumerations use their stored strings |
| `ui/report.rs` | Each `Outcome` renders the expected line, live lines line up with result lines and are cut to the width, and with colour off the output contains no escape bytes |
| `sync` | Every job finishes once, traces and ticks are sent only when asked, and a set stop flag starts nothing |

### Integration tests (`tests/`)

`tests/common` returns a `vuta` command already pointed at a temporary database and root. It includes `src/testutil.rs` with `#[path]`, so the integration tests and the unit tests share one fixture that builds a bare "upstream" and pushes commits to it. For that reason `testutil.rs` names nothing through `crate::`.

| Scenario | Expected |
| --- | --- |
| Path missing | Cloned; exit 0 |
| Upstream has new commits | Updated, with the right commit count; exit 0 |
| Nothing new | Up to date; exit 0 |
| Tracked file modified, upstream ahead | Skipped as `dirty`; working tree untouched; exit 0, or 1 with `--strict` |
| Local commit and upstream commit | Skipped as `diverged` |
| Detached HEAD, branch with no upstream | Skipped with the matching reason |
| `origin` points elsewhere | Skipped as `remote_mismatch` |
| Path is a plain directory with files | Failed as `not_a_checkout`; exit 1 |
| Upstream deleted | Failed as `fetch`; the other repositories still sync; exit 1 |
| Hook on a changed and an unchanged repository | Runs once, only for the changed one |
| Hook exits non-zero | Repository stays `updated`; exit 1 |
| `--color never` and `--color always` | No escape bytes, and escape bytes, in the same output |
| Unknown repository name | Exit 3 with a one-line error |
| Ctrl-C during a hook (Unix only, sent with `kill -INT`) | Exit 130; no further hook runs; the run row is closed with 130 |
| `allow_dirty`, local edit to a file upstream does not change | Updated; the edit is still there, unstaged |
| `allow_dirty`, local edit to a file upstream changes | Skipped as `dirty`, naming the file; working tree untouched |
| `rebase`, local commit and upstream commit | Updated, with 1 local commit rebased on top of upstream |
| `rebase`, local commit conflicting with upstream | Skipped as `rebase_stopped`; HEAD, branch, files and untracked files as before; no rebase in progress |
| `rebase` with local changes | Skipped as `dirty`, even with `allow_dirty` |
| `submodules`, clone and later update | Cloned and updated with "submodules ok"; the submodule follows its recorded commit |
| `submodules`, submodule upstream unreachable | Outcome kept, "submodules failed", exit 1, hooks still run; the next sync heals it once the upstream is back |
| `submodules`, commit on a submodule's detached HEAD | The step is refused and the commit stays checked out |

The scenarios live in `tests/sync.rs`, `tests/hooks.rs` and `tests/options.rs`. `tests/registry.rs` covers the registry commands, `scan`, `status`, the first-root error without a terminal, and the JSON of every command that has it. `tests/cli.rs` runs completion requests the way fish sends them, against a real registry and against a database that does not exist, which must stay missing. Live progress and the first-root question need a terminal, so they are checked by hand with `script -qec 'vuta sync' log`, whose log replays the cursor movements. Git refuses submodules at local paths unless `protocol.file.allow` permits them, so `tests/common` passes `protocol.file.allow=always` to vuta through `GIT_CONFIG_COUNT` (the unit tests can only check the steps that refuse before cloning). A rebase makes commits, so fixture clones get a repository-local committer identity and `commit.gpgsign=false`, and work on a CI runner with no global git identity. Hook commands in tests use only syntax that `sh -c` and `cmd /C` read the same way, such as `echo run>> hook.log` and `exit 3`; a test that needs shell-specific syntax adds one hook per platform.

### CI (`.github/workflows/ci.yml`)

```yaml
name: CI
on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v7
      - run: rustup toolchain install stable --profile minimal --component clippy,rustfmt
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace

  package:   # builds, installs and runs the .deb and .msix; see docs/PACKAGING.md
```

- `--workspace` makes clippy and the tests cover `xtask` as well.
- The `package` job proves the packaging tools work on every push. It uploads and releases nothing; packages for distribution are built locally with `cargo xtask`.

- GitHub's runners ship with `rustup` and `git`, so the workflow needs no third-party actions.
- `actions/checkout@v7` was the latest major version on 12 Nov 2026.
- Windows is in the matrix from the first commit. Path handling, `cmd /C` hooks and console colour are where a Linux-only test suite would miss bugs.

## 12. Milestones

Five milestones, each ending in something you can run. M3 makes Vuta usable day to day; M4 makes it fast and adds history.

| Milestone | Delivers | Done when |
| --- | --- | --- |
| **M0 Skeleton** | `cargo new`, clap CLI with global flags, colour plumbing and theme, database open and migration, `vuta db path`, `vuta doctor`, CI | CI is green on Linux and Windows, and `vuta doctor` reports a healthy empty database on both |
| **M1 Registry** | `root`, `add --no-clone`, `list`, `show`, `set`, `tag`, `remove`, `config` | You can register nine repositories and `vuta list` shows them correctly |
| **M2 Sync** | Git backend, per-repository decisions, result lines, `runs` and `run_results`, exit codes, `add` clones, one repository at a time | `vuta sync` clones and fast-forwards correctly, and every skip and failure case has a passing test |
| **M3 Scan and hooks** | `scan --tag-from-parent`, `status`, `hook add/list/remove`, hook runner | An existing tree of checkouts is adopted with `scan`, and V rebuilds only when it changed |
| **M4 Parallel and history** | Worker pool, sync lock, Ctrl-C handling, `log`, history pruning, `--fetch-only`, `--strict`, `--dry-run` | Nine repositories sync 4 at a time, Ctrl-C exits 130 with a closed run row, and `vuta log` shows past runs |

**Status on 12 Nov 2026:** M0 to M4 are implemented. Sync used the M4 worker pool from the start, since the engine was built before the commands.

M2 syncs sequentially on purpose. The per-repository logic is where correctness lives, and it is easier to test before threads are involved. M4 then wraps the same `sync_one` function in the worker pool without changing it.

### First hour in the new repository

1. `cargo new vuta` and paste the `Cargo.toml` from section 3.
2. Add `migrations/0001_init.sql` from section 4.
3. Create the empty module files from section 2 and declare them in `lib.rs`.
4. Write `cli.rs` with the global flags and a `db path` subcommand, `ui/theme.rs` from section 9, and `db/mod.rs` from section 4.
5. Export this document as Markdown to `docs/ARCHITECTURE.md`, add the CI workflow from section 11, and push.

## 13. Deferred

Each item below is left out of v1 on purpose, and each has a known place to plug in. [features_bucket.md](features_bucket.md) plans the remaining ones: what each involves, what to decide first, and when it is done.

| Later feature | Where it plugs in |
| --- | --- |
| Mercurial, Subversion, Fossil, Jujutsu | A new file in `vcs/` implementing `Vcs`, a new `VcsKind` variant, and a migration that widens the `repos.vcs` CHECK. `scan` gains detection of `.hg`, `.svn` and so on. The sync engine does not change. |
| Registry export and import | Done: `vuta export` and `vuta import` (section 6). |
| JSON output | Done for every command that lists or plans something (section 9). |
| Submodules | Done: `vuta add`/`set --submodules`, a step after a clone or update (section 7). |
| Other update strategies | Done: `vuta set --strategy rebase` beside the default `ff-only` (section 7). A `merge` strategy was left out on purpose: it would add a merge commit to the local branch on every diverged sync. |
| Fast-forward around local changes | Done: `vuta set --allow-dirty` (section 7). |
| Live progress | Done: a live line per worker on a terminal (section 9). |
| Shell completions | Done: `vuta completions <shell>`, completing names from the registry through clap_complete's dynamic engine (section 6). |
| Release binaries | Packages are built locally: `cargo xtask deb` and `cargo xtask msix` (docs/PACKAGING.md). Publishing them, and `cargo install vuta` once the crate is on crates.io, are still open. |

### Open questions

- **Licence.** This document assumes `MIT OR Apache-2.0`, the usual choice for Rust tools.
