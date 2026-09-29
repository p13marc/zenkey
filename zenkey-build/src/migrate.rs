//! A registry directory, respelled: TOML in, KDL out (RFC 08 §5.1, v1.44;
//! #374).
//!
//! §5.1 made KDL a second *spelling* of one document, so a migration has
//! exactly one obligation — the directory it leaves behind means what the
//! one it was handed meant — and this module is built so that it cannot do
//! otherwise:
//!
//! - **Values come from the build's own reader**, never from a second one:
//!   each file is read by [`zenkey::registry_doc::parse_raw`], the tree
//!   every lint and every line of codegen reads, and written by
//!   [`zenkey::registry_doc::write_kdl`]. Never through `RegistrySlice`,
//!   which drops the columns it does not know (logs' `reason`): a migration
//!   that lost a column would be a different document wearing the old name.
//! - **Comments come from `toml_edit`**, which is the only thing it is here
//!   for. A table's own comment block (its decor prefix) becomes its node's
//!   leading block, `#` respelled `//`; the file's header — the comment
//!   block before the first table, up to its last blank line — stays the
//!   file's header, and a comment after the last entry stays at the end.
//! - **Everything else is copied beside unchanged**: the three `.lock`
//!   ledgers stay line files beside a `.kdl` file as beside a `.toml` one,
//!   which is why `registry.lock` survives a migration byte for byte — its
//!   pins name entries, not spellings.
//! - **The result is linted before it is anything**: [`stage_kdl`] checks
//!   the source against the build's lints first (a registry that does not
//!   build is refused, not respelled), converts into a staging directory,
//!   proves every file reads back to the same tree, and lints the staged
//!   directory with the same [`Config::lint`] a consumer's build script
//!   runs. Committing the staged directory is the caller's (zenctl's
//!   `registry migrate`), so a failure anywhere here leaves the source as
//!   it was.
//!
//! ## The honest bound
//!
//! KDL comments have no place to stand *on* a property: a KDL node is one
//! line (or a `\`-continued one), and a comment between two of its
//! properties would read as belonging to neither. So a comment written on or
//! above a **key** — `path = "x"  # why`, a `# …` line inside a table, a
//! comment inside a multi-line array — is **hoisted** into its table's
//! leading block, as `// <key>: <text>`. Every comment survives; its column
//! is named rather than implied by position. Blank lines inside a comment
//! block survive as an empty `//` line. Nothing else is lost, and
//! [`Converted::hoisted`] counts what moved.
//!
//! Behind the `migrate` feature: this crate runs in consumers' build
//! scripts, and a comment-preserving TOML parser is an authoring tool's
//! dependency, not a build's.

use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, InlineTable, Item, RawString, Table, Value};
use zenkey::registry_doc::{RawTable, RawValue, SliceFormat, parse_raw, write_kdl};

use crate::{Config, Error, RegistryWarning};

/// Why a migration did not happen.
///
/// Two halves, and [`MigrateError::is_unaskable`] is the line between them:
/// the **input** was refused (nothing was attempted that could fail), or
/// the **migration** failed (it was attempted, and its result did not hold
/// up). A caller exit-codes on that split — zenctl's contract makes the
/// first a 2 and the second a 1.
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    /// A file this migration refuses: it does not parse, it holds a shape
    /// KDL cannot spell (RFC 08 §5.1), or the directory holds no TOML file
    /// at all.
    #[error("{file}: {message}")]
    Refused { file: String, message: String },
    /// The source directory does not pass the build's own checks — or could
    /// not be read. A registry that does not build is not respelled: the
    /// lint's message names the source's files, which a lint of the staged
    /// result would not.
    #[error("the source registry does not build, so it is not migrated: {0}")]
    Source(#[source] Error),
    /// A staged file does not read back to the document it was converted
    /// from — a defect of this module, never of the input.
    #[error("{file}: the KDL spelling does not mean the TOML document ({message})")]
    Unfaithful { file: String, message: String },
    /// The staged directory fails a lint the source passed — the same kind
    /// of defect, caught by the build's own checks.
    #[error("the migrated registry fails a lint its source passes: {0}")]
    Staged(#[source] Error),
    /// Writing the staging directory failed.
    #[error("{0:?}")]
    Io(PathBuf, #[source] std::io::Error),
}

impl MigrateError {
    /// Whether the input was refused before anything was attempted — the
    /// mirror of [`Error::is_unaskable`]. A refused file, or a source that
    /// does not build, is; a staged result that failed its checks, or a
    /// write that failed, is the migration itself failing.
    pub fn is_unaskable(&self) -> bool {
        matches!(self, MigrateError::Refused { .. } | MigrateError::Source(_))
    }
}

/// One file, respelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    /// The KDL text: the file's header, the document, its trailing comment.
    pub kdl: String,
    /// Comment lines carried across, in total.
    pub comments: usize,
    /// Of those, the ones written on or above a key and hoisted into their
    /// table's leading block as `// <key>: …` — the honest bound.
    pub hoisted: usize,
}

/// Respell one registry TOML file (a producer's, a service's or the type
/// table) as KDL, keeping its comments within the bound the module doc
/// states.
///
/// The document is `parse_raw`'s, so the KDL means exactly what the build
/// reads from the TOML; `toml_edit` contributes the comments and nothing
/// else.
pub fn toml_to_kdl(src: &str) -> Result<Converted, zenkey::slice::SliceError> {
    let mut raw = parse_raw(src, SliceFormat::Toml)?;
    let doc: DocumentMut = src
        .parse()
        .map_err(|e: toml_edit::TomlError| zenkey::slice::SliceError::Shape(e.to_string()))?;
    let (header_region, header) = header(src);
    let mut cx = Cx {
        header_region,
        header_taken: false,
        comments: header.iter().filter(|l| l.is_some()).count(),
        hoisted: 0,
    };
    attach(&mut raw, doc.as_table(), &mut cx);
    let trailer = block(doc.trailing());
    cx.comments += trailer.iter().filter(|l| l.is_some()).count();

    let mut kdl = String::new();
    if !header.is_empty() {
        write_block(&mut kdl, &header);
        kdl.push('\n');
    }
    kdl.push_str(&write_kdl(&raw)?);
    if !trailer.is_empty() {
        kdl.push('\n');
        write_block(&mut kdl, &trailer);
    }
    Ok(Converted {
        kdl,
        comments: cx.comments,
        hoisted: cx.hoisted,
    })
}

/// What [`stage_kdl`] did.
#[derive(Debug, Clone)]
pub struct Migration {
    /// Every respelled file: `(from, to, conversion)`, by file name, sorted.
    pub files: Vec<(String, String, Converted)>,
    /// Every other entry of the source copied beside unchanged — the
    /// ledgers, a `.kdl` file already there, a `schemas/` directory — by
    /// name, sorted.
    pub copied: Vec<String>,
    /// The staged directory's lint warnings — what its build would print.
    pub warnings: Vec<RegistryWarning>,
}

/// Respell every `*.toml` file of `src` into `staging` as `<stem>.kdl`, copy
/// every other entry beside unchanged, and prove the result.
///
/// `staging` must be an existing, empty directory the caller owns; this
/// function writes nothing anywhere else, so committing the result (or
/// throwing it away) is the caller's decision, and `src` is never touched.
/// In order, and stopping at the first failure:
///
/// 1. the source is linted as its build would lint it
///    ([`MigrateError::Source`]);
/// 2. each file converts ([`MigrateError::Refused`]) and reads back, as KDL,
///    to the tree it came from ([`MigrateError::Unfaithful`]);
/// 3. the staged directory is linted the same way ([`MigrateError::Staged`]).
pub fn stage_kdl(src: &Path, staging: &Path) -> Result<Migration, MigrateError> {
    let lint = |dir: &Path| Config::new().registry_dir(dir).no_rerun_if_changed().lint();
    lint(src).map_err(MigrateError::Source)?;

    let mut entries: Vec<PathBuf> = std::fs::read_dir(src)
        .map_err(|e| MigrateError::Source(Error::Io(src.to_path_buf(), e)))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    let is_toml = |p: &Path| {
        p.is_file() && p.extension().and_then(|e| e.to_str()) == Some(SliceFormat::Toml.extension())
    };
    if !entries.iter().any(|p| is_toml(p)) {
        return Err(MigrateError::Refused {
            file: src.display().to_string(),
            message: "holds no registry *.toml file — nothing to migrate".to_string(),
        });
    }

    let mut files = Vec::new();
    let mut copied = Vec::new();
    for path in &entries {
        let name = file_name(path);
        let target_io = |target: &Path, e| MigrateError::Io(target.to_path_buf(), e);
        if !is_toml(path) {
            let target = staging.join(&name);
            copy_all(path, &target).map_err(|e| target_io(&target, e))?;
            copied.push(name);
            continue;
        }
        let src_text = std::fs::read_to_string(path)
            .map_err(|e| MigrateError::Source(Error::Io(path.clone(), e)))?;
        let converted = toml_to_kdl(&src_text).map_err(|e| MigrateError::Refused {
            file: name.clone(),
            message: e.to_string(),
        })?;
        // The tree, twice: what the build reads from the TOML is what it
        // will read from the KDL. The staged lint below would catch most of
        // a divergence; this catches all of it, unknown columns included.
        let unfaithful = |message: String| MigrateError::Unfaithful {
            file: name.clone(),
            message,
        };
        let want =
            parse_raw(&src_text, SliceFormat::Toml).map_err(|e| unfaithful(e.to_string()))?;
        let got =
            parse_raw(&converted.kdl, SliceFormat::Kdl).map_err(|e| unfaithful(e.to_string()))?;
        if got != want {
            return Err(unfaithful("the two trees differ".to_string()));
        }
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let to = format!("{stem}.{}", SliceFormat::Kdl.extension());
        let target = staging.join(&to);
        std::fs::write(&target, &converted.kdl).map_err(|e| target_io(&target, e))?;
        files.push((name, to, converted));
    }

    let warnings = lint(staging).map_err(MigrateError::Staged)?;
    Ok(Migration {
        files,
        copied,
        warnings,
    })
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// A file, or a directory and everything under it.
fn copy_all(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

// ─── comments ───────────────────────────────────────────────────────────────

/// One line of a comment block: `Some(text)` for a comment (its `#` and one
/// following space removed), `None` for a blank line.
type Line = Option<String>;

struct Cx<'a> {
    /// The source text the file's header occupies, which the first table's
    /// decor prefix begins with (and must not be written twice).
    header_region: &'a str,
    header_taken: bool,
    comments: usize,
    hoisted: usize,
}

/// The file's header: the comment block before the first table, up to the
/// last blank line in it. Comments that touch the first table's header are
/// that table's, not the file's.
fn header(src: &str) -> (&str, Vec<Line>) {
    let mut end = 0; // byte offset just past the last blank line
    let mut offset = 0;
    let mut seen_comment = false;
    for line in src.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with('#') {
            seen_comment = true;
        } else if t.is_empty() {
            if seen_comment {
                end = offset + line.len();
            }
        } else {
            break;
        }
        offset += line.len();
    }
    let region = &src[..end];
    (region, trim_blank(lines(region)))
}

/// Every line of a decor string, as comment text or a blank.
fn lines(s: &str) -> Vec<Line> {
    s.lines()
        .map(|l| {
            let t = l.trim();
            t.strip_prefix('#')
                .map(|c| c.strip_prefix(' ').unwrap_or(c).trim_end().to_string())
        })
        .collect()
}

/// Without the blank lines around it — the writer spaces entries itself.
fn trim_blank(mut v: Vec<Line>) -> Vec<Line> {
    while v.last().is_some_and(Option::is_none) {
        v.pop();
    }
    let lead = v.iter().take_while(|l| l.is_none()).count();
    v.drain(..lead);
    v
}

/// A decor string as a comment block, blank lines at its edges dropped.
fn block(raw: &RawString) -> Vec<Line> {
    trim_blank(lines(raw.as_str().unwrap_or_default()))
}

/// The comments of a decor string alone — what a key or a value carries.
fn comments_of(raw: Option<&RawString>) -> Vec<String> {
    raw.and_then(RawString::as_str)
        .map(|s| lines(s).into_iter().flatten().collect())
        .unwrap_or_default()
}

fn write_block(out: &mut String, block: &[Line]) {
    for l in block {
        match l {
            None => out.push('\n'),
            Some(t) if t.is_empty() => out.push_str("//\n"),
            Some(t) => {
                out.push_str("// ");
                out.push_str(t);
                out.push('\n');
            }
        }
    }
}

/// Carry `t`'s comments onto `raw`, the table the build reads from it:
/// its own block to the node's leading block, every key's hoisted beside
/// it, and every sub-table's onto its own node.
fn attach(raw: &mut RawTable, t: &Table, cx: &mut Cx<'_>) {
    let mut prefix = t.decor().prefix().and_then(RawString::as_str).unwrap_or("");
    if !cx.header_taken
        && !cx.header_region.is_empty()
        && let Some(rest) = prefix.strip_prefix(cx.header_region)
    {
        cx.header_taken = true;
        prefix = rest;
    }
    let mut leading: Vec<String> = trim_blank(lines(prefix))
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect();
    // A comment on the header line itself (`[[subject]]  # …`) is about the
    // table: it joins the block unprefixed.
    leading.extend(comments_of(t.decor().suffix()));
    cx.comments += leading.len();

    for (key, item) in t.iter() {
        let mut hoist: Vec<String> = Vec::new();
        if let Some(k) = t.key(key) {
            hoist.extend(comments_of(k.leaf_decor().prefix()));
        }
        match item {
            Item::Value(v) => value_comments(v, &mut hoist),
            Item::Table(sub) => {
                if let Some(RawValue::Table(rt)) = raw_field(raw, key) {
                    attach(rt, sub, cx);
                }
            }
            Item::ArrayOfTables(rows) => {
                if let Some(RawValue::List(rrows)) = raw_field(raw, key) {
                    for (rt, sub) in rrows.iter_mut().zip(rows.iter()) {
                        if let RawValue::Table(rt) = rt {
                            attach(rt, sub, cx);
                        }
                    }
                }
            }
            Item::None => {}
        }
        cx.comments += hoist.len();
        cx.hoisted += hoist.len();
        leading.extend(hoist.into_iter().map(|c| {
            if c.is_empty() {
                format!("{key}:")
            } else {
                format!("{key}: {c}")
            }
        }));
    }
    raw.leading.extend(leading);
}

fn raw_field<'r>(raw: &'r mut RawTable, key: &str) -> Option<&'r mut RawValue> {
    raw.fields
        .iter_mut()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

/// Every comment a value carries — after it on its line, and anywhere
/// inside a multi-line array or inline table.
fn value_comments(v: &Value, out: &mut Vec<String>) {
    out.extend(comments_of(v.decor().prefix()));
    match v {
        Value::Array(a) => array_comments(a, out),
        Value::InlineTable(t) => inline_comments(t, out),
        _ => {}
    }
    out.extend(comments_of(v.decor().suffix()));
}

fn array_comments(a: &Array, out: &mut Vec<String>) {
    for v in a.iter() {
        value_comments(v, out);
    }
    out.extend(comments_of(Some(a.trailing())));
}

fn inline_comments(t: &InlineTable, out: &mut Vec<String>) {
    for (key, v) in t.iter() {
        if let Some(k) = t.key(key) {
            out.extend(comments_of(k.leaf_decor().prefix()));
        }
        value_comments(v, out);
    }
    out.extend(comments_of(Some(t.trailing())));
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "\
# The file's header.
#
#   an indented line
# Source: here.

# The registry table's own comment.
[registry]
version = \"0.1\"
app = \"t\"
convention = 1

[producer]  # on the header line
name = \"t\"
# why this column
description = \"a producer\"  # trailing

# ── state ──

# The health document.
[[subject]]
path = \"health\"
common = \"health\"
class = \"state\"
type = \"Health\"
since = \"0.1\"
when = [
    \"a\",  # the first
    # before the second
    \"b\",
]
reason = \"an unknown column crosses too\"

# after the last entry
";

    #[test]
    fn every_comment_lands_on_its_node_and_the_document_is_unchanged() {
        let c = toml_to_kdl(DOC).unwrap();
        let want = "\
// The file's header.
//
//   an indented line
// Source: here.

// The registry table's own comment.
registry version=\"0.1\" app=\"t\" convention=1

// on the header line
// description: why this column
// description: trailing
producer \"t\" description=\"a producer\"

// ── state ──
//
// The health document.
// when: the first
// when: before the second
subject \"health\" common=health class=state type=\"Health\" since=\"0.1\" \\
    reason=\"an unknown column crosses too\" {
    when \"a\" \"b\"
}

// after the last entry
";
        assert_eq!(c.kdl, want, "\n{}", c.kdl);
        assert_eq!(
            parse_raw(&c.kdl, SliceFormat::Kdl).unwrap(),
            parse_raw(DOC, SliceFormat::Toml).unwrap()
        );
        // header 4, registry 1, producer 1 + 2 hoisted, subject 3 + 2
        // hoisted, trailer 1.
        assert_eq!((c.comments, c.hoisted), (14, 4));
    }

    /// No header block when the first comment touches the first table:
    /// then it is that table's.
    #[test]
    fn a_comment_touching_the_first_table_is_the_tables() {
        let src = "# about registry\n[registry]\nversion = \"0.1\"\napp = \"t\"\nconvention = 1\n";
        let c = toml_to_kdl(src).unwrap();
        assert_eq!(
            c.kdl,
            "// about registry\nregistry version=\"0.1\" app=\"t\" convention=1\n"
        );
    }

    /// The type table: each `[types.<Name>]`'s block lands on its `type`
    /// node, and the header stays the file's.
    #[test]
    fn the_type_table_keeps_its_comments_per_type() {
        let src = "# header\n\n[types.A]\nkind = \"json-schema\"\n\n# about B\n[types.B]\nkind = \"json-schema\"\n";
        let c = toml_to_kdl(src).unwrap();
        assert_eq!(
            c.kdl,
            "// header\n\ntype \"A\" kind=\"json-schema\"\n// about B\ntype \"B\" kind=\"json-schema\"\n"
        );
    }

    /// A shape KDL cannot spell is refused, never dropped (§5.1).
    #[test]
    fn a_shape_without_a_kdl_spelling_is_refused() {
        assert!(toml_to_kdl("loose = 1\n[registry]\nversion = \"0.1\"\n").is_err());
        assert!(toml_to_kdl("[registry\n").is_err());
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "zenkey-build-migrate-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const T: &str = "# t\n\n[registry]\nversion = \"0.1\"\napp = \"t\"\nconvention = 1\ncompat = \"none\"\n\n[producer]\nname = \"t\"\n\n[[subject]]\npath = \"health\"\ncommon = \"health\"\nclass = \"state\"\ntype = \"HealthSnapshot\"\nttl_s = 60\nsince = \"0.1\"\ndescription = \"health\"\n";

    /// A directory: each TOML respelled, everything else copied beside,
    /// the staged result linted — and the source never written.
    #[test]
    fn a_directory_stages_whole_and_the_source_is_untouched() {
        let (src, out) = (scratch("src"), scratch("out"));
        std::fs::write(src.join("t.toml"), T).unwrap();
        std::fs::write(src.join("deprecated.lock"), "").unwrap();
        std::fs::create_dir_all(src.join("schemas")).unwrap();
        std::fs::write(src.join("schemas/x.json"), "{}").unwrap();
        let m = stage_kdl(&src, &out).unwrap();
        assert_eq!(m.files.len(), 1);
        assert_eq!(
            (m.files[0].0.as_str(), m.files[0].1.as_str()),
            ("t.toml", "t.kdl")
        );
        assert_eq!(m.copied, vec!["deprecated.lock", "schemas"]);
        assert_eq!(m.warnings.len(), 1, "compat = \"none\" still warns");
        assert!(out.join("t.kdl").is_file() && out.join("schemas/x.json").is_file());
        assert!(!out.join("t.toml").exists());
        assert_eq!(std::fs::read_to_string(src.join("t.toml")).unwrap(), T);
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// The refusals are the input's; a directory holding a stem in both
    /// spellings does not build, so it is refused as the source.
    #[test]
    fn the_input_refusals_are_unaskable() {
        let (src, out) = (scratch("both"), scratch("both-out"));
        std::fs::write(src.join("t.toml"), T).unwrap();
        std::fs::write(src.join("t.kdl"), "registry\n").unwrap();
        let e = stage_kdl(&src, &out).unwrap_err();
        assert!(matches!(e, MigrateError::Source(_)), "{e}");
        assert!(e.to_string().contains("one stem in two spellings"), "{e}");
        assert!(e.is_unaskable());

        let empty = scratch("empty");
        let e = stage_kdl(&empty, &out).unwrap_err();
        assert!(matches!(e, MigrateError::Refused { .. }), "{e}");
        assert!(e.is_unaskable());

        let e = stage_kdl(&empty.join("missing"), &out).unwrap_err();
        assert!(e.is_unaskable(), "a missing source is refused: {e}");

        assert!(!MigrateError::Staged(Error::NoOutDir).is_unaskable());
        for d in [src, out, empty] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
