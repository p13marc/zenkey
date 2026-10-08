//! The generated module, per interface (`docs/zk2/codegen.md`).
//!
//! Written as text: every path is absolute (`::zenkey::…`, `::std::…`) or
//! relative to the interface module (`super::__proto`, `super::__json`), so
//! the file can be `include!`d into any module of the consumer.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use zenkey_model::authoring::{Kind, ParamType};
use zenkey_model::contract::{Body, Contract, Fanout, Replies, Resource};
use zenkey_model::schema::TypeId;

use crate::Error;
use crate::json::JsonTypes;
use crate::names::{ident, module, resource_names, shouty};
use crate::proto::Protos;

pub struct Iface<'a> {
    pub contract: &'a Contract,
    pub source: PathBuf,
    pub fingerprint: String,
    pub bundle_path: PathBuf,
}

pub struct Unit<'a> {
    pub ifaces: &'a [Iface<'a>],
    pub protos: &'a Protos,
    pub proto_includes: Option<PathBuf>,
    pub jsons: &'a JsonTypes,
    pub json_files: &'a [(String, PathBuf)],
    pub contract_crate: bool,
}

const ALLOW: &str = "#[allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code, missing_docs, unused_imports, unused_qualifications, unreachable_pub)]";

/// A Rust string literal.
fn lit(s: &str) -> String {
    format!("{s:?}")
}

fn path_lit(p: &Path) -> String {
    lit(&p.display().to_string())
}

/// A `#[doc]` attribute from contract text: markup characters are escaped,
/// so a contract's `<host>` or `[x]` is text, not HTML or a link.
fn doc(indent: &str, text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('[', "\\[")
        .replace(']', "\\]");
    format!("{indent}#[doc = {}]\n", lit(&escaped))
}

/// A Rust type and its codec.
struct Ty {
    rust: String,
    codec: String,
}

/// One template parameter, as a Rust argument.
struct Param {
    name: String,
    ident: String,
    ty: ParamType,
}

/// Argument names the generated methods use themselves.
const RESERVED_ARGS: &[&str] = &[
    "at",
    "request",
    "call",
    "out",
    "timeout",
    "svc",
    "callback",
    "archive",
    "provider",
    "retention",
    "values",
    "handlers",
    "b",
    "session",
    "role",
    "service",
    "inner",
    "v",
];

fn params_of(r: &Resource) -> Vec<Param> {
    r.template
        .params()
        .map(|(n, _)| {
            let mut id = ident(n);
            if RESERVED_ARGS.contains(&id.as_str()) {
                id.push('_');
            }
            Param {
                name: n.to_owned(),
                ident: id,
                ty: r.params.get(n).copied().unwrap_or(ParamType::String),
            }
        })
        .collect()
}

fn arg_ty(t: ParamType, optional: bool) -> &'static str {
    match (t, optional) {
        (ParamType::String, false) => "&str",
        (ParamType::Uint, false) => "u64",
        (ParamType::Path, false) => "&[&str]",
        (ParamType::String, true) => "::std::option::Option<&str>",
        (ParamType::Uint, true) => "::std::option::Option<u64>",
        (ParamType::Path, true) => "::std::option::Option<&[&str]>",
    }
}

/// `, ns: &str, iface: &str`
fn args(ps: &[Param], optional: bool) -> String {
    ps.iter()
        .map(|p| format!(", {}: {}", p.ident, arg_ty(p.ty, optional)))
        .collect()
}

fn value_expr(p: &Param, var: &str) -> String {
    match p.ty {
        ParamType::String => format!("::std::vec![{var}.to_owned()]"),
        ParamType::Uint => format!("::std::vec![{var}.to_string()]"),
        ParamType::Path => format!("{var}.iter().map(|s| (*s).to_owned()).collect()"),
    }
}

/// A block evaluating to the template's `Bindings`.
fn bindings(ps: &[Param], optional: bool) -> String {
    if ps.is_empty() {
        return "::zenkey::model::template::Bindings::new()".to_owned();
    }
    let mut s = String::from("{ let mut v = ::zenkey::model::template::Bindings::new(); ");
    for p in ps {
        if optional {
            let _ = write!(
                s,
                "if let ::std::option::Option::Some(x) = {} {{ v.insert({}.to_owned(), {}); }} ",
                p.ident,
                lit(&p.name),
                value_expr(p, "x")
            );
        } else {
            let _ = write!(
                s,
                "v.insert({}.to_owned(), {}); ",
                lit(&p.name),
                value_expr(p, &p.ident)
            );
        }
    }
    s.push_str("v }");
    s
}

impl Unit<'_> {
    fn ty(&self, c: &Contract, t: Option<&TypeId>) -> Result<Ty, Error> {
        let iface = c.iface.to_string();
        Ok(match t {
            None => Ty {
                rust: "()".into(),
                codec: "::zenkey::codec::Nothing".into(),
            },
            Some(TypeId::Protobuf { name, .. }) => {
                let rust = self.protos.rust_type(name, "super::__proto")?;
                Ty {
                    codec: format!("::zenkey::codec::Protobuf<{rust}>"),
                    rust,
                }
            }
            Some(TypeId::JsonSchema { name, schema }) => {
                let rust = self.jsons.rust_type(&iface, name, schema, "super")?;
                Ty {
                    codec: format!("::zenkey::codec::Json<{rust}>"),
                    rust,
                }
            }
            Some(TypeId::Raw { .. }) => Ty {
                rust: "::std::vec::Vec<u8>".into(),
                codec: "::zenkey::codec::Raw".into(),
            },
        })
    }
}

/// The whole generated file.
pub fn emit(u: &Unit<'_>) -> Result<String, Error> {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "// @generated by zenkey-build {} from the contracts below. Do not edit: change\n// the contracts, and the build regenerates it (docs/zk2/codegen.md, #611).",
        env!("CARGO_PKG_VERSION")
    );
    for i in u.ifaces {
        let _ = writeln!(
            s,
            "//   {} {} ({})",
            i.contract.iface,
            i.fingerprint,
            i.source.display()
        );
    }
    s.push('\n');
    if let Some(inc) = &u.proto_includes {
        let _ = writeln!(
            s,
            "/// The protobuf types, compiled by prost from the bundles' own descriptor sets.\n{ALLOW}\npub mod __proto {{\n    include!({});\n}}\n",
            path_lit(inc)
        );
    }
    if !u.json_files.is_empty() {
        let _ = writeln!(
            s,
            "/// The JSON Schema types, generated by typify, one module per group of schema files.\n{ALLOW}\npub mod __json {{"
        );
        for (m, f) in u.json_files {
            let _ = writeln!(
                s,
                "    pub mod {m} {{\n        include!({});\n    }}",
                path_lit(f)
            );
        }
        s.push_str("}\n\n");
    }
    for i in u.ifaces {
        s.push_str(&emit_iface(u, i)?);
        s.push('\n');
    }
    Ok(s)
}

struct Op<'r> {
    r: &'r Resource,
    name: String,
    konst: String,
    params: Vec<Param>,
    req: Ty,
    resp: Ty,
    summary: Ty,
    many: bool,
    fanout: bool,
}

struct DataRes<'r> {
    r: &'r Resource,
    name: String,
    konst: String,
    params: Vec<Param>,
    ty: Ty,
    attachment: Ty,
}

#[allow(clippy::too_many_lines)]
fn emit_iface(u: &Unit<'_>, i: &Iface<'_>) -> Result<String, Error> {
    let c = i.contract;
    let iface = c.iface.to_string();
    let names = resource_names(&c.resources);
    let cfg = if u.contract_crate {
        "    #[cfg(feature = \"zenoh\")]\n"
    } else {
        ""
    };

    let mut ops = Vec::new();
    let mut data = Vec::new();
    for (r, name) in c.resources.iter().zip(&names) {
        let konst = shouty(name);
        let params = params_of(r);
        match &r.body {
            Body::Operation(o) => {
                let mut name = name.clone();
                if [
                    "bind",
                    "new",
                    "inner",
                    "from_inner",
                    "fleet",
                    "with_timeout",
                    "with_retries",
                    "with_metadata",
                    "declare",
                    "operations",
                ]
                .contains(&name.as_str())
                {
                    name.push_str("_op");
                }
                ops.push(Op {
                    r,
                    name,
                    konst,
                    params,
                    req: u.ty(c, Some(&o.request))?,
                    resp: u.ty(c, Some(&o.response))?,
                    summary: u.ty(c, o.summary.as_ref())?,
                    many: o.replies == Replies::Many,
                    fanout: o.fanout == Fanout::Allowed,
                });
            }
            Body::Data(d) => {
                let mut name = name.clone();
                if ["declare", "operations"].contains(&name.as_str()) {
                    name.push('_');
                }
                data.push(DataRes {
                    r,
                    name,
                    konst,
                    params,
                    ty: u.ty(c, Some(&d.type_))?,
                    attachment: u.ty(c, d.attachment.as_ref())?,
                });
            }
        }
    }

    let mut s = String::new();
    let mod_name = module(&c.iface);
    let title = match &c.summary {
        Some(sum) => format!("`{iface}`: {sum}"),
        None => format!("`{iface}`"),
    };
    s.push_str(&doc("", &title));
    let _ = writeln!(
        s,
        "///\n/// Generated from `{}` (fingerprint `{}`).\n{ALLOW}\npub mod {mod_name} {{",
        i.source
            .file_name()
            .map_or_else(String::new, |f| f.to_string_lossy().into_owned()),
        i.fingerprint
    );

    // Identity and the bundle.
    let _ = writeln!(
        s,
        r#"    /// The interface id.
    pub const IFACE: &str = {iface_lit};
    /// The contract's fingerprint (spec §9.5).
    pub const FINGERPRINT: &str = {fp};
    /// The bundle (spec §9.6): built once by `zenkey-build`, byte for byte what
    /// `zk2 contract bundle` builds, and served as it is; never rebuilt at run time.
    pub static BUNDLE: &[u8] = include_bytes!({bundle});

    /// The interface id, parsed.
    pub fn iface() -> ::zenkey::model::grammar::IfaceId {{
        IFACE.parse().expect("an interface id zenkey-build checked")
    }}

    /// This revision's implementation: the contract read from [`BUNDLE`], with
    /// those bytes (`Implementation::from_bundle`), made once.
    pub fn implementation() -> ::zenkey::Implementation {{
        static IMPLEMENTATION: ::std::sync::OnceLock<::zenkey::Implementation> =
            ::std::sync::OnceLock::new();
        IMPLEMENTATION
            .get_or_init(|| {{
                ::zenkey::Implementation::from_bundle(BUNDLE)
                    .expect("the bundle zenkey-build embedded verifies")
            }})
            .clone()
    }}

    /// The contract: what a consumer or client of this interface is compiled
    /// against (R4).
    pub fn contract() -> ::std::sync::Arc<::zenkey::model::contract::Contract> {{
        implementation().shared_contract()
    }}
"#,
        iface_lit = lit(&iface),
        fp = lit(&i.fingerprint),
        bundle = path_lit(&i.bundle_path),
    );

    // Types.
    s.push_str("    /// The payload types this interface's schemas define.\n    pub mod types {\n");
    for p in u.protos.packages_of(c) {
        let _ = writeln!(
            s,
            "        pub use super::super::__proto::{}::*;",
            Protos::package_path(&p)
        );
    }
    if let Some(m) = u.jsons.module_of(&iface) {
        let _ = writeln!(s, "        pub use super::super::__json::{m}::*;");
        for (name, path) in u.jsons.hinted_of(&iface) {
            let _ = writeln!(
                s,
                "        /// Bound by a name hint (`json_type`).\n        pub type {name} = {path};"
            );
        }
    }
    s.push_str("    }\n\n");

    // Resource names.
    s.push_str("    /// Resource names, `<kind token>/<template>`, as the descriptor and the\n    /// runtime spell them.\n    pub mod resource {\n");
    for (r, name) in c.resources.iter().zip(&names) {
        let full = format!("{}/{}", r.token, r.template);
        let mut d = format!("`{full}`");
        if let Some(text) = &r.doc {
            d.push_str(": ");
            d.push_str(text);
        }
        s.push_str(&doc("        ", &d));
        let _ = writeln!(
            s,
            "        pub const {}: &str = {};",
            shouty(name),
            lit(&full)
        );
    }
    s.push_str("    }\n\n");

    // Roles.
    if !c.requires.is_empty() {
        s.push_str("    /// The roles this contract requires (spec §3.1), bound by configuration\n    /// (R1). Each is consumed through the required interface's `Consumer`.\n    pub mod role {\n");
        for (role, req) in &c.requires {
            s.push_str(&doc(
                "        ",
                &format!(
                    "`{role}`: `{}`{}",
                    req.interface,
                    if req.optional { ", optional" } else { "" }
                ),
            ));
            let _ = writeln!(
                s,
                "        pub const {}: &str = {};",
                shouty(role),
                lit(role)
            );
        }
        s.push_str("    }\n\n");
    }

    if !ops.is_empty() {
        emit_handlers(&mut s, &iface, &ops);
        emit_api(&mut s, &ops);
    }
    emit_server(&mut s, cfg, &ops, &data);
    emit_consumer(&mut s, cfg, &data);
    if !ops.is_empty() {
        emit_client(&mut s, cfg, &ops);
    }
    s.push_str("}\n");
    Ok(s)
}

fn op_doc(o: &Op<'_>) -> String {
    let mut d = format!("`{}/{}`", o.r.token, o.r.template);
    if let Some(t) = &o.r.doc {
        d.push_str(": ");
        d.push_str(t);
    }
    d
}

fn emit_handlers(s: &mut String, iface: &str, ops: &[Op<'_>]) {
    s.push_str(
        "    /// The owner's side of the operations (spec §5): one method per operation,\n    /// taking the decoded request and what is known of the call, returning the\n    /// response or the error envelope (§5.2). A required operation is a required\n    /// method; an optional one answers `unavailable` (cause `build`) unless\n    /// overridden, or is listed unavailable before `Server::declare`. A\n    /// `replies = \"many\"` operation sends through a `Sink`: values, then its\n    /// summary (O6).\n    pub trait Handlers: Send + Sync + 'static {\n",
    );
    for o in ops {
        s.push_str(&doc("        ", &op_doc(o)));
        let sig = if o.many {
            format!(
                "fn {}(&self, request: {}, call: ::zenkey::CallInfo, out: ::zenkey::Sink<{}, {}>) -> impl ::std::future::Future<Output = ::std::result::Result<(), ::zenkey::OpError>> + Send",
                o.name, o.req.rust, o.resp.rust, o.summary.rust
            )
        } else {
            format!(
                "fn {}(&self, request: {}, call: ::zenkey::CallInfo) -> impl ::std::future::Future<Output = ::std::result::Result<{}, ::zenkey::OpError>> + Send",
                o.name, o.req.rust, o.resp.rust
            )
        };
        if o.r.optional {
            let unused = if o.many {
                "(request, call, out)"
            } else {
                "(request, call)"
            };
            let _ = writeln!(
                s,
                "        {sig} {{\n            let _ = {unused};\n            async {{\n                ::std::result::Result::Err(::zenkey::OpError::unavailable(\n                    ::zenkey::model::descriptor::Cause::Build,\n                    {},\n                ))\n            }}\n        }}",
                lit(&format!(
                    "{iface} {}/{} is not implemented by this build",
                    o.r.token, o.r.template
                ))
            );
        } else {
            let _ = writeln!(s, "        {sig};");
        }
    }
    s.push_str("    }\n\n");
}

fn emit_api(s: &mut String, ops: &[Op<'_>]) {
    s.push_str(
        "    /// The operations as a caller makes them, one concrete address at a time\n    /// (O1): the generated `Client` implements it, and a test implements it with a\n    /// double, so a component written against `impl Api` needs no bus.\n    pub trait Api: Send + Sync {\n",
    );
    for o in ops {
        s.push_str(&doc("        ", &op_doc(o)));
        let _ = writeln!(
            s,
            "        fn {}(&self, at: &::zenkey::model::grammar::Addr{}, request: {}) -> impl ::std::future::Future<Output = ::zenkey::Result<{}>> + Send;",
            o.name,
            args(&o.params, false),
            o.req.rust,
            api_out(o)
        );
    }
    s.push_str("    }\n\n");
}

fn api_out(o: &Op<'_>) -> String {
    if o.many {
        format!(
            "::zenkey::call::Replies<{}, {}>",
            o.resp.rust, o.summary.rust
        )
    } else {
        format!("::zenkey::call::Outcome<{}>", o.resp.rust)
    }
}

fn writer_ty(d: &DataRes<'_>) -> String {
    match d.r.kind {
        Kind::State => format!("::zenkey::typed::StateWriter<{}>", d.ty.codec),
        Kind::Event => format!("::zenkey::typed::EventWriter<{}>", d.ty.codec),
        _ => format!(
            "::zenkey::typed::Writer<{}, {}>",
            d.ty.codec, d.attachment.codec
        ),
    }
}

#[allow(clippy::too_many_lines)]
fn emit_server(s: &mut String, cfg: &str, ops: &[Op<'_>], data: &[DataRes<'_>]) {
    let fixed: Vec<&DataRes<'_>> = data.iter().filter(|d| d.params.is_empty()).collect();
    let templated: Vec<&DataRes<'_>> = data.iter().filter(|d| !d.params.is_empty()).collect();
    let _ = writeln!(
        s,
        "    /// The owner's side: a typed writer per data resource without parameters\n    /// (an optional one is `None` when absent here), a method per templated one,\n    /// and an operation server per operation, wired to the `Handlers`. QoS and\n    /// `Encoding` are the contract's (§2.4, §7.2).\n{cfg}    pub struct Server {{"
    );
    for d in &fixed {
        s.push_str(&doc("        ", &data_doc(d)));
        let ty = writer_ty(d);
        if d.r.optional {
            let _ = writeln!(s, "        pub {}: ::std::option::Option<{ty}>,", d.name);
        } else {
            let _ = writeln!(s, "        pub {}: {ty},", d.name);
        }
    }
    if !ops.is_empty() {
        s.push_str("        operations: ::std::vec::Vec<::zenkey::operation::OperationServer>,\n");
    }
    s.push_str("    }\n\n");

    let _ = writeln!(s, "{cfg}    impl Server {{");
    let (generic, handlers_arg) = if ops.is_empty() {
        ("", "")
    } else {
        ("<H: Handlers>", ", handlers: ::std::sync::Arc<H>")
    };
    let _ = writeln!(
        s,
        "        /// Declares this interface on `b`, before start (spec §8.2): implements it\n        /// if `b` does not yet, declares a writer per data resource without\n        /// parameters, exposes the templated ones (members come later, through\n        /// the methods below), and serves every operation. Optional resources\n        /// absent here (a capability not held, or listed unavailable on `b`\n        /// first) are skipped.\n        pub async fn declare{generic}(b: &mut ::zenkey::ServiceBuilder{handlers_arg}) -> ::zenkey::Result<Self> {{\n            let iface = iface();\n            if !b.implements(&iface) {{\n                b.implement(implementation())?;\n            }}\n            let none = ::zenkey::model::template::Bindings::new();"
    );
    for d in &fixed {
        let ctor = match d.r.kind {
            Kind::Event => format!(
                "<{}>::declare(b, &iface, resource::{}, &none)?",
                writer_ty(d),
                d.konst
            ),
            _ => format!(
                "<{}>::declare(b, &iface, resource::{}, &none).await?",
                writer_ty(d),
                d.konst
            ),
        };
        if d.r.optional {
            let _ = writeln!(
                s,
                "            let {} = if b.is_absent(&iface, resource::{})? {{ ::std::option::Option::None }} else {{ ::std::option::Option::Some({ctor}) }};",
                d.name, d.konst
            );
        } else {
            let _ = writeln!(s, "            let {} = {ctor};", d.name);
        }
    }
    let mut state_template = false;
    for d in &templated {
        let expose = format!("b.expose(&iface, resource::{})?;", d.konst);
        let serve = if d.r.kind == Kind::State {
            state_template = true;
            " b.serve_state(&iface)?;"
        } else {
            ""
        };
        if d.r.optional {
            let _ = writeln!(
                s,
                "            if !b.is_absent(&iface, resource::{})? {{ {expose}{serve} }}",
                d.konst
            );
        } else {
            let _ = writeln!(s, "            {expose}{serve}");
        }
    }
    let _ = state_template;
    if !ops.is_empty() {
        s.push_str("            let mut operations = ::std::vec::Vec::new();\n");
        for o in ops {
            let serve = if o.many {
                format!(
                    "::zenkey::typed::serve_many::<{}, {}, {}, _, _>(b, &iface, resource::{}, move |request, call, out| {{\n                    let h = ::std::sync::Arc::clone(&h);\n                    async move {{ h.{}(request, call, out).await }}\n                }})",
                    o.req.codec, o.resp.codec, o.summary.codec, o.konst, o.name
                )
            } else {
                format!(
                    "::zenkey::typed::serve_one::<{}, {}, _, _>(b, &iface, resource::{}, move |request, call| {{\n                    let h = ::std::sync::Arc::clone(&h);\n                    async move {{ h.{}(request, call).await }}\n                }})",
                    o.req.codec, o.resp.codec, o.konst, o.name
                )
            };
            let body = format!(
                "let h = ::std::sync::Arc::clone(&handlers);\n                operations.push({serve}.await?);"
            );
            if o.r.optional {
                let _ = writeln!(
                    s,
                    "            if !b.is_absent(&iface, resource::{})? {{\n                {body}\n            }}",
                    o.konst
                );
            } else {
                let _ = writeln!(s, "            {{\n                {body}\n            }}");
            }
        }
    }
    let fields: Vec<&str> = fixed.iter().map(|d| d.name.as_str()).collect();
    let mut init = fields.join(", ");
    if !ops.is_empty() {
        if !init.is_empty() {
            init.push_str(", ");
        }
        init.push_str("operations");
    }
    let _ = writeln!(
        s,
        "            let _ = &none;\n            ::std::result::Result::Ok(Self {{ {init} }})\n        }}"
    );
    if !ops.is_empty() {
        s.push_str("\n        /// The operation servers, one per operation served (O1).\n        pub fn operations(&self) -> &[::zenkey::operation::OperationServer] {\n            &self.operations\n        }\n");
    }
    for d in &templated {
        s.push('\n');
        s.push_str(&doc(
            "        ",
            &format!(
                "A writer on one member of {}, exposed by `declare`.",
                data_doc(d)
            ),
        ));
        let ty = writer_ty(d);
        let b = bindings(&d.params, false);
        let a = args(&d.params, false);
        match d.r.kind {
            Kind::State => {
                let _ = writeln!(
                    s,
                    "        pub async fn {}(&self, svc: &mut ::zenkey::Service{a}) -> ::zenkey::Result<{ty}> {{\n            <{ty}>::on(svc, &self::iface(), resource::{}, &{b}).await\n        }}",
                    d.name, d.konst
                );
            }
            Kind::Event => {
                let _ = writeln!(
                    s,
                    "        pub fn {}(&self, svc: &::zenkey::Service{a}) -> ::zenkey::Result<{ty}> {{\n            <{ty}>::on(svc, &self::iface(), resource::{}, &{b})\n        }}",
                    d.name, d.konst
                );
            }
            _ => {
                let _ = writeln!(
                    s,
                    "        pub async fn {}(&self, svc: &::zenkey::Service{a}) -> ::zenkey::Result<{ty}> {{\n            <{ty}>::on(svc, &self::iface(), resource::{}, &{b}).await\n        }}",
                    d.name, d.konst
                );
            }
        }
    }
    s.push_str("    }\n\n");
}

fn data_doc(d: &DataRes<'_>) -> String {
    let mut t = format!("`{}/{}`", d.r.token, d.r.template);
    if let Some(x) = &d.r.doc {
        t.push_str(": ");
        t.push_str(x);
    }
    t
}

fn emit_consumer(s: &mut String, cfg: &str, data: &[DataRes<'_>]) {
    let _ = writeln!(
        s,
        r#"    /// A role's consumer of this interface (spec §3.2): providers and parameter
    /// bindings from configuration (R1, R2). Typed subscriptions (attributed,
    /// R3), current state from its owners (S4), last-known state from an archive
    /// (S5) and event replay (§2.6).
{cfg}    pub struct Consumer {{
        inner: ::zenkey::consumer::Consumer,
    }}

{cfg}    impl Consumer {{
        /// The consumer of `role`, whose contract requires this interface.
        pub fn bind(service: &::zenkey::Service, role: &str) -> ::zenkey::Result<Self> {{
            ::std::result::Result::Ok(Self {{ inner: service.consumer(role, contract())? }})
        }}

        /// Wraps a consumer of this interface.
        pub fn from_inner(inner: ::zenkey::consumer::Consumer) -> ::zenkey::Result<Self> {{
            if *inner.interface() != iface() {{
                return ::std::result::Result::Err(::zenkey::Error::Contract(::std::format!(
                    "a consumer of {{}} is not one of {{IFACE}}",
                    inner.interface()
                )));
            }}
            ::std::result::Result::Ok(Self {{ inner }})
        }}

        /// The untyped consumer.
        pub fn inner(&self) -> &::zenkey::consumer::Consumer {{
            &self.inner
        }}"#
    );
    for d in data {
        let t = &d.ty.rust;
        let codec = &d.ty.codec;
        let _ = writeln!(
            s,
            "\n{}        pub async fn subscribe_{}<F>(&self, callback: F) -> ::zenkey::Result<::zenkey::consumer::Subscription>\n        where\n            F: Fn(::zenkey::typed::Received<{t}>) + Send + Sync + 'static,\n        {{\n            ::zenkey::typed::subscribe::<{codec}, F>(&self.inner, resource::{}, callback).await\n        }}",
            doc(
                "        ",
                &format!("Subscribes to {} across the bound providers.", data_doc(d))
            ),
            d.name,
            d.konst
        );
        match d.r.kind {
            Kind::State => {
                let _ = writeln!(
                    s,
                    "\n{}        pub async fn get_{}(&self, timeout: ::std::time::Duration) -> ::zenkey::Result<::zenkey::typed::StateGet<{t}>> {{\n            ::zenkey::typed::get::<{codec}>(&self.inner, resource::{}, ::std::option::Option::None, timeout).await\n        }}",
                    doc(
                        "        ",
                        &format!(
                            "Current {}, every member, from its owners (S4, S6).",
                            data_doc(d)
                        )
                    ),
                    d.name,
                    d.konst
                );
                let a = args(&d.params, false);
                let b = bindings(&d.params, false);
                if !d.params.is_empty() {
                    let _ = writeln!(
                        s,
                        "\n{}        pub async fn get_{}_member(&self{a}, timeout: ::std::time::Duration) -> ::zenkey::Result<::zenkey::typed::StateGet<{t}>> {{\n            ::zenkey::typed::get::<{codec}>(&self.inner, resource::{}, ::std::option::Option::Some(&{b}), timeout).await\n        }}",
                        doc(
                            "        ",
                            &format!(
                                "Current {}, one member, from its owners (S4, S6).",
                                data_doc(d)
                            )
                        ),
                        d.name,
                        d.konst
                    );
                }
                let _ = writeln!(
                    s,
                    "\n{}        pub async fn last_known_{}(&self, archive: &::zenkey::model::grammar::Addr, provider: &::zenkey::model::grammar::Addr{a}, timeout: ::std::time::Duration) -> ::zenkey::Result<::std::option::Option<::zenkey::typed::LastKnown<{t}>>> {{\n            ::zenkey::typed::last_known::<{codec}>(&self.inner, archive, provider, resource::{}, &{b}, timeout).await\n        }}",
                    doc(
                        "        ",
                        &format!(
                            "Last-known {} of `provider`, from the archive at `archive` (S5): never current (S6).",
                            data_doc(d)
                        )
                    ),
                    d.name,
                    d.konst
                );
            }
            Kind::Event => {
                let _ = writeln!(
                    s,
                    "\n{}        pub async fn replay_{}(&self, retention: ::std::time::Duration, timeout: ::std::time::Duration) -> ::zenkey::Result<::std::vec::Vec<::zenkey::typed::Received<{t}>>> {{\n            ::zenkey::typed::replay::<{codec}>(&self.inner, resource::{}, retention, timeout).await\n        }}",
                    doc(
                        "        ",
                        &format!("Replays {} within `retention` (§2.6).", data_doc(d))
                    ),
                    d.name,
                    d.konst
                );
            }
            _ => {}
        }
    }
    s.push_str("    }\n\n");
}

#[allow(clippy::too_many_lines)]
fn emit_client(s: &mut String, cfg: &str, ops: &[Op<'_>]) {
    let fanout: Vec<&Op<'_>> = ops.iter().filter(|o| o.fanout).collect();
    let fleet_fn = if fanout.is_empty() {
        String::new()
    } else {
        "\n        /// A fleet over this client's providers, for the `fanout = \"allowed\"`\n        /// operations (O2).\n        pub fn fleet(&self) -> Fleet {\n            Fleet { inner: self.inner.fleet() }\n        }\n".to_owned()
    };
    let _ = writeln!(
        s,
        r#"    /// A typed client: through a role (`bind`), or a tool's explicit providers
    /// (`new`). Its operations are the `Api` trait's.
{cfg}    #[derive(Clone)]
{cfg}    pub struct Client {{
        inner: ::zenkey::Client,
    }}

{cfg}    impl Client {{
        /// The client of `role`, whose contract requires this interface.
        pub fn bind(service: &::zenkey::Service, role: &str) -> ::zenkey::Result<Self> {{
            ::std::result::Result::Ok(Self {{ inner: service.client(role, contract())? }})
        }}

        /// A tool's client of `providers` (`<system>/<service>`, either `*`).
        pub fn new(session: &::zenkey::zenoh::Session, providers: &[&str]) -> ::zenkey::Result<Self> {{
            ::std::result::Result::Ok(Self {{ inner: ::zenkey::Client::new(session, contract(), providers)? }})
        }}

        /// Wraps a client of this interface.
        pub fn from_inner(inner: ::zenkey::Client) -> ::zenkey::Result<Self> {{
            if *inner.interface() != iface() {{
                return ::std::result::Result::Err(::zenkey::Error::Contract(::std::format!(
                    "a client of {{}} is not one of {{IFACE}}",
                    inner.interface()
                )));
            }}
            ::std::result::Result::Ok(Self {{ inner }})
        }}

        /// The untyped client.
        pub fn inner(&self) -> &::zenkey::Client {{
            &self.inner
        }}

        /// The reply timeout of every call, over the contract's `timeout_ms`.
        #[must_use]
        pub fn with_timeout(self, timeout: ::std::time::Duration) -> Self {{
            Self {{ inner: self.inner.with_timeout(timeout) }}
        }}

        /// Retries after silence, for idempotent operations only (O4).
        #[must_use]
        pub fn with_retries(self, retries: u32) -> Self {{
            Self {{ inner: self.inner.with_retries(retries) }}
        }}

        /// The call metadata every request carries (O7).
        #[must_use]
        pub fn with_metadata(self, metadata: ::zenkey::CallMetadata) -> Self {{
            Self {{ inner: self.inner.with_metadata(metadata) }}
        }}
{fleet_fn}    }}

{cfg}    impl Api for Client {{"#
    );
    for o in ops {
        let call = if o.many {
            format!(
                "::zenkey::typed::call_many::<{}, {}, {}>",
                o.req.codec, o.resp.codec, o.summary.codec
            )
        } else {
            format!("::zenkey::typed::call::<{}, {}>", o.req.codec, o.resp.codec)
        };
        let _ = writeln!(
            s,
            "        async fn {}(&self, at: &::zenkey::model::grammar::Addr{}, request: {}) -> ::zenkey::Result<{}> {{\n            {call}(&self.inner, at, resource::{}, &{}, request).await\n        }}",
            o.name,
            args(&o.params, false),
            o.req.rust,
            api_out(o),
            o.konst,
            bindings(&o.params, false)
        );
    }
    s.push_str("    }\n\n");

    if fanout.is_empty() {
        return;
    }
    let _ = writeln!(
        s,
        r#"    /// Fan-out calls (O2) to a selection spelled by name or wildcard, for the
    /// operations that allow them; every reply decoded and attributed.
{cfg}    #[derive(Clone)]
{cfg}    pub struct Fleet {{
        inner: ::zenkey::Fleet,
    }}

{cfg}    impl Fleet {{
        /// A fleet over `selection` (`<system>/<service>`, either `*`).
        pub fn new(session: &::zenkey::zenoh::Session, selection: &[&str]) -> ::zenkey::Result<Self> {{
            ::std::result::Result::Ok(Self {{ inner: ::zenkey::Fleet::new(session, contract(), selection)? }})
        }}

        /// The untyped fleet.
        pub fn inner(&self) -> &::zenkey::Fleet {{
            &self.inner
        }}

        /// The time every fan-out waits for replies, over the contract's `timeout_ms`.
        #[must_use]
        pub fn with_timeout(self, timeout: ::std::time::Duration) -> Self {{
            Self {{ inner: self.inner.with_timeout(timeout) }}
        }}"#
    );
    for o in fanout {
        let _ = writeln!(
            s,
            "\n{}        pub async fn {}(&self{}, request: {}) -> ::zenkey::Result<::zenkey::call::Replies<{}, {}>> {{\n            ::zenkey::typed::fan_out::<{}, {}, {}>(&self.inner, resource::{}, &{}, request).await\n        }}",
            doc(
                "        ",
                &format!(
                    "{}, fanned out; a parameter left `None` selects every member.",
                    op_doc(o)
                )
            ),
            o.name,
            args(&o.params, true),
            o.req.rust,
            o.resp.rust,
            o.summary.rust,
            o.req.codec,
            o.resp.codec,
            o.summary.codec,
            o.konst,
            bindings(&o.params, true)
        );
    }
    s.push_str("    }\n\n");
}
