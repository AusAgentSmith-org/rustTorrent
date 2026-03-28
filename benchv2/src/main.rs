mod bencode;
mod charts;
mod clients;
mod config;
mod datagen;
mod docker;
mod metrics;
mod mock_seeder;
mod report;
mod runner;
mod tracker;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "benchv2", about = "BitTorrent client benchmark: rtbit vs qBittorrent")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the benchmark orchestrator
    Run {
        /// Scenario group or comma-separated names
        #[arg(long, default_value = "quick", env = "SCENARIOS")]
        scenarios: String,
        /// Data directory for test files
        #[arg(long, default_value = "/data/testdata")]
        data_dir: PathBuf,
        /// Torrent file directory
        #[arg(long, default_value = "/data/torrents")]
        torrent_dir: PathBuf,
        /// Results output directory
        #[arg(long, default_value = "/results")]
        results_dir: PathBuf,
    },
    /// Run the HTTP BitTorrent tracker
    Tracker {
        #[arg(long, default_value = "6969")]
        port: u16,
    },
    /// Regenerate charts from existing benchmark JSON
    RegenCharts {
        /// Path to benchmark JSON file
        #[arg(long)]
        json: PathBuf,
        /// Output directory for charts
        #[arg(long)]
        out: PathBuf,
    },
    /// Run the mock BitTorrent seeder
    MockSeed {
        #[arg(long, default_value = "100", env = "MOCK_PEERS")]
        peers: usize,
        #[arg(long, default_value = "6900")]
        base_port: u16,
        #[arg(long, default_value = "http://tracker:6969/announce")]
        tracker_url: String,
        #[arg(long, default_value = "/data/testdata")]
        data_dir: PathBuf,
        #[arg(long, default_value = "/data/torrents")]
        torrent_dir: PathBuf,
        /// Health check port
        #[arg(long, default_value = "8080")]
        health_port: u16,
    },
}

fn main() -> anyhow::Result<()> {
    // Print before anything else — if we don't see this, the binary isn't running
    eprintln!(
        "[benchv2] PID={} args={:?}",
        std::process::id(),
        std::env::args().collect::<Vec<_>>()
    );

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    tracing::info!("Subcommand: {:?}", std::env::args().nth(1).unwrap_or_default());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(async {
        match cli.command {
            Command::Run {
                scenarios,
                data_dir,
                torrent_dir,
                results_dir,
            } => {
                runner::run(scenarios, data_dir, torrent_dir, results_dir).await
            }
            Command::RegenCharts { json, out } => {
                let data = std::fs::read_to_string(&json)?;
                let items: Vec<serde_json::Value> = serde_json::from_str(&data)?;
                let mut results: Vec<(runner::ClientResult, runner::ClientResult)> = Vec::new();
                for item in &items {
                    let rq: runner::ClientResult = serde_json::from_value(item["rtbit"].clone())?;
                    let qb: runner::ClientResult = serde_json::from_value(item["qbittorrent"].clone())?;
                    results.push((rq, qb));
                }
                std::fs::create_dir_all(&out)?;
                charts::generate_all(&results, &out)?;
                tracing::info!("Charts written to {}", out.display());
                Ok(())
            }
            Command::Tracker { port } => tracker::run(port).await,
            Command::MockSeed {
                peers,
                base_port,
                tracker_url,
                data_dir,
                torrent_dir,
                health_port,
            } => {
                mock_seeder::run(peers, base_port, tracker_url, data_dir, torrent_dir, health_port)
                    .await
            }
        }
    });

    if let Err(ref e) = result {
        tracing::error!("Fatal error: {e:#}");
    }
    result
}
