use crate::clients::{qbittorrent::QBittorrentClient, rtbit::RtbitClient, transmission::TransmissionClient};
use crate::config::{self, Scenario, MB, GB};
use crate::metrics::{MetricSample, MetricsCollector};
use crate::{datagen, docker, report, charts};
use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ClientResult {
    pub client: String,
    pub scenario: String,
    pub scenario_description: String,
    pub total_bytes: u64,
    pub duration_sec: f64,
    pub avg_speed_mbps: f64,
    pub peak_speed_mbps: f64,
    pub time_to_first_piece: f64,
    pub cpu_avg: f64,
    pub cpu_peak: f64,
    pub mem_avg_mb: f64,
    pub mem_peak_mb: f64,
    pub net_rx_avg_mbps: f64,
    pub net_rx_peak_mbps: f64,
    pub disk_write_avg_mbps: f64,
    pub disk_write_peak_mbps: f64,
    pub iowait_avg: f64,
    pub iowait_peak: f64,
    pub timeseries: Vec<MetricSample>,
}

fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

async fn wait_for_service(name: &str, url: &str, timeout_secs: u64) -> Result<()> {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("{name} not ready after {timeout_secs}s");
        }
        if let Ok(resp) = client
            .get(url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            if resp.status().as_u16() < 500 {
                tracing::info!("  {name}: ready");
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Tell the mock seeder to reload torrents from disk, then wait until it has
/// at least `min_torrents` loaded.  This closes the race where the orchestrator
/// generates test data but the mock seeder hasn't scanned for it yet.
async fn wait_for_mock_seeder_torrents(min_torrents: usize, timeout_secs: u64) -> Result<()> {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    // Trigger reload
    let _ = client
        .get("http://mock-seeder:8080/reload")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("mock-seeder did not load {min_torrents} torrent(s) within {timeout_secs}s");
        }
        if let Ok(resp) = client
            .get("http://mock-seeder:8080/status")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            if let Ok(body) = resp.text().await {
                // Parse {"torrents":N}
                if let Some(n) = body
                    .split("torrents\":")
                    .nth(1)
                    .and_then(|s| s.trim_end_matches('}').parse::<usize>().ok())
                {
                    if n >= min_torrents {
                        tracing::info!("  mock-seeder: {n} torrent(s) loaded (need {min_torrents})");
                        return Ok(());
                    }
                    tracing::debug!("  mock-seeder: {n}/{min_torrents} torrents, waiting...");
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Trigger a fresh announce on the mock seeder so the tracker has current peer entries.
async fn trigger_mock_seeder_announce() {
    let client = reqwest::Client::new();
    match client
        .get("http://mock-seeder:8080/reload")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(_) => tracing::info!("  mock-seeder: re-announced peers to tracker"),
        Err(e) => tracing::warn!("  mock-seeder: re-announce failed: {e}"),
    }
}

/// Drop the OS page cache for all mmap'd test data on the mock seeder.
/// This ensures neither client gets an unfair advantage from warm cache.
async fn drop_mock_seeder_cache() {
    let client = reqwest::Client::new();
    match client
        .get("http://mock-seeder:8080/drop-cache")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(_) => tracing::info!("  mock-seeder: page cache dropped"),
        Err(e) => tracing::warn!("  mock-seeder: drop-cache failed: {e}"),
    }
}

pub async fn run(
    scenario_selector: String,
    data_dir: PathBuf,
    torrent_dir: PathBuf,
    results_dir: PathBuf,
) -> Result<()> {
    tracing::info!("============================================================");
    tracing::info!("  BitTorrent Client Benchmark: rtbit vs qBittorrent (Rust)");
    tracing::info!("============================================================");
    tracing::info!("Scenarios: {scenario_selector}");
    tracing::info!("Data dir: {}", data_dir.display());

    // Connect to Docker
    tracing::info!("Connecting to Docker...");
    let docker_client = docker::connect().map_err(|e| {
        tracing::error!("Failed to connect to Docker: {e}");
        tracing::error!("Make sure /var/run/docker.sock is mounted");
        e
    })?;
    tracing::info!("Docker connected.");

    // Init clients
    let rtbit = RtbitClient::new(config::RTBIT_API);
    let qbt = QBittorrentClient::new(config::QBT_API);
    let mut metrics = MetricsCollector::new(docker::connect()?);

    // Wait for services
    tracing::info!("Waiting for services...");
    wait_for_service("tracker", "http://tracker:6969/health", 120).await?;
    wait_for_service("rtbit", &format!("{}/", config::RTBIT_API), 120).await?;
    wait_for_service("qBittorrent", &format!("{}/", config::QBT_API), 120).await?;
    wait_for_service("mock-seeder", "http://mock-seeder:8080/health", 120).await?;

    // Discover seeders
    let seeder_ips = docker::get_service_ips(&docker_client, "seeder").await?;
    tracing::info!("Discovered {} seeder(s)", seeder_ips.len());

    let seeders: Vec<TransmissionClient> = seeder_ips
        .iter()
        .map(|ip| TransmissionClient::new(&format!("http://{ip}:9091/transmission/rpc")))
        .collect();

    // Verify seeder RPC
    let mut ok = 0;
    for s in &seeders {
        if s.get_torrents().await.is_ok() {
            ok += 1;
        }
    }
    tracing::info!("  Seeders: {ok}/{} RPC-ready", seeders.len());

    // Resolve container IDs for stats collection
    metrics.resolve_container_id("rtbit").await;
    metrics.resolve_container_id("qbittorrent").await;

    // Configure qBittorrent
    tracing::info!("Configuring qBittorrent...");
    qbt.authenticate(&docker_client).await?;
    qbt.configure_for_bench().await?;

    // Resolve scenarios
    let scenarios = config::resolve_scenarios(&scenario_selector);
    if scenarios.is_empty() {
        return Ok(());
    }

    let total_data: u64 = scenarios.iter().map(|s| s.total_bytes()).sum();
    tracing::info!(
        "Running {} scenario(s), {:.1} GB total data",
        scenarios.len(),
        total_data as f64 / GB as f64
    );
    for s in &scenarios {
        tracing::info!(
            "  {:30} {:>5} MB x {:>3} files = {:>6} MB  {:>4} peers  timeout={}s",
            s.name,
            s.file_size / MB,
            s.num_files,
            s.total_bytes() / MB,
            s.total_peers(),
            s.timeout_secs,
        );
    }

    // Generate data
    datagen::prepare_data(&scenarios, &data_dir, &torrent_dir).await?;

    // Run scenarios
    let mut all_results: Vec<(ClientResult, ClientResult)> = Vec::new();

    for sc in &scenarios {
        tracing::info!("============================================================");
        tracing::info!("SCENARIO: {} — {}", sc.name, sc.description);
        tracing::info!("============================================================");

        // Clean download directories to prevent stale data
        tracing::info!("Cleaning download directories...");
        clean_download_dir(&docker_client, "rtbit", "/home/rtbit/downloads").await;
        clean_download_dir(&docker_client, "qbittorrent", "/downloads").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let tpaths = datagen::torrent_paths(sc, &torrent_dir);

        // Setup seeders
        if sc.real_seeders > 0 {
            tracing::info!("Setting up {} real seeder(s)...", sc.real_seeders);
            for s in &seeders {
                let _ = s.remove_all().await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            for tp in &tpaths {
                let tp_str = tp.to_string_lossy();
                for s in seeders.iter().take(sc.real_seeders) {
                    let _ = s.add_torrent(&tp_str, "/data/testdata").await;
                }
            }
            // Wait for seeding
            let verify_timeout = std::cmp::max(180, (sc.total_bytes() / (50 * MB)) as u64);
            tracing::info!("Waiting for seeders to verify (timeout {verify_timeout}s)...");
            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_secs(verify_timeout);
            loop {
                if tokio::time::Instant::now() > deadline {
                    tracing::error!("Seeders not ready — skipping scenario");
                    break;
                }
                let ready = futures_util::future::join_all(
                    seeders.iter().take(sc.real_seeders).map(|s| s.is_seeding()),
                )
                .await
                .into_iter()
                .filter(|&r| r)
                .count();
                if ready >= sc.real_seeders {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }

        if sc.mock_peers > 0 {
            tracing::info!("Mock seeder active with {} peers — triggering reload", sc.mock_peers);
            wait_for_mock_seeder_torrents(sc.num_files, 60).await?;
        }

        tracing::info!("Seeding ready. Running downloads sequentially.");

        // Run rtbit — drop page cache + re-announce for fair cold-cache start
        drop_mock_seeder_cache().await;
        if sc.mock_peers > 0 {
            trigger_mock_seeder_announce().await;
        }
        let rtbit_result =
            run_client("rtbit", sc, &tpaths, &rtbit, &qbt, &metrics).await;
        cleanup_rtbit(&rtbit).await;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        // Run qBittorrent — drop page cache again for equal footing
        drop_mock_seeder_cache().await;
        if sc.mock_peers > 0 {
            trigger_mock_seeder_announce().await;
        }
        let qbt_result =
            run_client("qbittorrent", sc, &tpaths, &rtbit, &qbt, &metrics).await;
        cleanup_qbt(&qbt).await;

        // Clean seeders
        if sc.real_seeders > 0 {
            for s in &seeders {
                let _ = s.remove_all().await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        all_results.push((rtbit_result, qbt_result));
    }

    // Generate reports
    tokio::fs::create_dir_all(&results_dir).await?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();

    report::write_json(&all_results, &results_dir, &timestamp)?;
    report::write_csv(&all_results, &results_dir, &timestamp)?;
    let summary = report::build_summary(&all_results);
    println!("\n{summary}");
    report::write_summary(&summary, &results_dir, &timestamp)?;

    let charts_dir = results_dir.join(format!("charts_{timestamp}"));
    std::fs::create_dir_all(&charts_dir)?;
    charts::generate_all(&all_results, &charts_dir)?;

    tracing::info!("Results: {}", results_dir.display());
    Ok(())
}

async fn run_client(
    client_name: &str,
    sc: &Scenario,
    tpaths: &[PathBuf],
    rtbit: &RtbitClient,
    qbt: &QBittorrentClient,
    metrics: &MetricsCollector,
) -> ClientResult {
    let mut result = ClientResult {
        client: client_name.to_string(),
        scenario: sc.name.clone(),
        scenario_description: sc.description.clone(),
        total_bytes: sc.total_bytes(),
        duration_sec: 0.0,
        avg_speed_mbps: 0.0,
        peak_speed_mbps: 0.0,
        time_to_first_piece: 0.0,
        cpu_avg: 0.0,
        cpu_peak: 0.0,
        mem_avg_mb: 0.0,
        mem_peak_mb: 0.0,
        net_rx_avg_mbps: 0.0,
        net_rx_peak_mbps: 0.0,
        disk_write_avg_mbps: 0.0,
        disk_write_peak_mbps: 0.0,
        iowait_avg: 0.0,
        iowait_peak: 0.0,
        timeseries: vec![],
    };

    tracing::info!("  [{client_name}] Adding {} torrent(s)...", tpaths.len());

    let mut rtbit_ids = Vec::new();
    if client_name == "rtbit" {
        for tp in tpaths {
            match rtbit.add_torrent(tp).await {
                Ok(id) => rtbit_ids.push(id),
                Err(e) => tracing::error!("  [rtbit] Failed to add {}: {e}", tp.display()),
            }
        }
    } else {
        for tp in tpaths {
            if let Err(e) = qbt.add_torrent(tp).await {
                tracing::error!("  [qbt] Failed to add {}: {e}", tp.display());
            }
        }
    }

    // Start real-time stats collection via Docker stats API
    let stats_handle = metrics.start_collecting(client_name);
    if stats_handle.is_none() {
        tracing::warn!("  [{client_name}] Could not start stats collection — container ID not resolved");
    }

    let start_instant = tokio::time::Instant::now();
    let mut first_piece_time: Option<f64> = None;
    let mut peak_speed: f64 = 0.0;
    let mut speeds = Vec::new();

    tracing::info!("  [{client_name}] Downloading (timeout {}s)...", sc.timeout_secs);

    let deadline = start_instant + std::time::Duration::from_secs(sc.timeout_secs);
    let mut timed_out = false;

    loop {
        if tokio::time::Instant::now() > deadline {
            tracing::warn!("  [{client_name}] TIMEOUT");
            timed_out = true;
            break;
        }

        let (finished, progress, speed) = if client_name == "rtbit" {
            let fin = rtbit.all_finished(&rtbit_ids).await.unwrap_or(false);
            let prog = rtbit.progress_fraction(&rtbit_ids).await;
            let spd = rtbit.aggregate_speed(&rtbit_ids).await;
            (fin, prog, spd)
        } else {
            let fin = qbt.all_finished().await.unwrap_or(false);
            let prog = qbt.progress_fraction().await;
            let spd = qbt.aggregate_speed().await;
            (fin, prog, spd)
        };

        if progress > 0.001 && first_piece_time.is_none() {
            first_piece_time = Some(start_instant.elapsed().as_secs_f64());
        }
        if speed > 0.0 {
            peak_speed = peak_speed.max(speed);
            speeds.push(speed);
        }

        if finished {
            tracing::info!("  [{client_name}] Reported complete — verifying...");
            // Verify the download actually happened
            let verified = if client_name == "rtbit" {
                verify_rtbit_download(rtbit, &rtbit_ids, sc.total_bytes()).await
            } else {
                verify_qbt_download(qbt, sc.total_bytes()).await
            };
            if !verified {
                tracing::error!(
                    "  [{client_name}] VERIFICATION FAILED — client reported finished but \
                     downloaded bytes don't match expected {} MB",
                    sc.total_bytes() / MB
                );
            }
            break;
        }

        let bar_len = 30;
        let filled = (bar_len as f64 * progress) as usize;
        let bar: String = "#".repeat(filled) + &"-".repeat(bar_len - filled);
        let speed_mb = speed / MB as f64;
        eprint!(
            "\r  [{client_name}] [{bar}] {:5.1}% @ {:.1} MB/s",
            progress * 100.0,
            speed_mb
        );

        tokio::time::sleep(std::time::Duration::from_millis(config::POLL_INTERVAL_MS)).await;
    }
    eprintln!(); // newline after progress bar

    result.duration_sec = start_instant.elapsed().as_secs_f64();
    result.time_to_first_piece = first_piece_time.unwrap_or(result.duration_sec);
    result.peak_speed_mbps = peak_speed * 8.0 / 1_000_000.0;
    if !speeds.is_empty() {
        let avg = speeds.iter().sum::<f64>() / speeds.len() as f64;
        result.avg_speed_mbps = avg * 8.0 / 1_000_000.0;
    } else if result.duration_sec > 0.0 {
        result.avg_speed_mbps = result.total_bytes as f64 * 8.0 / result.duration_sec / 1_000_000.0;
    }

    // Stop real-time stats collection and retrieve samples
    tracing::info!("  [{client_name}] Collecting metrics...");
    let samples = if let Some(handle) = stats_handle {
        handle.stop().await
    } else {
        vec![]
    };
    tracing::info!("  [{client_name}] Got {} metric samples", samples.len());

    if !samples.is_empty() {
        let cpus: Vec<f64> = samples.iter().map(|s| s.cpu_pct).collect();
        let mems: Vec<u64> = samples.iter().map(|s| s.mem_bytes).collect();
        let rxs: Vec<f64> = samples.iter().map(|s| s.net_rx_bps).collect();
        let dws: Vec<f64> = samples.iter().map(|s| s.disk_write_bps).collect();
        let iow: Vec<f64> = samples.iter().map(|s| s.iowait_pct).collect();

        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
        let max_f = |v: &[f64]| v.iter().cloned().fold(0.0f64, f64::max);
        let avg_u = |v: &[u64]| v.iter().sum::<u64>() as f64 / v.len().max(1) as f64;
        let max_u = |v: &[u64]| v.iter().cloned().max().unwrap_or(0);

        result.cpu_avg = avg(&cpus);
        result.cpu_peak = max_f(&cpus);
        result.mem_avg_mb = avg_u(&mems) / MB as f64;
        result.mem_peak_mb = max_u(&mems) as f64 / MB as f64;
        result.net_rx_avg_mbps = avg(&rxs) * 8.0 / 1e6;
        result.net_rx_peak_mbps = max_f(&rxs) * 8.0 / 1e6;
        result.disk_write_avg_mbps = avg(&dws) / MB as f64;
        result.disk_write_peak_mbps = max_f(&dws) / MB as f64;
        result.iowait_avg = avg(&iow);
        result.iowait_peak = max_f(&iow);
        result.timeseries = samples;
    }

    tracing::info!(
        "  [{client_name}] Done: {:.1}s, avg {:.1} Mbps",
        result.duration_sec,
        result.avg_speed_mbps
    );
    result
}

/// Verify rtbit actually downloaded the expected bytes (not stale/cached data).
async fn verify_rtbit_download(rtbit: &RtbitClient, ids: &[u64], expected_bytes: u64) -> bool {
    let mut total_progress = 0u64;
    for &id in ids {
        if let Ok(s) = rtbit.stats(id).await {
            let progress = s
                .get("progress_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let total = s
                .get("total_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if progress < total {
                tracing::warn!(
                    "  [rtbit] Torrent {id}: {progress}/{total} bytes — incomplete!"
                );
                return false;
            }
            total_progress += progress;
        }
    }
    if total_progress < expected_bytes {
        tracing::warn!(
            "  [rtbit] Total downloaded {total_progress} < expected {expected_bytes}"
        );
        return false;
    }
    tracing::info!(
        "  [rtbit] Verified: {} MB downloaded",
        total_progress / MB
    );
    true
}

/// Verify qBittorrent actually downloaded the expected bytes.
async fn verify_qbt_download(qbt: &QBittorrentClient, expected_bytes: u64) -> bool {
    let torrents = qbt.get_torrents().await.unwrap_or_default();
    let mut total_downloaded = 0u64;
    for t in &torrents {
        let progress = t
            .get("progress")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let size = t
            .get("total_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if progress < 1.0 {
            tracing::warn!("  [qbt] Torrent incomplete: {:.1}%", progress * 100.0);
            return false;
        }
        total_downloaded += size;
    }
    if total_downloaded < expected_bytes {
        tracing::warn!(
            "  [qbt] Total downloaded {total_downloaded} < expected {expected_bytes}"
        );
        return false;
    }
    tracing::info!(
        "  [qbt] Verified: {} MB downloaded",
        total_downloaded / MB
    );
    true
}

/// Clean downloaded files from a container's directory via Docker exec.
async fn clean_download_dir(docker_client: &bollard::Docker, service: &str, dir: &str) {
    if let Some(cid) = docker::get_container_id(docker_client, service).await {
        match docker::exec_in_container(
            docker_client,
            &cid,
            vec!["sh", "-c", &format!("rm -rf {dir}/* {dir}/.[!.]* 2>/dev/null; echo ok")],
        ).await {
            Ok(_) => tracing::info!("  [{service}] Cleaned {dir}"),
            Err(e) => tracing::warn!("  [{service}] Failed to clean {dir}: {e}"),
        }
    }
}

async fn cleanup_rtbit(rtbit: &RtbitClient) {
    tracing::info!("  [rtbit] Cleaning up...");
    let _ = rtbit.delete_all().await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    if let Ok(remaining) = rtbit.list_torrents().await {
        if !remaining.is_empty() {
            tracing::warn!("  [rtbit] {} torrent(s) still present, retrying...", remaining.len());
            let _ = rtbit.delete_all().await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

async fn cleanup_qbt(qbt: &QBittorrentClient) {
    tracing::info!("  [qbittorrent] Cleaning up...");
    let _ = qbt.delete_all().await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
}
