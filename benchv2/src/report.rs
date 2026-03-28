use crate::runner::ClientResult;
use anyhow::Result;
use std::path::Path;

pub fn write_json(
    results: &[(ClientResult, ClientResult)],
    dir: &Path,
    timestamp: &str,
) -> Result<()> {
    let data: Vec<serde_json::Value> = results
        .iter()
        .map(|(rq, qb)| {
            serde_json::json!({
                "scenario": rq.scenario,
                "rtbit": rq,
                "qbittorrent": qb,
            })
        })
        .collect();
    let path = dir.join(format!("benchmark_{timestamp}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&data)?)?;
    tracing::info!("JSON: {}", path.display());
    Ok(())
}

pub fn write_csv(
    results: &[(ClientResult, ClientResult)],
    dir: &Path,
    timestamp: &str,
) -> Result<()> {
    let path = dir.join(format!("benchmark_{timestamp}.csv"));
    let mut out = String::from(
        "scenario,client,total_bytes,duration_sec,avg_speed_mbps,peak_speed_mbps,\
         time_to_first_piece,cpu_avg,cpu_peak,mem_avg_mb,mem_peak_mb,\
         net_rx_avg_mbps,net_rx_peak_mbps,disk_write_avg_mbps,disk_write_peak_mbps,\
         iowait_avg,iowait_peak\n",
    );
    for (rq, qb) in results {
        for r in [rq, qb] {
            out.push_str(&format!(
                "{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.4},{:.4}\n",
                r.scenario, r.client, r.total_bytes, r.duration_sec,
                r.avg_speed_mbps, r.peak_speed_mbps, r.time_to_first_piece,
                r.cpu_avg, r.cpu_peak, r.mem_avg_mb, r.mem_peak_mb,
                r.net_rx_avg_mbps, r.net_rx_peak_mbps,
                r.disk_write_avg_mbps, r.disk_write_peak_mbps,
                r.iowait_avg, r.iowait_peak,
            ));
        }
    }
    std::fs::write(&path, &out)?;
    tracing::info!("CSV: {}", path.display());
    Ok(())
}

pub fn build_summary(results: &[(ClientResult, ClientResult)]) -> String {
    let mut lines = Vec::new();
    lines.push(String::new());
    lines.push("=".repeat(78));
    lines.push("  BENCHMARK RESULTS: rtbit vs qBittorrent".into());
    lines.push("=".repeat(78));

    for (rq, qb) in results {
        lines.push(String::new());
        lines.push(format!("  Scenario: {} — {}", rq.scenario, rq.scenario_description));
        lines.push("-".repeat(78));
        lines.push(format!(
            "  {:24} {:>15} {:>15} {:>12}",
            "Metric", "rtbit", "qBittorrent", "Delta"
        ));
        lines.push("-".repeat(78));

        let metrics: Vec<(&str, String, String, f64, f64, bool)> = vec![
            ("Duration", format!("{:.1}s", rq.duration_sec), format!("{:.1}s", qb.duration_sec), rq.duration_sec, qb.duration_sec, true),
            ("Avg Speed", format!("{:.1} Mbps", rq.avg_speed_mbps), format!("{:.1} Mbps", qb.avg_speed_mbps), rq.avg_speed_mbps, qb.avg_speed_mbps, false),
            ("Peak Speed", format!("{:.1} Mbps", rq.peak_speed_mbps), format!("{:.1} Mbps", qb.peak_speed_mbps), rq.peak_speed_mbps, qb.peak_speed_mbps, false),
            ("Time to 1st Piece", format!("{:.2}s", rq.time_to_first_piece), format!("{:.2}s", qb.time_to_first_piece), rq.time_to_first_piece, qb.time_to_first_piece, true),
            ("CPU Avg", format!("{:.1}%", rq.cpu_avg), format!("{:.1}%", qb.cpu_avg), rq.cpu_avg, qb.cpu_avg, true),
            ("CPU Peak", format!("{:.1}%", rq.cpu_peak), format!("{:.1}%", qb.cpu_peak), rq.cpu_peak, qb.cpu_peak, true),
            ("Memory Avg", format!("{:.1} MB", rq.mem_avg_mb), format!("{:.1} MB", qb.mem_avg_mb), rq.mem_avg_mb, qb.mem_avg_mb, true),
            ("Memory Peak", format!("{:.1} MB", rq.mem_peak_mb), format!("{:.1} MB", qb.mem_peak_mb), rq.mem_peak_mb, qb.mem_peak_mb, true),
            ("Net RX Avg", format!("{:.1} Mbps", rq.net_rx_avg_mbps), format!("{:.1} Mbps", qb.net_rx_avg_mbps), rq.net_rx_avg_mbps, qb.net_rx_avg_mbps, false),
            ("Disk Write Avg", format!("{:.1} MB/s", rq.disk_write_avg_mbps), format!("{:.1} MB/s", qb.disk_write_avg_mbps), rq.disk_write_avg_mbps, qb.disk_write_avg_mbps, false),
            ("IO Wait Avg", format!("{:.2}%", rq.iowait_avg), format!("{:.2}%", qb.iowait_avg), rq.iowait_avg, qb.iowait_avg, true),
            ("IO Wait Peak", format!("{:.2}%", rq.iowait_peak), format!("{:.2}%", qb.iowait_peak), rq.iowait_peak, qb.iowait_peak, true),
        ];

        for (label, rq_s, qb_s, rq_v, qb_v, lower_better) in &metrics {
            let delta = delta_str(*rq_v, *qb_v, *lower_better);
            lines.push(format!("  {:<24} {:>15} {:>15} {:>12}", label, rq_s, qb_s, delta));
        }
        lines.push("-".repeat(78));
    }

    lines.push(String::new());
    lines.push("  Lower-is-better: Duration, Time to 1st Piece, CPU, Memory, IO Wait".into());
    lines.push("  Higher-is-better: Speed, Net RX, Disk Write".into());
    lines.push("  Delta shows rtbit advantage: positive = rtbit wins".into());
    lines.push(String::new());
    lines.join("\n")
}

pub fn write_summary(summary: &str, dir: &Path, timestamp: &str) -> Result<()> {
    let path = dir.join(format!("summary_{timestamp}.txt"));
    std::fs::write(&path, summary)?;
    tracing::info!("Summary: {}", path.display());
    Ok(())
}

fn delta_str(rq: f64, qb: f64, lower_better: bool) -> String {
    if qb == 0.0 && rq == 0.0 {
        return "\u{2014}".to_string(); // em dash
    }
    if qb == 0.0 {
        return "\u{2014}".to_string();
    }
    let mut pct = (qb - rq) / qb * 100.0;
    if !lower_better {
        pct = -pct;
    }
    let prefix = if pct > 0.0 { "+" } else { "" };
    format!("{prefix}{pct:.1}%")
}
