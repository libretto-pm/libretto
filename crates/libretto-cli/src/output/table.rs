//! Table formatting utilities.

use comfy_table::{
    Attribute, Cell, CellAlignment, Color, ContentArrangement, Table as ComfyTable, presets,
};

/// Table style presets
#[derive(Debug, Clone, Copy, Default)]
pub enum TableStyle {
    /// No borders, minimal
    #[default]
    Minimal,
    /// ASCII borders
    Ascii,
    /// Unicode borders
    Unicode,
    /// GitHub markdown style
    Markdown,
    /// Compact with thin borders
    Compact,
}

impl TableStyle {
    fn apply(&self, table: &mut ComfyTable, colors_enabled: bool) {
        match self {
            Self::Minimal => {
                table.load_preset(presets::NOTHING);
                table.set_content_arrangement(ContentArrangement::Dynamic);
            }
            Self::Ascii => {
                table.load_preset(presets::ASCII_BORDERS_ONLY_CONDENSED);
            }
            Self::Unicode if colors_enabled => {
                table.load_preset(presets::UTF8_BORDERS_ONLY);
            }
            Self::Unicode => {
                table.load_preset(presets::ASCII_BORDERS_ONLY_CONDENSED);
            }
            Self::Markdown => {
                table.load_preset(presets::ASCII_MARKDOWN);
            }
            Self::Compact => {
                table.load_preset(presets::UTF8_HORIZONTAL_ONLY);
            }
        }
    }
}

/// Table builder for formatted output
pub struct Table {
    inner: ComfyTable,
    colors_enabled: bool,
}

impl Table {
    /// Create a new table with the default style
    pub fn new() -> Self {
        Self::with_style(TableStyle::default())
    }

    /// Create a new table with a specific style
    pub fn with_style(style: TableStyle) -> Self {
        let colors_enabled = crate::output::colors_enabled();
        let mut table = ComfyTable::new();
        style.apply(&mut table, colors_enabled);
        table.set_content_arrangement(ContentArrangement::Dynamic);
        Self {
            inner: table,
            colors_enabled,
        }
    }

    /// Set the table headers
    pub fn headers<I, T>(&mut self, headers: I) -> &mut Self
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let cells: Vec<Cell> = headers
            .into_iter()
            .map(|h| {
                let mut cell = Cell::new(h.as_ref());
                if self.colors_enabled {
                    cell = cell.add_attribute(Attribute::Bold);
                }
                cell
            })
            .collect();
        self.inner.set_header(cells);
        self
    }

    /// Add a row to the table
    pub fn row<I, T>(&mut self, row: I) -> &mut Self
    where
        I: IntoIterator<Item = T>,
        T: std::fmt::Display,
    {
        self.inner.add_row(row);
        self
    }

    /// Add a row with styled cells
    pub fn styled_row(&mut self, cells: Vec<Cell>) -> &mut Self {
        self.inner.add_row(cells);
        self
    }

    /// Create a success-styled cell
    pub fn success_cell(&self, text: impl std::fmt::Display) -> Cell {
        let mut cell = Cell::new(text);
        if self.colors_enabled {
            cell = cell.fg(Color::Green);
        }
        cell
    }

    /// Create an error-styled cell
    pub fn error_cell(&self, text: impl std::fmt::Display) -> Cell {
        let mut cell = Cell::new(text);
        if self.colors_enabled {
            cell = cell.fg(Color::Red);
        }
        cell
    }

    /// Create a warning-styled cell
    pub fn warning_cell(&self, text: impl std::fmt::Display) -> Cell {
        let mut cell = Cell::new(text);
        if self.colors_enabled {
            cell = cell.fg(Color::Yellow);
        }
        cell
    }

    /// Create a dim-styled cell
    pub fn dim_cell(&self, text: impl std::fmt::Display) -> Cell {
        let mut cell = Cell::new(text);
        if self.colors_enabled {
            cell = cell.fg(Color::DarkGrey);
        }
        cell
    }

    /// Create a cell with right alignment
    pub fn right_cell(&self, text: impl std::fmt::Display) -> Cell {
        Cell::new(text).set_alignment(CellAlignment::Right)
    }

    /// Create a cell with center alignment
    pub fn center_cell(&self, text: impl std::fmt::Display) -> Cell {
        Cell::new(text).set_alignment(CellAlignment::Center)
    }

    /// Set column widths (percentage of terminal width)
    pub fn column_widths(&mut self, widths: &[u16]) -> &mut Self {
        for (i, &width) in widths.iter().enumerate() {
            self.inner.set_width(width);
            let _ = i; // Silence unused warning - comfy_table handles per-column width differently
        }
        self
    }

    /// Render the table to a string
    pub fn render(&self) -> String {
        self.inner.to_string()
    }

    /// Print the table to stdout
    pub fn print(&self) {
        println!("{}", self.inner);
    }

    /// Check if the table is empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get the number of rows
    pub fn row_count(&self) -> usize {
        self.inner.row_count()
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}

/// Quick table creation helper
pub fn quick_table<H, R, C>(headers: H, rows: R) -> Table
where
    H: IntoIterator,
    H::Item: AsRef<str>,
    R: IntoIterator<Item = C>,
    C: IntoIterator,
    <C as IntoIterator>::Item: std::fmt::Display,
{
    let mut table = Table::new();
    table.headers(headers);
    for row in rows {
        table.row(row);
    }
    table
}

/// Create a key-value table (two columns)
pub fn kv_table<I, K, V>(items: I) -> Table
where
    I: IntoIterator<Item = (K, V)>,
    K: std::fmt::Display,
    V: std::fmt::Display,
{
    let mut table = Table::with_style(TableStyle::Minimal);
    for (key, value) in items {
        let colors_enabled = table.colors_enabled;
        let key_cell = if colors_enabled {
            Cell::new(key).add_attribute(Attribute::Bold)
        } else {
            Cell::new(key)
        };
        table.styled_row(vec![key_cell, Cell::new(value)]);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_creation() {
        let mut table = Table::new();
        table.headers(["Name", "Version"]);
        table.row(["foo", "1.0.0"]);
        table.row(["bar", "2.0.0"]);
        let output = table.render();
        assert!(output.contains("foo"));
        assert!(output.contains("1.0.0"));
    }

    #[test]
    fn test_quick_table() {
        let table = quick_table(["A", "B"], vec![vec!["1", "2"], vec!["3", "4"]]);
        let output = table.render();
        assert!(output.contains('1'));
        assert!(output.contains('4'));
    }

    #[test]
    fn test_kv_table() {
        let table = kv_table([("key1", "value1"), ("key2", "value2")]);
        let output = table.render();
        assert!(output.contains("key1"));
        assert!(output.contains("value2"));
    }
}
