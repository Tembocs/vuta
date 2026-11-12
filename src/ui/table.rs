//! Column alignment. Cells are padded as plain text and the padded text is
//! then styled, so escape codes never shift a column.

use anstyle::Style;

use super::Styled;
use super::theme::THEME;

/// Space between columns.
const GAP: &str = "   ";

#[derive(Debug, Clone)]
pub struct Cell {
    text: String,
    style: Style,
    /// Visible width of `text`.
    width: usize,
}

impl Cell {
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        let text = text.into();
        Cell {
            width: super::text_width(&text),
            text,
            style,
        }
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Cell::new(text, Style::new())
    }

    /// A cell already styled in several parts, such as a result detail.
    pub fn styled(text: Styled) -> Self {
        Cell {
            width: text.width(),
            text: text.into_string(),
            style: Style::new(),
        }
    }
}

/// A table with a bold header row. The last column is not padded, so lines
/// carry no trailing spaces.
#[derive(Debug, Clone)]
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<Cell>>,
}

impl Table {
    pub fn new<S: Into<String>>(headers: impl IntoIterator<Item = S>) -> Self {
        Table {
            headers: headers.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
        }
    }

    /// Add a row. Missing trailing cells render empty.
    pub fn push(&mut self, row: Vec<Cell>) {
        self.rows.push(row);
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The table as styled lines, header first.
    pub fn render(&self) -> Vec<String> {
        let columns = self
            .rows
            .iter()
            .map(Vec::len)
            .chain([self.headers.len()])
            .max()
            .unwrap_or(0);
        let mut widths = vec![0; columns];
        for (i, header) in self.headers.iter().enumerate() {
            widths[i] = super::text_width(header);
        }
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.width);
            }
        }

        let header: Vec<Cell> = self
            .headers
            .iter()
            .map(|h| Cell::new(h.clone(), THEME.name))
            .collect();
        std::iter::once(&header)
            .chain(&self.rows)
            .map(|row| render_row(row, &widths))
            .collect()
    }
}

fn render_row(row: &[Cell], widths: &[usize]) -> String {
    // Drop trailing empty cells so the line has no trailing gap.
    let used = row
        .iter()
        .rposition(|c| !c.text.is_empty())
        .map_or(0, |i| i + 1);
    let mut line = Styled::new();
    for (i, cell) in row[..used].iter().enumerate() {
        if i > 0 {
            line.plain(GAP);
        }
        line.push(&cell.text, cell.style);
        if i + 1 < used {
            line.plain(&" ".repeat(widths[i].saturating_sub(cell.width)));
        }
    }
    line.into_string()
}

#[cfg(test)]
mod tests {
    use anstyle::AnsiColor;

    use super::*;

    fn sample(styled: bool) -> Table {
        let style = |s: Style| if styled { s } else { Style::new() };
        let mut table = Table::new(["NAME", "TAGS", "PATH", "REV"]);
        table.push(vec![
            Cell::new("edit", style(THEME.name)),
            Cell::plain("rust"),
            Cell::new("rust/edit", style(THEME.muted)),
            Cell::new("7c1d2ab", style(AnsiColor::Magenta.on_default())),
        ]);
        table.push(vec![
            Cell::new("surrealdb", style(THEME.name)),
            Cell::plain(""),
            Cell::new("rust/surrealdb", style(THEME.muted)),
        ]);
        let mut rev = Styled::new();
        rev.push("8be", style(THEME.rev))
            .push("04d7", style(THEME.name));
        table.push(vec![
            Cell::styled(rev),
            Cell::plain("v"),
            Cell::plain("v/v"),
            Cell::plain("-"),
        ]);
        table
    }

    fn strip(s: &str) -> String {
        anstream::adapter::strip_str(s).to_string()
    }

    #[test]
    fn aligns_columns() {
        let lines: Vec<String> = sample(false).render().iter().map(|l| strip(l)).collect();
        assert_eq!(
            lines,
            [
                "NAME        TAGS   PATH             REV",
                "edit        rust   rust/edit        7c1d2ab",
                "surrealdb          rust/surrealdb",
                "8be04d7     v      v/v              -",
            ]
        );
    }

    #[test]
    fn colour_does_not_change_widths() {
        let coloured = sample(true).render();
        let plain = sample(false).render();
        assert!(coloured.iter().any(|l| l.contains('\x1b')));
        let stripped: Vec<String> = coloured.iter().map(|l| strip(l)).collect();
        let plain: Vec<String> = plain.iter().map(|l| strip(l)).collect();
        assert_eq!(stripped, plain);
    }
}
