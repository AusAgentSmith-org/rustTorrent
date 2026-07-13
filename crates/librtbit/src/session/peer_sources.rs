use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, bail};
use dht::Id20;
use http::StatusCode;
use itertools::Itertools;
use tracing::trace;

use crate::{
    ApiError, CreateTorrentOptions,
    api_error::WithStatus,
    create_torrent,
    create_torrent_file::CreateTorrentResult,
    dht_utils::{ReadMetainfoResult, read_metainfo_from_peer_receiver},
    peer_connection::PeerConnectionOptions,
    torrent_state::{ManagedTorrentHandle, TorrentMetadata},
    type_aliases::PeerStream,
};
use tracker_comms::{TrackerComms, TrackerStatusRegistry};

use super::Session;
use super::helpers::merge_two_optional_streams;
use super::types::{
    AddTorrent, AddTorrentOptions, PeerRxTorrentInfo, ResolveMagnetResult,
    torrent_file_from_info_bytes,
};

use crate::ManagedTorrent;

impl Session {
    pub fn make_peer_rx_managed_torrent(
        self: &Arc<Self>,
        t: &Arc<ManagedTorrent>,
        announce: bool,
    ) -> Option<PeerStream> {
        let is_private = t.with_metadata(|m| m.info.info().private).unwrap_or(false);
        self.make_peer_rx(
            t.info_hash(),
            t.shared().trackers.iter().cloned().collect(),
            announce,
            t.shared().options.force_tracker_interval,
            t.shared().options.initial_peers.clone(),
            is_private,
            Some(t.shared().tracker_status.clone()),
        )
    }

    // Get a peer stream from both DHT and trackers.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn make_peer_rx(
        self: &Arc<Self>,
        info_hash: Id20,
        mut trackers: Vec<url::Url>,
        announce: bool,
        force_tracker_interval: Option<Duration>,
        initial_peers: Vec<SocketAddr>,
        is_private: bool,
        tracker_status: Option<Arc<TrackerStatusRegistry>>,
    ) -> Option<PeerStream> {
        let dht_rx = if is_private {
            None
        } else {
            self.dht.as_ref().map(|dht| {
                dht.get_peers(info_hash, if announce { self.announce_port } else { None })
            })
        };

        let lsd_rx = if is_private {
            None
        } else {
            self.lsd.as_ref().map(|lsd| {
                lsd.announce(info_hash, if announce { self.announce_port } else { None })
            })
        };

        apply_tracker_policy(
            &mut trackers,
            &self.trackers,
            self.disable_trackers,
            is_private,
        );

        let tracker_rx_stats = PeerRxTorrentInfo {
            info_hash,
            session: self.clone(),
        };
        let tracker_rx = TrackerComms::start(
            info_hash,
            self.peer_id,
            trackers.into_iter().collect(),
            Box::new(tracker_rx_stats),
            force_tracker_interval,
            self.announce_port().unwrap_or(4240),
            self.reqwest_client.clone(),
            self.udp_tracker_client.clone(),
            tracker_status,
        );

        let initial_peers_rx = if initial_peers.is_empty() {
            None
        } else {
            Some(futures::stream::iter(initial_peers))
        };
        merge_two_optional_streams(
            merge_two_optional_streams(
                merge_two_optional_streams(dht_rx, tracker_rx),
                initial_peers_rx,
            ),
            lsd_rx,
        )
    }

    pub(super) async fn resolve_magnet(
        self: &Arc<Self>,
        info_hash: Id20,
        peer_rx: PeerStream,
        trackers: &[url::Url],
        peer_opts: Option<PeerConnectionOptions>,
    ) -> anyhow::Result<ResolveMagnetResult> {
        match read_metainfo_from_peer_receiver(
            self.peer_id,
            info_hash,
            Default::default(),
            peer_rx,
            Some(self.merge_peer_opts(peer_opts)),
            self.connector.clone(),
        )
        .await
        {
            ReadMetainfoResult::Found {
                info,
                info_bytes,
                rx,
                seen,
            } => {
                trace!(?info, "received result from DHT");
                let info = info.validate()?;
                Ok(ResolveMagnetResult {
                    metadata: TorrentMetadata::new(
                        info,
                        torrent_file_from_info_bytes(info_bytes.as_ref(), trackers)?,
                        info_bytes.0,
                    )?,
                    peer_rx: rx,
                    seen_peers: {
                        let seen = seen.into_iter().collect_vec();
                        for peer in &seen {
                            trace!(?peer, "seen")
                        }
                        seen
                    },
                })
            }
            ReadMetainfoResult::ChannelClosed { .. } => {
                bail!("input address stream exhausted, no way to discover torrent metainfo")
            }
        }
    }

    pub async fn create_and_serve_torrent(
        self: &Arc<Self>,
        path: &Path,
        opts: CreateTorrentOptions<'_>,
    ) -> Result<(CreateTorrentResult, ManagedTorrentHandle), ApiError> {
        if !path.exists() {
            return Err(ApiError::from((
                StatusCode::BAD_REQUEST,
                "path doesn't exist",
            )));
        }

        let torrent = create_torrent(path, opts, &self.spawner)
            .await
            .with_status(StatusCode::BAD_REQUEST)?;

        let bytes = torrent.as_bytes()?;

        let handle = self
            .add_torrent(
                AddTorrent::TorrentFileBytes(bytes.clone()),
                Some(AddTorrentOptions {
                    paused: false,
                    overwrite: true,
                    output_folder: Some(
                        torrent
                            .output_folder
                            .to_str()
                            .context("invalid utf-8")?
                            .to_owned(),
                    ),
                    ..Default::default()
                }),
            )
            .await?
            .into_handle()
            .context("error adding to session")?;

        Ok((torrent, handle))
    }
}

fn apply_tracker_policy(
    torrent_trackers: &mut Vec<url::Url>,
    session_trackers: &std::collections::HashSet<url::Url>,
    disable_trackers: bool,
    is_private: bool,
) {
    if disable_trackers {
        torrent_trackers.clear();
    } else if !is_private {
        torrent_trackers.extend(session_trackers.iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::apply_tracker_policy;

    fn url(s: &str) -> url::Url {
        s.parse().unwrap()
    }

    #[test]
    fn private_torrents_keep_all_declared_trackers() {
        let mut torrent_trackers = vec![
            url("https://tracker.example/announce"),
            url("https://backup.example/announce"),
        ];
        let session_trackers = HashSet::from([url("https://global.example/announce")]);

        apply_tracker_policy(&mut torrent_trackers, &session_trackers, false, true);

        assert_eq!(
            torrent_trackers,
            vec![
                url("https://tracker.example/announce"),
                url("https://backup.example/announce"),
            ]
        );
    }

    #[test]
    fn public_torrents_include_session_trackers() {
        let mut torrent_trackers = vec![url("https://tracker.example/announce")];
        let session_trackers = HashSet::from([url("https://global.example/announce")]);

        apply_tracker_policy(&mut torrent_trackers, &session_trackers, false, false);

        assert!(torrent_trackers.contains(&url("https://tracker.example/announce")));
        assert!(torrent_trackers.contains(&url("https://global.example/announce")));
    }

    #[test]
    fn disabling_trackers_clears_private_and_public_trackers() {
        let mut torrent_trackers = vec![url("https://tracker.example/announce")];
        let session_trackers = HashSet::from([url("https://global.example/announce")]);

        apply_tracker_policy(&mut torrent_trackers, &session_trackers, true, true);

        assert!(torrent_trackers.is_empty());
    }
}
