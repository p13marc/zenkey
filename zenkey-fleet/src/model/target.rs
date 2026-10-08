//! What a resolved verb aims at, settled without a session: an address or
//! a pattern, one resource of one revision, the template values given — and
//! for a call, whether it fans out and the request's bytes (spec §3.2,
//! §5.1, §7.2).
//!
//! Everything a tool can refuse about its input is refused here, before
//! anything is sent: an address that is not `<system>/<service>`, a
//! resource the revision does not declare, a parameter its template does
//! not have, a fan-out to an operation that forbids one (O2), a request
//! that does not encode as the operation's type. Each is
//! [`Error::Unaskable`](crate::Error::Unaskable): the caller's input to fix.
//!
//! Session-free, so `zenctl call --contracts …` refuses all of them with no
//! bus at all.

use std::collections::BTreeSet;

use prost_reflect::{DescriptorPool, DynamicMessage};
use zenkey_model::authoring::Kind;
use zenkey_model::contract::{Body, Fanout, Operation, Resource};
use zenkey_model::grammar::Addr;
use zenkey_model::schema::{ArtifactData, TypeId};
use zenkey_model::template::Bindings;

use crate::model::catalog::Revision;
use crate::{Error, Result};

/// An address as a resolved verb takes it: `<system>/<service>`, either
/// position `*` (R1's binding syntax).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// As given.
    pub address: String,
    /// The one service it names, when neither position is `*`.
    pub concrete: Option<Addr>,
}

impl Target {
    /// Parses `<system>/<service>`, either position `*`.
    pub fn parse(address: &str) -> Result<Target> {
        zk2::consumer::Provider::parse(address)
            .map_err(|e| Error::unaskable(address, strip_contract(&e.to_string())))?;
        let concrete = if address.contains('*') {
            None
        } else {
            Some(
                address
                    .parse::<Addr>()
                    .map_err(|e| Error::unaskable_from(address, e))?,
            )
        };
        Ok(Target {
            address: address.to_owned(),
            concrete,
        })
    }

    /// The one service, or the refusal a verb of one address gives a
    /// pattern.
    pub fn one(&self, verb: &str) -> Result<&Addr> {
        self.concrete.as_ref().ok_or_else(|| {
            Error::unaskable(
                &self.address,
                format!("{verb} reads one address: name the service, without `*`"),
            )
        })
    }

    /// Whether `addr` is one of the services this target names.
    pub fn matches(&self, addr: &Addr) -> bool {
        zk2::consumer::Provider::parse(&self.address).is_ok_and(|p| p.matches(addr))
    }
}

/// The runtime's contract errors carry a `contract: ` prefix that says
/// nothing to a person who typed an address.
fn strip_contract(s: &str) -> String {
    s.strip_prefix("contract: ").unwrap_or(s).to_owned()
}

/// The resource `want` names among `kinds`, refused in the revision's own
/// words when it declares none ([`Revision::resource`]).
pub fn resource<'r>(revision: &'r Revision, want: &str, kinds: &[Kind]) -> Result<&'r Resource> {
    revision
        .resource(want, kinds)
        .map_err(|e| Error::unaskable(want, e))
}

/// Checks the template values given against `r`'s template: every name a
/// parameter of it, one value each except a rest parameter's chunks.
/// Returns the parameters left unbound, in template order: each is a
/// wildcard.
pub fn check_values(r: &Resource, values: &Bindings) -> Result<Vec<String>> {
    let params: Vec<(&str, bool)> = r.template.params().collect();
    let known: BTreeSet<&str> = params.iter().map(|(n, _)| *n).collect();
    for (name, vs) in values {
        let Some((_, rest)) = params.iter().find(|(n, _)| n == name) else {
            return Err(Error::unaskable(
                format!("--param {name}"),
                if known.is_empty() {
                    format!("{} has no template parameters", r.template)
                } else {
                    format!(
                        "not a parameter of {}; its parameters: {}",
                        r.template,
                        known.iter().copied().collect::<Vec<_>>().join(", ")
                    )
                },
            ));
        };
        if vs.is_empty() || (vs.len() > 1 && !rest) {
            return Err(Error::unaskable(
                format!("--param {name}"),
                format!(
                    "takes one value ({} values given); only a rest parameter (`{{{name}...}}`) \
                     takes one per chunk",
                    vs.len()
                ),
            ));
        }
    }
    Ok(params
        .iter()
        .filter(|(n, _)| !values.contains_key(*n))
        .map(|(n, _)| (*n).to_owned())
        .collect())
}

/// A key a zk2 service owns (P3, spec §6): one under
/// `zk2/<system>/<service>/`, its data and its `@zk` control keys alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedKey {
    /// The service that owns it, and alone writes it.
    pub owner: Addr,
    /// The key from its `zk2` chunk on.
    pub relative: String,
    /// Whatever came before that chunk, as the wire carries it. Not
    /// attributed to a namespace: a namespace may hold a `zk2` chunk of its
    /// own (the tooling guide's O3).
    pub prefix: String,
}

/// Whether a wire key, as an un-namespaced tool sees it, is a key a zk2
/// service owns: from some `zk2` chunk on, it parses as one of the key forms
/// under an address (§1.1). A contract bundle's location-free key is no
/// service's.
///
/// Detection, not attribution: a refusal needs to know that an owner
/// exists, never which deployment it runs in, so every `zk2` chunk is tried
/// and the prefix is reported as found.
pub fn owned_key(key: &str) -> Option<OwnedKey> {
    use zenkey_model::grammar::{GRAMMAR, ZkKey, parse};
    let chunks: Vec<&str> = key.split('/').collect();
    chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == GRAMMAR)
        .find_map(|(i, _)| {
            let relative = chunks[i..].join("/");
            let owner = match parse(&relative).ok()? {
                ZkKey::Data { addr, .. }
                | ZkKey::Instance { addr, .. }
                | ZkKey::Alive { addr, .. }
                | ZkKey::Member { addr, .. } => addr,
                ZkKey::Contract { .. } => return None,
            };
            Some(OwnedKey {
                owner,
                relative,
                prefix: chunks[..i].join("/"),
            })
        })
}

/// One call, planned (spec §5.1).
#[derive(Debug, Clone)]
pub struct CallPlan {
    pub target: Target,
    /// The operation's resource.
    pub resource: Resource,
    pub operation: Operation,
    /// `@op/<template>`.
    pub name: String,
    /// The template values given, unslugged.
    pub values: Bindings,
    /// Parameters not given: wildcards.
    pub unbound: Vec<String>,
}

impl CallPlan {
    /// Whether the call's key expression has a wildcard: a `*` position in
    /// the address or an unbound parameter (O2).
    pub fn is_fanout(&self) -> bool {
        self.target.concrete.is_none() || !self.unbound.is_empty()
    }
}

/// Plans a call of `operation` at `target` with `values`, refusing a fan-out
/// to an operation that does not declare `fanout = "allowed"` before
/// anything is sent (O2).
pub fn plan_call(
    revision: &Revision,
    target: Target,
    operation: &str,
    values: Bindings,
) -> Result<CallPlan> {
    let r = resource(revision, operation, &[Kind::Operation])?;
    let Body::Operation(op) = &r.body else {
        return Err(Error::unaskable(operation, "not an operation"));
    };
    let unbound = check_values(r, &values)?;
    let plan = CallPlan {
        target,
        resource: r.clone(),
        operation: op.clone(),
        name: zk2::implementation::resource_name(r),
        values,
        unbound,
    };
    if plan.is_fanout() && op.fanout != Fanout::Allowed {
        let why = match (&plan.target.concrete, plan.unbound.as_slice()) {
            (None, _) => format!("the address {} has a `*`", plan.target.address),
            (Some(_), names) => format!(
                "{} not given ({})",
                if names.len() == 1 {
                    "a parameter is"
                } else {
                    "parameters are"
                },
                names
                    .iter()
                    .map(|n| format!("--param {n}=…"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        return Err(Error::unaskable(
            format!("{} {}", revision.iface(), plan.name),
            format!(
                "declares fanout = \"forbidden\", and this call is a fan-out: {why}. \
                 Call one address with every parameter given (spec §5.1 O2); nothing was sent"
            ),
        ));
    }
    Ok(plan)
}

/// The request's bytes, encoded as the operation's `request` type (§7.2):
/// JSON or CBOR for a JSON Schema type, as the contract's `encoding` says;
/// a protobuf message built from its JSON form through the bundle's
/// descriptor set; a raw type's bytes as given. `input` is what the caller
/// typed: JSON text, except for a raw type.
///
/// The value is not checked against its JSON Schema: the owner refuses a
/// request that does not decode as `invalid_request` (§5.1).
pub fn encode_request(revision: &Revision, plan: &CallPlan, input: &[u8]) -> Result<Vec<u8>> {
    let what = format!("the request for {}", plan.name);
    let ty = &plan.operation.request;
    let json = || -> Result<serde_json::Value> {
        serde_json::from_slice(input).map_err(|e| {
            Error::unaskable(
                &what,
                format!(
                    "is not JSON ({e}); a {} request is given as JSON",
                    kind_word(ty)
                ),
            )
        })
    };
    match ty {
        TypeId::Raw { .. } => Ok(input.to_vec()),
        TypeId::JsonSchema { .. } => {
            use zk2::codec::Codec as _;
            zk2::codec::Json::<serde_json::Value>::encode(&json()?, plan.operation.encoding)
                .map_err(|e| Error::unaskable(&what, e))
        }
        TypeId::Protobuf { name, schema } => {
            let value = json()?;
            let Some(ArtifactData::Protobuf(set)) =
                revision.contract().artifacts.get(schema).map(|a| &a.data)
            else {
                return Err(Error::malformed(
                    format!("{}@{}", revision.iface(), revision.fingerprint()),
                    format!("declares {name} in artifact {schema}, which it does not carry"),
                ));
            };
            let pool = DescriptorPool::decode(set.as_slice()).map_err(|e| {
                Error::malformed_with(
                    format!("artifact {schema}"),
                    "is not a descriptor set",
                    e.to_string(),
                )
            })?;
            let desc = pool.get_message_by_name(name).ok_or_else(|| {
                Error::malformed(
                    format!("artifact {schema}"),
                    format!("does not define {name}"),
                )
            })?;
            let msg = DynamicMessage::deserialize(desc, &value).map_err(|e| {
                Error::unaskable(&what, format!("is not a {name} in its JSON form: {e}"))
            })?;
            Ok(zk2::prost::Message::encode_to_vec(&msg))
        }
    }
}

fn kind_word(ty: &TypeId) -> &'static str {
    match ty {
        TypeId::Protobuf { .. } => "protobuf",
        TypeId::JsonSchema { .. } => "JSON Schema",
        TypeId::Raw { .. } => "raw",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::ContractSource;

    fn revision() -> Revision {
        let dir = std::env::temp_dir().join(format!("zenkey-fleet-target-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("m.proto"),
            "syntax = \"proto3\";\npackage m.v1;\nmessage Go { double x = 1; string frame = 2; }\n",
        )
        .expect("write");
        std::fs::write(
            dir.join("s.json"),
            r#"{"$defs": {"Req": {"type": "object"}, "Res": {"type": "object"}}}"#,
        )
        .expect("write");
        let toml = "[interface]\nname = \"m\"\nmajor = 1\nminor = 0\n\
             [schemas]\nprotobuf = [\"m.proto\"]\njsonschema = [\"s.json\"]\n\
             [resources.\"ports/{port}/set\"]\nkind = \"operation\"\n\
             request = \"json:Req\"\nresponse = \"json:Res\"\nencoding = \"cbor\"\n\
             params = { port = \"string\" }\ncardinality = 8\n\
             [resources.diagnostics]\nkind = \"operation\"\nrequest = \"json:Req\"\n\
             response = \"json:Res\"\nidempotent = true\nfanout = \"allowed\"\n\
             [resources.go]\nkind = \"operation\"\nrequest = \"m.v1.Go\"\nresponse = \"m.v1.Go\"\n\
             [resources.blob]\nkind = \"operation\"\nrequest = { raw = \"image/png\" }\n\
             response = { raw = \"image/png\" }\n";
        let l = zenkey_model::contract::load_str(toml, &dir, None);
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        Revision::from_contract(c, ContractSource::File)
    }

    fn one(name: &str, value: &str) -> Bindings {
        [(name.to_owned(), vec![value.to_owned()])].into()
    }

    /// P3: every key under an address is its owner's, under any prefix;
    /// a contract bundle's key and a foreign key are no service's.
    #[test]
    fn an_owned_key_is_found_under_any_prefix() {
        let k = owned_key("zk2/host-a/tc/tc.netif.v1/state/namespaces").expect("owned");
        assert_eq!(k.owner.to_string(), "host-a/tc");
        assert_eq!(k.prefix, "");
        let k = owned_key("site/zk2/x/zk2/host-a/tc/tc.netif.v1/@op/diagnostics").expect("owned");
        assert_eq!(k.prefix, "site/zk2/x");
        assert_eq!(k.relative, "zk2/host-a/tc/tc.netif.v1/@op/diagnostics");
        assert!(owned_key("zk2/host-a/tc/@zk/instance/0123456789abcdef").is_some());
        assert_eq!(
            owned_key(&format!("zk2/@zk/contract/tc.netif.v1/{}", "a".repeat(64))),
            None
        );
        assert_eq!(owned_key("v1/h-3fa9c2d41b7e/state/sysinfo/health"), None);
        assert_eq!(owned_key("demo/zk2"), None);
    }

    /// O2 before anything is sent: a `*` or a missing parameter makes a
    /// fan-out, and only `fanout = "allowed"` takes one.
    #[test]
    fn a_fan_out_is_refused_unless_the_operation_allows_it() {
        let rev = revision();
        let t = |a: &str| Target::parse(a).expect("an address");
        let plan = plan_call(&rev, t("h1/tc"), "ports/{port}/set", one("port", "p1")).unwrap();
        assert!(!plan.is_fanout());
        assert_eq!(plan.name, "@op/ports/{port}/set");

        let e = plan_call(&rev, t("*/tc"), "ports/{port}/set", one("port", "p1")).unwrap_err();
        assert!(e.is_unaskable(), "{e}");
        assert!(e.to_string().contains("has a `*`"), "{e}");
        let e = plan_call(&rev, t("h1/tc"), "@op/ports/{port}/set", Bindings::new()).unwrap_err();
        assert!(e.to_string().contains("--param port=…"), "{e}");
        assert!(e.to_string().contains("nothing was sent"), "{e}");

        let plan = plan_call(&rev, t("*/tc"), "diagnostics", Bindings::new()).unwrap();
        assert!(plan.is_fanout());

        let e = plan_call(&rev, t("h1/tc"), "diagnostics", one("port", "p1")).unwrap_err();
        assert!(e.to_string().contains("no template parameters"), "{e}");
        let e = plan_call(&rev, t("h1/tc"), "nope", Bindings::new()).unwrap_err();
        assert!(
            e.to_string().contains("declares no operation resource"),
            "{e}"
        );
        assert!(Target::parse("h1").is_err());
        assert!(Target::parse("H1/tc").is_err());
    }

    /// Each type kind encodes as §7.2 says: CBOR when the contract says so,
    /// protobuf from its JSON form, raw bytes as given; bad JSON is refused.
    #[test]
    fn requests_encode_as_the_contract_types() {
        let rev = revision();
        let t = Target::parse("h1/tc").unwrap();
        let set = plan_call(&rev, t.clone(), "ports/{port}/set", one("port", "p1")).unwrap();
        let cbor = encode_request(&rev, &set, br#"{"up": true}"#).unwrap();
        let back: serde_json::Value = ciborium::from_reader(cbor.as_slice()).unwrap();
        assert_eq!(back, serde_json::json!({"up": true}));
        let e = encode_request(&rev, &set, b"{not json").unwrap_err();
        assert!(e.is_unaskable());
        assert!(e.to_string().contains("is not JSON"), "{e}");

        let go = plan_call(&rev, t.clone(), "go", Bindings::new()).unwrap();
        let bytes = encode_request(&rev, &go, br#"{"x": 1.5, "frame": "map"}"#).unwrap();
        let mut want = vec![0x09];
        want.extend_from_slice(&1.5f64.to_le_bytes());
        want.extend_from_slice(&[0x12, 3, b'm', b'a', b'p']);
        assert_eq!(bytes, want);
        let e = encode_request(&rev, &go, br#"{"nope": 1}"#).unwrap_err();
        assert!(e.to_string().contains("is not a m.v1.Go"), "{e}");

        let blob = plan_call(&rev, t, "blob", Bindings::new()).unwrap();
        assert_eq!(
            encode_request(&rev, &blob, &[0x89, b'P']).unwrap(),
            [0x89, b'P']
        );
    }
}
