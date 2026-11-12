//! Typed queries on the `runs` and `run_results` tables.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::DbError;
use crate::model::{HookOutcome, Outcome, Run, RunResult, RunSummary, Tally};

const RUN_COLUMNS: &str = "r.id, r.started_at, r.finished_at, r.vuta_version, r.args, r.exit_code";

fn run_from_row(row: &Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: row.get(0)?,
        started_at: row.get(1)?,
        finished_at: row.get(2)?,
        vuta_version: row.get(3)?,
        args: row.get(4)?,
        exit_code: row.get(5)?,
    })
}

const RESULT_COLUMNS: &str = "rr.id, rr.run_id, r.started_at, rr.repo_id, p.name, rr.outcome, \
     rr.reason, rr.old_rev, rr.new_rev, rr.commits, rr.duration_ms, rr.hook_outcome, rr.message";

const RESULT_FROM: &str = "FROM run_results rr JOIN runs r ON r.id = rr.run_id \
     JOIN repos p ON p.id = rr.repo_id";

fn result_from_row(row: &Row) -> rusqlite::Result<RunResult> {
    Ok(RunResult {
        id: row.get(0)?,
        run_id: row.get(1)?,
        run_started_at: row.get(2)?,
        repo_id: row.get(3)?,
        repo_name: row.get(4)?,
        outcome: row.get(5)?,
        reason: row.get(6)?,
        old_rev: row.get(7)?,
        new_rev: row.get(8)?,
        commits: row.get(9)?,
        duration_ms: u64::try_from(row.get::<_, i64>(10)?).unwrap_or(0),
        hook_outcome: row.get(11)?,
        message: row.get(12)?,
    })
}

/// Open a run and return its id. `finished_at` stays NULL until `finish`.
pub fn start(conn: &Connection, vuta_version: &str, args: &str) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO runs (vuta_version, args) VALUES (?1, ?2)",
        params![vuta_version, args],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Store one repository's outcome and refresh the `last_*` cache on the
/// repository, together. Returns the result id, for `set_hook_outcome`.
pub fn record_result(
    conn: &Connection,
    run_id: i64,
    repo_id: i64,
    outcome: &Outcome,
    duration: Duration,
) -> Result<i64, DbError> {
    let (reason, old_rev, commits, message): (
        Option<&str>,
        Option<&str>,
        Option<u32>,
        Option<String>,
    ) = match outcome {
        Outcome::Cloned { .. } | Outcome::UpToDate { .. } => (None, None, None, None),
        Outcome::Updated { old, commits, .. } => (None, Some(old), Some(*commits), None),
        Outcome::Skipped { reason, behind } => (
            Some(reason.kind().as_str()),
            None,
            Some(*behind),
            Some(reason.describe(*behind)),
        ),
        Outcome::Failed { reason, message } => {
            (Some(reason.as_str()), None, None, Some(message.clone()))
        }
    };
    let new_rev = outcome.rev();
    let duration_ms = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO run_results
             (run_id, repo_id, outcome, reason, old_rev, new_rev, commits, duration_ms, message)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            run_id,
            repo_id,
            outcome.kind(),
            reason,
            old_rev,
            new_rev,
            commits,
            duration_ms,
            message
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE repos SET last_rev = COALESCE(?1, last_rev), last_outcome = ?2,
                          last_synced_at = unixepoch()
         WHERE id = ?3",
        params![new_rev, outcome.kind(), repo_id],
    )?;
    tx.commit()?;
    Ok(id)
}

/// Record how a repository's hooks went.
pub fn set_hook_outcome(
    conn: &Connection,
    result_id: i64,
    outcome: HookOutcome,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE run_results SET hook_outcome = ?1 WHERE id = ?2",
        params![outcome, result_id],
    )?;
    Ok(())
}

/// Close a run with its exit code.
pub fn finish(conn: &Connection, run_id: i64, exit_code: i32) -> Result<(), DbError> {
    conn.execute(
        "UPDATE runs SET finished_at = unixepoch(), exit_code = ?1 WHERE id = ?2",
        params![exit_code, run_id],
    )?;
    Ok(())
}

/// Delete all but the newest `keep` runs; their results go by cascade.
/// Returns how many runs were deleted.
pub fn prune(conn: &Connection, keep: u32) -> Result<usize, DbError> {
    Ok(conn.execute(
        "DELETE FROM runs WHERE id NOT IN (SELECT id FROM runs ORDER BY id DESC LIMIT ?1)",
        [keep],
    )?)
}

pub fn by_id(conn: &Connection, id: i64) -> Result<Option<Run>, DbError> {
    let sql = format!("SELECT {RUN_COLUMNS} FROM runs r WHERE r.id = ?1");
    Ok(conn.query_row(&sql, [id], run_from_row).optional()?)
}

/// The newest `limit` runs with their outcome counts, newest first.
pub fn recent(conn: &Connection, limit: u32) -> Result<Vec<RunSummary>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS},
             COUNT(rr.id) FILTER (WHERE rr.outcome = 'cloned'),
             COUNT(rr.id) FILTER (WHERE rr.outcome = 'updated'),
             COUNT(rr.id) FILTER (WHERE rr.outcome = 'up_to_date'),
             COUNT(rr.id) FILTER (WHERE rr.outcome = 'skipped'),
             COUNT(rr.id) FILTER (WHERE rr.outcome = 'failed'),
             COUNT(rr.id) FILTER (WHERE rr.hook_outcome = 'failed')
         FROM runs r LEFT JOIN run_results rr ON rr.run_id = r.id
         GROUP BY r.id ORDER BY r.id DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map([limit], |row| {
        Ok(RunSummary {
            run: run_from_row(row)?,
            tally: Tally {
                cloned: row.get(6)?,
                updated: row.get(7)?,
                up_to_date: row.get(8)?,
                skipped: row.get(9)?,
                failed: row.get(10)?,
                hooks_failed: row.get(11)?,
            },
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every result of one run, ordered by repository name.
pub fn results_for_run(conn: &Connection, run_id: i64) -> Result<Vec<RunResult>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RESULT_COLUMNS} {RESULT_FROM} WHERE rr.run_id = ?1 ORDER BY p.name"
    ))?;
    let rows = stmt.query_map([run_id], result_from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A repository's newest `limit` results, newest first.
pub fn results_for_repo(
    conn: &Connection,
    repo_id: i64,
    limit: u32,
) -> Result<Vec<RunResult>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RESULT_COLUMNS} {RESULT_FROM} WHERE rr.repo_id = ?1 ORDER BY rr.run_id DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![repo_id, limit], result_from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::db::repos::tests::add_repo;
    use crate::db::tests::fresh;
    use crate::db::{repos, roots};
    use crate::model::{FailReason, OutcomeKind, SkipReason};

    #[test]
    fn a_run_records_results_and_updates_the_cache() {
        let (_dir, conn) = fresh();
        let root = roots::add(&conn, "src", Path::new("/src"), false).unwrap();
        let zed = add_repo(&conn, root.id, "zed", &[]);
        let edit = add_repo(&conn, root.id, "edit", &[]);
        let vpn = add_repo(&conn, root.id, "openvpn", &[]);

        let run = start(&conn, "0.1.0", "sync").unwrap();
        let ms = Duration::from_millis(1200);
        let updated = Outcome::Updated {
            old: "aaa".into(),
            new: "bbb".into(),
            commits: 42,
        };
        let zed_result = record_result(&conn, run, zed.id, &updated, ms).unwrap();
        let skipped = Outcome::Skipped {
            reason: SkipReason::Dirty,
            behind: 3,
        };
        record_result(&conn, run, edit.id, &skipped, ms).unwrap();
        let failed = Outcome::Failed {
            reason: FailReason::Fetch,
            message: "fatal: nope".into(),
        };
        record_result(&conn, run, vpn.id, &failed, ms).unwrap();
        set_hook_outcome(&conn, zed_result, HookOutcome::Failed).unwrap();
        finish(&conn, run, 1).unwrap();

        let stored = by_id(&conn, run).unwrap().unwrap();
        assert_eq!(stored.exit_code, Some(1));
        assert!(stored.finished_at.is_some());

        let results = results_for_run(&conn, run).unwrap();
        let summary: Vec<_> = results
            .iter()
            .map(|r| {
                (
                    r.repo_name.as_str(),
                    r.outcome,
                    r.reason.as_deref(),
                    r.commits,
                    r.message.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "edit",
                    OutcomeKind::Skipped,
                    Some("dirty"),
                    Some(3),
                    Some("local changes; 3 commits behind")
                ),
                (
                    "openvpn",
                    OutcomeKind::Failed,
                    Some("fetch"),
                    None,
                    Some("fatal: nope")
                ),
                ("zed", OutcomeKind::Updated, None, Some(42), None),
            ]
        );
        assert_eq!(results[2].hook_outcome, Some(HookOutcome::Failed));
        assert_eq!(results[2].duration_ms, 1200);

        let entries = repos::select(&conn, &repos::Selection::default()).unwrap();
        let zed_entry = entries.iter().find(|e| e.repo.name == "zed").unwrap();
        assert_eq!(zed_entry.last.rev.as_deref(), Some("bbb"));
        assert_eq!(zed_entry.last.outcome, Some(OutcomeKind::Updated));
        assert!(zed_entry.last.at.is_some());

        let tally = recent(&conn, 10).unwrap()[0].tally;
        assert_eq!(
            tally,
            Tally {
                updated: 1,
                skipped: 1,
                failed: 1,
                hooks_failed: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn failure_keeps_the_last_known_rev() {
        let (_dir, conn) = fresh();
        let root = roots::add(&conn, "src", Path::new("/src"), false).unwrap();
        let zed = add_repo(&conn, root.id, "zed", &[]);
        let run = start(&conn, "0.1.0", "sync").unwrap();
        let d = Duration::ZERO;
        record_result(
            &conn,
            run,
            zed.id,
            &Outcome::Cloned { rev: "abc".into() },
            d,
        )
        .unwrap();
        let failed = Outcome::Failed {
            reason: FailReason::Fetch,
            message: String::new(),
        };
        record_result(&conn, run, zed.id, &failed, d).unwrap();
        let entry = &repos::select(&conn, &repos::Selection::default()).unwrap()[0];
        assert_eq!(entry.last.rev.as_deref(), Some("abc"));
        assert_eq!(entry.last.outcome, Some(OutcomeKind::Failed));
        assert_eq!(
            results_for_repo(&conn, zed.id, 1).unwrap()[0].outcome,
            OutcomeKind::Failed
        );
    }

    #[test]
    fn prune_keeps_the_newest_runs() {
        let (_dir, conn) = fresh();
        let ids: Vec<i64> = (0..5)
            .map(|_| start(&conn, "0.1.0", "sync").unwrap())
            .collect();
        assert_eq!(prune(&conn, 2).unwrap(), 3);
        let left: Vec<i64> = recent(&conn, 10)
            .unwrap()
            .iter()
            .map(|s| s.run.id)
            .collect();
        assert_eq!(left, [ids[4], ids[3]]);
    }
}
