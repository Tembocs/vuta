//! `vuta tag <name> <+tag|-tag>...`: add or remove tags.

use std::process::ExitCode;

use super::{Ctx, check_label, repo_by_name};
use crate::Error;
use crate::cli::TagArgs;
use crate::db;
use crate::ui::Styled;
use crate::ui::theme::THEME;

/// Split `+a`, `-b` and `c` into tags to add and tags to remove.
fn parse(changes: &[String]) -> Result<(Vec<String>, Vec<String>), Error> {
    let mut add = Vec::new();
    let mut remove = Vec::new();
    for change in changes {
        let (list, tag) = match change.as_bytes().first() {
            Some(b'+') => (&mut add, &change[1..]),
            Some(b'-') => (&mut remove, &change[1..]),
            _ => (&mut add, change.as_str()),
        };
        check_label("tag", tag)?;
        list.push(tag.to_owned());
    }
    Ok((add, remove))
}

pub fn run(ctx: &mut Ctx, args: &TagArgs) -> Result<ExitCode, Error> {
    let conn = ctx.open_db()?;
    let ui = &*ctx.ui;

    let repo = repo_by_name(&conn, &args.name)?;
    let (add, remove) = parse(&args.changes)?;
    // Removals first, so `-x +x` leaves the tag on.
    db::repos::remove_tags(&conn, repo.id, &remove)?;
    db::repos::add_tags(&conn, repo.id, &add)?;

    let tags = db::repos::tags(&conn, repo.id)?;
    let mut line = Styled::new();
    line.push(&repo.name, THEME.name).plain("  ");
    if tags.is_empty() {
        line.push("no tags", THEME.muted);
    } else {
        line.plain(&tags.join(", "));
    }
    ui.result(line);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_signs() {
        let changes: Vec<String> = ["+rust", "-editor", "tools"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (add, remove) = parse(&changes).unwrap();
        assert_eq!(add, ["rust", "tools"]);
        assert_eq!(remove, ["editor"]);
        assert!(parse(&["+".to_owned()]).is_err());
        assert!(parse(&["++x".to_owned()]).is_err());
    }
}
