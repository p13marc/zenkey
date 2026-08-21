//! The table primitive (#199): one place that decides padding, truncation and
//! what "unknown" looks like.
//!
//! Before this, `output.rs` hand-built columns at 19 sites with widths written
//! as literals per function — which is why `node list`, `storage list` and
//! `blob list` disagreed about how wide an origin is, and why one long cell
//! silently shifted every column after it. `cmd/watch.rs`, `cmd/node.rs` and
//! `cmd/cache.rs` re-implemented five of those tables again, three of them
//! byte for byte.
//!
//! ## Two honesty rules, and they are the reason this file exists
//!
//! **`—` is not the empty string.** `Cell::Unknown` renders `—` and means the
//! question was never put to the bus; an empty [`Cell::Text`] means it was
//! asked and the answer was nothing (RFC 09 §5.1 O4). They are different
//! variants rather than different strings, so the distinction survives a
//! refactor that a convention would not. There is deliberately no
//! `From<&str>` and no `From<Option<T>>`: [`Cell::asked`] is the only bridge
//! from an `Option`, and it means *not asked*. "Asked, nothing there" has to be
//! spelled at the call site, which forces the author to have decided.
//!
//! **A truncated number is a wrong number.** Text truncates with `…`; an
//! `Int` or a `Num` widens its column instead. A shortened string is still
//! recognisably a name, and `1234` cut to `12…` is a lie with a mark on it.
//!
//! ## What a "table" is here
//!
//! Not a grid. `topic_list` interleaves per-producer headings with rows and
//! variable-length row suffixes; `blob list` follows each row with six
//! indented lines; `service info` is four lines per record and no columns at
//! all. [`Grid::group`] and [`Grid::detail`] are what let those families use
//! this primitive instead of keeping their own `println!` prose — without
//! them #199 would cover about a third of what it claims.

use std::fmt::Write as _;

/// How wide the rendering may be.
///
/// `Unbounded` when stdout is not a terminal — deliberately, and never a
/// guessed 80: a piped table must not be truncated by a number nobody chose,
/// and unbounded is what makes `--format table | cat` byte-stable enough to
/// snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Unbounded,
    Cells(usize),
}

/// The narrowest a column may be squeezed to before the budget gives up.
const MIN_COL: usize = 6;

/// Two spaces, everywhere. The 19 hand-built sites varied between one and
/// several, which is most of why they looked different.
const GAP: &str = "  ";

/// One cell.
///
/// See the module doc: the `Unknown`/`Empty` split is the RFC 09 §5.1 O4
/// distinction expressed as a type rather than as a spelling convention.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// A value the tool has. May be empty — *asked, and the answer was
    /// nothing*.
    Text(String),
    /// **Not asked.** Renders `—`.
    Unknown,
    /// A count. Right-aligned by default and never truncated.
    Int(u64),
    /// A measurement and its decimals. Never truncated.
    Num(f64, u8),
}

impl Cell {
    pub fn text(s: impl Into<String>) -> Cell {
        Cell::Text(s.into())
    }

    /// The **only** bridge from an `Option`, and `None` means *not asked*.
    ///
    /// If the option means "asked, and there was nothing", do not use this —
    /// write `Cell::text(v.unwrap_or_default())` and mean it.
    pub fn asked<T: Into<String>>(v: Option<T>) -> Cell {
        match v {
            Some(v) => Cell::Text(v.into()),
            None => Cell::Unknown,
        }
    }

    pub fn int(n: impl Into<u64>) -> Cell {
        Cell::Int(n.into())
    }

    pub fn num(v: f64, decimals: u8) -> Cell {
        Cell::Num(v, decimals)
    }

    /// The rendering before any width decision.
    fn plain(&self) -> String {
        match self {
            Cell::Text(s) => s.clone(),
            Cell::Unknown => "—".to_string(),
            Cell::Int(n) => n.to_string(),
            Cell::Num(v, d) => format!("{v:.*}", *d as usize),
        }
    }

    /// Character count, not byte length.
    ///
    /// Cells are ASCII plus a fixed set of width-1 symbols (`— … ✓ ✗ ⚠ · →
    /// ←`), so `chars().count()` is the display width and a `unicode-width`
    /// dependency would buy nothing — while touching `Cargo.lock`, which
    /// every CI step passes `--locked` to.
    fn width(&self) -> usize {
        self.plain().chars().count()
    }

    /// Numbers keep every digit; only text is cut.
    fn truncatable(&self) -> bool {
        matches!(self, Cell::Text(_))
    }
}

/// A line of the rendering that is not a row.
#[derive(Debug, Clone)]
enum Item {
    Row(Vec<Cell>),
    /// A section break: a blank line, then a heading.
    Group(String),
    /// Indented continuation lines bound to the row above — deliberately not
    /// columns, because they are not tabular.
    Detail(Vec<String>),
}

/// One aligned block: a header row (optionally), rows, and the groups and
/// detail lines that belong between them.
///
/// A [`Table`] holds a sequence of these, because a real rendering is rarely
/// one grid — `storage list` prints storages and then coverage, and their
/// columns have nothing to do with each other. Each grid computes its own
/// widths, which is the honest answer: forcing two unrelated blocks onto one
/// set of column widths is how a table starts lying about what lines up.
#[derive(Debug, Clone)]
pub struct Grid {
    headers: Vec<&'static str>,
    items: Vec<Item>,
    right: Vec<bool>,
    max: Vec<Option<usize>>,
    rigid: Vec<bool>,
}

impl Grid {
    pub fn new<const N: usize>(headers: [&'static str; N]) -> Grid {
        Grid {
            headers: headers.to_vec(),
            items: Vec::new(),
            right: vec![false; N],
            max: vec![None; N],
            rigid: vec![false; N],
        }
    }

    /// A grid with no header line — the common shape in this tool, where the
    /// columns are self-evident and the prose around them does the naming.
    pub fn unheaded(columns: usize) -> Grid {
        Grid {
            headers: Vec::new(),
            items: Vec::new(),
            right: vec![false; columns],
            max: vec![None; columns],
            rigid: vec![false; columns],
        }
    }

    /// Right-align a column. `Int` and `Num` cells are right-aligned anyway.
    pub fn right(mut self, col: usize) -> Grid {
        self.right[col] = true;
        self
    }

    /// Cap a column's width; longer text is truncated with `…`.
    pub fn max(mut self, col: usize, cells: usize) -> Grid {
        self.max[col] = Some(cells);
        self
    }

    /// Exempt a column from the terminal-budget squeeze — for a column whose
    /// value is meaningless when shortened, like a key.
    pub fn rigid(mut self, col: usize) -> Grid {
        self.rigid[col] = true;
        self
    }

    pub fn row<const N: usize>(&mut self, cells: [Cell; N]) -> &mut Grid {
        debug_assert_eq!(
            N,
            self.right.len(),
            "a row must have as many cells as the table has columns"
        );
        self.items.push(Item::Row(cells.to_vec()));
        self
    }

    /// A section break plus a heading — the per-producer grouping `topic
    /// list`, `service list` and `node list` all do by hand today.
    pub fn group(&mut self, heading: impl Into<String>) -> &mut Grid {
        self.items.push(Item::Group(heading.into()));
        self
    }

    /// Indented lines bound to the row above.
    pub fn detail(&mut self, lines: impl IntoIterator<Item = String>) -> &mut Grid {
        self.items.push(Item::Detail(lines.into_iter().collect()));
        self
    }

    pub fn is_empty(&self) -> bool {
        !self.items.iter().any(|i| matches!(i, Item::Row(_)))
    }

    /// Column widths: content, capped, then squeezed to fit the budget.
    fn widths(&self, budget: Width) -> Vec<usize> {
        let cols = self.right.len();
        let mut w = vec![0usize; cols];
        for (i, h) in self.headers.iter().enumerate() {
            w[i] = h.chars().count();
        }
        for item in &self.items {
            if let Item::Row(cells) = item {
                for (i, c) in cells.iter().enumerate() {
                    w[i] = w[i].max(c.width());
                }
            }
        }
        for (width, cap) in w.iter_mut().zip(&self.max) {
            if let Some(cap) = cap {
                *width = (*width).min(*cap);
            }
        }
        let Width::Cells(budget) = budget else {
            return w;
        };
        // Squeeze the widest squeezable column first, one cell at a time, so
        // the result is a function of the widths alone — the same input gives
        // the same table in every family, which is what makes "degrades
        // identically" a testable claim rather than an aspiration.
        let gaps = cols.saturating_sub(1) * GAP.len();
        while w.iter().sum::<usize>() + gaps > budget {
            let victim = (0..cols)
                .filter(|&i| !self.rigid[i] && w[i] > MIN_COL)
                .max_by_key(|&i| w[i]);
            match victim {
                Some(i) => w[i] -= 1,
                None => break,
            }
        }
        w
    }

    /// Render, with every line right-trimmed: trailing padding is what breaks
    /// `cut` and makes a snapshot diff unreadable.
    pub(crate) fn render(&self, budget: Width) -> String {
        let w = self.widths(budget);
        let mut out = String::new();
        if !self.headers.is_empty() && !self.is_empty() {
            let cells: Vec<Cell> = self.headers.iter().map(|h| Cell::text(*h)).collect();
            push_row(&mut out, &cells, &w, &self.right);
        }
        for item in &self.items {
            match item {
                Item::Row(cells) => push_row(&mut out, cells, &w, &self.right),
                Item::Group(h) => {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    let _ = writeln!(out, "{h}");
                }
                Item::Detail(lines) => {
                    for l in lines {
                        let _ = writeln!(out, "{}", l.trim_end());
                    }
                }
            }
        }
        out
    }
}

fn push_row(out: &mut String, cells: &[Cell], w: &[usize], right: &[bool]) {
    let mut line = String::new();
    for (i, c) in cells.iter().enumerate() {
        if i > 0 {
            line.push_str(GAP);
        }
        let text = fit(c, w[i]);
        let pad = w[i].saturating_sub(text.chars().count());
        let right = right[i] || matches!(c, Cell::Int(_) | Cell::Num(..));
        if right {
            for _ in 0..pad {
                line.push(' ');
            }
            line.push_str(&text);
        } else {
            line.push_str(&text);
            for _ in 0..pad {
                line.push(' ');
            }
        }
    }
    let _ = writeln!(out, "{}", line.trim_end());
}

/// Cut text to the column, marking the cut. A number that does not fit keeps
/// every digit and overflows its column instead.
fn fit(c: &Cell, w: usize) -> String {
    let s = c.plain();
    if !c.truncatable() || s.chars().count() <= w {
        return s;
    }
    if w <= 1 {
        return "…".to_string();
    }
    let mut out: String = s.chars().take(w - 1).collect();
    out.push('…');
    out
}

/// One rendering: a sequence of aligned blocks, prose lines and notes.
///
/// This is what a [`super::Render`] impl draws into. It is a *document*
/// rather than a grid because the renderings this tool has always produced
/// are documents: a header block, then a grid, then a sentence, then another
/// grid whose columns mean something else entirely. Pretending otherwise is
/// what forced eleven renderers to keep their own `println!` prose beside a
/// hand-built column layout.
#[derive(Debug, Clone, Default)]
pub struct Table {
    blocks: Vec<Block>,
    notes: Vec<super::Note>,
}

#[derive(Debug, Clone)]
enum Block {
    Grid(Grid),
    /// A line of prose, verbatim.
    Line(String),
    /// A blank separator; consecutive ones collapse.
    Blank,
}

impl Table {
    pub fn new() -> Table {
        Table::default()
    }

    /// Add an aligned block.
    pub fn grid(&mut self, grid: Grid) -> &mut Table {
        self.blocks.push(Block::Grid(grid));
        self
    }

    /// A line of prose — a header, a label, a sentence that is not a note.
    pub fn line(&mut self, text: impl Into<String>) -> &mut Table {
        self.blocks.push(Block::Line(text.into()));
        self
    }

    /// A blank line. Consecutive blanks collapse, and a leading one is
    /// dropped, so a renderer can ask for separation without counting.
    pub fn blank(&mut self) -> &mut Table {
        self.blocks.push(Block::Blank);
        self
    }

    /// A note about *this rendering* — never about the fleet.
    ///
    /// The fleet's notes come from [`super::Render::notes`], and only those
    /// reach the machine-readable formats: there is no table over there for a
    /// rendering note to be about. The canonical case is `interface show`
    /// listing twenty carriers of two hundred — honest as a rendering note,
    /// because json mode emits every one of them as a row and nothing was
    /// actually dropped.
    pub fn note(&mut self, note: super::Note) -> &mut Table {
        self.notes.push(note);
        self
    }

    pub(crate) fn rendering_notes(&self) -> &[super::Note] {
        &self.notes
    }

    /// True when nothing at all was drawn — which is what an empty result
    /// looks like here, because the sentence explaining it is a note.
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(|b| match b {
            Block::Grid(g) => g.is_empty(),
            Block::Line(_) => false,
            Block::Blank => true,
        })
    }

    pub(crate) fn render(&self, budget: Width) -> String {
        let mut out = String::new();
        let mut pending_blank = false;
        for block in &self.blocks {
            let text = match block {
                Block::Blank => {
                    pending_blank = !out.is_empty();
                    continue;
                }
                Block::Line(l) => format!("{}\n", l.trim_end()),
                Block::Grid(g) => g.render(budget),
            };
            if text.is_empty() {
                continue;
            }
            if pending_blank {
                out.push('\n');
                pending_blank = false;
            }
            out.push_str(&text);
        }
        out
    }
}
