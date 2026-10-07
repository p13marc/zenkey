//! S7, bundle stability and classifier feasibility (#603, r3 §3.11). Decides
//! U7 and the classifier's strategy per schema kind (#618).
//!
//! 1. **Determinism:** each example's protobuf set compiled with protox twice,
//!    with protoc and with `buf build`; bytes compared, then a normalized
//!    semantic comparison (the retention rule's identity check).
//! 2. **Protobuf classifiers** on prost-reflect, over the matrix in
//!    `s7/matrix/<case>/{old,new}`:
//!    - `buf`: buf's WIRE_JSON rules, evaluated old→new and new→old, cross-
//!      checked against `buf breaking` in both argument orders;
//!    - `zk2`: directional reader/writer semantics (unknown fields ignored,
//!      missing fields defaulted; a change breaks when the wire or JSON
//!      encoding of the same field diverges), evaluated both ways.
//! 3. **Bundles for the cross-language check:** every example's bundle file,
//!    for `s7/verify_bundles.py` (Python `rfc8785`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use prost::Message as _;
use prost_reflect::{Cardinality, DescriptorPool, Kind, MessageDescriptor};
use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::load_path;
use zk2rt::metrics::csv_row;

fn compile(dir: &Path, file: &str) -> Result<(Vec<u8>, DescriptorPool)> {
    let mut c = protox::Compiler::new([dir]).map_err(|e| anyhow!("{e}"))?;
    c.include_imports(true).include_source_info(false);
    c.open_file(dir.join(file)).map_err(|e| anyhow!("{e}"))?;
    Ok((c.encode_file_descriptor_set(), c.descriptor_pool()))
}

/// A normalized description of a descriptor set: files sorted by name,
/// source info dropped, `json_name` dropped where it equals the default.
fn normalize(bytes: &[u8]) -> Result<prost_types::FileDescriptorSet> {
    let mut fds = prost_types::FileDescriptorSet::decode(bytes)?;
    fds.file.sort_by(|a, b| a.name.cmp(&b.name));
    for f in &mut fds.file {
        f.source_code_info = None;
        fn walk(m: &mut prost_types::DescriptorProto) {
            for fld in &mut m.field {
                if let (Some(j), Some(n)) = (&fld.json_name, &fld.name) {
                    if *j == default_json_name(n) {
                        fld.json_name = None;
                    }
                }
            }
            for n in &mut m.nested_type {
                walk(n);
            }
        }
        for m in &mut f.message_type {
            walk(m);
        }
    }
    Ok(fds)
}

fn default_json_name(n: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in n.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

struct DetRow {
    contract: String,
    file: String,
    protox_twice: bool,
    protoc_bytes: String,
    buf_bytes: String,
    semantic: String,
}

fn determinism(examples: &Path, buf: &Path, scratch: &Path) -> Result<Vec<DetRow>> {
    let mut rows = Vec::new();
    let mut files = Vec::new();
    for e in walk(examples) {
        if e.extension().is_some_and(|x| x == "toml") && !e.to_string_lossy().ends_with(".bindings.toml") {
            files.push(e);
        }
    }
    files.sort();
    for f in files {
        let text = std::fs::read_to_string(&f)?;
        let Ok(cf) = toml::from_str::<zenkey_model::authoring::ContractFile>(&text) else { continue };
        let dir = f.parent().unwrap_or(Path::new("."));
        let inc = dir.join("proto");
        for p in &cf.schemas.protobuf {
            let rel = p.strip_prefix("proto/").unwrap_or(p);
            let (a, _) = compile(&inc, rel)?;
            let (b, _) = compile(&inc, rel)?;
            let out_c = scratch.join("protoc.binpb");
            let pc = Command::new("protoc")
                .arg(format!("-I{}", inc.display()))
                .arg("--include_imports")
                .arg(format!("--descriptor_set_out={}", out_c.display()))
                .arg(inc.join(rel))
                .status()?;
            let c = if pc.success() { std::fs::read(&out_c)? } else { Vec::new() };
            let bdir = scratch.join("bufmod");
            let _ = std::fs::remove_dir_all(&bdir);
            std::fs::create_dir_all(bdir.join(Path::new(rel).parent().unwrap_or(Path::new(""))))?;
            std::fs::copy(inc.join(rel), bdir.join(rel))?;
            std::fs::write(bdir.join("buf.yaml"), "version: v2\n")?;
            let out_b = scratch.join("buf.binpb");
            let bb = Command::new(buf).current_dir(&bdir).args(["build", "--as-file-descriptor-set", "-o"]).arg(&out_b).status()?;
            let bufb = if bb.success() { std::fs::read(&out_b)? } else { Vec::new() };
            let na = normalize(&a)?;
            let same = |x: &[u8]| -> String {
                if x.is_empty() {
                    return "n/a".into();
                }
                match normalize(x) {
                    Ok(n) if n == na => "identical".into(),
                    Ok(_) => "differs".into(),
                    Err(e) => format!("error {e}"),
                }
            };
            rows.push(DetRow {
                contract: cf.interface.name.clone() + &format!(".v{}", cf.interface.major),
                file: rel.to_owned(),
                protox_twice: a == b,
                protoc_bytes: if c.is_empty() { "n/a".into() } else if c == a { "equal".into() } else { format!("differ ({} vs {} B)", c.len(), a.len()) },
                buf_bytes: if bufb.is_empty() { "n/a".into() } else if bufb == a { "equal".into() } else { format!("differ ({} vs {} B)", bufb.len(), a.len()) },
                semantic: format!("protoc {}, buf {}", same(&c), same(&bufb)),
            });
        }
    }
    Ok(rows)
}

fn walk(p: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let q = e.path();
            if q.is_dir() {
                out.extend(walk(&q));
            } else {
                out.push(q);
            }
        }
    }
    out
}

fn reserved_number(m: &MessageDescriptor, n: u32) -> bool {
    m.reserved_ranges().any(|r| r.contains(&n))
}

fn reserved_name(m: &MessageDescriptor, name: &str) -> bool {
    m.reserved_names().any(|r| r == name)
}

fn kind_str(k: &Kind) -> String {
    match k {
        Kind::Message(m) => format!("message {}", m.full_name()),
        Kind::Enum(e) => format!("enum {}", e.full_name()),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// buf's WIRE_JSON rules, for `new` checked against `old` (one order).
fn buf_rules(old: &DescriptorPool, new: &DescriptorPool) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for om in old.all_messages() {
        let Some(nm) = new.get_message_by_name(om.full_name()) else { continue };
        for of in om.fields() {
            match nm.get_field(of.number()) {
                None => {
                    if !reserved_number(&nm, of.number()) {
                        out.insert("FIELD_NO_DELETE_UNLESS_NUMBER_RESERVED");
                    }
                    if !reserved_name(&nm, of.name()) {
                        out.insert("FIELD_NO_DELETE_UNLESS_NAME_RESERVED");
                    }
                }
                Some(nf) => {
                    if nf.name() != of.name() {
                        out.insert("FIELD_SAME_NAME");
                    }
                    if nf.json_name() != of.json_name() {
                        out.insert("FIELD_SAME_JSON_NAME");
                    }
                    if kind_str(&nf.kind()) != kind_str(&of.kind()) {
                        out.insert("FIELD_WIRE_JSON_COMPATIBLE_TYPE");
                    }
                    if (nf.cardinality() == Cardinality::Repeated) != (of.cardinality() == Cardinality::Repeated) {
                        out.insert("FIELD_WIRE_JSON_COMPATIBLE_CARDINALITY");
                    }
                    let oo = of.containing_oneof().filter(|o| !o.is_synthetic()).map(|o| o.name().to_owned());
                    let no = nf.containing_oneof().filter(|o| !o.is_synthetic()).map(|o| o.name().to_owned());
                    if oo != no {
                        out.insert("FIELD_SAME_ONEOF");
                    }
                }
            }
        }
        for r in om.reserved_ranges() {
            if !r.clone().all(|n| reserved_number(&nm, n)) {
                out.insert("RESERVED_RANGE_NO_DELETE");
            }
        }
        for n in om.reserved_names() {
            if !reserved_name(&nm, n) {
                out.insert("RESERVED_NAME_NO_DELETE");
            }
        }
    }
    for oe in old.all_enums() {
        let Some(ne) = new.get_enum_by_name(oe.full_name()) else { continue };
        for ov in oe.values() {
            match ne.get_value(ov.number()) {
                None => {
                    if !ne.reserved_ranges().any(|r| r.contains(&ov.number())) {
                        out.insert("ENUM_VALUE_NO_DELETE_UNLESS_NUMBER_RESERVED");
                    }
                    if !ne.reserved_names().any(|r| r == ov.name()) {
                        out.insert("ENUM_VALUE_NO_DELETE_UNLESS_NAME_RESERVED");
                    }
                }
                Some(nv) => {
                    if nv.name() != ov.name() {
                        out.insert("ENUM_VALUE_SAME_NAME");
                    }
                }
            }
        }
    }
    out
}

/// zk2's directional semantics: can a reader with schema `reader` read
/// data written with schema `writer`, in binary and in protobuf JSON
/// (unknown fields ignored, missing fields defaulted)?
fn zk2_reads(reader: &DescriptorPool, writer: &DescriptorPool, json: bool) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for wm in writer.all_messages() {
        let Some(rm) = reader.get_message_by_name(wm.full_name()) else { continue };
        for wf in wm.fields() {
            // The same field under another number: a delete plus an add, so
            // its data is silently dropped in both directions.
            if rm.get_field_by_name(wf.name()).is_some_and(|rf| rf.number() != wf.number()) {
                out.insert("field renumbered (data silently dropped)");
            }
            let Some(rf) = rm.get_field(wf.number()) else { continue }; // unknown: ignored
            if json && rf.json_name() != wf.json_name() {
                out.insert("json name differs");
            }
            if kind_str(&rf.kind()) != kind_str(&wf.kind()) {
                out.insert("type differs");
            }
            if (rf.cardinality() == Cardinality::Repeated) != (wf.cardinality() == Cardinality::Repeated) {
                out.insert("cardinality differs");
            }
            let a = rf.containing_oneof().filter(|o| !o.is_synthetic()).map(|o| o.name().to_owned());
            let b = wf.containing_oneof().filter(|o| !o.is_synthetic()).map(|o| o.name().to_owned());
            if a != b {
                out.insert("oneof membership differs");
            }
        }
    }
    if json {
        for we in writer.all_enums() {
            let Some(re) = reader.get_enum_by_name(we.full_name()) else { continue };
            for wv in we.values() {
                match re.get_value(wv.number()) {
                    None => {
                        out.insert("enum value unknown to the reader (JSON names it)");
                    }
                    Some(rv) if rv.name() != wv.name() => {
                        out.insert("enum value renamed (JSON names it)");
                    }
                    Some(_) => {}
                }
            }
        }
    }
    out
}

/// A hygiene lint, not a compatibility verdict: a field or enum value
/// deleted without reserving its number (reuse is caught against history).
fn zk2_lint(old: &DescriptorPool, new: &DescriptorPool) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for om in old.all_messages() {
        let Some(nm) = new.get_message_by_name(om.full_name()) else { continue };
        for of in om.fields() {
            if nm.get_field(of.number()).is_none() && !reserved_number(&nm, of.number()) {
                out.insert("W: field deleted without reserving its number");
            }
        }
    }
    out
}

struct MatrixRow {
    case: String,
    expect: String,
    buf_fwd: String,
    buf_rev: String,
    ours_fwd: String,
    ours_rev: String,
    agree: bool,
    zk2: String,
}

fn buf_breaking(buf: &Path, dir: &Path, input: &str, against: &str) -> Result<(bool, String)> {
    let o = Command::new(buf).current_dir(dir).args(["breaking", input, "--against", against]).output()?;
    let text = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
    Ok((o.status.code() == Some(100), text.lines().count().to_string()))
}

fn matrix(dir: &Path, buf: &Path) -> Result<Vec<MatrixRow>> {
    let mut cases: Vec<PathBuf> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    cases.sort();
    let mut rows = Vec::new();
    for c in cases {
        let (_, old) = compile(&c.join("old"), "m.proto")?;
        let (_, new) = compile(&c.join("new"), "m.proto")?;
        let fwd = buf_rules(&old, &new);
        let rev = buf_rules(&new, &old);
        let (bf, _) = buf_breaking(buf, &c, "new", "old")?;
        let (br, _) = buf_breaking(buf, &c, "old", "new")?;
        let agree = bf == !fwd.is_empty() && br == !rev.is_empty();
        let new_reads_old = zk2_reads(&new, &old, true);
        let old_reads_new = zk2_reads(&old, &new, true);
        let wire = zk2_reads(&new, &old, false).len() + zk2_reads(&old, &new, false).len() == 0;
        let lint = zk2_lint(&old, &new);
        let verdict = if new_reads_old.is_empty() && old_reads_new.is_empty() { "compatible both ways" } else { "breaking" };
        let fmt = |s: &BTreeSet<&str>| if s.is_empty() { "ok".to_owned() } else { s.iter().copied().collect::<Vec<_>>().join(" + ") };
        rows.push(MatrixRow {
            case: c.file_name().unwrap().to_string_lossy().into_owned(),
            expect: std::fs::read_to_string(c.join("expect.txt")).unwrap_or_default().trim().to_owned(),
            buf_fwd: if bf { "breaking".into() } else { "ok".into() },
            buf_rev: if br { "breaking".into() } else { "ok".into() },
            ours_fwd: fmt(&fwd),
            ours_rev: fmt(&rev),
            agree,
            zk2: format!(
                "WIRE_JSON: {verdict} (new reads old: {}; old reads new: {}){}. WIRE only: {}",
                fmt(&new_reads_old),
                fmt(&old_reads_new),
                if lint.is_empty() { String::new() } else { format!("; {}", fmt(&lint)) },
                if wire { "compatible" } else { "breaking" }
            ),
        });
    }
    Ok(rows)
}

/// Writes every example contract's bundle to `out` as `<iface>.<hex>.bundle.json`.
fn bundles(examples: &Path, out: &Path) -> Result<usize> {
    std::fs::create_dir_all(out)?;
    let mut n = 0;
    for f in walk(examples) {
        if !(f.extension().is_some_and(|x| x == "toml") && !f.to_string_lossy().ends_with(".bindings.toml")) {
            continue;
        }
        let l = load_path(&f);
        let Some(c) = l.contract else { continue };
        let b = Bundle::build(&c);
        std::fs::write(out.join(format!("{}.{}.bundle.json", c.iface, Fingerprint::of(&c).hex())), b.to_bytes())?;
        n += 1;
    }
    Ok(n)
}

pub async fn run(results: &Path, examples: &Path, matrix_dir: &Path, buf: &Path) -> Result<()> {
    std::fs::create_dir_all(results)?;
    let scratch = results.join("scratch");
    std::fs::create_dir_all(&scratch)?;
    let scratch = scratch.canonicalize()?;
    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let mut md = format!("# S7 — bundle stability and classifier feasibility (#603)\n\nWritten by `spike s7`; the latest run (`unix_s` {unix}). protox 0.9.1, protoc (system), buf 1.73.0.\n\n## Determinism\n\n| Contract | File | protox twice | protoc bytes | buf bytes | Normalized |\n|---|---|---|---|---|---|\n");
    for r in determinism(examples, buf, &scratch)? {
        csv_row(&results.join("determinism.csv"), &["unix_s", "contract", "file", "protox_twice_equal", "protoc_bytes", "buf_bytes", "normalized"], &[unix.clone(), r.contract.clone(), r.file.clone(), r.protox_twice.to_string(), r.protoc_bytes.clone(), r.buf_bytes.clone(), r.semantic.clone()])?;
        md.push_str(&format!("| {} | {} | {} | {} | {} | {} |\n", r.contract, r.file, r.protox_twice, r.protoc_bytes, r.buf_bytes, r.semantic));
    }
    md.push_str("\n## Protobuf classifiers (WIRE_JSON)\n\n`buf` columns: `buf breaking new --against old` (forward) and the swapped order (reverse). `ours`: buf's rules re-implemented on prost-reflect, same two orders. `zk2`: the directional reader/writer semantics proposed for #618.\n\n| Case | Expected | buf fwd | buf rev | ours fwd | ours rev | agree | zk2 |\n|---|---|---|---|---|---|---|---|\n");
    for r in matrix(matrix_dir, buf)? {
        csv_row(&results.join("matrix.csv"), &["unix_s", "case", "expected", "buf_fwd", "buf_rev", "ours_fwd", "ours_rev", "agree", "zk2"], &[unix.clone(), r.case.clone(), r.expect.clone(), r.buf_fwd.clone(), r.buf_rev.clone(), r.ours_fwd.clone(), r.ours_rev.clone(), r.agree.to_string(), r.zk2.clone()])?;
        md.push_str(&format!("| {} | {} | {} | {} | {} | {} | {} | {} |\n", r.case, r.expect, r.buf_fwd, r.buf_rev, r.ours_fwd, r.ours_rev, r.agree, r.zk2));
        println!("{:24} buf {}/{} ours {}/{} agree {} | zk2 {}", r.case, r.buf_fwd, r.buf_rev, r.ours_fwd, r.ours_rev, r.agree, r.zk2);
    }
    md.push_str("\n## JSON Schema: the zk2 subset and its classifier\n\nSubset keywords: `type`, `properties`, `required`, `additionalProperties`, `items`, `enum`, `const`, numeric/length/item bounds, local `$ref`, `oneOf`, and annotations (`description`, `title`, `default`, `examples`, `format`, `deprecated`, `readOnly`, `writeOnly`, `$comment`). Rule: a reader accepts every instance its writer produces, with readers tolerating unknown properties and writers sending only what their schema declares (as protobuf); full compatibility needs both directions. `jsoncompat` judges literal containment: exit 1 means incompatible in that role.\n\n| Case | Expected | new reads old | old reads new | zk2 verdict | jsoncompat (old → new) |\n|---|---|---|---|---|---|\n");
    let jc = std::env::var_os("JSONCOMPAT").map(PathBuf::from);
    for r in json_matrix(&results.join("../../s7/json-matrix"), jc.as_deref())? {
        csv_row(&results.join("json-matrix.csv"), &["unix_s", "case", "expected", "new_reads_old", "old_reads_new", "verdict", "jsoncompat"], &[unix.clone(), r.case.clone(), r.expect.clone(), r.new_reads_old.clone(), r.old_reads_new.clone(), r.verdict.clone(), r.oracle.clone()])?;
        md.push_str(&format!("| {} | {} | {} | {} | {} | {} |\n", r.case, r.expect, r.new_reads_old, r.old_reads_new, r.verdict, r.oracle));
        println!("json {:30} {} | oracle {}", r.case, r.verdict, r.oracle);
    }
    let n = bundles(examples, &results.join("bundles"))?;
    md.push_str(&format!("\n## Bundles for the cross-language check\n\n{n} example bundles written to `bundles/`; `s7/verify_bundles.py` checks them with Python `rfc8785` (see `python.txt`).\n"));
    std::fs::write(results.join("summary.md"), md)?;
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

// ---- JSON Schema: the zk2 subset of 2020-12 and its classifier ----------

/// The keywords of the zk2 subset. Everything else is refused, because
/// containment (does every instance of W satisfy R?) is not decidable for it.
const JSON_SUBSET: &[&str] = &[
    "$schema", "$defs", "$ref", "$comment", "type", "properties", "required", "additionalProperties", "items", "enum",
    "const", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "minLength", "maxLength", "minItems",
    "maxItems", "oneOf", "description", "title", "default", "examples", "format", "deprecated", "readOnly", "writeOnly",
];

fn outside_subset(v: &serde_json::Value, out: &mut BTreeSet<String>) {
    let Some(o) = v.as_object() else { return };
    for (k, x) in o {
        if !JSON_SUBSET.contains(&k.as_str()) {
            out.insert(k.clone());
        }
        match k.as_str() {
            "properties" | "$defs" => {
                for s in x.as_object().into_iter().flat_map(|m| m.values()) {
                    outside_subset(s, out);
                }
            }
            "items" | "additionalProperties" => outside_subset(x, out),
            "oneOf" => {
                for s in x.as_array().into_iter().flatten() {
                    outside_subset(s, out);
                }
            }
            _ => {}
        }
    }
}

fn deref<'a>(doc: &'a serde_json::Value, v: &'a serde_json::Value) -> &'a serde_json::Value {
    let mut cur = v;
    for _ in 0..16 {
        match cur.get("$ref").and_then(serde_json::Value::as_str).and_then(|r| r.strip_prefix('#')) {
            Some(ptr) => cur = doc.pointer(ptr).unwrap_or(cur),
            None => break,
        }
    }
    cur
}

fn types(v: &serde_json::Value) -> Option<BTreeSet<String>> {
    match v.get("type") {
        Some(serde_json::Value::String(t)) => Some([t.clone()].into()),
        Some(serde_json::Value::Array(a)) => Some(a.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect()),
        _ if v.get("properties").is_some() => Some(["object".to_owned()].into()),
        _ => None,
    }
}

fn values(v: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    if let Some(c) = v.get("const") {
        return Some(vec![c.clone()]);
    }
    v.get("enum").and_then(serde_json::Value::as_array).cloned()
}

fn num(v: &serde_json::Value, k: &str) -> Option<f64> {
    v.get(k).and_then(serde_json::Value::as_f64)
}

/// Can a reader with schema `r` accept every instance a writer with schema
/// `w` produces? Sufficient conditions only (conservative): a finding means
/// "not shown compatible".
fn json_reads(rdoc: &serde_json::Value, r: &serde_json::Value, wdoc: &serde_json::Value, w: &serde_json::Value, at: &str, out: &mut BTreeSet<String>, depth: u32) {
    if depth > 32 {
        return;
    }
    let (r, w) = (deref(rdoc, r), deref(wdoc, w));
    if r == w {
        return;
    }
    // Values.
    match (values(r), values(w)) {
        (Some(rv), Some(wv)) => {
            if wv.iter().any(|x| !rv.contains(x)) {
                out.insert(format!("{at}: the writer may produce a value the reader's enum/const lacks"));
            }
        }
        (Some(_), None) => {
            out.insert(format!("{at}: the reader restricts values (enum/const) and the writer does not"));
        }
        _ => {}
    }
    // Types.
    if let Some(rt) = types(r) {
        match types(w) {
            None if values(w).is_none() => {
                out.insert(format!("{at}: the writer's type is unconstrained"));
            }
            None => {}
            Some(wt) => {
                for t in &wt {
                    let ok = rt.contains(t) || (t == "integer" && rt.contains("number"));
                    if !ok {
                        out.insert(format!("{at}: the writer may produce `{t}`, which the reader rejects"));
                    }
                }
            }
        }
    }
    // Bounds: the reader's must be at least as loose as the writer's.
    for (k, looser_is_lower) in [("minimum", true), ("exclusiveMinimum", true), ("minLength", true), ("minItems", true), ("maximum", false), ("exclusiveMaximum", false), ("maxLength", false), ("maxItems", false)] {
        if let Some(rb) = num(r, k) {
            let ok = match num(w, k) {
                Some(wb) => if looser_is_lower { rb <= wb } else { rb >= wb },
                None => false,
            };
            if !ok {
                out.insert(format!("{at}: the reader's `{k}` is tighter than the writer's"));
            }
        }
    }
    // Objects.
    let empty = serde_json::Map::new();
    let rp = r.get("properties").and_then(serde_json::Value::as_object).unwrap_or(&empty);
    let wp = w.get("properties").and_then(serde_json::Value::as_object).unwrap_or(&empty);
    let req = |v: &serde_json::Value| -> BTreeSet<String> { v.get("required").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect() };
    let (rreq, wreq) = (req(r), req(w));
    for p in rreq.difference(&wreq) {
        out.insert(format!("{at}.{p}: the reader requires it and the writer may omit it"));
    }
    let r_closed = r.get("additionalProperties") == Some(&serde_json::Value::Bool(false));
    let w_closed = w.get("additionalProperties") == Some(&serde_json::Value::Bool(false));
    for (p, ws) in wp {
        match rp.get(p) {
            Some(rs) => json_reads(rdoc, rs, wdoc, ws, &format!("{at}.{p}"), out, depth + 1),
            None => {
                if let Some(ap) = r.get("additionalProperties").filter(|x| x.is_object()) {
                    json_reads(rdoc, ap, wdoc, ws, &format!("{at}.{p}"), out, depth + 1);
                }
            }
        }
    }
    // zk2's rule (as protobuf's): readers tolerate unknown properties and
    // writers send only what their schema declares, so `additionalProperties:
    // false` constrains writers only, and an undeclared property is never
    // produced. Literal JSON Schema containment (jsoncompat) differs here.
    let _ = (r_closed, w_closed);
    // Arrays.
    if let (Some(ri), Some(wi)) = (r.get("items"), w.get("items")) {
        json_reads(rdoc, ri, wdoc, wi, &format!("{at}[]"), out, depth + 1);
    }
    // oneOf: every writer branch must be read by some reader branch.
    let branches = |v: &serde_json::Value| v.get("oneOf").and_then(serde_json::Value::as_array).cloned();
    match (branches(r), branches(w)) {
        (Some(rb), Some(wb)) => {
            for (i, b) in wb.iter().enumerate() {
                let fits = rb.iter().any(|x| {
                    let mut f = BTreeSet::new();
                    json_reads(rdoc, x, wdoc, b, at, &mut f, depth + 1);
                    f.is_empty()
                });
                if !fits {
                    out.insert(format!("{at}: the writer's oneOf branch {i} fits no reader branch"));
                }
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            out.insert(format!("{at}: oneOf on one side only"));
        }
        (None, None) => {}
    }
}

struct JsonRow {
    case: String,
    expect: String,
    new_reads_old: String,
    old_reads_new: String,
    verdict: String,
    oracle: String,
}

fn json_matrix(dir: &Path, oracle: Option<&Path>) -> Result<Vec<JsonRow>> {
    let mut cases: Vec<PathBuf> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    cases.sort();
    let mut rows = Vec::new();
    for c in cases {
        let old: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(c.join("old.json"))?)?;
        let new: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(c.join("new.json"))?)?;
        let (o, n) = (&old["$defs"]["Status"], &new["$defs"]["Status"]);
        let mut outside = BTreeSet::new();
        outside_subset(&old, &mut outside);
        outside_subset(&new, &mut outside);
        let (mut a, mut b) = (BTreeSet::new(), BTreeSet::new());
        json_reads(&new, n, &old, o, "Status", &mut a, 0);
        json_reads(&old, o, &new, n, "Status", &mut b, 0);
        let fmt = |s: &BTreeSet<String>| if s.is_empty() { "ok".to_owned() } else { s.iter().cloned().collect::<Vec<_>>().join("; ") };
        let verdict = if !outside.is_empty() {
            format!("refused: outside the zk2 subset ({})", outside.into_iter().collect::<Vec<_>>().join(", "))
        } else if a.is_empty() && b.is_empty() {
            "compatible both ways".into()
        } else {
            "breaking".into()
        };
        let oracle = match oracle {
            None => "not run".into(),
            Some(bin) => {
                let mut v = Vec::new();
                for role in ["serializer", "deserializer"] {
                    let o = Command::new(bin).arg("compat").arg(c.join("old.json")).arg(c.join("new.json")).args(["--role", role]).output();
                    v.push(match o {
                        Ok(o) => format!("{role}: exit {:?}", o.status.code()),
                        Err(e) => format!("{role}: {e}"),
                    });
                }
                v.join(", ")
            }
        };
        rows.push(JsonRow {
            case: c.file_name().unwrap().to_string_lossy().into_owned(),
            expect: std::fs::read_to_string(c.join("expect.txt")).unwrap_or_default().trim().to_owned(),
            new_reads_old: fmt(&a),
            old_reads_new: fmt(&b),
            verdict,
            oracle,
        });
    }
    Ok(rows)
}
