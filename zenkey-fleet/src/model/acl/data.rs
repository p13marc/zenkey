//! A router's configuration read as data (#1044): the blocks `check face`
//! and `acl gen --check` judge, read from the file's own tree in zenoh
//! 1.10.1's shapes, for a router whose zenoh is not stock (`link.v2` §2.4,
//! [`ConfigRead::Data`]).
//!
//! Stock zenoh's loader refuses the fork's link protocols
//! (`unixsock-dgram`), so it cannot read a `zm-1.10.2-r1` router's block,
//! and the fork's loader is not this crate's. Each block is read as zenoh
//! 1.10.1's own types state it (`zenoh-config-1.10.1/src/lib.rs`,
//! `gateway.rs`): no field it does not know, every field it requires, each
//! list it holds non-empty non-empty, each value from its vocabulary, each
//! key expression canonical, and each link protocol one of the binding's.
//! A block or field the file omits takes zenoh's default: access control
//! not enabled, under `deny`; no downsampling; no gateway. A stated
//! transport key is read as written, so a string or a fraction is
//! `transport_differs` (`link.v2` §2.19).
//!
//! Every key that does not read is named at its path, not only the first.
//! A block read in part is not judged, so the check is not asked.

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use zenkey_model::link::{self, Binding};
use zenoh::key_expr::OwnedKeyExpr;

use crate::report::{
    AclConfigDoc, AclFlow, AclMessage, AclPermission, AclPolicyDoc, AclRuleDoc, AclSubjectDoc,
    ConfigRead, DownsamplingDoc, DownsamplingRuleDoc,
};

use super::RouterConfig;

/// zenoh's `NEVec`: a list that refuses to be empty.
struct NonEmpty<T>(Vec<T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for NonEmpty<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let items = Vec::<T>::deserialize(d)?;
        if items.is_empty() {
            return Err(D::Error::invalid_length(0, &"a non-empty list"));
        }
        Ok(NonEmpty(items))
    }
}

fn deny() -> AclPermission {
    AclPermission::Deny
}

/// `AclConfig`: every field defaulted, as zenoh's loader fills them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Acl {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "deny")]
    default_permission: AclPermission,
    #[serde(default)]
    rules: Option<Value>,
    #[serde(default)]
    subjects: Option<Value>,
    #[serde(default)]
    policies: Option<Value>,
}

/// `AclConfigRule`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    id: String,
    key_exprs: NonEmpty<String>,
    messages: NonEmpty<AclMessage>,
    flows: Option<NonEmpty<AclFlow>>,
    permission: AclPermission,
}

/// `AclConfigSubjects`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Subject {
    id: String,
    interfaces: Option<NonEmpty<String>>,
    cert_common_names: Option<NonEmpty<String>>,
    usernames: Option<NonEmpty<String>>,
    link_protocols: Option<NonEmpty<String>>,
    zids: Option<NonEmpty<String>>,
}

/// `AclConfigPolicyEntry`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    id: Option<String>,
    rules: Vec<String>,
    subjects: Vec<String>,
}

/// `DataMessage`, the messages a downsampling item selects.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum DataMessage {
    Put,
    Delete,
    Query,
    Reply,
}

impl DataMessage {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Put => "put",
            Self::Delete => "delete",
            Self::Query => "query",
            Self::Reply => "reply",
        }
    }
}

/// `DownsamplingItemConf`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    id: Option<String>,
    interfaces: Option<NonEmpty<String>>,
    link_protocols: Option<NonEmpty<String>>,
    messages: NonEmpty<DataMessage>,
    rules: NonEmpty<ItemRule>,
    flows: Option<NonEmpty<AclFlow>>,
}

/// `DownsamplingRuleConf`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemRule {
    key_expr: String,
    freq: f64,
}

/// `GatewayConf`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Gateway {
    south: Option<South>,
}

/// `GatewaySouthConf`: the `auto` preset, or the subregions.
#[derive(Deserialize)]
#[serde(untagged)]
#[allow(dead_code)] // the preset is read for its shape
enum South {
    Preset(Preset),
    Custom(Vec<Value>),
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Preset {
    Auto,
}

/// `GatewayExplicitSouthConf`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SouthItem {
    filters: Option<Vec<Value>>,
}

/// `WhatAmI`, the modes a gateway filter selects.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)] // read for its shape
enum Mode {
    Router,
    Peer,
    Client,
}

/// `GatewayFiltersConf`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // read for its shape: the check reads modes and region names
struct Filter {
    modes: Option<NonEmpty<Mode>>,
    interfaces: Option<NonEmpty<String>>,
    zids: Option<NonEmpty<String>>,
    region_names: Option<NonEmpty<String>>,
    #[serde(default)]
    negated: bool,
}

/// What did not read, each at its path.
struct Unread(Vec<String>);

impl Unread {
    fn at(&mut self, path: &str, why: impl std::fmt::Display) {
        self.0.push(format!("{path}: {why}"));
    }

    /// `value` as `T`, or `None`, named.
    fn read<T: DeserializeOwned>(&mut self, path: &str, value: &Value) -> Option<T> {
        match serde_json::from_value(value.clone()) {
            Ok(t) => Some(t),
            Err(e) => {
                self.at(path, e);
                None
            }
        }
    }

    /// Each item of the list at `path` as `T`, an item that does not read
    /// named by its index and its `id`, if it states one. `null` is an empty
    /// list, as zenoh's `Option<Vec<_>>` reads it.
    fn items<T: DeserializeOwned>(&mut self, path: &str, value: Option<&Value>) -> Vec<T> {
        match value {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    let at = match item.get("id").and_then(Value::as_str) {
                        Some(id) => format!("{path}[{i}] (id {id:?})"),
                        None => format!("{path}[{i}]"),
                    };
                    self.read(&at, item)
                })
                .collect(),
            Some(other) => {
                self.at(path, format_args!("{}, not a list", kind(other)));
                Vec::new()
            }
        }
    }

    /// Each link protocol one of `binding`'s (`link.v2` §2.4).
    fn protocols(&mut self, path: &str, given: Option<&NonEmpty<String>>, binding: Binding) {
        for p in given.map_or(&[][..], |p| p.0.as_slice()) {
            if !binding.protocols().contains(&p.as_str()) {
                self.at(
                    path,
                    format_args!(
                        "{p:?} is not a link protocol of {binding} ({})",
                        binding.protocols().join(", ")
                    ),
                );
            }
        }
    }

    /// Each key expression canonical, as zenoh's `OwnedKeyExpr` reads one.
    fn key_exprs<'a>(&mut self, path: &str, given: impl IntoIterator<Item = &'a String>) {
        for k in given {
            if let Err(e) = OwnedKeyExpr::try_from(k.clone()) {
                self.at(path, format_args!("{k:?} is not a key expression: {e}"));
            }
        }
    }

    /// Each zid one zenoh reads.
    fn zids(&mut self, path: &str, given: Option<&NonEmpty<String>>) {
        for z in given.map_or(&[][..], |z| z.0.as_slice()) {
            if z.parse::<zenoh::session::ZenohId>().is_err() {
                self.at(path, format_args!("{z:?} is not a zid"));
            }
        }
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "a block",
    }
}

fn strings(v: Option<NonEmpty<String>>) -> Option<Vec<String>> {
    v.map(|v| v.0)
}

fn access_control(value: Option<&Value>, binding: Binding, unread: &mut Unread) -> AclConfigDoc {
    let empty = Value::Object(serde_json::Map::new());
    let Some(acl) = unread.read::<Acl>("access_control", value.unwrap_or(&empty)) else {
        return AclConfigDoc::default();
    };
    let rules = unread
        .items::<Rule>("access_control.rules", acl.rules.as_ref())
        .into_iter()
        .map(|r| {
            unread.key_exprs(
                &format!("access_control.rules (id {:?})", r.id),
                &r.key_exprs.0,
            );
            AclRuleDoc {
                id: r.id,
                key_exprs: r.key_exprs.0,
                messages: r.messages.0.iter().map(|m| m.as_str().to_owned()).collect(),
                flows: r
                    .flows
                    .map(|f| f.0.iter().map(|f| f.as_str().to_owned()).collect()),
                permission: r.permission.as_str().to_owned(),
            }
        })
        .collect();
    let subjects = unread
        .items::<Subject>("access_control.subjects", acl.subjects.as_ref())
        .into_iter()
        .map(|s| {
            let at = format!("access_control.subjects (id {:?})", s.id);
            unread.protocols(&at, s.link_protocols.as_ref(), binding);
            unread.zids(&at, s.zids.as_ref());
            AclSubjectDoc {
                id: s.id,
                cert_common_names: strings(s.cert_common_names),
                zids: strings(s.zids),
                interfaces: strings(s.interfaces),
                usernames: strings(s.usernames),
                link_protocols: strings(s.link_protocols),
            }
        })
        .collect();
    let policies = unread
        .items::<Policy>("access_control.policies", acl.policies.as_ref())
        .into_iter()
        .map(|p| AclPolicyDoc {
            id: p.id,
            rules: p.rules,
            subjects: p.subjects,
        })
        .collect();
    AclConfigDoc {
        enabled: acl.enabled,
        default_permission: acl.default_permission.as_str().to_owned(),
        rules,
        subjects,
        policies,
    }
}

fn downsampling(
    value: Option<&Value>,
    binding: Binding,
    unread: &mut Unread,
) -> Vec<DownsamplingDoc> {
    unread
        .items::<Item>("downsampling", value)
        .into_iter()
        .enumerate()
        .map(|(i, d)| {
            let at = match &d.id {
                Some(id) => format!("downsampling (id {id:?})"),
                None => format!("downsampling[{i}]"),
            };
            unread.protocols(&at, d.link_protocols.as_ref(), binding);
            unread.key_exprs(&at, d.rules.0.iter().map(|r| &r.key_expr));
            DownsamplingDoc {
                id: d.id,
                interfaces: strings(d.interfaces),
                link_protocols: strings(d.link_protocols),
                messages: d.messages.0.iter().map(|m| m.as_str().to_owned()).collect(),
                rules: d
                    .rules
                    .0
                    .into_iter()
                    .map(|r| DownsamplingRuleDoc {
                        key_expr: r.key_expr,
                        freq: r.freq,
                    })
                    .collect(),
                flows: d
                    .flows
                    .map(|f| f.0.iter().map(|f| f.as_str().to_owned()).collect()),
            }
        })
        .collect()
}

/// The gateway as written, once it reads as zenoh's `GatewayConf`.
fn gateway(value: Option<&Value>, unread: &mut Unread) -> Option<Value> {
    let value = value?;
    let gw = unread.read::<Gateway>("gateway", value)?;
    if let Some(South::Custom(items)) = &gw.south {
        for (i, item) in items.iter().enumerate() {
            let at = format!("gateway.south[{i}]");
            let Some(south) = unread.read::<SouthItem>(&at, item) else {
                continue;
            };
            for (j, filter) in south.filters.iter().flatten().enumerate() {
                let at = format!("{at}.filters[{j}]");
                if let Some(f) = unread.read::<Filter>(&at, filter) {
                    unread.zids(&at, f.zids.as_ref());
                }
            }
        }
    }
    Some(value.clone())
}

/// [`RouterConfig::read_data`].
pub(super) fn read(stated: &Value, binding: Binding) -> std::result::Result<RouterConfig, String> {
    let mut unread = Unread(Vec::new());
    if !stated.is_object() {
        return Err(format!("{}, not a configuration", kind(stated)));
    }
    let acl = access_control(stated.get("access_control"), binding, &mut unread);
    let downsampling = downsampling(stated.get("downsampling"), binding, &mut unread);
    let gateway = gateway(stated.get("gateway"), &mut unread);
    if !unread.0.is_empty() {
        return Err(format!(
            "read as data under {binding} (link.v2 §2.4), {} key(s) do not read as zenoh \
             1.10.1's: {}",
            unread.0.len(),
            unread.0.join("; ")
        ));
    }
    Ok(RouterConfig {
        acl,
        downsampling,
        gateway,
        transport: link::StatedTransport::read(stated),
        read: ConfigRead::Data,
    })
}
