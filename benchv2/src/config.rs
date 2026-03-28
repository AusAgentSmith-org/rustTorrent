use serde::Serialize;

pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;

pub const TRACKER_ANNOUNCE: &str = "http://tracker:6969/announce";
pub const RTBIT_API: &str = "http://rtbit:3030";
pub const QBT_API: &str = "http://qbittorrent:8080";
pub const PROMETHEUS_API: &str = "http://prometheus:9090";

pub const POLL_INTERVAL_MS: u64 = 1000;

#[derive(Debug, Clone, Serialize)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    pub file_size: u64,
    pub num_files: usize,
    pub real_seeders: usize,
    pub mock_peers: usize,
    pub timeout_secs: u64,
}

impl Scenario {
    pub fn total_peers(&self) -> usize {
        self.real_seeders + self.mock_peers
    }
    pub fn total_bytes(&self) -> u64 {
        self.file_size * self.num_files as u64
    }
}

pub fn size_label(size: u64) -> String {
    if size >= GB {
        format!("{}gb", size / GB)
    } else {
        format!("{}mb", size / MB)
    }
}

fn generate_all() -> Vec<Scenario> {
    let sizes_gb: Vec<u64> = vec![2, 4, 6, 8, 10, 12, 14, 16, 18, 20];
    let file_counts: Vec<usize> = vec![1, 10, 50, 100];
    let peer_configs: Vec<(usize, usize, &str)> = vec![
        (3, 0, "3p"),
        (10, 0, "10p"),
        (0, 50, "50p"),
        (0, 100, "100p"),
        (0, 250, "250p"),
        (0, 500, "500p"),
        (0, 1000, "1000p"),
    ];

    let mut scenarios = Vec::new();
    for &total_gb in &sizes_gb {
        for &nf in &file_counts {
            let per_file = (total_gb * GB) / nf as u64;
            if per_file < MB {
                continue;
            }
            for &(real_s, mock_p, peer_label) in &peer_configs {
                let total_peers = real_s + mock_p;
                let name = format!("sz{}g_f{}_{}", total_gb, nf, peer_label);
                let desc = format!(
                    "{} GB total, {} file(s) x {} MB, {} peers",
                    total_gb,
                    nf,
                    per_file / MB,
                    total_peers
                );
                let timeout = std::cmp::max(300, total_gb * 120);
                scenarios.push(Scenario {
                    name,
                    description: desc,
                    file_size: per_file,
                    num_files: nf,
                    real_seeders: real_s,
                    mock_peers: mock_p,
                    timeout_secs: timeout,
                });
            }
        }
    }
    scenarios
}

fn pick(all: &[Scenario], names: &[&str]) -> Vec<Scenario> {
    names
        .iter()
        .filter_map(|n| all.iter().find(|s| s.name == *n).cloned())
        .collect()
}

pub fn resolve_scenarios(selector: &str) -> Vec<Scenario> {
    let all = generate_all();
    let sel = selector.trim().to_lowercase();

    match sel.as_str() {
        "all" => all,
        "quick" => pick(&all, &["sz8g_f1_3p"]),
        "medium" => pick(
            &all,
            &[
                "sz8g_f1_3p",
                "sz16g_f1_3p",
                "sz8g_f10_3p",
                "sz8g_f100_3p",
                "sz8g_f1_100p",
                "sz8g_f1_500p",
            ],
        ),
        "size_ramp" => all
            .iter()
            .filter(|s| s.num_files == 1 && s.real_seeders == 3 && s.mock_peers == 0)
            .cloned()
            .collect(),
        "file_ramp" => all
            .iter()
            .filter(|s| s.name.starts_with("sz10g") && s.real_seeders == 3 && s.mock_peers == 0)
            .cloned()
            .collect(),
        "peer_ramp" => all
            .iter()
            .filter(|s| s.name.starts_with("sz10g") && s.num_files == 1)
            .cloned()
            .collect(),
        _ => {
            // Comma-separated individual names
            let names: Vec<&str> = sel.split(',').map(|s| s.trim()).collect();
            let matched: Vec<Scenario> = all
                .iter()
                .filter(|s| names.contains(&s.name.as_str()))
                .cloned()
                .collect();
            if matched.is_empty() {
                tracing::error!("No scenarios matched: {selector}");
                tracing::info!(
                    "Groups: all, quick, medium, size_ramp, file_ramp, peer_ramp"
                );
            }
            matched
        }
    }
}
