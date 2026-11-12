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
- **Never damages a checkout.** Updates are fast-forward only. Vuta never merges, rebases, stashes or resets.
- **One command syncs everything.** Repositories sync in parallel, each gets one coloured result line, and the exit code says whether anything failed.
- **All state in one file.** The registry, tags, hooks, settings and run history live in SQLite.
- **Another VCS is a new backend, not a redesign.** Everything Git-specific sits behind one trait.

### Out of scope for v1

- Mercurial, Subversion and other VCS backends.
- Config files, and import or export of the registry.
- Anything that writes to a remote: commit, push, tag.
- Submodule management, progress bars and JSON output.

Section 13 says where each of these plugs in later.

## 2. Repository layout

One Cargo package with a thin `main.rs` and a library crate, so integration tests and unit tests reach the same code.

```text
vuta/
├── Cargo.toml
├── README.md
├── docs/
│   └── ARCHITECTURE.md        this document, exported as Markdown
├── LICENSE-MIT
├── LICENSE-APACHE
├── .gitignore                 /target
├── .github/workflows/ci.yml
├── migrations/
│   └── 0001_init.sql          the schema in section 4
├── src/
│   ├── main.rs                parse args, set colour, open DB, dispatch, map errors to exit codes
│   ├── lib.rs                 module declarations
│   ├── cli.rs                 clap definitions only, no logic
│   ├── error.rs               Error enum and exit codes
│   ├── model.rs               Root, Repo, Hook, Run, Outcome, SkipReason
│   ├── paths.rs               data directory, path normalisation, root + rel_path joins
│   ├── proc.rs                spawn a child process, capture output, shared environment
│   ├── commands/              one file per subcommand: read args, call core, render
│   │   ├── mod.rs
│   │   ├── root.rs  add.rs  scan.rs  sync.rs  list.rs  status.rs
│   │   ├── show.rs  set.rs  tag.rs   remove.rs  hook.rs  log.rs
│   │   └── config.rs  db.rs  doctor.rs
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
│       └── report.rs          sync result lines and the summary
└── tests/
    ├── common/mod.rs          temp dirs, local bare repos, vuta command builder
    ├── cli.rs                 global flags, db path, doctor, exit codes
    ├── registry.rs
    ├── sync.rs
    └── hooks.rs
```

### Three rules that keep the modules honest

1. **Only `db` runs SQL.** Commands call typed functions such as `db::repos::by_name`.
2. **Only `vcs::git` (through `proc`) spawns `git`.** The sync engine sees the `Vcs` trait and nothing else.
3. **Only `ui` writes to the terminal.** No `println!` outside it, which is what makes colour and quiet mode consistent.

&#91;embedded content: module map · 6 modules, 3 outside resources\]

Arrows point from caller to callee. `sync` reaches Git only through `vcs`, and it reports results back to its caller instead of writing to the database or the terminal itself. `model.rs`, `paths.rs` and `error.rs` are plain types and helpers used by every module.

## 3. Dependencies

Eight runtime crates, all checked against crates.io on 12 Nov 2026. The minimum Rust version is 1.89 because `rusqlite` 0.42 requires it and the sync lock uses `File::try_lock`.

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
rusqlite = { version = "0.42", features = ["bundled"] }
directories = "6"
anstream = "1"
anstyle = "1"
thiserror = "2"
anyhow = "1"
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
| `rusqlite` | SQLite access. `bundled` compiles SQLite in, so Windows needs no system library and both platforms get the same SQLite version. |
| `directories` | Finds the per-user data directory on each OS. |
| `anstream`, `anstyle` | Colour output. clap already depends on both, so they add nothing to the build. Section 9 covers their use. |
| `thiserror` | The typed `Error` enum that maps to exit codes. |
| `anyhow` | Error context inside commands, where the exact type does not matter. |
| `ctrlc` | Ctrl-C handling on both platforms. `termination` adds SIGTERM. |
| `assert_cmd`, `tempfile` | Integration tests that run the real binary against throwaway directories. |

### Deliberately not used

- **`git2` / libgit2.** Vuta shells out to `git`, which reuses your SSH config and credential helpers and matches how every later backend will work.
- **An async runtime.** The work is a handful of child processes. Scoped threads from the standard library are enough.
- **A migration crate.** The runner is about 15 lines (section 4) and avoids pinning to another crate's `rusqlite` version range.
- **A date-time crate.** Timestamps are Unix seconds from SQLite's `unixepoch()`. SQLite formats them for display.
- **A table crate.** `ui/table.rs` pads columns itself, which keeps padding correct when cells carry colour codes.

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

### What each table is for

| Table | Holds | Notes |
| --- | --- | --- |
| `roots` | Named base directories | At most one row is the default. |
| `repos` | One row per managed checkout | Location is `roots.path` joined with `rel_path`. `last_*` columns cache the latest result so `vuta list` needs no join. |
| `repo_tags` | Free-form labels | Group repositories, for example by language. Drives `vuta sync --tag rust`. |
| `hooks` | Commands to run after a sync | Ordered by `position` within a repository. |
| `runs` | One row per `vuta sync` | `finished_at` stays NULL if the process was killed. |
| `run_results` | One row per repository per run | Feeds `vuta log` and `vuta show`. |
| `settings` | Key-value preferences | `sync.jobs` (default 4), `history.keep_runs` (default 200), `ui.glyphs` (`auto`, `unicode`, `ascii`). |

### Conventions

- **Paths.** `roots.path` is absolute, made with `std::path::absolute`. `fs::canonicalize` is avoided because on Windows it returns `\\?\` paths that many tools reject. `rel_path` always uses `/` and is converted on use.
- **Moving a partition.** `vuta root move <name> <new-path>` updates one row and every repository under it follows.
- **Names.** Repository, root and tag names compare case-insensitively, so `vuta sync Zed` and `vuta sync zed` are the same.
- **Time.** All timestamps are Unix seconds in UTC.
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
}

pub enum VcsKind { Git }

pub enum Outcome {
    Cloned     { rev: String },
    Updated    { old: String, new: String, commits: u32 },
    UpToDate   { rev: String, ahead: u32 },
    Skipped    { reason: SkipReason, behind: u32 },
    Failed     { reason: FailReason, message: String },
}

pub enum SkipReason {
    Dirty,
    Diverged       { ahead: u32 },
    Detached       { rev: String },
    NoUpstream     { branch: String },
    RemoteMismatch { found: String, expected: String },
    Disabled,
    FetchOnly,     // --fetch-only fetched but left the tree alone
}
pub enum FailReason { NotACheckout, Clone, Fetch, FastForward, Interrupted }
```

`Outcome` is the one value a worker returns per repository. The database row, the coloured result line and the exit code are all derived from it. Skip reasons carry what their result line needs (section 7); `SkipReason::describe` gives the plain text stored in `run_results.message`.

`model.rs` also holds the row types the other layers exchange: `Hook`, `Run`, `RunResult`, `RepoEntry` (a repository with its root name, tags and cached last result), and `Tally` (outcome counts for the summary line and `vuta log`). Every enum stored as text (`OutcomeKind`, `SkipKind`, `FailReason`, `HookOutcome`, `Platform`, `RunWhen`, `GlyphMode`) parses case-insensitively from the same strings the schema's CHECK constraints allow.

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

pub trait Vcs: Send + Sync {
    fn is_checkout(&self, path: &Path) -> bool;
    fn remote_url(&self, path: &Path) -> anyhow::Result<Option<String>>;
    fn head(&self, path: &Path) -> anyhow::Result<Head>;
    fn is_dirty(&self, path: &Path) -> anyhow::Result<bool>;
    fn clone_into(&self, spec: &CloneSpec, dest: &Path) -> anyhow::Result<()>;
    fn fetch(&self, path: &Path) -> anyhow::Result<()>;
    /// `None` when the current branch has no upstream.
    fn divergence(&self, path: &Path) -> anyhow::Result<Option<Divergence>>;
    fn fast_forward(&self, path: &Path) -> anyhow::Result<()>;
}
```

`vcs::backend(kind, tracer)` returns the implementation for a `VcsKind`. The optional tracer sees every command and its output, which is how `--verbose` reaches `ui` without the backend printing anything.

The methods are small questions and single actions. All policy (what to do when dirty, diverged or detached) lives in `sync/worker.rs`, so a second backend inherits the same safety rules.

### Git backend (`vcs/git.rs`)

| Method | Command |
| --- | --- |
| `is_checkout` | `git -C <p> rev-parse --show-toplevel`, and the result must equal `<p>` |
| `remote_url` | `git -C <p> remote get-url origin` |
| `head` | `git -C <p> symbolic-ref --quiet --short HEAD` (exit 1 means detached), then `git -C <p> rev-parse HEAD` |
| `is_dirty` | `git -C <p> status --porcelain --untracked-files=no` prints anything |
| `clone_into` | `git clone --origin origin [--branch B] [--filter F] [--depth N] <url> <dest>` |
| `fetch` | `git -C <p> fetch --prune origin` |
| `divergence` | `git -C <p> rev-list --left-right --count HEAD...@{upstream}` prints `ahead<TAB>behind` |
| `fast_forward` | `git -C <p> merge --ff-only @{upstream}` |

This sequence was run against a local repository with Git 2.43 on 12 Nov 2026. Each command behaved as the table says, including the diverged, detached and no-upstream cases.

- **`is_checkout` compares the top level** because a plain folder inside another repository also answers "yes" to simpler checks.
- **Untracked files do not count as dirty.** Build outputs such as V's compiled binary would otherwise block every update.
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

The command is `vuta <subcommand>`. Fifteen subcommands cover the registry, syncing and inspection; section 12 says in which milestone each one lands.

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
| `vuta set <name> [options]` | Change name, URL, branch, path or enabled state |
| `vuta tag <name> <+tag\|-tag>...` | Add or remove tags, for example `vuta tag zed +rust -editor` |
| `vuta remove <name>` (alias `rm`) | Forget a repository. `--delete-files` also deletes the checkout after a confirmation. |
| `vuta hook add\|list\|remove` | Manage post-sync commands (section 8) |
| `vuta log [name]` | Recent sync runs and their results |
| `vuta config get\|set\|list` | Read and write rows in `settings` |
| `vuta db path` | Print the database location |
| `vuta doctor` | Check that `git` is on PATH, the database is sound, and every root exists |

### `vuta add`

```text
vuta add <url> [path] [--name N] [--root R] [--tag T]... [--branch B]
                      [--filter F] [--depth N] [--no-clone]
```

- `path` is relative to the root and defaults to the inferred name. `vuta add https://github.com/zed-industries/zed.git rust/zed --tag rust` places Zed under `rust/zed` in the default root.
- `--filter blob:none` makes a partial clone, which suits large repositories you only build, such as Zed or Modular.
- `--no-clone` only records the repository. The next `vuta sync` clones it.

### `vuta scan`

```text
vuta scan [dir] [--root R] [--tag-from-parent] [--max-depth N] [--dry-run]
```

- Walks `dir` (default: the default root, 4 levels deep) and registers every Git checkout that has an `origin` remote.
- Does not descend into a checkout once it finds one, so submodules and vendored repositories are not registered separately.
- `--tag-from-parent` tags each repository with its parent directory name: `rust/zed` gets the tag `rust`.
- A checkout with no `origin`, or outside every root, is listed as skipped with the reason.

### `vuta sync`

```text
vuta sync [name]... [--tag T]... [--root R] [-j N] [--fetch-only]
          [--no-hooks] [--force-hooks] [--strict] [--dry-run]
```

- With no names or filters it syncs every enabled repository.
- `-j N` overrides `sync.jobs` for this run.
- `--fetch-only` fetches and reports how far behind each checkout is, without changing any working tree.
- `--strict` makes a skipped repository count as a failure for the exit code.
- `--dry-run` prints what would be cloned or fetched and exits.

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

A sync run fetches every selected repository in parallel and changes a working tree only when a clean fast-forward is possible.

### Phases of a run

1. **Select.** Load enabled repositories that match the names, tags and root given.
2. **Lock.** Take an exclusive lock on `vuta.lock` beside the database with `File::try_lock`. If another sync holds it, exit with code 3 and say so.
3. **Record.** Insert a `runs` row.
4. **Sync.** Start the worker pool. Each worker takes a repository, decides what to do (below) and reports one `Outcome`.
5. **Hooks.** Run hooks one repository at a time, in name order, for repositories whose revision changed.
6. **Finish.** Update `runs`, prune old history, print the summary, and exit with the code from section 10.

### Decisions for one repository

&#91;embedded content: per-repository sync · 7 checks, 2 actions, 9 outcomes\]

Checks run top to bottom and the first one that fails decides the outcome. Green outcomes exit 0, amber ones are skips, red ones are failures. A clone or fast-forward that git rejects becomes `failed` with reason `clone` or `fast_forward`.

| Result | Reason | What the line says |
| --- | --- | --- |
| skipped | `remote_mismatch` | origin is `<url>`, expected `<url>` |
| skipped | `detached` | HEAD is detached at `<rev>` |
| skipped | `no_upstream` | branch `<name>` tracks no remote branch |
| skipped | `diverged` | 2 local commits, 5 behind |
| skipped | `dirty` | local changes; 3 commits behind |
| skipped | `fetch_only` | 3 commits behind (only with `--fetch-only`) |
| failed | `not_a_checkout` | path exists but is not a Git checkout |
| failed | `clone`, `fetch`, `fast_forward` | the last line of git's error output |

A checkout that is ahead of its upstream and not behind counts as up to date, with "2 local commits" noted on the line.

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
- **Output order.** Result lines print as repositories finish, each with a `[3/9]` counter. A dim "cloning zed" line prints when a clone starts, because clones can run for minutes.
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
| When it runs | After a clone, or after an update that moved the revision. `--always` runs it on every sync. |
| Platform | A hook marked `unix` or `windows` is ignored on the other. The V build is two hooks: `make` and `make.bat`. |
| Shell | `sh -c <command>` on Unix, `cmd /C <command>` on Windows |
| Working directory | The repository's checkout |
| Environment | `VUTA_REPO_NAME`, `VUTA_REPO_PATH`, `VUTA_OLD_REV` (empty after a clone), `VUTA_NEW_REV` |
| Output | Streamed straight to the terminal under a header line, not captured |
| Order | By `position` within a repository. The first failure stops that repository's remaining hooks. |
| Failure | A non-zero exit sets `hook_outcome = failed` and makes the run exit 1. The repository's own outcome stays `updated`. |
| Overrides | `vuta sync --no-hooks` skips all hooks. `--force-hooks` runs them even when nothing changed. |

Hooks are arbitrary commands, so they only ever enter the database through `vuta hook add`. `vuta scan` and `vuta add` never read hooks from a repository's contents.

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

### Rules for `ui`

- **Results go to stdout; progress, warnings and errors go to stderr.** `vuta list | grep rust` sees only the table.
- **Pad first, then style.** `ui/table.rs` measures plain text and wraps the padded cell in a style, so escape codes never shift a column.
- **`--quiet`** prints failures, skips and the summary. **`--verbose`** adds each `git` command and its output in dim text.

## 10. Errors and exit codes

The exit code tells a script or scheduled task what happened without parsing output.

| Code | Meaning |
| --- | --- |
| 0 | Everything requested succeeded. Skipped repositories are allowed unless `--strict` is set. |
| 1 | At least one repository or hook failed |
| 2 | Usage error: unknown flag, missing argument. This is clap's own code. |
| 3 | Vuta could not start the work: `git` not found, database unreadable or newer than the binary, unknown repository or root name, another sync holds the lock |
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
| `vcs/remote.rs` | Name inference and URL comparison across `https://`, `ssh://` and `git@host:path` forms |
| `paths.rs` | Joining a root with `rel_path`, `/` to `\` conversion, rejecting `..` and absolute `rel_path` values |
| `db` | Migrations apply to an in-memory database, a second `open` changes nothing, and a too-new `user_version` is refused |
| `ui/table.rs` | Column widths are identical with colour on and off |
| `ui/report.rs` | Each `Outcome` renders the expected line, and with colour off the output contains no escape bytes |

### Integration tests (`tests/`)

`tests/common` builds a bare "upstream", pushes commits to it, and returns a `vuta` command already pointed at a temporary database and root.

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
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test
```

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

M2 syncs sequentially on purpose. The per-repository logic is where correctness lives, and it is easier to test before threads are involved. M4 then wraps the same `sync_one` function in the worker pool without changing it.

### First hour in the new repository

1. `cargo new vuta` and paste the `Cargo.toml` from section 3.
2. Add `migrations/0001_init.sql` from section 4.
3. Create the empty module files from section 2 and declare them in `lib.rs`.
4. Write `cli.rs` with the global flags and a `db path` subcommand, `ui/theme.rs` from section 9, and `db/mod.rs` from section 4.
5. Export this document as Markdown to `docs/ARCHITECTURE.md`, add the CI workflow from section 11, and push.

## 13. Deferred

Each item below is left out of v1 on purpose, and each has a known place to plug in.

| Later feature | Where it plugs in |
| --- | --- |
| Mercurial, Subversion, Fossil, Jujutsu | A new file in `vcs/` implementing `Vcs`, a new `VcsKind` variant, and a migration that widens the `repos.vcs` CHECK. `scan` gains detection of `.hg`, `.svn` and so on. The sync engine does not change. |
| Registry export and import | `vuta export` and `vuta import` as a transfer format between machines. The database stays the source of truth. |
| JSON output | A `--format json` flag handled inside `ui`, since `ui` is already the only writer. |
| Submodules | A per-repository `submodules` column and one extra step after a fast-forward. |
| Other update strategies | A per-repository `strategy` column (`ff-only`, `rebase`) read by `sync/worker.rs`. |
| Live progress | Spinner lines per worker, drawn by `ui` from the `Started` and `Finished` events it already receives. |
| Shell completions | `clap_complete`, generated from the existing `cli.rs`. |
| Release binaries | A release workflow that attaches Linux and Windows builds to a GitHub release, then `cargo install vuta` once the crate is published. |

### Open questions

- **Licence.** This document assumes `MIT OR Apache-2.0`, the usual choice for Rust tools.
- **Dirty checkouts.** v1 skips any checkout with modified tracked files. Git can often fast-forward safely around local edits; allowing that would be a per-repository opt-in.
- **Default root on first use.** v1 requires `vuta root add` before `vuta add`. An alternative is to offer the current directory as the first root.
