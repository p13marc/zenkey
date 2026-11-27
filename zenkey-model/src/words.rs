//! Words by table: what the profiles whose records carry an index where
//! an earlier major carried text have in common (`desired.v2` §2.10,
//! `health.v2` §2.10; #986's Q2).
//!
//! A record's field holds a number, a contract's annotation holds a list,
//! and the entry at that number is the word. A number the list in hand
//! does not reach is shown as the number, and one reserved code says the
//! writer's table held no such word:
//!
//! ```text
//! list(value, max, entry, once) ─▶ an annotation's value as a table: a list of strings, or none
//! Index::at(i, max)             ─▶ a place in a table as a record carries it, or out_of_range
//! write(table, word, max)       ─▶ the index of a word its table holds, else out_of_range
//! read(table, index)            ─▶ the entry │ the number, past the table or with none in hand │ out_of_range
//! uses(contract, profile)       ─▶ whether the contract may declare that profile's tables
//! instance_table(named)         ─▶ an instance's table among the revisions its descriptor names: none │ the one │ differ
//! Lint                          ─▶ one finding of a profile's lint of a contract's tables
//! ```
//!
//! What a number means stays each profile's: `desired.v2`'s two tables are
//! a report resource's, indexed from 0 to 254, and `health.v2`'s three are
//! an instance's, in the upper half of a 6-bit word under the profile's
//! own names. So does each profile's `Major`, which reads a contract's
//! `uses` for one and a descriptor's `interfaces` for the other. An
//! instance's table ([`Instance`]) is `health.v2`'s and `alarms.v2`'s, over
//! a list of names for one and of rules for the other.

use serde_json::{Value, json};

use crate::contract::{Contract, Resource};
use crate::packed::WriteRefusal;

/// The reserved word of a field that holds an index: the writer's table
/// did not hold the word (core §7.6's reserved code, by its name).
pub const OUT_OF_RANGE: &str = "out_of_range";

/// Whether a contract lists `profile` in `uses`: one that may declare
/// that profile's tables (core §10).
#[must_use]
pub fn uses(c: &Contract, profile: &str) -> bool {
    c.uses.iter().any(|u| u.to_string() == profile)
}

/// An index into a table, as a record carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Index {
    /// An entry's place in its table, from 0.
    At(u8),
    /// A word the writer's table does not hold.
    OutOfRange,
}

impl Index {
    /// The place `i` of a table whose greatest index is `max`:
    /// `out_of_range` above it, where a record has no code for the place.
    #[must_use]
    pub fn at(i: usize, max: u8) -> Index {
        match u8::try_from(i) {
            Ok(i) if i <= max => Index::At(i),
            _ => Index::OutOfRange,
        }
    }

    /// Core §7.6's JSON reading: the number, or the reserved code's name.
    #[must_use]
    pub fn to_json(self) -> Value {
        match self {
            Index::At(i) => json!(i),
            Index::OutOfRange => json!(OUT_OF_RANGE),
        }
    }

    /// [`Index::to_json`] read back.
    ///
    /// # Errors
    /// [`WriteRefusal::Range`] for a number that is no `u8`, and
    /// [`WriteRefusal::Value`] for any other value.
    pub fn of_json(v: &Value) -> Result<Index, WriteRefusal> {
        match v {
            Value::String(s) if s == OUT_OF_RANGE => Ok(Index::OutOfRange),
            Value::Number(n) => n
                .as_u64()
                .and_then(|i| u8::try_from(i).ok())
                .map(Index::At)
                .ok_or(WriteRefusal::Range),
            _ => Err(WriteRefusal::Value),
        }
    }
}

/// What an index reads as, with the table a reader holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Word<'a> {
    /// The table's entry.
    Entry(&'a str),
    /// An index past the table the reader holds, a later revision's, or
    /// any index with no table in hand: shown as its number.
    Number(u8),
    /// `out_of_range`: the writer's table did not hold the word. A reader
    /// shows that, never an entry.
    OutOfRange,
}

/// The word of `index` in `table`, `None` when the reader holds no table.
#[must_use]
pub fn read<S: AsRef<str>>(table: Option<&[S]>, index: Index) -> Word<'_> {
    match index {
        Index::OutOfRange => Word::OutOfRange,
        Index::At(i) => table
            .and_then(|t| t.get(usize::from(i)))
            .map_or(Word::Number(i), |w| Word::Entry(w.as_ref())),
    }
}

/// The index a writer gives `word`: its first place in `table`, where
/// that is at most `max`, and `out_of_range` for a word the table does not
/// hold there, whose text stays in the writer's log.
#[must_use]
pub fn write<S: AsRef<str>>(table: &[S], word: &str, max: u8) -> Index {
    table
        .iter()
        .position(|w| w.as_ref() == word)
        .map_or(Index::OutOfRange, |i| Index::at(i, max))
}

/// An annotation's value read as a table: a list of at most `max`
/// strings, each one `entry` accepts, and none twice where `once`. `None`
/// for any other value, which is no table.
#[must_use]
pub fn list(
    v: &Value,
    max: usize,
    entry: impl Fn(&str) -> bool,
    once: bool,
) -> Option<Vec<String>> {
    let a = v.as_array()?;
    if a.len() > max {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(a.len());
    for x in a {
        let x = x.as_str()?;
        if !entry(x) || (once && out.iter().any(|seen| seen == x)) {
            return None;
        }
        out.push(x.to_owned());
    }
    Some(out)
}

/// An instance's table (`health.v2` §2.10, `alarms.v2` §4): the one table
/// among the contracts it implements, the revisions its descriptor names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instance<T> {
    /// No named revision in hand holds one.
    None,
    /// The one list they hold.
    Table(Vec<T>),
    /// Two hold two different lists: no table.
    Differ,
}

impl<T> Instance<T> {
    /// The table, where the instance has one.
    #[must_use]
    pub fn table(&self) -> Option<&[T]> {
        match self {
            Instance::Table(t) => Some(t),
            Instance::None | Instance::Differ => None,
        }
    }
}

/// An instance's table, from the table of each revision its descriptor
/// names that the reader holds (`None`: that revision holds none, or its
/// declaration is invalid). Never a revision the descriptor does not
/// name: a table read under the wrong revision gives wrong words.
#[must_use]
pub fn instance_table<'a, T: PartialEq + Clone + 'a>(
    named: impl IntoIterator<Item = Option<&'a [T]>>,
) -> Instance<T> {
    let mut held: Option<&[T]> = None;
    for table in named.into_iter().flatten() {
        match held {
            None => held = Some(table),
            Some(first) if first == table => {}
            Some(_) => return Instance::Differ,
        }
    }
    held.map_or(Instance::None, |t| Instance::Table(t.to_vec()))
}

/// One finding of a profile's lint of a contract's tables and
/// declarations: what a build and `zk2 contract lint` report as an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    /// The profile whose rule the finding is under, as `uses` lists it.
    pub profile: &'static str,
    /// The resource whose declaration drew it ([`Lint::place`]).
    pub at: String,
    /// The annotation key the finding is of, where it is of one key.
    pub key: Option<&'static str>,
    /// The reason's token, as the profile's fixtures spell it.
    pub token: &'static str,
    /// The reason in words.
    pub says: &'static str,
}

impl Lint {
    /// How a finding names a resource: its kind token and its template.
    #[must_use]
    pub fn place(r: &Resource) -> String {
        format!("{}/{}", r.token, r.template.as_str())
    }
}

impl std::fmt::Display for Lint {
    /// A linter's line: `error[<profile> <token>] <at>: [<key>: ]<says>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error[{} {}] {}: ", self.profile, self.token, self.at)?;
        if let Some(key) = self.key {
            write!(f, "{key}: ")?;
        }
        f.write_str(self.says)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_reads_as_its_entry_its_number_or_out_of_range() {
        let table = ["a".to_owned(), "b".to_owned()];
        assert_eq!(read(Some(&table[..]), Index::At(1)), Word::Entry("b"));
        assert_eq!(read(Some(&table[..]), Index::At(2)), Word::Number(2));
        assert_eq!(read::<String>(None, Index::At(0)), Word::Number(0));
        assert_eq!(read(Some(&table[..]), Index::OutOfRange), Word::OutOfRange);
        assert_eq!(write(&table, "b", 254), Index::At(1));
        assert_eq!(write(&table, "b", 0), Index::OutOfRange, "past the code");
        assert_eq!(write(&table, "c", 254), Index::OutOfRange);
        assert_eq!(Index::at(255, 254), Index::OutOfRange);
        for i in [Index::At(7), Index::OutOfRange] {
            assert_eq!(Index::of_json(&i.to_json()), Ok(i));
        }
        assert_eq!(Index::of_json(&json!(256)), Err(WriteRefusal::Range));
        assert_eq!(Index::of_json(&json!("x")), Err(WriteRefusal::Value));
    }

    #[test]
    fn an_instance_has_the_one_table_its_named_revisions_hold() {
        let (a, b) = (vec![1, 2], vec![1, 3]);
        assert_eq!(instance_table::<u8>([None, None]), Instance::None);
        assert_eq!(
            instance_table([Some(&a[..]), None, Some(&a[..])]),
            Instance::Table(a.clone())
        );
        assert_eq!(
            instance_table([Some(&a[..]), Some(&b[..])]),
            Instance::Differ
        );
        assert_eq!(Instance::<u8>::Differ.table(), None);
        assert_eq!(Instance::Table(a.clone()).table(), Some(&a[..]));
    }

    #[test]
    fn a_list_is_a_table_within_its_bound_and_its_entries_rule() {
        let v = json!(["a", "b", "a"]);
        assert_eq!(list(&v, 3, |_| true, false).map(|t| t.len()), Some(3));
        assert_eq!(list(&v, 3, |_| true, true), None, "one twice");
        assert_eq!(list(&v, 2, |_| true, false), None, "one too many");
        assert_eq!(list(&v, 3, |s| s != "b", false), None);
        assert_eq!(list(&json!(["a", 1]), 3, |_| true, false), None);
        assert_eq!(list(&json!("a"), 3, |_| true, false), None);
    }
}
