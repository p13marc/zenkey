//! A router with zenoh 1.10.1's storage manager linked in statically, with
//! memory storages, for S5 (#601) and S12 (#595). Linking it avoids any
//! version or rustc skew between a router and a dynamically loaded plugin.

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};
use zk2rt::config::{Mode, Topo};

/// `storages`: (name, key expression). GC period and lifespan in seconds.
pub async fn run(listen: Vec<String>, connect: Vec<String>, storages: Vec<String>, gc_period: u64, gc_lifespan: u64, replication: bool) -> Result<()> {
    let mut c = Topo { mode: Mode::Router, listen, connect, namespace: None, shm: None }.config()?;
    let mut st = Map::new();
    for s in &storages {
        let (name, ke) = s.split_once('=').ok_or_else(|| anyhow!("--storage is name=keyexpr"))?;
        let mut cfg = json!({
            "key_expr": ke,
            "volume": "memory",
            "garbage_collection": {"period": gc_period, "lifespan": gc_lifespan},
        });
        if replication {
            cfg["replication"] = json!({"interval": 1, "sub_intervals": 5, "hot": 6, "warm": 30, "propagation_delay": 250});
        }
        st.insert(name.to_owned(), cfg);
    }
    let plugin = json!({ "storages": Value::Object(st) });
    c.insert_json5("plugins/storage_manager", &plugin.to_string()).map_err(|e| anyhow!("{e}"))?;
    let mut pm = zenoh::internal::plugins::PluginsManager::static_plugins_only();
    pm.declare_static_plugin::<zenoh_plugin_storage_manager::StoragesPlugin, &str>("storage_manager", true);
    let mut rt = zenoh::internal::runtime::RuntimeBuilder::new(c).plugins_manager(pm).build().await.map_err(|e| anyhow!("runtime: {e}"))?;
    rt.start().await.map_err(|e| anyhow!("start: {e}"))?;
    println!("ready {}", rt.zid());
    tokio::signal::ctrl_c().await?;
    Ok(())
}
