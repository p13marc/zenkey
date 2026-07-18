//! The zenkey-build fixture consumer: compiles the corpus registry exactly the
//! way a real application does (build.rs → include!).

pub mod registry {
    include!(concat!(env!("OUT_DIR"), "/zenkey_registry.rs"));
}
