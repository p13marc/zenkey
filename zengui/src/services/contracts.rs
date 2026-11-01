//! The contracts the window holds offline (#614, FL6a): `--contracts`, or
//! the Settings overlay's list, loaded as zenctl and zenwatch load them
//! (spec §9.7, §8.5) — off the update thread, because a `.history` root is
//! a directory walk and a fingerprint per bundle (#255).
//!
//! What a load cannot do is fail as a whole. A path that is not there, or
//! loads nothing, is a problem *by where it was named*; a file in a named
//! directory that is not a contract (a bindings file beside them) is a
//! problem too; and everything that did load is held. The status strip says
//! both, so a typo in one path never hides the contracts the others gave.

use std::path::PathBuf;
use std::sync::Arc;

use iced::Task;
use zenkey_fleet::{Catalogs, ContractSet, LoadProblem};

use crate::message::{BusMsg, Message};

/// What one load gave: the revisions held, and what it could not read.
#[derive(Debug, Clone, Default)]
pub struct LoadedContracts {
    /// The paths asked for, in order — what the strip and the Settings
    /// overlay show the load was *of*.
    pub paths: Vec<PathBuf>,
    pub set: Arc<ContractSet>,
    /// One per path that is not there or loaded nothing, one per file
    /// that did not read as a contract, and one per catalog refused.
    pub problems: Vec<LoadProblem>,
    /// The deployment catalogs asked for (core §8.6, `--catalog`), and
    /// those that read.
    pub catalog_paths: Vec<PathBuf>,
    pub catalogs: Arc<Catalogs>,
}

impl LoadedContracts {
    /// Whether `other` holds the same catalogs, by id.
    pub fn same_catalogs(&self, other: &LoadedContracts) -> bool {
        let ids = |c: &Catalogs| c.held().iter().map(|c| c.id()).collect::<Vec<_>>();
        ids(&self.catalogs) == ids(&other.catalogs)
    }
}

/// Load `paths`, every one, in order: [`ContractSet::load_path`] each, a
/// missing or empty one a problem by its own name. Pure apart from the
/// reads, and the one place zengui reads contracts from disk.
pub fn load_all(paths: &[PathBuf]) -> LoadedContracts {
    load_with(paths, &[])
}

/// [`load_all`], and the deployment catalogs `catalog_paths` name (core
/// §8.6), each read as a reader reads it.
pub fn load_with(paths: &[PathBuf], catalog_paths: &[PathBuf]) -> LoadedContracts {
    let mut set = ContractSet::new();
    let mut problems = Vec::new();
    for path in paths {
        if !path.exists() {
            problems.push(LoadProblem {
                at: path.display().to_string(),
                message: "no such file or directory".to_string(),
            });
            continue;
        }
        let (loaded, skipped) = ContractSet::load_path(path);
        if loaded.is_empty() {
            problems.push(LoadProblem {
                at: path.display().to_string(),
                message: if skipped.is_empty() {
                    "no contract loaded from it (no authoring file, and not a .history \
                     root with bundles)"
                        .to_string()
                } else {
                    "no contract loaded from it".to_string()
                },
            });
        }
        problems.extend(skipped);
        set.extend(loaded);
    }
    // Core §8.6: each catalog read as a reader reads it, a refusal a
    // problem by its own name, never a reason to drop the others.
    let mut held = Vec::new();
    for path in catalog_paths {
        match Catalogs::load(path) {
            Ok(c) => held.push(c),
            Err(message) => problems.push(LoadProblem {
                at: path.display().to_string(),
                message,
            }),
        }
    }
    LoadedContracts {
        paths: paths.to_vec(),
        set: Arc::new(set),
        problems,
        catalog_paths: catalog_paths.to_vec(),
        catalogs: Arc::new(Catalogs::new(held)),
    }
}

/// Load the contracts off the update thread; lands on
/// [`BusMsg::ContractsLoaded`]. An empty list loads nothing and still
/// lands, so "contracts: from the bus only" is a state the strip can say
/// rather than an absence.
pub fn load(paths: Vec<PathBuf>, catalogs: Vec<PathBuf>) -> Task<Message> {
    Task::perform(
        async move { Arc::new(load_with(&paths, &catalogs)) },
        |loaded| Message::Bus(BusMsg::ContractsLoaded(loaded)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn examples(dir: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/zk2")
            .join(dir)
    }

    /// The walkthrough and tcgui contracts load, a missing path is a problem
    /// by its own name, and the one does not hide the other.
    #[test]
    fn a_missing_path_is_a_problem_and_never_hides_what_did_load() {
        let loaded = load_all(&[
            examples("walkthrough"),
            PathBuf::from("/definitely/not/here"),
            examples("tcgui"),
        ]);
        assert!(loaded.set.len() >= 2, "both directories loaded");
        let missing: Vec<_> = loaded
            .problems
            .iter()
            .filter(|p| p.at.contains("/definitely/not/here"))
            .collect();
        assert_eq!(missing.len(), 1, "{:?}", loaded.problems);
        // tcgui keeps a bindings file beside its contracts: said, skipped,
        // and never a reason to drop the contracts beside it.
        assert!(
            loaded
                .problems
                .iter()
                .all(|p| p.at.contains("/definitely/not/here") || p.at.contains("bindings")),
            "{:?}",
            loaded.problems
        );
        assert_eq!(loaded.paths.len(), 3);
    }

    /// Nothing asked, nothing loaded — and that is a state, not an error.
    #[test]
    fn no_paths_is_an_empty_set_without_problems() {
        let loaded = load_all(&[]);
        assert!(loaded.set.is_empty());
        assert!(loaded.problems.is_empty());
    }
}
