use std::time::Duration;

use anyhow::Context;
use tempfile::TempDir;
use tokio::time::timeout;

use crate::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, Session, SessionOptions,
    SessionPersistenceConfig, create_torrent,
    spawn_utils::BlockingSpawner,
    storage::{StorageFactoryExt, filesystem::FilesystemStorageFactory},
};

async fn create_read_only_session(
    output_dir: &std::path::Path,
) -> anyhow::Result<std::sync::Arc<Session>> {
    Session::new_with_opts(
        output_dir.to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_trackers: true,
            disable_local_service_discovery: true,
            default_storage_factory: Some(FilesystemStorageFactory::read_only().boxed()),
            ..Default::default()
        },
    )
    .await
}

async fn create_single_file_torrent(payload_dir: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let torrent = create_torrent(
        payload_dir,
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await?;
    Ok(torrent.as_bytes()?.to_vec())
}

async fn create_multi_file_torrent(payload_dir: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let torrent = create_torrent(
        payload_dir,
        CreateTorrentOptions {
            piece_length: Some(16_384),
            ..Default::default()
        },
        &BlockingSpawner::new(1),
    )
    .await?;
    Ok(torrent.as_bytes()?.to_vec())
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_session_verifies_existing_payload_without_changing_it() -> anyhow::Result<()> {
    let payload_dir = TempDir::with_prefix("rtbit_read_only_existing")?;
    let payload_path = payload_dir.path().join("payload.bin");
    let payload = vec![0x5a; 128 * 1024];
    std::fs::write(&payload_path, &payload)?;
    let torrent = create_single_file_torrent(payload_dir.path()).await?;

    let session_dir = TempDir::with_prefix("rtbit_read_only_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(payload_dir.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;

    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for read-only initialization")??;

    let stats = handle.stats();
    assert!(stats.finished);
    assert_eq!(stats.progress_bytes, stats.total_bytes);
    assert_eq!(std::fs::read(&payload_path)?, payload);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_session_does_not_create_missing_payload() -> anyhow::Result<()> {
    let source_dir = TempDir::with_prefix("rtbit_read_only_source")?;
    let source_path = source_dir.path().join("payload.bin");
    std::fs::write(&source_path, vec![0xa5; 64 * 1024])?;
    let torrent = create_single_file_torrent(source_dir.path()).await?;

    let destination_root = TempDir::with_prefix("rtbit_read_only_missing")?;
    let output_dir = destination_root.path().join("missing-output");
    let payload_path = output_dir.join("payload.bin");
    assert!(!output_dir.exists());

    let session_dir = TempDir::with_prefix("rtbit_read_only_session")?;
    let session = create_read_only_session(session_dir.path()).await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder: Some(output_dir.to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;

    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for read-only initialization")??;

    let stats = handle.stats();
    assert!(!stats.finished);
    assert_eq!(stats.progress_bytes, 0);
    assert!(!output_dir.exists());
    assert!(!payload_path.exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_session_verifies_qbittorrent_multi_file_save_path() -> anyhow::Result<()> {
    let payload_root = TempDir::with_prefix("rtbit_qbit_multi_root")?;
    let torrent_root = payload_root.path().join("multi-file-payload");
    std::fs::create_dir(&torrent_root)?;
    std::fs::write(torrent_root.join("first.bin"), vec![0x12; 48 * 1024])?;
    std::fs::write(torrent_root.join("second.bin"), vec![0x34; 32 * 1024])?;
    let torrent = create_multi_file_torrent(&torrent_root).await?;

    let persistence_dir = TempDir::with_prefix("rtbit_qbit_multi_session")?;
    let new_session = || {
        Session::new_with_opts(
            payload_root.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_trackers: true,
                disable_local_service_discovery: true,
                default_storage_factory: Some(FilesystemStorageFactory::read_only().boxed()),
                persistence: Some(SessionPersistenceConfig::Json {
                    folder: Some(persistence_dir.path().to_owned()),
                }),
                ..Default::default()
            },
        )
    };

    let session = new_session().await?;
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                output_folder_root: Some(payload_root.path().to_string_lossy().into_owned()),
                ..Default::default()
            }),
        )
        .await?
        .into_handle()
        .context("expected a torrent handle")?;
    timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .context("timed out waiting for qBittorrent-layout initialization")??;
    assert!(handle.stats().finished);
    assert_eq!(handle.shared().options.output_folder, torrent_root);
    assert_eq!(
        handle.shared().options.output_folder_root.as_deref(),
        Some(payload_root.path())
    );

    let info_hash = handle.info_hash();
    drop(handle);
    drop(session);
    let restored = new_session().await?;
    let restored_handle = restored
        .get(info_hash.into())
        .context("expected persisted torrent")?;
    timeout(
        Duration::from_secs(10),
        restored_handle.wait_until_initialized(),
    )
    .await
    .context("timed out waiting for restored qBittorrent-layout initialization")??;
    assert!(restored_handle.stats().finished);
    assert_eq!(restored_handle.shared().options.output_folder, torrent_root);
    assert_eq!(
        restored_handle
            .shared()
            .options
            .output_folder_root
            .as_deref(),
        Some(payload_root.path())
    );
    Ok(())
}
