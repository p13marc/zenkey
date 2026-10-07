//! Session configurations for the spike's topologies: routers, peers and
//! clients, with multicast scouting off (every topology is wired
//! explicitly) and the HLC on everywhere, so producers stamp their own
//! samples (r3 §3.6, S1).

use anyhow::{Result, anyhow};

/// How a session joins the topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Router,
    Peer,
    Client,
}

impl Mode {
    fn as_json(self) -> &'static str {
        match self {
            Self::Router => "\"router\"",
            Self::Peer => "\"peer\"",
            Self::Client => "\"client\"",
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "router" => Ok(Self::Router),
            "peer" => Ok(Self::Peer),
            "client" => Ok(Self::Client),
            _ => Err(anyhow!("mode {s:?} is not router, peer or client")),
        }
    }
}

/// One session's place in a topology.
#[derive(Debug, Clone)]
pub struct Topo {
    pub mode: Mode,
    pub listen: Vec<String>,
    pub connect: Vec<String>,
    /// The session namespace (the deployment prefix), if any.
    pub namespace: Option<String>,
}

impl Topo {
    #[must_use]
    pub fn client(connect: &[String]) -> Self {
        Self {
            mode: Mode::Client,
            listen: Vec::new(),
            connect: connect.to_vec(),
            namespace: None,
        }
    }

    /// The zenoh configuration.
    pub fn config(&self) -> Result<zenoh::Config> {
        let mut c = zenoh::Config::default();
        let set = |c: &mut zenoh::Config, k: &str, v: &str| {
            c.insert_json5(k, v).map_err(|e| anyhow!("config {k} = {v}: {e}"))
        };
        set(&mut c, "mode", self.mode.as_json())?;
        set(&mut c, "scouting/multicast/enabled", "false")?;
        set(&mut c, "timestamping/enabled", "{ router: true, peer: true, client: true }")?;
        let list = |v: &[String]| serde_json::to_string(v).expect("strings serialize");
        if !self.listen.is_empty() {
            set(&mut c, "listen/endpoints", &list(&self.listen))?;
        }
        if !self.connect.is_empty() {
            set(&mut c, "connect/endpoints", &list(&self.connect))?;
        }
        if let Some(ns) = &self.namespace {
            set(&mut c, "namespace", &serde_json::to_string(ns)?)?;
        }
        Ok(c)
    }

    /// Opens the session.
    pub async fn open(&self) -> Result<zenoh::Session> {
        zenoh::open(self.config()?).await.map_err(|e| anyhow!("open {:?}: {e}", self.mode))
    }
}

/// A free TCP port on 127.0.0.1, by binding to port 0 and releasing it.
/// Racy by nature, but never a fixed port that a parallel run also uses.
pub fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}
