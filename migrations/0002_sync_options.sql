-- Per-repository sync options (docs/ARCHITECTURE.md section 7), all off by
-- default so existing repositories keep fast-forward-only behaviour.

-- Fast-forward a checkout with modified tracked files when upstream does
-- not change those files.
ALTER TABLE repos ADD COLUMN allow_dirty INTEGER NOT NULL DEFAULT 0
    CHECK (allow_dirty IN (0, 1));

-- Run `git submodule update --init --recursive` after a clone or update.
ALTER TABLE repos ADD COLUMN submodules INTEGER NOT NULL DEFAULT 0
    CHECK (submodules IN (0, 1));

-- How a checkout with local commits follows upstream.
ALTER TABLE repos ADD COLUMN strategy TEXT NOT NULL DEFAULT 'ff-only'
    CHECK (strategy IN ('ff-only', 'rebase'));

-- Local commits replayed onto upstream by an update; NULL when none were.
ALTER TABLE run_results ADD COLUMN rebased INTEGER;

-- How the submodule step went; NULL when it did not run. A failure keeps
-- the repository's own outcome and stores git's output in `message`.
ALTER TABLE run_results ADD COLUMN submodules TEXT
    CHECK (submodules IN ('ok', 'failed'));
