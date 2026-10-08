//! A contract crate (#611; zenoh-modem's `modem-contract` pattern,
//! `examples/zk2/zenoh-modem/contract-crate.md`): the walkthrough's
//! interfaces, their types, embedded bundles and `Handlers`/`Api` traits,
//! with no zenoh dependency by default. An out-of-tree implementation
//! depends on it as it is; a daemon serving it enables `zenoh` and gets the
//! generated `Server`, `Consumer`, `Client` and `Fleet`.

include!(concat!(env!("OUT_DIR"), "/zk2.rs"));
