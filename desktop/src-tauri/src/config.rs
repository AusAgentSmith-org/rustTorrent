use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
    time::Duration,
};

use librtbit::{
    ConnectionOptions, ListenerMode, ListenerOptions, PeerConnectionOptions, dht::PersistentDht,
    limits::LimitsConfig,
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RtbitDesktopConfigDht {
    pub disable: bool,
    pub disable_persistence: bool,
    pub persistence_filename: PathBuf,
}

impl Default for RtbitDesktopConfigDht {
    fn default() -> Self {
        Self {
            disable: false,
            disable_persistence: false,
            persistence_filename: PersistentDht::default_persistence_filename().unwrap(),
        }
    }
}

#[serde_as]
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RtbitDesktopConfigConnections {
    pub enable_tcp_listen: bool,
    pub enable_tcp_outgoing: bool,
    pub enable_utp: bool,
    pub enable_upnp_port_forward: bool,
    pub socks_proxy: String,
    pub listen_port: u16,

    #[serde_as(as = "serde_with::DurationSeconds")]
    pub peer_connect_timeout: Duration,
    #[serde_as(as = "serde_with::DurationSeconds")]
    pub peer_read_write_timeout: Duration,
}

impl RtbitDesktopConfigConnections {
    pub fn as_listener_and_connect_opts(&self) -> (Option<ListenerOptions>, ConnectionOptions) {
        let mode = match (self.enable_tcp_listen, self.enable_utp) {
            (true, true) => Some(ListenerMode::TcpAndUtp),
            (true, false) => Some(ListenerMode::TcpOnly),
            (false, true) => Some(ListenerMode::UtpOnly),
            (false, false) => None,
        };
        let listener_opts = mode.map(|mode| ListenerOptions {
            mode,
            listen_addr: (Ipv4Addr::UNSPECIFIED, self.listen_port).into(),
            enable_upnp_port_forwarding: self.enable_upnp_port_forward,
            ..Default::default()
        });
        let connect_opts = ConnectionOptions {
            proxy_url: if self.socks_proxy.is_empty() {
                None
            } else {
                Some(self.socks_proxy.clone())
            },
            enable_tcp: self.enable_tcp_outgoing,
            peer_opts: Some(PeerConnectionOptions {
                connect_timeout: Some(self.peer_connect_timeout),
                read_write_timeout: Some(self.peer_read_write_timeout),
                ..Default::default()
            }),
        };
        (listener_opts, connect_opts)
    }
}

impl Default for RtbitDesktopConfigConnections {
    fn default() -> Self {
        Self {
            enable_tcp_listen: true,
            enable_tcp_outgoing: true,
            enable_utp: false,
            enable_upnp_port_forward: true,
            listen_port: 4240,
            socks_proxy: String::new(),
            peer_connect_timeout: Duration::from_secs(2),
            peer_read_write_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RtbitDesktopConfigPersistence {
    pub disable: bool,

    #[serde(default)]
    pub folder: PathBuf,

    #[serde(default)]
    pub fastresume: bool,

    #[serde(default)]
    pub fastresume_validation_denom: Option<u32>,

    /// Deprecated, but keeping for backwards compat for serialized / deserialized config.
    #[serde(default)]
    pub filename: PathBuf,
}

impl RtbitDesktopConfigPersistence {
    pub(crate) fn fix_backwards_compat(&mut self) {
        if self.folder != Path::new("") {
            return;
        }
        if self.filename != Path::new("")
            && let Some(parent) = self.filename.parent()
        {
            self.folder = parent.to_owned();
        }
    }
}

impl Default for RtbitDesktopConfigPersistence {
    fn default() -> Self {
        let folder = librtbit::SessionPersistenceConfig::default_json_persistence_folder().unwrap();
        Self {
            disable: false,
            folder,
            fastresume: false,
            fastresume_validation_denom: None,
            filename: PathBuf::new(),
        }
    }
}

#[serde_as]
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RtbitDesktopConfigHttpApi {
    pub disable: bool,
    pub listen_addr: SocketAddr,
    pub read_only: bool,
}

impl Default for RtbitDesktopConfigHttpApi {
    fn default() -> Self {
        Self {
            disable: Default::default(),
            listen_addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 3030)),
            read_only: false,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct RtbitDesktopConfigUpnp {
    #[serde(default)]
    pub enable_server: bool,

    #[serde(default)]
    pub server_friendly_name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RtbitDesktopConfig {
    pub default_download_location: PathBuf,

    /// Move completed torrents to this folder.
    #[serde(default)]
    pub completed_folder: Option<PathBuf>,

    #[cfg(feature = "disable-upload")]
    #[serde(default)]
    pub disable_upload: bool,

    pub dht: RtbitDesktopConfigDht,
    pub connections: RtbitDesktopConfigConnections,
    pub upnp: RtbitDesktopConfigUpnp,
    pub persistence: RtbitDesktopConfigPersistence,
    pub http_api: RtbitDesktopConfigHttpApi,

    #[serde(default)]
    pub ratelimits: LimitsConfig,

    /// RSS feed history limit: how many feed items to keep (None = keep all, default 500).
    #[serde(default = "default_rss_history_limit")]
    pub rss_history_limit: Option<usize>,
}

fn default_rss_history_limit() -> Option<usize> {
    Some(500)
}

impl Default for RtbitDesktopConfig {
    fn default() -> Self {
        let userdirs = directories::UserDirs::new().expect("directories::UserDirs::new()");
        let download_folder = userdirs
            .download_dir()
            .map(|d| d.to_owned())
            .unwrap_or_else(|| userdirs.home_dir().join("Downloads"));

        Self {
            default_download_location: download_folder,
            completed_folder: None,
            dht: Default::default(),
            connections: Default::default(),
            upnp: Default::default(),
            persistence: Default::default(),
            http_api: Default::default(),
            ratelimits: Default::default(),
            rss_history_limit: default_rss_history_limit(),
            #[cfg(feature = "disable-upload")]
            disable_upload: false,
        }
    }
}

impl RtbitDesktopConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.upnp.enable_server {
            if self.http_api.disable {
                anyhow::bail!("if UPnP server is enabled, you need to enable the HTTP API also.")
            }
            if self.http_api.listen_addr.ip().is_loopback() {
                anyhow::bail!(
                    "if UPnP server is enabled, you need to set HTTP API IP to 0.0.0.0 or at least non-localhost address."
                )
            }
        }
        Ok(())
    }
}
