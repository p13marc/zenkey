//! `WireEncoding` — how a payload is framed on the wire.
//!
//! A separate axis from *what* the payload means: one `json-schema` document
//! describes both the JSON and the CBOR framing of a type (RFC 08 §7), and a
//! registry entry declares the framing it publishes (RFC 08 §2).
//!
//! It lives in a module of its own, ungated, because both of those callers
//! need it and only one of them is optional: [`crate::schema`] is behind the
//! `schema` feature, [`crate::slice`] is not, and a registry slice carries an
//! `encoding` column whatever this build was compiled to decode.
//! [`crate::schema`] re-exports it, so the older path still resolves.

/// The wire framing of a payload — a separate axis from its schema
/// (RFC 08 §7): one `json-schema` document describes both the JSON and the
/// CBOR framing of a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireEncoding {
    Json,
    Cbor,
    Protobuf,
    /// OMG CDR, the DDS / ROS 2 framing (v1.10).
    Cdr,
    /// Anything else — carried verbatim, decoded only by sniff.
    Other(String),
}

impl WireEncoding {
    /// Map a middleware/registry encoding string (`application/cbor`, …).
    ///
    /// The alias sets are deliberately short. A spelling is listed once it has
    /// been *seen*, not once it has been imagined: mapping a guessed media
    /// type to a codec is how a tool ends up confidently decoding the wrong
    /// bytes, and `Other` already renders honestly.
    pub fn from_encoding_str(s: &str) -> WireEncoding {
        match s {
            "application/json" | "text/json" => WireEncoding::Json,
            "application/cbor" => WireEncoding::Cbor,
            "application/protobuf" | "application/x-protobuf" => WireEncoding::Protobuf,
            "application/cdr" | "application/x-cdr" => WireEncoding::Cdr,
            other => WireEncoding::Other(other.to_string()),
        }
    }

    /// The canonical media type for this encoding — the inverse of
    /// [`from_encoding_str`](WireEncoding::from_encoding_str) for everything
    /// this build names, and the carried token for everything it does not.
    ///
    /// Not a byte-exact inverse, deliberately: the alias sets fold several
    /// spellings onto one variant (`text/json` reads as [`Json`](Self::Json)
    /// and writes as `application/json`), so a round trip is identity on the
    /// *encoding*, not on the string. `Other` is byte-exact, because there is
    /// no canonical spelling to prefer.
    pub fn as_encoding_str(&self) -> &str {
        match self {
            WireEncoding::Json => "application/json",
            WireEncoding::Cbor => "application/cbor",
            WireEncoding::Protobuf => "application/protobuf",
            WireEncoding::Cdr => "application/cdr",
            WireEncoding::Other(s) => s,
        }
    }
}

impl std::fmt::Display for WireEncoding {
    /// The canonical media type — [`as_encoding_str`](WireEncoding::as_encoding_str).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_encoding_str())
    }
}
