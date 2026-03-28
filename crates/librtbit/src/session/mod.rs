mod helpers;
mod network;
mod peer_sources;
mod types;

pub use types::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListOnlyResponse, SessionOptions,
    SessionPersistenceConfig,
};
// Public API: used by the CLI binary
pub(crate) use types::CheckedIncomingConnection;
#[allow(unused_imports)]
pub use types::read_local_file_including_stdin;

#[cfg(test)]
use types::torrent_file_from_info_bytes;
use types::torrent_from_bytes;

use std::{
    borrow::Cow,
    collections::HashSet,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use crate::{
    ManagedTorrent, ManagedTorrentShared,
    api::TorrentIdOrHash,
    bitv_factory::{BitVFactory, NonPersistentBitVFactory},
    category::CategoryManager,
    ip_ranges::IpRanges,
    limits::Limits,
    merge_streams::merge_streams,
    peer_connection::PeerConnectionOptions,
    session_persistence::{SessionPersistenceStore, json::JsonSessionPersistenceStore},
    session_stats::SessionStats,
    spawn_utils::BlockingSpawner,
    storage::{
        BoxStorageFactory, StorageFactoryExt, TorrentStorage, filesystem::FilesystemStorageFactory,
    },
    stream_connect::{SocksProxyConfig, StreamConnector, StreamConnectorArgs},
    torrent_state::{
        ManagedTorrentHandle, ManagedTorrentLocked, ManagedTorrentOptions, ManagedTorrentState,
        TorrentMetadata, initializing::TorrentStateInitializing,
    },
};
use anyhow::{Context, bail};
use arc_swap::ArcSwapOption;
use buffers::ByteBufOwned;
use dht::{Dht, DhtBuilder, DhtConfig, Id20, PersistentDht};
use futures::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use itertools::Itertools;
use librqbit_utp::BindDevice;
use librtbit_core::{
    crate_version, peer_id::generate_azereus_style, spawn_utils::spawn_with_cancel,
    torrent_metainfo::ValidatedTorrentMetaV1Info,
};
use librtbit_lsd::{LocalServiceDiscovery, LocalServiceDiscoveryOptions};
use parking_lot::RwLock;
use tokio::sync::Notify;
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{Instrument, debug, debug_span, error, info, trace, warn};
use tracker_comms::UdpTrackerClient;

use helpers::{compute_only_files, remove_files_and_dirs};
use types::{InternalAddResult, SessionDatabase};

pub const SUPPORTED_SCHEMES: [&str; 3] = ["http:", "https:", "magnet:"];

pub type TorrentId = usize;

pub struct Session {
    // Core state and services
    pub(crate) db: RwLock<SessionDatabase>,
    next_id: AtomicUsize,
    pub(crate) bitv_factory: Arc<dyn BitVFactory>,
    pub(super) spawner: BlockingSpawner,

    // Network
    peer_id: Id20,
    announce_port: Option<u16>,
    listen_addr: Option<SocketAddr>,
    dht: Option<Dht>,
    pub(crate) connector: Arc<StreamConnector>,
    reqwest_client: reqwest::Client,
    udp_tracker_client: Option<UdpTrackerClient>,
    disable_trackers: bool,

    // Lifecycle management
    cancellation_token: CancellationToken,
    _cancellation_token_drop_guard: DropGuard,

    // Runtime settings
    output_folder: RwLock<PathBuf>,
    peer_opts: PeerConnectionOptions,
    default_storage_factory: Option<BoxStorageFactory>,
    persistence: Option<Arc<dyn SessionPersistenceStore>>,
    trackers: HashSet<url::Url>,

    lsd: Option<LocalServiceDiscovery>,

    // Categories
    category_manager: CategoryManager,

    // Limits and throttling
    pub(crate) concurrent_initialize_semaphore: Arc<tokio::sync::Semaphore>,
    pub ratelimits: Limits,
    pub(crate) peer_limit_atomic: AtomicUsize,
    pub(crate) concurrent_init_limit_atomic: AtomicUsize,

    pub blocklist: IpRanges,
    pub allowlist: Option<IpRanges>,

    // Monitoring / tracing / logging
    pub(crate) stats: Arc<SessionStats>,
    root_span: Option<tracing::Span>,

    // Feature flags
    #[cfg(feature = "disable-upload")]
    _disable_upload: bool,
    pub ipv4_only: bool,
    pub peer_limit: Option<usize>,

    /// Configurable cap for probabilistic fastresume validation denominator.
    pub(crate) fastresume_validation_denom: Option<u32>,
    // Move-on-complete: destination folder for finished torrents
    completed_folder: RwLock<Option<PathBuf>>,
}

impl Session {
    /// Create a new session with default options.
    /// The passed in folder will be used as a default unless overridden per torrent.
    /// It will run a DHT server/client, a TCP listener and .
    #[inline(never)]
    pub fn new(default_output_folder: PathBuf) -> BoxFuture<'static, anyhow::Result<Arc<Self>>> {
        Self::new_with_opts(default_output_folder, SessionOptions::default())
    }

    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    pub fn get_peer_limit(&self) -> usize {
        self.peer_limit_atomic
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_peer_limit(&self, limit: usize) {
        self.peer_limit_atomic
            .store(limit, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn get_concurrent_init_limit(&self) -> usize {
        self.concurrent_init_limit_atomic
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_concurrent_init_limit(&self, limit: usize) {
        self.concurrent_init_limit_atomic
            .store(limit, std::sync::atomic::Ordering::Relaxed);
    }

    /// Create a new session with options.
    #[inline(never)]
    pub fn new_with_opts(
        default_output_folder: PathBuf,
        mut opts: SessionOptions,
    ) -> BoxFuture<'static, anyhow::Result<Arc<Self>>> {
        async move {
            let peer_id = opts
                .peer_id
                .unwrap_or_else(|| generate_azereus_style(*b"rQ", crate_version!()));
            let token = opts.cancellation_token.take().unwrap_or_default();

            #[cfg(feature = "disable-upload")]
            if opts.disable_upload {
                warn!("uploading disabled");
            }

            let bind_device = match opts.bind_device_name.as_ref() {
                Some(name) => Some(
                    BindDevice::new_from_name(name)
                        .with_context(|| format!("error creating bind device {name}"))?,
                ),
                None => None,
            };

            let listen_result = if let Some(listen_opts) = opts.listen.take() {
                Some(
                    listen_opts
                        .start(
                            opts.root_span.as_ref().and_then(|s| s.id()),
                            token.child_token(),
                            bind_device.as_ref(),
                        )
                        .await
                        .context("error starting listeners")?,
                )
            } else {
                None
            };

            let has_proxy = opts.connect.as_ref().and_then(|s| s.proxy_url.as_ref()).is_some();

            let dht = if opts.disable_dht || has_proxy {
                if has_proxy && !opts.disable_dht {
                    warn!("DHT disabled: SOCKS proxy configured (DHT uses UDP which cannot be proxied)");
                }
                None
            } else {
                let dht = if opts.disable_dht_persistence {
                    DhtBuilder::with_config(DhtConfig {
                        bootstrap_addrs: opts.dht_bootstrap_addrs.clone(),
                        cancellation_token: Some(token.child_token()),
                        bind_device: bind_device.as_ref(),
                        ..Default::default()
                    })
                    .await
                    .context("error initializing DHT")?
                } else {
                    let pdht_config = opts.dht_config.take().unwrap_or_default();
                    PersistentDht::create(
                        Some(pdht_config),
                        Some(token.clone()),
                        bind_device.as_ref(),
                    )
                    .await
                    .context("error initializing persistent DHT")?
                };

                Some(dht)
            };
            let peer_opts = opts
                .connect
                .as_ref()
                .and_then(|p| p.peer_opts)
                .unwrap_or_default();

            async fn persistence_factory(
                opts: &SessionOptions,
                spawner: BlockingSpawner,
            ) -> anyhow::Result<(
                Option<Arc<dyn SessionPersistenceStore>>,
                Arc<dyn BitVFactory>,
            )> {
                macro_rules! make_result {
                    ($store:expr) => {
                        if opts.fastresume {
                            Ok((Some($store.clone()), $store))
                        } else {
                            Ok((Some($store), Arc::new(NonPersistentBitVFactory {})))
                        }
                    };
                }

                match &opts.persistence {
                    Some(SessionPersistenceConfig::Json { folder }) => {
                        let folder = match folder.as_ref() {
                            Some(f) => f.clone(),
                            None => SessionPersistenceConfig::default_json_persistence_folder()?,
                        };

                        let s = Arc::new(
                            JsonSessionPersistenceStore::new(folder, spawner)
                                .await
                                .context("error initializing JsonSessionPersistenceStore")?,
                        );

                        make_result!(s)
                    }
                    #[cfg(feature = "postgres")]
                    Some(SessionPersistenceConfig::Postgres { connection_string }) => {
                        use crate::session_persistence::postgres::PostgresSessionStorage;
                        let p = Arc::new(PostgresSessionStorage::new(connection_string).await?);
                        make_result!(p)
                    }
                    None => Ok((None, Arc::new(NonPersistentBitVFactory {}))),
                }
            }

            const DEFAULT_BLOCKING_THREADS_IF_NOT_SET: usize = 8;
            let spawner = BlockingSpawner::new(
                opts.runtime_worker_threads
                    .unwrap_or(DEFAULT_BLOCKING_THREADS_IF_NOT_SET),
            );

            let (persistence, bitv_factory) = persistence_factory(&opts, spawner.clone())
                .await
                .context("error initializing session persistence store")?;

            let proxy_url = opts.connect.as_ref().and_then(|s| s.proxy_url.as_ref());
            let proxy_config = match proxy_url {
                Some(pu) => Some(
                    SocksProxyConfig::parse(pu)
                        .with_context(|| format!("error parsing proxy url {pu}"))?,
                ),
                None => None,
            };

            let reqwest_client = {
                let builder = if let Some(proxy_url) = proxy_url {
                    let proxy = reqwest::Proxy::all(proxy_url)
                        .context("error creating socks5 proxy for HTTP")?;
                    reqwest::Client::builder().proxy(proxy)
                } else {
                    #[allow(unused_mut)]
                    let mut b = reqwest::Client::builder();
                    #[cfg(not(windows))]
                    if let Some(bd) = opts.bind_device_name.as_ref() {
                        b = b.interface(bd);
                    }
                    b
                };

                builder.build().context("error building HTTP(S) client")?
            };

            let stream_connector = Arc::new(
                StreamConnector::new(StreamConnectorArgs {
                    enable_tcp: opts.connect.as_ref().map(|c| c.enable_tcp).unwrap_or(true),
                    socks_proxy_config: proxy_config,
                    utp_socket: listen_result.as_ref().and_then(|l| l.utp_socket.clone()),
                    bind_device: bind_device.clone(),
                    ipv4_only: opts.ipv4_only,
                })
                .await
                .context("error creating stream connector")?,
            );

            let blocklist = if let Some(blocklist_url) = opts.blocklist_url {
                info!(url = blocklist_url, "loading p2p blocklist");
                let bl = IpRanges::load_from_url(&blocklist_url)
                    .await
                    .with_context(|| format!("error reading blocklist from {blocklist_url}"))?;
                info!(len = bl.len(), "loaded blocklist");
                bl
            } else {
                IpRanges::default()
            };

            let allowlist = if let Some(allowlist_url) = opts.allowlist_url {
                info!(url = allowlist_url, "loading p2p allowlist");
                let al = IpRanges::load_from_url(&allowlist_url)
                    .await
                    .with_context(|| format!("error reading allowlist from {allowlist_url}"))?;
                info!(len = al.len(), "loaded allowlist");
                Some(al)
            } else {
                None
            };

            let udp_tracker_client = if has_proxy {
                warn!("UDP trackers disabled: SOCKS proxy configured");
                None
            } else {
                Some(
                    UdpTrackerClient::new(token.clone(), bind_device.as_ref())
                        .await
                        .context("error creating UDP tracker client")?,
                )
            };

            let category_manager = if let Some(p) = persistence.as_ref() {
                match p.load_categories().await {
                    Ok(cats) => CategoryManager::from_categories(cats),
                    Err(e) => {
                        warn!("error loading categories from persistence: {e:#}");
                        CategoryManager::new()
                    }
                }
            } else {
                CategoryManager::new()
            };

            let lsd = {
                if opts.disable_local_service_discovery || has_proxy {
                    if has_proxy && !opts.disable_local_service_discovery {
                        warn!("Local service discovery disabled: SOCKS proxy configured (LSD uses multicast which cannot be proxied)");
                    }
                    None
                } else {
                    LocalServiceDiscovery::new(LocalServiceDiscoveryOptions {
                        cancel_token: token.clone(),
                        bind_device: bind_device.as_ref(),
                        ..Default::default()
                    })
                    .await
                    .inspect_err(|e| warn!("error starting local service discovery: {e:#}"))
                    .ok()
                }
            };

            let resolved_peer_limit = opts.peer_limit.unwrap_or(200);
            let resolved_concurrent_init_limit = opts.concurrent_init_limit.unwrap_or(3);

            let session = Arc::new(Self {
                persistence,
                bitv_factory,
                peer_id,
                dht,
                peer_opts,
                spawner: spawner.clone(),
                output_folder: RwLock::new(default_output_folder),
                next_id: AtomicUsize::new(0),
                db: RwLock::new(Default::default()),
                _cancellation_token_drop_guard: token.clone().drop_guard(),
                cancellation_token: token,
                announce_port: listen_result.as_ref().and_then(|l| l.announce_port),
                listen_addr: listen_result.as_ref().map(|l| l.addr),
                default_storage_factory: opts.default_storage_factory,
                reqwest_client,
                connector: stream_connector,
                root_span: opts.root_span,
                stats: Arc::new(SessionStats::new()),
                concurrent_initialize_semaphore: Arc::new(tokio::sync::Semaphore::new(
                    resolved_concurrent_init_limit,
                )),
                udp_tracker_client,
                ratelimits: Limits::new(opts.ratelimits),
                peer_limit_atomic: AtomicUsize::new(resolved_peer_limit),
                concurrent_init_limit_atomic: AtomicUsize::new(resolved_concurrent_init_limit),
                ipv4_only: opts.ipv4_only,
                trackers: opts.trackers,
                disable_trackers: opts.disable_trackers,
                peer_limit: opts.peer_limit.or(Some(200)),

                #[cfg(feature = "disable-upload")]
                _disable_upload: opts.disable_upload,
                blocklist,
                allowlist,
                lsd,
                fastresume_validation_denom: opts.fastresume_validation_denom,
                category_manager,
                completed_folder: RwLock::new(opts.completed_folder),
            });

            if let Some(mut listen) = listen_result {
                if let Some(tcp) = listen.tcp_socket.take() {
                    session.spawn(
                        debug_span!(parent: session.rs(), "tcp_listen", addr = ?listen.addr),
                        "tcp_listen",
                        {
                            let this = session.clone();
                            async move { this.task_listener(tcp).await }
                        },
                    );
                }
                if let Some(utp) = listen.utp_socket.take() {
                    session.spawn(
                        debug_span!(parent: session.rs(), "utp_listen", addr = ?listen.addr),
                        "utp_listen",
                        {
                            let this = session.clone();
                            async move { this.task_listener(utp).await }
                        },
                    );
                }
                if listen.enable_upnp_port_forwarding
                    && let Some(announce_port) = listen.announce_port
                {
                    info!(port = announce_port, "starting UPnP port forwarder");
                    let bind_device = bind_device.clone();
                    session.spawn(
                        debug_span!(parent: session.rs(), "upnp_forward", port = announce_port),
                        "upnp_forward",
                        Self::task_upnp_port_forwarder(announce_port, bind_device),
                    );
                }
            }

            if let Some(persistence) = session.persistence.as_ref() {
                info!("will use {persistence:?} for session persistence");

                let mut ps = persistence.stream_all().await?;
                let mut added_all = false;
                let mut futs = FuturesUnordered::new();

                while !added_all || !futs.is_empty() {
                    // NOTE: this closure exists purely to workaround rustfmt screwing up when inlining it.
                    let add_torrent_span = |info_hash: &Id20| -> tracing::Span {
                        debug_span!(parent: session.rs(), "add_torrent", info_hash=?info_hash)
                    };
                    tokio::select! {
                        Some(res) = futs.next(), if !futs.is_empty() => {
                            if let Err(e) = res {
                                error!("error adding torrent to session: {e:#}");
                            }
                        }
                        st = ps.next(), if !added_all => {
                            match st {
                                Some(st) => {
                                    let (id, st) = st?;
                                    let span = add_torrent_span(st.info_hash());
                                    let (add_torrent, mut opts) = st.into_add_torrent()?;
                                    opts.preferred_id = Some(id);
                                    let fut = session.add_torrent(add_torrent, Some(opts));
                                    let fut = fut.instrument(span);
                                    futs.push(fut);
                                },
                                None => added_all = true
                            };
                        }
                    };
                }
            }

            session.start_speed_estimator_updater();

            Ok(session)
        }
        .boxed()
    }

    pub fn get_dht(&self) -> Option<&Dht> {
        self.dht.as_ref()
    }

    pub(super) fn merge_peer_opts(
        &self,
        other: Option<PeerConnectionOptions>,
    ) -> PeerConnectionOptions {
        let other = match other {
            Some(o) => o,
            None => self.peer_opts,
        };
        PeerConnectionOptions {
            connect_timeout: other.connect_timeout.or(self.peer_opts.connect_timeout),
            read_write_timeout: other
                .read_write_timeout
                .or(self.peer_opts.read_write_timeout),
            keep_alive_interval: other
                .keep_alive_interval
                .or(self.peer_opts.keep_alive_interval),
        }
    }

    /// Spawn a task in the context of the session.
    #[track_caller]
    pub fn spawn(
        &self,
        span: tracing::Span,
        name: impl Into<Cow<'static, str>>,
        fut: impl std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    ) {
        spawn_with_cancel(span, name, self.cancellation_token.clone(), fut);
    }

    pub(crate) fn rs(&self) -> Option<tracing::Id> {
        self.root_span.as_ref().and_then(|s| s.id())
    }

    /// Stop the session and all managed tasks.
    pub async fn stop(&self) {
        let torrents = self
            .db
            .read()
            .torrents
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for torrent in torrents {
            if let Err(e) = torrent.pause() {
                debug!("error pausing torrent: {e:#}");
            }
        }
        self.cancellation_token.cancel();
        // this sucks, but hopefully will be enough
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    /// Run a callback given the currently managed torrents.
    pub fn with_torrents<R>(
        &self,
        callback: impl Fn(&mut dyn Iterator<Item = (TorrentId, &ManagedTorrentHandle)>) -> R,
    ) -> R {
        callback(&mut self.db.read().torrents.iter().map(|(id, t)| (*id, t)))
    }

    /// Add a torrent to the session.
    #[inline(never)]
    pub fn add_torrent<'a>(
        self: &'a Arc<Self>,
        add: AddTorrent<'a>,
        opts: Option<AddTorrentOptions>,
    ) -> BoxFuture<'a, anyhow::Result<AddTorrentResponse>> {
        async move {
            let mut opts = opts.unwrap_or_default();
            let add_res = match add {
                AddTorrent::Url(magnet) if magnet.starts_with("magnet:") || magnet.len() == 40 => {
                    let magnet = librtbit_core::magnet::Magnet::parse(&magnet)
                        .context("provided path is not a valid magnet URL")?;
                    let info_hash = magnet
                        .as_id20()
                        .context("magnet link didn't contain a BTv1 infohash")?;
                    if let Some(so) = magnet.get_select_only() {
                        // Only overwrite opts.only_files if user didn't specify
                        if opts.only_files.is_none() {
                            opts.only_files = Some(so);
                        }
                    }

                    InternalAddResult {
                        info_hash,
                        trackers: magnet
                            .trackers
                            .into_iter()
                            .filter_map(|t| url::Url::parse(&t).ok())
                            .collect(),
                        metadata: None,
                        name: magnet.name,
                        web_seed_urls: Vec::new(),
                    }
                }
                other => {
                    let torrent = match other {
                        AddTorrent::Url(url)
                            if url.starts_with("http://") || url.starts_with("https://") =>
                        {
                            helpers::torrent_from_url(&self.reqwest_client, &url).await?
                        }
                        AddTorrent::Url(url) => {
                            bail!(
                                "unsupported URL {:?}. Supporting magnet:, http:, and https",
                                url
                            )
                        }
                        AddTorrent::TorrentFileBytes(bytes) => {
                            torrent_from_bytes(bytes).context("error decoding torrent")?
                        }
                    };

                    let mut trackers = torrent
                        .meta
                        .iter_announce()
                        .unique()
                        .filter_map(|tracker| match std::str::from_utf8(tracker.as_ref()) {
                            Ok(url) => Some(url.to_owned()),
                            Err(_) => {
                                warn!("cannot parse tracker url as utf-8, ignoring");
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    if let Some(custom_trackers) = opts.trackers.clone() {
                        trackers.extend(custom_trackers);
                    }

                    let web_seed_urls: Vec<String> = torrent
                        .meta
                        .url_list
                        .iter()
                        .flat_map(|ul| ul.iter())
                        .filter_map(|url| std::str::from_utf8(url.as_ref()).ok())
                        .map(|s| s.to_owned())
                        .collect();

                    InternalAddResult {
                        info_hash: torrent.meta.info_hash,
                        metadata: Some(TorrentMetadata::new(
                            torrent.meta.info.data.validate()?,
                            torrent.torrent_bytes,
                            torrent.meta.info.raw_bytes.0,
                        )?),
                        trackers: trackers
                            .iter()
                            .filter_map(|t| url::Url::parse(t).ok())
                            .collect(),
                        name: None,
                        web_seed_urls,
                    }
                }
            };

            self.add_torrent_internal(add_res, opts).await
        }
        .instrument(debug_span!(parent: self.rs(), "add_torrent"))
        .boxed()
    }

    fn get_default_subfolder_for_torrent(
        &self,
        info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
        magnet_name: Option<&str>,
    ) -> anyhow::Result<Option<PathBuf>> {
        let files = info
            .iter_file_details()
            .map(|fd| Ok((fd.filename.to_pathbuf(), fd.len)))
            .collect::<anyhow::Result<Vec<(PathBuf, u64)>>>()?;
        if files.len() < 2 {
            return Ok(None);
        }

        fn check_valid(pb: &Path) -> anyhow::Result<()> {
            if pb.components().any(|x| !matches!(x, Component::Normal(_))) {
                bail!("path traversal in torrent name detected")
            }
            Ok(())
        }

        if let Some(name) = info.name()
            && !name.is_empty()
        {
            let pb = PathBuf::from(name.as_ref());
            check_valid(&pb)?;
            return Ok(Some(pb));
        };
        if let Some(name) = magnet_name {
            let pb = PathBuf::from(name);
            check_valid(&pb)?;
            return Ok(Some(pb));
        }
        // Let the subfolder name be the longest filename
        let longest = files
            .iter()
            .max_by_key(|(_, l)| l)
            .unwrap()
            .0
            .file_stem()
            .context("can't determine longest filename")?;
        Ok::<_, anyhow::Error>(Some(PathBuf::from(longest)))
    }

    async fn add_torrent_internal(
        self: &Arc<Self>,
        add_res: InternalAddResult,
        mut opts: AddTorrentOptions,
    ) -> anyhow::Result<AddTorrentResponse> {
        let InternalAddResult {
            info_hash,
            metadata,
            trackers,
            name,
            web_seed_urls,
        } = add_res;

        let private = metadata.as_ref().is_some_and(|m| m.info.info().private);

        let make_peer_rx = || {
            self.make_peer_rx(
                info_hash,
                trackers.clone(),
                !opts.paused && !opts.list_only,
                opts.force_tracker_interval,
                opts.initial_peers.clone().unwrap_or_default(),
                private,
            )
        };

        let mut seen_peers = Vec::new();

        let (metadata, peer_rx) = {
            match metadata {
                Some(metadata) => {
                    let mut peer_rx = None;
                    if !opts.paused && !opts.list_only {
                        peer_rx = make_peer_rx();
                    }
                    (metadata, peer_rx)
                }
                None => {
                    let peer_rx = make_peer_rx().context(
                        "no known way to resolve peers (no DHT, no trackers, no initial_peers)",
                    )?;

                    const DEFAULT_MAGNET_TIMEOUT_SECS: u64 = 120;
                    let timeout_secs = opts
                        .magnet_resolution_timeout_secs
                        .unwrap_or(DEFAULT_MAGNET_TIMEOUT_SECS);

                    let resolve_fut =
                        self.resolve_magnet(info_hash, peer_rx, &trackers, opts.peer_opts);

                    let resolved_magnet = if timeout_secs == 0 {
                        resolve_fut.await?
                    } else {
                        tokio::time::timeout(Duration::from_secs(timeout_secs), resolve_fut)
                            .await
                            .context(format!(
                                "magnet metadata resolution timed out after {timeout_secs}s"
                            ))??
                    };

                    // Add back seen_peers into the peer stream, as we consumed some peers
                    // while resolving the magnet.
                    seen_peers = resolved_magnet.seen_peers.clone();
                    let peer_rx = Some(
                        merge_streams(
                            resolved_magnet.peer_rx,
                            futures::stream::iter(resolved_magnet.seen_peers),
                        )
                        .boxed(),
                    );
                    (resolved_magnet.metadata, peer_rx)
                }
            }
        };

        trace!("Torrent metadata: {:#?}", &metadata.info.info());

        let only_files = compute_only_files(
            &metadata.info,
            opts.only_files,
            opts.only_files_regex,
            opts.list_only,
        )?;

        let output_folder = match (opts.output_folder, opts.sub_folder) {
            (None, None) => self.output_folder.read().join(
                self.get_default_subfolder_for_torrent(&metadata.info, name.as_deref())?
                    .unwrap_or_default(),
            ),
            (Some(o), None) => PathBuf::from(o),
            (Some(_), Some(_)) => {
                bail!("you can't provide both output_folder and sub_folder")
            }
            (None, Some(s)) => self.output_folder.read().join(s),
        };

        if opts.list_only {
            return Ok(AddTorrentResponse::ListOnly(ListOnlyResponse {
                info_hash,
                info: metadata.info,
                only_files,
                output_folder,
                seen_peers,
                torrent_bytes: metadata.torrent_bytes,
            }));
        }

        let storage_factory = opts
            .storage_factory
            .take()
            .or_else(|| self.default_storage_factory.as_ref().map(|f| f.clone_box()))
            .unwrap_or_else(|| FilesystemStorageFactory::default().boxed());

        let id = if let Some(id) = opts.preferred_id {
            id
        } else if let Some(p) = self.persistence.as_ref() {
            p.next_id().await?
        } else {
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        };

        let _permit = self.spawner.semaphore().acquire_owned().await?;

        let (managed_torrent, metadata) = {
            let mut g = self.db.write();
            // Check for duplicate by ID or info_hash using the index
            let existing = g.torrents.get(&id).map(|t| (id, t.clone())).or_else(|| {
                g.get_by_info_hash(info_hash)
                    .map(|(eid, t)| (eid, t.clone()))
            });
            if let Some((id, handle)) = existing {
                return Ok(AddTorrentResponse::AlreadyManaged(id, handle));
            }

            let span = debug_span!(parent: self.rs(), "torrent", id);
            let peer_opts = self.merge_peer_opts(opts.peer_opts);
            let metadata = Arc::new(metadata);
            let minfo = Arc::new(ManagedTorrentShared {
                id,
                span,
                info_hash,
                trackers: trackers.into_iter().collect(),
                spawner: self.spawner.clone(),
                peer_id: self.peer_id,
                storage_factory,
                options: ManagedTorrentOptions {
                    force_tracker_interval: opts.force_tracker_interval,
                    peer_connect_timeout: peer_opts.connect_timeout,
                    peer_read_write_timeout: peer_opts.read_write_timeout,
                    allow_overwrite: opts.overwrite,
                    output_folder,
                    ratelimits: opts.ratelimits,
                    initial_peers: opts.initial_peers.clone().unwrap_or_default(),
                    peer_limit: opts.peer_limit.or(self.peer_limit),
                    #[cfg(feature = "disable-upload")]
                    _disable_upload: self._disable_upload,
                },
                connector: self.connector.clone(),
                session: Arc::downgrade(self),
                magnet_name: name,
                web_seed_urls,
                category: RwLock::new(opts.category.clone()),
                cancellation_token: parking_lot::Mutex::new(self.cancellation_token.child_token()),
            });

            let initializing = Arc::new(TorrentStateInitializing::new(
                minfo.clone(),
                metadata.clone(),
                only_files.clone(),
                self.spawner
                    .block_in_place(|| minfo.storage_factory.create_and_init(&minfo, &metadata))?,
                false,
            ));
            let handle = Arc::new(ManagedTorrent {
                locked: RwLock::new(ManagedTorrentLocked {
                    paused: opts.paused,
                    state: ManagedTorrentState::Initializing(initializing),
                    only_files,
                }),
                state_change_notify: Notify::new(),
                shared: minfo,
                metadata: ArcSwapOption::new(Some(metadata.clone())),
            });

            g.add_torrent(handle.clone(), id);
            (handle, metadata)
        };

        if let Some(p) = self.persistence.as_ref()
            && let Err(e) = p.store(id, &managed_torrent).await
        {
            self.db.write().remove_torrent(&id);
            return Err(e);
        }

        let _e = managed_torrent.shared.span.clone().entered();

        managed_torrent
            .start(peer_rx, opts.paused)
            .context("error starting torrent")?;

        if let Some(name) = metadata.info.name() {
            info!(?name, "added torrent");
        }

        // Spawn a completion watcher if a completed_folder is configured
        if self.completed_folder.read().is_some() {
            self.spawn_completion_watcher(id, managed_torrent.clone());
        }

        Ok(AddTorrentResponse::Added(id, managed_torrent))
    }

    pub fn get(&self, id: TorrentIdOrHash) -> Option<ManagedTorrentHandle> {
        let db = self.db.read();
        match id {
            TorrentIdOrHash::Id(id) => db.torrents.get(&id).cloned(),
            TorrentIdOrHash::Hash(info_hash) => {
                db.get_by_info_hash(info_hash).map(|(_, v)| v.clone())
            }
        }
    }

    pub async fn delete(&self, id: TorrentIdOrHash, delete_files: bool) -> anyhow::Result<()> {
        let id = match id {
            TorrentIdOrHash::Id(id) => id,
            TorrentIdOrHash::Hash(h) => self
                .db
                .read()
                .get_by_info_hash(h)
                .map(|(id, _)| id)
                .context("no such torrent in db")?,
        };
        let removed = self
            .db
            .write()
            .remove_torrent(&id)
            .with_context(|| format!("torrent with id {id} did not exist"))?;

        // Cancel the per-torrent token to stop any running init/check tasks.
        // This must happen before pause() so initializing torrents are promptly cancelled.
        removed.shared.cancel_token();

        if let Err(e) = removed.pause() {
            debug!("error pausing torrent before deletion: {e:#}")
        }

        let metadata = removed
            .metadata
            .load_full()
            .context("torrent metadata was not loaded")?;

        let storage = removed
            .with_state_mut(|s| match s.take() {
                ManagedTorrentState::Initializing(p) => p.files.take().ok(),
                ManagedTorrentState::Paused(p) => Some(p.files),
                ManagedTorrentState::Live(l) => l
                    .pause()
                    // inspect_err not available in 1.75
                    .map_err(|e| {
                        warn!(?id, "error pausing torrent: {e:#}");
                        e
                    })
                    .ok()
                    .map(|p| p.files),
                _ => None,
            })
            .map(Ok)
            .unwrap_or_else(|| {
                removed
                    .shared
                    .storage_factory
                    .create(removed.shared(), &metadata)
            });

        if let Some(p) = self.persistence.as_ref() {
            if let Err(e) = p.delete(id).await {
                error!(
                    ?id,
                    "error deleting torrent from persistence database: {e:#}"
                );
            } else {
                debug!(?id, "deleted torrent from persistence database")
            }
        }

        match (storage, delete_files) {
            (Err(e), true) => return Err(e).context("torrent deleted, but could not delete files"),
            (Ok(storage), true) => {
                debug!("will delete files");
                remove_files_and_dirs(&metadata.file_infos, &storage);
                if removed.shared().options.output_folder != *self.output_folder.read()
                    && let Err(e) = storage.remove_directory_if_empty(Path::new(""))
                {
                    warn!(
                        ?id,
                        "error removing {:?}: {e:#}",
                        removed.shared().options.output_folder
                    )
                }
            }
            (_, false) => {
                debug!("not deleting files")
            }
        };

        info!(id, "deleted torrent");
        Ok(())
    }

    pub(crate) async fn try_update_persistence_metadata(&self, handle: &ManagedTorrentHandle) {
        if let Some(p) = self.persistence.as_ref()
            && let Err(e) = p.update_metadata(handle.id(), handle).await
        {
            warn!(storage=?p, error=?e, "error updating metadata")
        }
    }

    pub async fn pause(&self, handle: &ManagedTorrentHandle) -> anyhow::Result<()> {
        handle.pause()?;
        self.try_update_persistence_metadata(handle).await;
        Ok(())
    }

    pub async fn unpause(self: &Arc<Self>, handle: &ManagedTorrentHandle) -> anyhow::Result<()> {
        let peer_rx = self.make_peer_rx_managed_torrent(handle, true);
        handle.start(peer_rx, false)?;
        self.try_update_persistence_metadata(handle).await;
        Ok(())
    }

    pub async fn update_only_files(
        self: &Arc<Self>,
        handle: &ManagedTorrentHandle,
        only_files: &HashSet<usize>,
    ) -> anyhow::Result<()> {
        handle.update_only_files(only_files)?;
        self.try_update_persistence_metadata(handle).await;
        Ok(())
    }

    /// Get the configured completed folder.
    pub fn completed_folder(&self) -> Option<PathBuf> {
        self.completed_folder.read().clone()
    }

    /// Set the completed folder (move-on-complete destination).
    pub fn set_completed_folder(&self, folder: Option<PathBuf>) {
        *self.completed_folder.write() = folder;
    }

    /// Get the default download/output folder.
    pub fn output_folder(&self) -> PathBuf {
        self.output_folder.read().clone()
    }

    /// Set the default download/output folder.
    pub fn set_output_folder(&self, folder: PathBuf) {
        *self.output_folder.write() = folder;
    }

    /// Spawn a task that watches for a torrent to complete and moves its files
    /// to the completed folder.
    fn spawn_completion_watcher(self: &Arc<Self>, id: TorrentId, handle: ManagedTorrentHandle) {
        let session = Arc::downgrade(self);
        let handle_weak = Arc::downgrade(&handle);
        let span = handle.shared.span.clone();
        self.spawn(
            debug_span!(parent: span, "completion_watcher", id),
            format!("[{id}]completion_watcher"),
            async move {
                // Wait for the torrent to finish downloading
                if let Err(e) = handle.wait_until_completed().await {
                    debug!(id, "torrent errored while waiting for completion: {e:#}");
                    return Ok(());
                }

                let session = match session.upgrade() {
                    Some(s) => s,
                    None => return Ok(()),
                };

                let completed_folder = match session.completed_folder.read().clone() {
                    Some(f) => f,
                    None => return Ok(()),
                };

                let handle = match handle_weak.upgrade() {
                    Some(h) => h,
                    None => return Ok(()),
                };

                info!(id, target_folder=?completed_folder, "torrent completed, moving to completed folder");

                if let Err(e) = session
                    .move_completed_torrent(id, &handle, completed_folder)
                    .await
                {
                    error!(id, "error moving completed torrent: {e:#}");
                }
                Ok(())
            },
        );
    }

    /// Move all files of a completed torrent from its current output folder
    /// to the target folder.
    async fn move_completed_torrent(
        &self,
        id: TorrentId,
        handle: &ManagedTorrentHandle,
        target_folder: PathBuf,
    ) -> anyhow::Result<()> {
        let current_output = handle.shared().options.output_folder.clone();

        // Don't move if already in the target folder
        if current_output == target_folder {
            debug!(
                id,
                "torrent is already in the completed folder, skipping move"
            );
            return Ok(());
        }

        let metadata = handle
            .metadata
            .load_full()
            .context("torrent metadata not loaded")?;

        // Determine the subfolder name: if the torrent's output_folder differs from
        // the session's default output_folder, it means there's a subfolder (e.g. for
        // multi-file torrents). We want to replicate the relative structure under the
        // completed folder.
        let relative_path = current_output
            .strip_prefix(&*self.output_folder.read())
            .unwrap_or(Path::new(""));

        let dest_base = if relative_path == Path::new("") {
            target_folder.clone()
        } else {
            target_folder.join(relative_path)
        };

        // Create the destination base directory
        tokio::fs::create_dir_all(&dest_base)
            .await
            .with_context(|| format!("error creating completed directory {dest_base:?}"))?;

        // Move each file
        for fi in metadata.file_infos.iter() {
            if fi.attrs.padding {
                continue;
            }

            let src = current_output.join(&fi.relative_filename);
            let dst = dest_base.join(&fi.relative_filename);

            // Create parent directories for the destination if needed
            if let Some(parent) = dst.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("error creating directory {parent:?}"))?;
            }

            // Try rename first (fast, same filesystem)
            match tokio::fs::rename(&src, &dst).await {
                Ok(()) => {
                    debug!(id, ?src, ?dst, "moved file via rename");
                }
                Err(rename_err) => {
                    // Rename failed (likely cross-device), fall back to copy + delete
                    debug!(
                        id,
                        ?src,
                        ?dst,
                        "rename failed ({rename_err}), falling back to copy+delete"
                    );
                    tokio::fs::copy(&src, &dst)
                        .await
                        .with_context(|| format!("error copying {src:?} to {dst:?}"))?;
                    tokio::fs::remove_file(&src).await.with_context(|| {
                        format!("error removing source file {src:?} after copy")
                    })?;
                    debug!(id, ?src, ?dst, "moved file via copy+delete");
                }
            }
        }

        // Try to clean up empty source directories
        if current_output != *self.output_folder.read()
            && let Err(e) = tokio::fs::remove_dir(&current_output).await
        {
            debug!(
                id,
                ?current_output,
                "could not remove source directory (may not be empty): {e}"
            );
        }

        info!(id, ?dest_base, "completed torrent moved successfully");

        // Note: we don't update the torrent's output_folder or persistence here because
        // the torrent is already finished. The files are now in the completed folder.
        // If we wanted to update persistence for the new location, the ManagedTorrentOptions
        // output_folder would need to be mutable (wrapped in RwLock), which is a larger change.
        // For now, the move is fire-and-forget from the torrent's perspective.

        Ok(())
    }

    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.listen_addr
    }

    pub fn announce_port(&self) -> Option<u16> {
        self.announce_port
    }

    pub fn categories(&self) -> &CategoryManager {
        &self.category_manager
    }

    pub fn set_torrent_category(
        &self,
        id: TorrentIdOrHash,
        category: Option<String>,
    ) -> anyhow::Result<()> {
        let handle = self.get(id).context("torrent not found")?;
        *handle.shared().category.write() = category;
        Ok(())
    }

    pub async fn persist_categories(&self) -> anyhow::Result<()> {
        if let Some(p) = self.persistence.as_ref() {
            let snapshot = self.category_manager.snapshot();
            p.store_categories(&snapshot).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use buffers::ByteBuf;
    use itertools::Itertools;
    use librtbit_core::torrent_metainfo::{TorrentMetaV1, torrent_from_bytes};

    use super::torrent_file_from_info_bytes;

    #[test]
    fn test_torrent_file_from_info_and_bytes() {
        fn get_trackers(info: &TorrentMetaV1<ByteBuf>) -> Vec<url::Url> {
            info.iter_announce()
                .filter_map(|t| std::str::from_utf8(t.as_ref()).ok().map(|t| t.to_owned()))
                .filter_map(|t| t.parse().ok())
                .collect_vec()
        }

        let orig_full_torrent =
            include_bytes!("../../resources/ubuntu-21.04-desktop-amd64.iso.torrent");
        let parsed = torrent_from_bytes(&orig_full_torrent[..]).unwrap();
        let parsed_trackers = get_trackers(&parsed);

        let generated_torrent =
            torrent_file_from_info_bytes(parsed.info.raw_bytes.as_ref(), &parsed_trackers).unwrap();
        let generated_parsed = torrent_from_bytes(generated_torrent.as_ref()).unwrap();
        assert_eq!(parsed.info_hash, generated_parsed.info_hash);
        assert_eq!(parsed.info, generated_parsed.info);
        assert_eq!(parsed_trackers, get_trackers(&generated_parsed));
    }

    /// Test that magnet resolution respects the configured timeout and returns
    /// an appropriate error when it expires. Uses a magnet with unreachable
    /// initial peers so resolution will hang until the timeout.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_magnet_resolution_timeout() {
        use std::net::SocketAddr;
        use std::str::FromStr;
        use std::time::Instant;

        use crate::{AddTorrent, AddTorrentOptions, SessionOptions};

        let tempdir = tempfile::TempDir::new().unwrap();
        let session = super::Session::new_with_opts(
            tempdir.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_trackers: true,
                disable_local_service_discovery: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // A random info hash with no real peers -- resolution will hang.
        let magnet = "magnet:?xt=urn:btih:0000000000000000000000000000000000000001";

        let start = Instant::now();
        let result = session
            .add_torrent(
                AddTorrent::from_url(magnet),
                Some(AddTorrentOptions {
                    // Set a short timeout so the test doesn't wait long.
                    magnet_resolution_timeout_secs: Some(2),
                    // Provide an unreachable peer so peer discovery doesn't
                    // immediately fail with "no known way to resolve peers".
                    initial_peers: Some(vec![SocketAddr::from_str("192.0.2.1:6881").unwrap()]),
                    ..Default::default()
                }),
            )
            .await;

        let elapsed = start.elapsed();
        assert!(result.is_err(), "expected a timeout error");
        let err_msg = format!("{:#}", result.err().unwrap());
        assert!(
            err_msg.contains("magnet metadata resolution timed out"),
            "error message should mention timeout, got: {err_msg}"
        );
        // Verify the timeout was roughly respected (within a generous margin).
        assert!(
            elapsed.as_secs() < 10,
            "should have timed out quickly, took {elapsed:?}"
        );
    }

    /// Test that the concurrent init limit is configurable: setting it to a
    /// custom value creates a semaphore with the correct number of permits.
    #[tokio::test]
    async fn test_concurrent_init_limit_configurable() {
        use crate::SessionOptions;

        let tempdir = tempfile::TempDir::new().unwrap();

        // Create session with a custom concurrent init limit.
        let session = super::Session::new_with_opts(
            tempdir.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_trackers: true,
                disable_local_service_discovery: true,
                concurrent_init_limit: Some(5),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // The semaphore should have 5 available permits.
        assert_eq!(
            session.concurrent_initialize_semaphore.available_permits(),
            5,
            "semaphore should have 5 permits"
        );

        // Default (no explicit limit) should give 3 permits.
        let session_default = super::Session::new_with_opts(
            tempdir.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_trackers: true,
                disable_local_service_discovery: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            session_default
                .concurrent_initialize_semaphore
                .available_permits(),
            3,
            "default semaphore should have 3 permits"
        );
    }

    /// Test that when the init semaphore is full and a permit is dropped
    /// (e.g. when a torrent finishes init or is deleted), the slot is freed
    /// so that a waiting task can proceed.
    #[tokio::test]
    async fn test_init_semaphore_doesnt_deadlock() {
        use std::sync::Arc;
        use std::time::Duration;

        // Create a semaphore with 1 permit (simulating concurrent_init_limit = 1).
        let sem = Arc::new(tokio::sync::Semaphore::new(1));

        // Acquire the only permit.
        let permit = sem.clone().acquire_owned().await.unwrap();

        // Spawn a task that tries to acquire the semaphore -- it should block.
        let sem2 = sem.clone();
        let acquired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let acquired2 = acquired.clone();
        let handle = tokio::spawn(async move {
            let _p = sem2.acquire().await.unwrap();
            acquired2.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        // Give the spawned task a moment to start waiting.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !acquired.load(std::sync::atomic::Ordering::SeqCst),
            "second acquire should be blocked"
        );

        // Drop the permit (simulating torrent deletion freeing the slot).
        drop(permit);

        // The spawned task should now be able to acquire the permit.
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("task should complete after permit is freed")
            .unwrap();

        assert!(
            acquired.load(std::sync::atomic::Ordering::SeqCst),
            "second acquire should have succeeded after permit was dropped"
        );
    }
}
