mod executor;
mod model;
mod tui;

use anyhow::{Context, Result};
use model::StepFile;
use std::fs;

fn main() -> Result<()> {
    let yaml_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "steps.yaml".to_string());

    let yaml_content =
        fs::read_to_string(&yaml_path).with_context(|| format!("Failed to read {}", yaml_path))?;

    let steps_file: StepFile =
        serde_yaml::from_str(&yaml_content).context("Failed to parse YAML")?;

    // NEW: schema validation with friendly errors
    steps_file
        .validate()
        .context("YAML failed validation")?;

    tui::run_tui(&steps_file.steps)
}
