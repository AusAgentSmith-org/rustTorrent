pub mod initializing;
pub mod live;
pub mod paused;
pub mod stats;
mod streaming;
pub mod utils;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use anyhow::bail;
use arc_swap::ArcSwapOption;
use buffers::ByteBufOwned;
use bytes::Bytes;
use futures::FutureExt;
use futures::future::BoxFuture;
use librtbit_core::hash_id::Id20;
use librtbit_core::lengths::Lengths;

use librtbit_core::spawn_utils::spawn_with_cancel;
use librtbit_core::torrent_metainfo::ValidatedTorrentMetaV1Info;
pub use live::*;
use parking_lot::RwLock;

use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use tracing::debug_span;
use tracing::trace;
use tracing::warn;

use crate::Session;
use crate::chunk_tracker::ChunkTracker;
use crate::file_info::FileInfo;
use crate::limits::LimitsConfig;
use crate::session::TorrentId;
use crate::spawn_utils::BlockingSpawner;
use crate::storage::BoxStorageFactory;
use crate::stream_connect::StreamConnector;
use crate::torrent_state::stats::LiveStats;
use crate::type_aliases::FileInfos;
use crate::type_aliases::PeerStream;

use initializing::TorrentStateInitializing;

use self::paused::TorrentStatePaused;
pub use self::stats::{TorrentStats, TorrentStatsState};
pub use self::streaming::FileStream;

// State machine transitions.
//
// - magnet_pending -> initializing (after magnet metadata resolved)
// - magnet_pending -> error (resolution failed/timed out)
// - error -> initializing
// - initializing -> paused
// - paused -> live
// - live -> paused
//
// - initializing -> error
// - live -> error
pub enum ManagedTorrentState {
    /// Waiting for magnet metadata to be resolved via DHT/peers.
    MagnetPending,
    Initializing(Arc<TorrentStateInitializing>),
    Paused(TorrentStatePaused),
    Live(Arc<TorrentStateLive>),
    Error(anyhow::Error),

    // This is used when swapping between states, outside world should never see it.
    None,
}

impl ManagedTorrentState {
    pub fn name(&self) -> &'static str {
        match self {
            ManagedTorrentState::MagnetPending => "magnet_pending",
            ManagedTorrentState::Initializing(_) => "initializing",
            ManagedTorrentState::Paused(_) => "paused",
            ManagedTorrentState::Live(_) => "live",
            ManagedTorrentState::Error(_) => "error",
            ManagedTorrentState::None => "<invalid: none>",
        }
    }

    fn assert_paused(self) -> TorrentStatePaused {
        match self {
            Self::Paused(paused) => paused,
            _ => panic!("Expected paused state"),
        }
    }

    pub(crate) fn take(&mut self) -> Self {
        std::mem::replace(self, Self::None)
    }
}

pub(crate) struct ManagedTorrentLocked {
    // The torrent might not be in "paused" state technically,
    // but the intention might be for it to stay paused.
    //
    // This should change only on "unpause".
    pub(crate) paused: bool,
    pub(crate) state: ManagedTorrentState,
    pub(crate) only_files: Option<Vec<usize>>,
}

#[derive(Default)]
pub(crate) struct ManagedTorrentOptions {
    pub force_tracker_interval: Option<Duration>,
    pub peer_connect_timeout: Option<Duration>,
    pub peer_read_write_timeout: Option<Duration>,
    pub allow_overwrite: bool,
    pub output_folder: PathBuf,
    pub ratelimits: LimitsConfig,
    pub initial_peers: Vec<SocketAddr>,
    pub peer_limit: Option<usize>,
    #[cfg(feature = "disable-upload")]
    pub _disable_upload: bool,
}

impl ManagedTorrentOptions {
    #[cfg(feature = "disable-upload")]
    pub fn disable_upload(&self) -> bool {
        self._disable_upload
    }

    #[cfg(not(feature = "disable-upload"))]
    pub const fn disable_upload(&self) -> bool {
        false
    }
}

// Torrent bencodee "info" + some precomputed fields based on it for frequent access.
pub struct TorrentMetadata {
    pub info: ValidatedTorrentMetaV1Info<ByteBufOwned>,
    pub torrent_bytes: Bytes,
    pub info_bytes: Bytes,
    pub file_infos: FileInfos,
}

impl TorrentMetadata {
    pub(crate) fn new(
        info: ValidatedTorrentMetaV1Info<ByteBufOwned>,
        torrent_bytes: Bytes,
        info_bytes: Bytes,
    ) -> anyhow::Result<Self> {
        let file_infos = info
            .iter_file_details_ext()
            .map(|fd| {
                Ok::<_, anyhow::Error>(FileInfo {
                    relative_filename: fd.details.filename.to_pathbuf(),
                    offset_in_torrent: fd.offset,
                    piece_range: fd.pieces,
                    len: fd.details.len,
                    attrs: fd.details.attrs(),
                })
            })
            .collect::<anyhow::Result<Vec<FileInfo>>>()?;

        Ok(Self {
            info,
            torrent_bytes,
            info_bytes,
            file_infos,
        })
    }

    pub fn lengths(&self) -> &Lengths {
        self.info.lengths()
    }
}

/// Common information about torrent shared among all possible states.
///
// The reason it's not inlined into ManagedTorrent is to break the Arc cycle:
// ManagedTorrent contains the current torrent state, which in turn needs access to a bunch
// of stuff, but it shouldn't access the state.
pub struct ManagedTorrentShared {
    pub id: TorrentId,
    pub info_hash: Id20,
    pub(crate) spawner: BlockingSpawner,
    pub trackers: HashSet<url::Url>,
    pub peer_id: Id20,
    pub span: tracing::Span,
    pub(crate) options: ManagedTorrentOptions,
    pub(crate) connector: Arc<StreamConnector>,
    pub(crate) storage_factory: BoxStorageFactory,
    pub(crate) session: Weak<Session>,

    // "dn" from magnet link
    pub(crate) magnet_name: Option<String>,

    /// BEP 19: WebSeed URLs parsed from the torrent's url-list field.
    pub web_seed_urls: Vec<String>,

    /// Category assigned to this torrent.
    pub category: RwLock<Option<String>>,

    /// Per-torrent cancellation token, child of session token.
    /// Used to cancel init tasks, checking, etc. when the torrent is deleted/forgotten.
    /// Wrapped in a Mutex so it can be reset after a pause-during-init.
    pub(crate) cancellation_token: parking_lot::Mutex<CancellationToken>,
}

impl ManagedTorrentShared {
    /// Cancel the per-torrent token and replace it with a fresh child of the session token.
    /// Returns the old (now-cancelled) token. Used when pausing during initialization.
    pub(crate) fn cancel_and_reset_token(&self) {
        let mut guard = self.cancellation_token.lock();
        guard.cancel();
        if let Some(session) = self.session.upgrade() {
            *guard = session.cancellation_token().child_token();
        }
    }

    /// Cancel the per-torrent token permanently. Used when deleting/forgetting a torrent.
    pub(crate) fn cancel_token(&self) {
        self.cancellation_token.lock().cancel();
    }

    /// Create a child token from the per-torrent token for spawning tasks.
    pub(crate) fn child_token(&self) -> CancellationToken {
        self.cancellation_token.lock().child_token()
    }
}

pub struct ManagedTorrent {
    // Static torrent configuration that doesn't change.
    pub shared: Arc<ManagedTorrentShared>,
    // Torrent metadata. Maybe be None when the magnet is resolving (not implemented yet)
    pub metadata: ArcSwapOption<TorrentMetadata>,
    pub(crate) state_change_notify: Notify,
    pub(crate) locked: RwLock<ManagedTorrentLocked>,
}

impl ManagedTorrent {
    pub fn id(&self) -> TorrentId {
        self.shared.id
    }

    pub fn name(&self) -> Option<String> {
        if let Some(m) = &*self.metadata.load() {
            return m
                .info
                .name()
                .map(|n| n.into_owned())
                .or_else(|| self.shared.magnet_name.clone());
        }
        self.shared.magnet_name.clone()
    }

    pub fn shared(&self) -> &ManagedTorrentShared {
        &self.shared
    }

    pub fn with_metadata<R>(
        &self,
        mut f: impl FnMut(&Arc<TorrentMetadata>) -> R,
    ) -> anyhow::Result<R> {
        let r = self.metadata.load();
        let r = r.as_ref().context("torrent is not resolved")?;
        Ok(f(r))
    }

    pub fn info_hash(&self) -> Id20 {
        self.shared.info_hash
    }

    pub fn only_files(&self) -> Option<Vec<usize>> {
        self.locked.read().only_files.clone()
    }

    pub fn with_state<R>(&self, f: impl FnOnce(&ManagedTorrentState) -> R) -> R {
        f(&self.locked.read().state)
    }

    pub(crate) fn with_state_mut<R>(&self, f: impl FnOnce(&mut ManagedTorrentState) -> R) -> R {
        f(&mut self.locked.write().state)
    }

    pub(crate) fn with_chunk_tracker<R>(
        &self,
        f: impl FnOnce(&ChunkTracker) -> R,
    ) -> anyhow::Result<R> {
        let g = self.locked.read();
        match &g.state {
            ManagedTorrentState::Paused(p) => Ok(f(&p.chunk_tracker)),
            ManagedTorrentState::Live(l) => Ok(f(l
                .lock_read("chunk_tracker")
                .get_chunks()
                .context("error getting chunks")?)),
            _ => bail!("no chunk tracker, torrent neither paused nor live"),
        }
    }

    /// Get the live state if the torrent is live.
    pub fn live(&self) -> Option<Arc<TorrentStateLive>> {
        let g = self.locked.read();
        match &g.state {
            ManagedTorrentState::Live(live) => Some(live.clone()),
            _ => None,
        }
    }

    // Get live torrent but wait a bit until it's initialized if it is
    pub(crate) async fn live_wait_initializing(
        &self,
        duration: Duration,
    ) -> Option<Arc<TorrentStateLive>> {
        timeout(duration, self.wait_until_initialized())
            .await
            .ok()?
            .ok()?;
        self.live()
    }

    pub(crate) fn stop_with_error(&self, error: anyhow::Error) {
        let mut g = self.locked.write();

        match g.state.take() {
            ManagedTorrentState::Live(live) => {
                if let Err(err) = live.pause() {
                    warn!(
                        id = self.shared.id,
                        info_hash = ?self.shared.info_hash,
                        "error pausing live torrent during fatal error handling: {err:#}",
                    );
                }
            }
            ManagedTorrentState::Error(e) => {
                warn!(
                    id = self.shared.id,
                    info_hash = ?self.shared.info_hash,
                    "bug: torrent already was in error state when trying to stop it. Previous error was: {e:#}",
                );
            }
            ManagedTorrentState::None => {
                warn!(
                    id = self.shared.id,
                    info_hash = ?self.shared.info_hash,
                    "bug: torrent encountered in None state during fatal error handling"
                )
            }
            _ => {}
        };

        self.state_change_notify.notify_waiters();

        g.state = ManagedTorrentState::Error(error)
    }

    /// peer_rx: the peer stream. If start_paused=false, must be set.
    /// start_paused: if set, the torrent will initialize (check file integrity), but will not start
    pub(crate) fn start(
        self: &Arc<Self>,
        peer_rx: Option<PeerStream>,
        start_paused: bool,
    ) -> anyhow::Result<()> {
        fn _start<'a>(
            t: &'a Arc<ManagedTorrent>,
            peer_rx: Option<PeerStream>,
            start_paused: bool,
            session: Arc<Session>,
            g: Option<parking_lot::RwLockWriteGuard<'a, ManagedTorrentLocked>>,
            token: CancellationToken,
        ) -> anyhow::Result<()> {
            let mut g = g.unwrap_or_else(|| t.locked.write());

            match &g.state {
                ManagedTorrentState::Live(_) => {
                    bail!("torrent is already live");
                }
                ManagedTorrentState::MagnetPending => {
                    // Magnet is still resolving — nothing to start yet.
                    // The background resolver will call start() once metadata arrives.
                    return Ok(());
                }
                ManagedTorrentState::Initializing(init) => {
                    let init = init.clone();
                    let t = t.clone();
                    let span = t.shared().span.clone();
                    let token = token.clone();

                    spawn_with_cancel(
                        debug_span!(parent: span.clone(), "initialize_and_start"),
                        "initialize_and_start",
                        token.clone(),
                        async move {
                            let concurrent_init_semaphore =
                                session.concurrent_initialize_semaphore.clone();
                            init.queued_for_init
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            let _permit = concurrent_init_semaphore
                                .acquire()
                                .await
                                .context("bug: concurrent init semaphore was closed")?;
                            init.queued_for_init
                                .store(false, std::sync::atomic::Ordering::Relaxed);

                            match init.check().await {
                                Ok(paused) => {
                                    let mut g = t.locked.write();
                                    if let ManagedTorrentState::Initializing(_) = &g.state {
                                    } else {
                                        debug!(
                                            "no need to start torrent anymore, as it switched state from initializing"
                                        );
                                        return Ok(());
                                    }

                                    g.state = ManagedTorrentState::Paused(paused);
                                    t.state_change_notify.notify_waiters();
                                    _start(&t, peer_rx, start_paused, session, Some(g), token)
                                }
                                Err(err) => {
                                    let result = anyhow::anyhow!("{:?}", err);
                                    t.locked.write().state = ManagedTorrentState::Error(err);
                                    t.state_change_notify.notify_waiters();
                                    Err(result)
                                }
                            }
                        },
                    );
                    Ok(())
                }
                ManagedTorrentState::Paused(_) => {
                    if start_paused {
                        return Ok(());
                    }
                    let paused = g.state.take().assert_paused();
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let live = TorrentStateLive::new(paused, tx, token.clone())?;
                    g.state = ManagedTorrentState::Live(live.clone());
                    t.state_change_notify.notify_waiters();

                    spawn_fatal_errors_receiver(t, rx, token);
                    if let Some(peer_rx) = peer_rx {
                        spawn_peer_adder(&live, peer_rx);
                    }
                    Ok(())
                }
                ManagedTorrentState::Error(_) => {
                    let metadata = t
                        .metadata
                        .load_full()
                        .context("torrent metadata was not loaded")?;
                    // Use false: fast resume validation already checks bitfield size,
                    // validates at least 1 piece per file, and probabilistically checks
                    // remaining pieces. Passing true would force a full disk recheck
                    // even for transient errors (network, tracker timeouts).
                    let initializing = Arc::new(TorrentStateInitializing::new(
                        t.shared.clone(),
                        metadata.clone(),
                        g.only_files.clone(),
                        t.shared
                            .storage_factory
                            .create_and_init(t.shared(), &metadata)?,
                        false,
                    ));
                    g.state = ManagedTorrentState::Initializing(initializing.clone());
                    t.state_change_notify.notify_waiters();

                    // Recurse.
                    _start(t, peer_rx, start_paused, session, Some(g), token)
                }
                ManagedTorrentState::None => bail!("bug: torrent is in empty state"),
            }
        }

        let session = self
            .shared
            .session
            .upgrade()
            .context("session is dead, cannot start torrent")?;
        let mut g = self.locked.write();
        g.paused = start_paused;
        let cancellation_token = self.shared.child_token();

        _start(
            self,
            peer_rx,
            start_paused,
            session,
            Some(g),
            cancellation_token,
        )
    }

    pub fn is_paused(&self) -> bool {
        self.locked.read().paused
    }

    /// Pause the torrent if it's live or initializing.
    pub(crate) fn pause(&self) -> anyhow::Result<()> {
        let mut g = self.locked.write();
        match &g.state {
            ManagedTorrentState::Live(live) => {
                let paused = live.pause()?;
                g.state = ManagedTorrentState::Paused(paused);
                g.paused = true;
                self.state_change_notify.notify_waiters();
                Ok(())
            }
            ManagedTorrentState::Initializing(_) => {
                // Cancel the init task via the per-torrent cancellation token,
                // then reset the token so the torrent can be restarted later.
                // Transition to Error state so it can be re-initialized on unpause.
                self.shared.cancel_and_reset_token();
                g.state =
                    ManagedTorrentState::Error(anyhow::anyhow!("paused during initialization"));
                g.paused = true;
                self.state_change_notify.notify_waiters();
                Ok(())
            }
            ManagedTorrentState::Paused(_) => {
                bail!("torrent is already paused");
            }
            ManagedTorrentState::MagnetPending => {
                // Cancel the magnet resolution task, mark as paused.
                self.shared.cancel_and_reset_token();
                g.state =
                    ManagedTorrentState::Error(anyhow::anyhow!("paused during magnet resolution"));
                g.paused = true;
                self.state_change_notify.notify_waiters();
                Ok(())
            }
            ManagedTorrentState::Error(_) => {
                bail!("can't pause torrent in error state")
            }
            ManagedTorrentState::None => bail!("bug: torrent is in empty state"),
        }
    }

    /// Get stats.
    pub fn stats(&self) -> TorrentStats {
        use stats::TorrentStatsState as S;
        let mut resp = TorrentStats {
            total_bytes: self
                .metadata
                .load()
                .as_ref()
                .map(|r| r.info.lengths().total_length())
                .unwrap_or_default(),
            file_progress: Vec::new(),
            state: S::Error,
            error: None,
            progress_bytes: 0,
            uploaded_bytes: 0,
            finished: false,
            live: None,
            queued_for_init: None,
        };

        self.with_state(|s| {
            match s {
                ManagedTorrentState::MagnetPending => {
                    resp.state = S::MagnetPending;
                }
                ManagedTorrentState::Initializing(i) => {
                    resp.state = S::Initializing;
                    resp.progress_bytes = i.checked_bytes.load(Ordering::Relaxed);
                    resp.queued_for_init = Some(i.is_queued_for_init());
                }
                ManagedTorrentState::Paused(p) => {
                    resp.state = S::Paused;
                    let hns = p.hns();
                    resp.total_bytes = hns.total();
                    resp.progress_bytes = hns.progress();
                    resp.finished = hns.finished();
                    resp.file_progress = p.chunk_tracker.per_file_have_bytes().to_owned();
                }
                ManagedTorrentState::Live(l) => {
                    resp.state = S::Live;
                    let live_stats = LiveStats::from(l.as_ref());
                    let hns = l.get_hns().unwrap_or_default();
                    resp.total_bytes = hns.total();
                    resp.progress_bytes = hns.progress();
                    resp.finished = hns.finished();
                    resp.uploaded_bytes = l.get_uploaded_bytes();
                    resp.file_progress = l
                        .lock_read("file_progress")
                        .get_chunks()
                        .ok()
                        .map(|c| c.per_file_have_bytes().to_owned())
                        .unwrap_or_default();
                    resp.live = Some(live_stats);
                }
                ManagedTorrentState::Error(e) => {
                    resp.state = S::Error;
                    resp.error = Some(format!("{e:?}"))
                }
                ManagedTorrentState::None => {
                    resp.state = S::Error;
                    resp.error = Some("bug: torrent in broken \"None\" state".to_string());
                }
            }
            resp
        })
    }

    #[inline(never)]
    pub fn wait_until_initialized(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        async move {
            // TODO: rewrite, this polling is horrible
            loop {
                let done = self.with_state(|s| match s {
                    ManagedTorrentState::MagnetPending
                    | ManagedTorrentState::Initializing(_) => Ok(false),
                    ManagedTorrentState::Error(e) => bail!("{:?}", e),
                    ManagedTorrentState::None => bail!("bug: torrent state is None"),
                    _ => Ok(true),
                })?;
                if done {
                    return Ok(());
                }
                let _ = timeout(
                    Duration::from_millis(100),
                    self.state_change_notify.notified(),
                )
                .await;
            }
        }
        .boxed()
    }

    #[inline(never)]
    pub fn wait_until_completed(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        async move {
            // TODO: rewrite, this polling is horrible
            let live = loop {
                let live = self.with_state(|s| match s {
                    ManagedTorrentState::MagnetPending
                    | ManagedTorrentState::Initializing(_)
                    | ManagedTorrentState::Paused(_) => {
                        Ok(None)
                    }
                    ManagedTorrentState::Live(l) => Ok(Some(l.clone())),
                    ManagedTorrentState::Error(e) => bail!("{:?}", e),
                    ManagedTorrentState::None => bail!("bug: torrent state is None"),
                })?;
                if let Some(live) = live {
                    break live;
                }
                let _ = timeout(Duration::from_secs(1), self.state_change_notify.notified()).await;
            };

            live.wait_until_completed().await;
            Ok(())
        }
        .boxed()
    }

    // Returns true if needed to unpause torrent.
    // This is just implementation detail - it's easier to pause/unpause than to tinker with internals.
    pub(crate) fn update_only_files(&self, only_files: &HashSet<usize>) -> anyhow::Result<()> {
        let metadata = self.metadata.load();
        let metadata = metadata.as_ref().context("torrent is not resolved")?;
        let file_count = metadata.file_infos.len();
        for f in only_files.iter().copied() {
            if f >= file_count {
                anyhow::bail!("only_files contains invalid value {f}")
            }
        }

        // if live, need to update chunk tracker
        // - if already finished: need to pause, then unpause (to reopen files etc)
        // if paused, need to update chunk tracker

        let mut g = self.locked.write();
        match &mut g.state {
            ManagedTorrentState::MagnetPending => bail!("can't update torrent while magnet is resolving"),
            ManagedTorrentState::Initializing(_) => bail!("can't update initializing torrent"),
            ManagedTorrentState::Error(_) => {}
            ManagedTorrentState::None => {}
            ManagedTorrentState::Paused(p) => {
                p.update_only_files(only_files)?;
            }
            ManagedTorrentState::Live(l) => {
                l.update_only_files(only_files)?;
            }
        };

        g.only_files = Some(only_files.iter().copied().collect());
        Ok(())
    }
}

pub type ManagedTorrentHandle = Arc<ManagedTorrent>;

fn spawn_fatal_errors_receiver(
    state: &Arc<ManagedTorrent>,
    rx: tokio::sync::oneshot::Receiver<anyhow::Error>,
    token: CancellationToken,
) {
    let span = state.shared.span.clone();
    let id = state.shared.id;
    let info_hash = state.shared.info_hash;
    let state = Arc::downgrade(state);
    spawn_with_cancel::<&'static str>(
        debug_span!(parent: span, "fatal_errors_receiver"),
        "fatal_errors_receiver",
        token,
        async move {
            let e = match rx.await {
                Ok(e) => e,
                Err(_) => return Ok(()),
            };
            if let Some(state) = state.upgrade() {
                state.stop_with_error(e);
            } else {
                warn!(
                    ?id,
                    ?info_hash,
                    "tried to stop the torrent with error, but couldn't upgrade the arc"
                );
            }
            Ok(())
        },
    );
}

fn spawn_peer_adder(live: &Arc<TorrentStateLive>, peer_rx: PeerStream) {
    live.spawn(
        debug_span!(parent: live.torrent().span.clone(), "external_peer_adder"),
        format!("[{}]external_peer_adder", live.shared.id),
        {
            let live = live.clone();
            async move {
                let live = {
                    let weak = Arc::downgrade(&live);
                    drop(live);
                    weak
                };

                drain_peer_stream(&live, peer_rx).await?;

                // The initial peer stream is exhausted. Now wait for rediscovery
                // signals and create new peer streams on demand.
                debug!("initial peer_rx exhausted, waiting for rediscovery signals");
                loop {
                    let state = match live.upgrade() {
                        Some(s) => s,
                        None => return Ok(()),
                    };

                    // Wait for rediscovery_notify to be signaled by the health monitor.
                    // We must hold the Arc alive while awaiting since Notified borrows Notify.
                    state.rediscovery_notify.notified().await;

                    // Don't re-discover if we're finished.
                    if state.is_finished_and_no_active_streams() {
                        continue;
                    }

                    debug!(
                        id = state.shared.id,
                        "rediscovery signal received, requesting new peers from DHT/trackers"
                    );

                    let session = match state.shared.session.upgrade() {
                        Some(s) => s,
                        None => return Ok(()),
                    };

                    // Create a fresh peer stream from DHT and trackers.
                    let is_private = state.metadata.info.info().private;
                    let new_peer_rx = session.make_peer_rx(
                        state.shared.info_hash,
                        state.shared.trackers.iter().cloned().collect(),
                        true, // announce
                        state.shared.options.force_tracker_interval,
                        Vec::new(), // no initial peers on re-discovery
                        is_private,
                    );
                    drop(state);

                    if let Some(rx) = new_peer_rx {
                        drain_peer_stream(&live, rx).await?;
                        debug!("rediscovery peer stream exhausted");
                    }
                }
            }
        },
    );
}

/// Drains a peer stream, adding each discovered peer to the torrent.
/// Returns when the stream ends or the torrent is no longer live.
async fn drain_peer_stream(
    live: &std::sync::Weak<TorrentStateLive>,
    mut peer_rx: PeerStream,
) -> crate::Result<()> {
    loop {
        match timeout(Duration::from_secs(5), peer_rx.next()).await {
            Ok(Some(peer)) => {
                trace!(?peer, "received peer");
                let state = match live.upgrade() {
                    Some(state) => state,
                    None => return Ok(()),
                };
                state.add_peer_if_not_seen(peer)?;
            }
            Ok(None) => {
                return Ok(());
            }
            // If timeout, check if the torrent is live.
            Err(_) if live.strong_count() == 0 => {
                debug!("timed out waiting for peers, torrent isn't live");
                return Ok(());
            }
            Err(_) => continue,
        }
    }
}
