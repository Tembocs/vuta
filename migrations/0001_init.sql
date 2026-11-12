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
