use anyhow::{Context, bail};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
fn run_cmd(cwd: &Path, cmd: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    let (shell, shell_args) = ("powershell", ["-command"].as_slice());
    #[cfg(not(target_os = "windows"))]
    let (shell, shell_args) = ("sh", ["-c"].as_slice());

    // Run "npm install" in the webui directory
    let output = Command::new(shell)
        .args(shell_args)
        .arg(cmd)
        .current_dir(cwd)
        .output()
        .with_context(|| {
            format!(
                "Failed to execute {} in {:?}. PATH: {:?}",
                cmd,
                cwd,
                std::env::var("PATH").unwrap_or_default()
            )
        })?;

    if !output.status.success() {
        bail!(
            "\"{}\" failed\n\nstderr: {}\n\nstdout: {}",
            cmd,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }

    // Optionally print the stdout output if you want to see the build logs
    println!("{}", String::from_utf8_lossy(&output.stdout));

    Ok(())
}

#[allow(dead_code)]
fn run_cmd_with_retry(cwd: &Path, cmd: &str, attempts: usize) -> anyhow::Result<()> {
    let mut last_error = None;

    for attempt in 1..=attempts {
        match run_cmd(cwd, cmd) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < attempts => {
                println!("cargo:warning={cmd} attempt {attempt}/{attempts} failed; retrying");
                last_error = Some(error);
                thread::sleep(Duration::from_secs(5 * attempt as u64));
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.expect("attempts must be greater than zero"))
}

fn main() {
    #[cfg(feature = "webui")]
    {
        let webui_dir = Path::new("webui");
        let webui_src_dir = webui_dir.join("src");

        println!("cargo:rerun-if-changed={}", webui_src_dir.to_str().unwrap());

        // Registry connections can reset during slow cross-architecture
        // container builds. Retry only the network-dependent install step;
        // keep the deterministic build command fail-fast.
        run_cmd_with_retry(webui_dir, "npm ci --no-audit --no-fund", 3).unwrap();
        run_cmd(webui_dir, "npm run build").unwrap();
    }
}
