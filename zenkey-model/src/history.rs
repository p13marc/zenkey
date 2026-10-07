//! Published revisions: `contracts/.history/<iface>.v<N>/<hex>.bundle.json`
//! (r3 §3.11).
//!
//! The history is committed and append-only. Every revision of a major is
//! kept, because compatibility is FULL_TRANSITIVE: a new revision is checked
//! against every earlier one (#618). [`check`] verifies what is there: the
//! layout, each bundle's integrity, its fingerprint against its file name,
//! and its interface against its directory. That a file was never removed
//! or rewritten is the VCS's to show (CI diffs the directory against the
//! base branch).

use std::path::{Path, PathBuf};

use crate::bundle::Bundle;
use crate::canonical::Fingerprint;
use crate::contract::Contract;
use crate::grammar::IfaceId;

/// Where a revision's bundle lives under `root` (the `.history` directory).
#[must_use]
pub fn path_for(root: &Path, iface: &IfaceId, fp: &Fingerprint) -> PathBuf {
    root.join(iface.to_string())
        .join(format!("{}.bundle.json", fp.hex()))
}

/// Writes the contract's bundle unless that revision is already there.
/// Returns the path and whether it was written.
pub fn append(root: &Path, c: &Contract) -> std::io::Result<(PathBuf, bool)> {
    let b = Bundle::build(c);
    let path = path_for(root, &c.iface, &b.fingerprint());
    if path.exists() {
        return Ok((path, false));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, b.to_bytes())?;
    Ok((path, true))
}

/// Verifies a history directory. Returns one message per problem.
#[must_use]
pub fn check(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(root) else {
        return out;
    };
    let mut dirs: Vec<_> = dirs.flatten().collect();
    dirs.sort_by_key(std::fs::DirEntry::file_name);
    for d in dirs {
        let name = d.file_name().to_string_lossy().into_owned();
        let Ok(iface) = name.parse::<IfaceId>() else {
            out.push(format!(
                "{name}: not an interface directory <name>.v<major>"
            ));
            continue;
        };
        let Ok(files) = std::fs::read_dir(d.path()) else {
            out.push(format!("{name}: not a directory"));
            continue;
        };
        let mut files: Vec<_> = files.flatten().collect();
        files.sort_by_key(std::fs::DirEntry::file_name);
        for f in files {
            let fname = f.file_name().to_string_lossy().into_owned();
            let at = format!("{name}/{fname}");
            let Some(fp) = fname
                .strip_suffix(".bundle.json")
                .and_then(|h| Fingerprint::parse(&format!("sha256:{h}")).ok())
            else {
                out.push(format!("{at}: not <64 lowercase hex>.bundle.json"));
                continue;
            };
            let bytes = match std::fs::read(f.path()) {
                Ok(b) => b,
                Err(e) => {
                    out.push(format!("{at}: {e}"));
                    continue;
                }
            };
            match Bundle::verify_expecting(&bytes, &fp) {
                Err(e) => out.push(format!("{at}: {e}")),
                Ok(b) => {
                    if b.contract.get("interface").and_then(|v| v.as_str())
                        != Some(&iface.to_string())
                    {
                        out.push(format!("{at}: the bundle's interface is not {iface}"));
                    }
                    if b.to_bytes() != bytes {
                        out.push(format!("{at}: not in JCS form"));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::load_str;

    #[test]
    fn append_is_idempotent_and_checked() {
        let tmp = std::env::temp_dir().join(format!("zk2-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let src = "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\n\
                   [resources.\"a\"]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n";
        let c = load_str(src, Path::new("."), None).contract.unwrap();
        let (p, wrote) = append(&tmp, &c).unwrap();
        assert!(wrote);
        assert!(!append(&tmp, &c).unwrap().1);
        assert!(check(&tmp).is_empty(), "{:?}", check(&tmp));

        let moved = p.with_file_name(format!("{}.bundle.json", "0".repeat(64)));
        std::fs::rename(&p, &moved).unwrap();
        assert_eq!(check(&tmp).len(), 1);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
