use std::{collections::HashSet, net::SocketAddr};

use always_cell::AlwaysCell;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub bind: SocketAddr,
    pub name: String,
    pub targets: HashSet<Target>,
    pub interval: u64,
    pub timeout: u64,
}

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone)]
pub struct Target {
    pub host: String,
    pub port: Option<u16>,
    pub mode: Mode,
    /// HTTP path
    pub path: Option<String>,
    /// HTTP status
    pub status: Option<u16>,
    /// DNS
    pub r#type: Option<String>,
    pub dns_name: Option<String>,
    pub interval: Option<u64>,
}

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Ping,
    Tcp,
    Http,
    Https,
    Dns,
}

impl Mode {
    pub const fn port(&self) -> u16 {
        match self {
            Mode::Ping => 0,
            Mode::Tcp => 80,
            Mode::Http => 80,
            Mode::Https => 443,
            Mode::Dns => 53,
        }
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Mode::Ping => "ping",
            Mode::Tcp => "tcp",
            Mode::Http => "http",
            Mode::Https => "https",
            Mode::Dns => "dns",
        }
    }
}

lazy_static::lazy_static! {
    pub static ref CONFIG_FILE: String = {
        let base = std::env::var("REACHR_CONF").unwrap_or_default();
        if base.is_empty() {
            "./config.yaml".to_string()
        } else {
            base
        }
    };
}

pub static CONFIG: AlwaysCell<watch::Receiver<Config>> = AlwaysCell::new();
