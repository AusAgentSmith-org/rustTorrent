use crate::config::MB;
use crate::runner::ClientResult;
use anyhow::Result;
use std::path::Path;

const RTBIT_COLOR: &str = "#E45932";
const QBT_COLOR: &str = "#2681FF";
const BG_COLOR: &str = "#1a1a2e";
const PANEL_BG: &str = "#16213e";
const TEXT_COLOR: &str = "#ddd";
const GRID_COLOR: &str = "#333";
const MUTED_TEXT: &str = "#888";
const GREEN: &str = "#4CAF50";
const RED: &str = "#FF5722";

/// Generate all charts as SVGs.
pub fn generate_all(results: &[(ClientResult, ClientResult)], dir: &Path) -> Result<()> {
    tracing::info!("Generating charts...");
    for (rq, qb) in results {
        write_bar_chart(rq, qb, dir)?;
        write_timeseries(rq, qb, dir)?;
    }
    if results.len() > 1 {
        write_cross_scenario(results, dir)?;
    }
    write_dashboard(results, dir)?;
    let count = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "svg"))
        .count();
    tracing::info!("  Generated {count} chart(s)");
    Ok(())
}

/// Escape XML special characters in text content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn write_bar_chart(rq: &ClientResult, qb: &ClientResult, dir: &Path) -> Result<()> {
    // Group metrics into categories, each with its own scale
    let categories: Vec<(&str, Vec<(&str, f64, f64, &str)>)> = vec![
        (
            "Performance",
            vec![
                ("Duration", rq.duration_sec, qb.duration_sec, "s"),
                ("Avg Speed", rq.avg_speed_mbps, qb.avg_speed_mbps, "Mbps"),
                ("Peak Speed", rq.peak_speed_mbps, qb.peak_speed_mbps, "Mbps"),
                (
                    "Time to 1st Piece",
                    rq.time_to_first_piece,
                    qb.time_to_first_piece,
                    "s",
                ),
            ],
        ),
        (
            "Resource Usage",
            vec![
                ("CPU Avg", rq.cpu_avg, qb.cpu_avg, "%"),
                ("CPU Peak", rq.cpu_peak, qb.cpu_peak, "%"),
                ("Mem Avg", rq.mem_avg_mb, qb.mem_avg_mb, "MB"),
                ("Mem Peak", rq.mem_peak_mb, qb.mem_peak_mb, "MB"),
                ("IO Wait", rq.iowait_avg, qb.iowait_avg, "%"),
            ],
        ),
    ];

    // Filter out zero metrics and flatten
    let mut rows: Vec<(&str, &str, f64, f64, &str)> = Vec::new(); // (category, label, rq, qb, unit)
    let mut last_cat = "";
    for (cat, metrics) in &categories {
        for (label, r, q, unit) in metrics {
            if *r != 0.0 || *q != 0.0 {
                rows.push((cat, label, *r, *q, unit));
                last_cat = cat;
            }
        }
    }
    let _ = last_cat;

    let w = 900;
    let row_h = 44;
    // Count category headers
    let mut cat_headers = 0;
    let mut prev_cat = "";
    for (cat, _, _, _, _) in &rows {
        if *cat != prev_cat {
            cat_headers += 1;
            prev_cat = cat;
        }
    }
    let h = 90 + rows.len() * row_h + cat_headers * 30;
    let bar_w = 320.0;
    let label_x = 160.0;
    let bar_x = label_x + 30.0;

    let desc = xml_escape(&rq.scenario_description);

    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" style="background:{BG_COLOR}">
<text x="{}" y="28" fill="{TEXT_COLOR}" font-size="16" text-anchor="middle" font-family="monospace" font-weight="bold">Metric Comparison</text>
<text x="{}" y="48" fill="{MUTED_TEXT}" font-size="12" text-anchor="middle" font-family="monospace">{desc}</text>
<text x="{}" y="68" fill="{RTBIT_COLOR}" font-size="11" font-family="monospace">■ rtbit</text>
<text x="{}" y="68" fill="{QBT_COLOR}" font-size="11" font-family="monospace">■ qBittorrent</text>"#,
        w / 2,
        w / 2,
        w - 220,
        w - 110,
    );

    let mut y = 80;
    let mut prev_cat = "";
    for (cat, label, rq_v, qb_v, unit) in &rows {
        // Category header
        if *cat != prev_cat {
            y += 8;
            svg.push_str(&format!(
                r#"<text x="20" y="{}" fill="{MUTED_TEXT}" font-size="10" font-family="monospace" text-transform="uppercase">{cat}</text>"#,
                y + 12,
            ));
            svg.push_str(&format!(
                r#"<line x1="20" y1="{}" x2="{}" y2="{}" stroke="{GRID_COLOR}" stroke-width="1"/>"#,
                y + 16,
                w - 20,
                y + 16,
            ));
            y += 22;
            prev_cat = cat;
        }

        // Per-metric max for independent scaling
        let max_val = rq_v.max(*qb_v).max(0.001);
        let rq_w = rq_v / max_val * bar_w;
        let qb_w = qb_v / max_val * bar_w;

        // Label
        svg.push_str(&format!(
            r#"<text x="{label_x}" y="{}" fill="{TEXT_COLOR}" font-size="11" text-anchor="end" font-family="monospace">{label}</text>"#,
            y + 12,
        ));

        // rtbit bar
        svg.push_str(&format!(
            r#"<rect x="{bar_x}" y="{y}" width="{rq_w:.1}" height="16" fill="{RTBIT_COLOR}" opacity="0.9" rx="2"/>"#,
        ));
        svg.push_str(&format!(
            r#"<text x="{}" y="{}" fill="{TEXT_COLOR}" font-size="9" font-family="monospace">{:.1} {unit}</text>"#,
            bar_x + rq_w + 5.0,
            y + 12,
            rq_v,
        ));

        // qBittorrent bar
        let y2 = y + 20;
        svg.push_str(&format!(
            r#"<rect x="{bar_x}" y="{y2}" width="{qb_w:.1}" height="16" fill="{QBT_COLOR}" opacity="0.9" rx="2"/>"#,
        ));
        svg.push_str(&format!(
            r#"<text x="{}" y="{}" fill="{TEXT_COLOR}" font-size="9" font-family="monospace">{:.1} {unit}</text>"#,
            bar_x + qb_w + 5.0,
            y2 + 12,
            qb_v,
        ));

        y += row_h;
    }

    svg.push_str("</svg>");
    let path = dir.join(format!("{}_comparison.svg", rq.scenario));
    std::fs::write(&path, &svg)?;
    Ok(())
}

fn write_timeseries(rq: &ClientResult, qb: &ClientResult, dir: &Path) -> Result<()> {
    let panels: Vec<(&str, &str, Box<dyn Fn(&crate::metrics::MetricSample) -> f64>)> = vec![
        ("CPU", "%", Box::new(|s| s.cpu_pct)),
        ("Memory", "MB", Box::new(|s| s.mem_bytes as f64 / MB as f64)),
        ("Net RX", "Mbps", Box::new(|s| s.net_rx_bps * 8.0 / 1e6)),
        ("Disk Write", "MB/s", Box::new(|s| s.disk_write_bps / MB as f64)),
    ];

    let panel_h: usize = 160;
    let panel_gap: usize = 40;
    let w: usize = 850;
    let px: usize = 70;
    let pw: usize = w - 100;
    let h = 80 + panels.len() * (panel_h + panel_gap);

    let desc = xml_escape(&rq.scenario_description);

    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" style="background:{BG_COLOR}">
<text x="{}" y="28" fill="{TEXT_COLOR}" font-size="16" text-anchor="middle" font-family="monospace" font-weight="bold">Time Series</text>
<text x="{}" y="48" fill="{MUTED_TEXT}" font-size="12" text-anchor="middle" font-family="monospace">{desc}</text>
<text x="{}" y="66" fill="{RTBIT_COLOR}" font-size="11" font-family="monospace">— rtbit</text>
<text x="{}" y="66" fill="{QBT_COLOR}" font-size="11" font-family="monospace">— qBittorrent</text>"#,
        w / 2,
        w / 2,
        w - 220,
        w - 110,
    );

    // Compute shared time axis (max duration across both clients)
    let rq_dur = if rq.timeseries.is_empty() {
        0.0
    } else {
        rq.timeseries.last().unwrap().ts - rq.timeseries[0].ts
    };
    let qb_dur = if qb.timeseries.is_empty() {
        0.0
    } else {
        qb.timeseries.last().unwrap().ts - qb.timeseries[0].ts
    };
    let max_t = rq_dur.max(qb_dur).max(1.0);

    for (pi, (label, unit, extractor)) in panels.iter().enumerate() {
        let py = 75 + pi * (panel_h + panel_gap);

        // Compute shared Y max across both clients for this metric
        let rq_max = rq
            .timeseries
            .iter()
            .map(|s| extractor(s))
            .fold(0.0f64, f64::max);
        let qb_max = qb
            .timeseries
            .iter()
            .map(|s| extractor(s))
            .fold(0.0f64, f64::max);
        let max_v = rq_max.max(qb_max).max(0.01);

        // Panel background
        svg.push_str(&format!(
            r#"<rect x="{px}" y="{py}" width="{pw}" height="{panel_h}" fill="{PANEL_BG}" rx="4"/>"#,
        ));

        // Panel label
        svg.push_str(&format!(
            r#"<text x="{}" y="{}" fill="{TEXT_COLOR}" font-size="13" text-anchor="middle" font-family="monospace" font-weight="bold">{label} ({unit})</text>"#,
            px + pw / 2,
            py - 8,
        ));

        // Y-axis gridlines and labels (4 ticks)
        for tick in 0..=4 {
            let frac = tick as f64 / 4.0;
            let val = max_v * frac;
            let gy = (py + panel_h) as f64 - frac * (panel_h - 10) as f64;
            svg.push_str(&format!(
                r#"<line x1="{px}" y1="{gy:.0}" x2="{}" y2="{gy:.0}" stroke="{GRID_COLOR}" stroke-width="0.5"/>"#,
                px + pw,
            ));
            let label_text = if max_v >= 100.0 {
                format!("{:.0}", val)
            } else if max_v >= 1.0 {
                format!("{:.1}", val)
            } else {
                format!("{:.2}", val)
            };
            svg.push_str(&format!(
                r#"<text x="{}" y="{}" fill="{MUTED_TEXT}" font-size="8" text-anchor="end" font-family="monospace">{label_text}</text>"#,
                px - 4,
                gy + 3.0,
            ));
        }

        // X-axis time labels (5 ticks)
        for tick in 0..=4 {
            let frac = tick as f64 / 4.0;
            let t_val = max_t * frac;
            let gx = px as f64 + frac * pw as f64;
            svg.push_str(&format!(
                r#"<text x="{gx:.0}" y="{}" fill="{MUTED_TEXT}" font-size="8" text-anchor="middle" font-family="monospace">{:.0}s</text>"#,
                py + panel_h + 12,
                t_val,
            ));
        }

        // Plot data for both clients
        for (result, color, label) in [
            (rq, RTBIT_COLOR, "rtbit"),
            (qb, QBT_COLOR, "qBittorrent"),
        ] {
            if result.timeseries.is_empty() {
                continue;
            }
            let t0 = result.timeseries[0].ts;
            let dur = result.timeseries.last().unwrap().ts - t0;
            let points: Vec<(f64, f64)> = result
                .timeseries
                .iter()
                .map(|s| {
                    let t = s.ts - t0;
                    let v = extractor(s);
                    let x = px as f64 + (t / max_t) * pw as f64;
                    let y = (py + panel_h) as f64 - (v / max_v) * (panel_h - 10) as f64;
                    (x, y)
                })
                .collect();

            let pts_str: String = points
                .iter()
                .map(|(x, y)| format!("{:.1},{:.1}", x, y))
                .collect::<Vec<_>>()
                .join(" ");

            if !pts_str.is_empty() {
                svg.push_str(&format!(
                    r#"<polyline points="{pts_str}" fill="none" stroke="{color}" stroke-width="2" opacity="0.9"/>"#,
                ));
            }

            // "done" marker at the end of the line if it finishes before the chart ends
            if dur < max_t * 0.9 {
                if let Some(&(end_x, end_y)) = points.last() {
                    // Vertical dashed line
                    svg.push_str(&format!(
                        r#"<line x1="{end_x:.0}" y1="{}" x2="{end_x:.0}" y2="{}" stroke="{color}" stroke-dasharray="3" stroke-width="1" opacity="0.6"/>"#,
                        py,
                        py + panel_h,
                    ));
                    // Dot at end point
                    svg.push_str(&format!(
                        r#"<circle cx="{end_x:.1}" cy="{end_y:.1}" r="3" fill="{color}"/>"#,
                    ));
                    // "done" label (only on first panel to avoid clutter)
                    if pi == 0 {
                        svg.push_str(&format!(
                            r#"<text x="{:.0}" y="{}" fill="{color}" font-size="8" font-family="monospace">{label} done ({dur:.0}s)</text>"#,
                            end_x + 4.0,
                            py + 10,
                        ));
                    }
                }
            }
        }
    }

    svg.push_str("</svg>");
    let path = dir.join(format!("{}_timeseries.svg", rq.scenario));
    std::fs::write(&path, &svg)?;
    Ok(())
}

fn write_cross_scenario(results: &[(ClientResult, ClientResult)], dir: &Path) -> Result<()> {
    let n = results.len();
    let w = 950;
    let row_h = 50;
    let h = 90 + n * row_h;

    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" style="background:{BG_COLOR}">
<text x="{}" y="28" fill="{TEXT_COLOR}" font-size="16" text-anchor="middle" font-family="monospace" font-weight="bold">Cross-Scenario: Speed Ratio (qBittorrent duration / rtbit duration)</text>
<text x="{}" y="50" fill="{GREEN}" font-size="10" font-family="monospace">Green = rtbit faster</text>
<text x="{}" y="50" fill="{RED}" font-size="10" font-family="monospace">Red = qBittorrent faster</text>
<line x1="500" y1="60" x2="500" y2="{}" stroke="{MUTED_TEXT}" stroke-dasharray="4" stroke-width="1"/>
<text x="500" y="72" fill="{MUTED_TEXT}" font-size="8" text-anchor="middle" font-family="monospace">1.0x (equal)</text>"#,
        w / 2,
        w / 2 - 120,
        w / 2 + 60,
        80 + n * row_h,
    );

    let max_bar = 400.0;
    // Find max ratio to scale bars
    let max_ratio = results
        .iter()
        .map(|(rq, qb)| {
            if rq.duration_sec > 0.0 {
                qb.duration_sec / rq.duration_sec
            } else {
                1.0
            }
        })
        .fold(0.0f64, f64::max)
        .max(2.0);

    for (i, (rq, qb)) in results.iter().enumerate() {
        let y = 80 + i * row_h;
        let ratio = if rq.duration_sec > 0.0 {
            qb.duration_sec / rq.duration_sec
        } else {
            1.0
        };
        let bar_w = (ratio / max_ratio * max_bar).min(max_bar);
        let color = if ratio >= 1.0 { GREEN } else { RED };
        let desc = xml_escape(&rq.scenario_description);

        // Scenario description
        svg.push_str(&format!(
            r#"<text x="290" y="{}" fill="{TEXT_COLOR}" font-size="10" text-anchor="end" font-family="monospace">{}</text>"#,
            y + 16,
            xml_escape(&rq.scenario),
        ));
        svg.push_str(&format!(
            r#"<text x="290" y="{}" fill="{MUTED_TEXT}" font-size="8" text-anchor="end" font-family="monospace">{desc}</text>"#,
            y + 28,
        ));

        // Bar
        svg.push_str(&format!(
            r#"<rect x="300" y="{}" width="{bar_w:.0}" height="24" fill="{color}" opacity="0.85" rx="3"/>"#,
            y + 6,
        ));
        svg.push_str(&format!(
            r#"<text x="{}" y="{}" fill="{TEXT_COLOR}" font-size="11" font-family="monospace" font-weight="bold">{:.2}x</text>"#,
            300.0 + bar_w + 8.0,
            y + 23,
            ratio,
        ));

        // Reference line at 1.0 for this row
        let ref_x = 300.0 + (1.0 / max_ratio * max_bar);
        svg.push_str(&format!(
            r#"<line x1="{ref_x:.0}" y1="{}" x2="{ref_x:.0}" y2="{}" stroke="{MUTED_TEXT}" stroke-dasharray="4" stroke-width="1"/>"#,
            y + 6,
            y + 30,
        ));
    }

    svg.push_str("</svg>");
    std::fs::write(dir.join("cross_scenario.svg"), &svg)?;
    Ok(())
}

fn write_dashboard(results: &[(ClientResult, ClientResult)], dir: &Path) -> Result<()> {
    let n = results.len();
    let w = 1100;
    let row_h = 90;
    let h = 110 + n * row_h;

    let col_labels = ["Duration (s)", "Avg Speed (Mbps)", "CPU Peak (%)", "Mem Peak (MB)"];

    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" style="background:{BG_COLOR}">
<text x="{}" y="28" fill="{TEXT_COLOR}" font-size="18" text-anchor="middle" font-family="monospace" font-weight="bold">Benchmark Dashboard: rtbit vs qBittorrent</text>
<text x="60" y="55" fill="{RTBIT_COLOR}" font-size="12" font-family="monospace">■ rtbit</text>
<text x="160" y="55" fill="{QBT_COLOR}" font-size="12" font-family="monospace">■ qBittorrent</text>"#,
        w / 2,
    );

    // Column headers
    for (ci, col_label) in col_labels.iter().enumerate() {
        let cx = 320 + ci * 190;
        svg.push_str(&format!(
            r#"<text x="{cx}" y="75" fill="{MUTED_TEXT}" font-size="10" font-family="monospace">{col_label}</text>"#,
        ));
    }

    // Header divider
    svg.push_str(&format!(
        r#"<line x1="10" y1="82" x2="{}" y2="82" stroke="{GRID_COLOR}" stroke-width="1"/>"#,
        w - 10,
    ));

    for (i, (rq, qb)) in results.iter().enumerate() {
        let y = 90 + i * row_h;
        let max_dur = rq.duration_sec.max(qb.duration_sec).max(1.0);
        let max_spd = rq.avg_speed_mbps.max(qb.avg_speed_mbps).max(1.0);
        let max_cpu = rq.cpu_peak.max(qb.cpu_peak).max(0.1);
        let max_mem = rq.mem_peak_mb.max(qb.mem_peak_mb).max(1.0);

        // Scenario name + description
        svg.push_str(&format!(
            r#"<text x="15" y="{}" fill="{TEXT_COLOR}" font-size="11" font-family="monospace" font-weight="bold">{}</text>"#,
            y + 18,
            xml_escape(&rq.scenario),
        ));
        svg.push_str(&format!(
            r#"<text x="15" y="{}" fill="{MUTED_TEXT}" font-size="9" font-family="monospace">{}</text>"#,
            y + 32,
            xml_escape(&rq.scenario_description),
        ));

        let cols: Vec<(f64, f64, f64)> = vec![
            (rq.duration_sec, qb.duration_sec, max_dur),
            (rq.avg_speed_mbps, qb.avg_speed_mbps, max_spd),
            (rq.cpu_peak, qb.cpu_peak, max_cpu),
            (rq.mem_peak_mb, qb.mem_peak_mb, max_mem),
        ];

        for (ci, (rv, qv, maxv)) in cols.iter().enumerate() {
            let cx = 320 + ci * 190;
            let bw = 140.0;
            let rw = rv / maxv * bw;
            let qw = qv / maxv * bw;

            svg.push_str(&format!(
                r#"<rect x="{cx}" y="{}" width="{rw:.0}" height="14" fill="{RTBIT_COLOR}" opacity="0.9" rx="2"/>"#,
                y + 5,
            ));
            svg.push_str(&format!(
                r#"<rect x="{cx}" y="{}" width="{qw:.0}" height="14" fill="{QBT_COLOR}" opacity="0.9" rx="2"/>"#,
                y + 22,
            ));
            svg.push_str(&format!(
                r#"<text x="{}" y="{}" fill="{TEXT_COLOR}" font-size="8" font-family="monospace">{:.1}</text>"#,
                cx as f64 + rw + 3.0,
                y + 15,
                rv,
            ));
            svg.push_str(&format!(
                r#"<text x="{}" y="{}" fill="{TEXT_COLOR}" font-size="8" font-family="monospace">{:.1}</text>"#,
                cx as f64 + qw + 3.0,
                y + 32,
                qv,
            ));
        }

        // Row divider
        if i < n - 1 {
            svg.push_str(&format!(
                r#"<line x1="10" y1="{}" x2="{}" y2="{}" stroke="{GRID_COLOR}" stroke-width="1"/>"#,
                y + row_h - 5,
                w - 10,
                y + row_h - 5,
            ));
        }
    }

    svg.push_str("</svg>");
    std::fs::write(dir.join("dashboard.svg"), &svg)?;
    Ok(())
}
