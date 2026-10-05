use std::{
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// Ceiling on exact duplicate code percentage across the workspace.
///
/// This is a ratchet, not a target: `cargo-dupes` also counts normalised SQL
/// strings (such as distinct `init_schema` definitions) and repeated
/// structure across crates, so zero is not reachable. The check fails when
/// the measured value is above the ceiling, and also when it is more than
/// `DUPLICATION_SLACK` below it. Then the ceiling must be lowered in the same
/// change, so quality that was gained cannot be given back later. Below
/// `DUPLICATION_FLOOR` the ceiling stops moving.
const MAX_EXACT_DUPLICATE_PERCENT: f64 = 8.0;
const DUPLICATION_SLACK: f64 = 0.3;
const DUPLICATION_FLOOR: f64 = 5.0;

#[derive(Deserialize)]
struct DupesStats {
    exact_duplicate_percent: f64,
}

fn measure_exact_duplicate_percent(workspace_root: &Path) -> Result<f64> {
    let output = Command::new("cargo")
        .args([
            "dupes",
            "stats",
            "--format",
            "json",
            "--exclude",
            "bindings.rs",
            "--exclude",
            "target",
        ])
        .current_dir(workspace_root)
        .stderr(Stdio::inherit())
        .output()
        .context(
            "failed to run `cargo dupes stats` -- is cargo-dupes installed? (`cargo install \
             cargo-dupes`)",
        )?;
    if !output.status.success() {
        bail!("`cargo dupes stats` failed: {}", output.status);
    }
    let stats: DupesStats = serde_json::from_slice(&output.stdout)
        .context("could not parse `cargo dupes stats` JSON output")?;
    Ok(stats.exact_duplicate_percent)
}

/// The highest ceiling `check_duplication` accepts for `measured`, rounded
/// down to a tenth. Rounding down keeps it at or above `measured`.
fn highest_accepted_ceiling(measured: f64) -> f64 {
    (((measured + DUPLICATION_SLACK).max(DUPLICATION_FLOOR)) * 10.0).floor() / 10.0
}

fn duplication_verdict(measured: f64, ceiling: f64) -> Result<()> {
    if measured > ceiling {
        bail!("Duplication check failed: {measured:.2}% exceeds the ceiling of {ceiling:.1}%");
    }
    let highest = highest_accepted_ceiling(measured);
    if ceiling > highest {
        bail!(
            "Duplication is {measured:.2}%, well below the ceiling of {ceiling:.1}%. Lower \
             MAX_EXACT_DUPLICATE_PERCENT in xtask/src/duplication.rs to {highest:.1}."
        );
    }
    Ok(())
}

pub fn check_duplication() -> Result<()> {
    println!(
        "Checking exact-duplicate code percentage (ceiling {MAX_EXACT_DUPLICATE_PERCENT:.1}%)..."
    );
    let measured = measure_exact_duplicate_percent(&crate::get_workspace_root())?;
    println!("Exact duplication: {measured:.2}%");
    duplication_verdict(measured, MAX_EXACT_DUPLICATE_PERCENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(measured: f64, ceiling: f64) -> String {
        duplication_verdict(measured, ceiling).err().map(|e| e.to_string()).unwrap_or_default()
    }

    #[test]
    fn above_ceiling_fails() {
        assert!(message(8.1, 8.0).contains("exceeds the ceiling"));
    }

    #[test]
    fn within_slack_passes() {
        assert!(duplication_verdict(7.9, 8.0).is_ok());
        assert!(duplication_verdict(8.0, 8.0).is_ok());
        assert!(duplication_verdict(7.7, 8.0).is_ok());
    }

    #[test]
    fn far_below_asks_to_lower() {
        assert!(message(7.5, 8.0).contains("to 7.8."));
    }

    #[test]
    fn suggested_ceiling_passes_on_the_next_run() {
        for hundredths in 100..1000 {
            let measured = f64::from(hundredths) / 100.0;
            let suggested = highest_accepted_ceiling(measured);
            assert!(suggested >= measured, "{measured}: {suggested} is below it");
            assert!(
                duplication_verdict(measured, suggested).is_ok(),
                "measured {measured}: suggested {suggested} fails again"
            );
        }
    }

    #[test]
    fn floor_stops_further_lowering() {
        assert!(duplication_verdict(3.0, DUPLICATION_FLOOR).is_ok());
        assert!(message(3.0, 6.0).contains("to 5.0."));
    }
}
