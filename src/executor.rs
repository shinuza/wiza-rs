use crate::model::*;
use anyhow::{anyhow, Context, Result};
use std::process::{Command, ExitStatus, Output};


/// Run a command through `bash -c` and capture output.
pub fn run_command(cmd: &str) -> Result<Output> {
    let output = Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .output()
        .with_context(|| format!("Failed to execute command: {}", cmd))?;
    Ok(output)
}

/// Run a command through `bash -c` and stream output directly to the terminal.
/// This is useful for long-running installs (e.g. apt-get) where we want
/// to see progress in real time rather than only after completion.
pub fn run_command_streaming(cmd: &str) -> Result<ExitStatus> {
    let status = Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .status()
        .with_context(|| format!("Failed to execute command: {}", cmd))?;
    Ok(status)
}

fn append_output(log: &mut String, label: &str, out: &Output) {
    use std::str;

    let status_code = out.status.code().unwrap_or(-1);
    log.push_str(&format!("\n$ {}\n", label));
    if !out.stdout.is_empty() {
        let stdout = str::from_utf8(&out.stdout).unwrap_or("<invalid utf-8>");
        log.push_str(stdout);
    }
    if !out.stderr.is_empty() {
        let stderr = str::from_utf8(&out.stderr).unwrap_or("<invalid utf-8>");
        log.push_str("\n[stderr]\n");
        log.push_str(stderr);
    }
    log.push_str(&format!("\n[exit code: {}]\n", status_code));
}

/// Start sudo session at startup.
pub fn start_sudo_session(log: &mut String) -> Result<()> {
    log.push_str("Initializing sudo session with `sudo -v`...\n");
    let output = run_command("sudo -v")?;
    append_output(log, "sudo -v", &output);
    if !output.status.success() {
        return Err(anyhow!("sudo -v failed; sudo may not be available"));
    }
    Ok(())
}

/// Run a single step (pre/script/post + task-specific logic).  
/// Returns updated StepRuntime.
pub fn run_step(step: &Step, runtime: &mut StepRuntime) -> Result<()> {
    runtime.status = StepStatus::Running;
    runtime.log.push_str(&format!("== Running step: {} ==\n", step.name));

    // Run pre_script if any.
    if let Some(pre) = &step.pre_script {
        runtime.log.push_str("\n--- pre_script ---\n");
        let out = run_command(pre)?;
        append_output(&mut runtime.log, pre, &out);
        if !out.status.success() {
            runtime.log.push_str("\npre_script failed; step will be skipped.\n");
            runtime.status = StepStatus::Skipped;
            return Ok(());
        }
    }

    // Dispatch main task depending on type.
    match &step.kind {
        StepKind::Script => {
            if let Some(script) = &step.script {
                runtime.log.push_str("\n--- script ---\n");
                let out = run_command(script)?;
                append_output(&mut runtime.log, script, &out);
                if !out.status.success() {
                    runtime.status = StepStatus::Failed;
                    return Ok(());
                }
            } else {
                runtime.log.push_str("\nNo script specified for script step.\n");
            }
        }
        StepKind::AddText { params } => {
            runtime.log
                .push_str(&format!("\n--- add_text to {} ---\n", params.file));
            run_add_text(params, &mut runtime.log)?;
        }
        StepKind::GitConfig { params: _ } => {
            // For git_config, the interactive UI (ratatui) is responsible for
            // gathering values and invoking the actual configuration logic.
            runtime.log.push_str("\n--- git_config (handled by TUI) ---\n");
        }
        StepKind::AppSelection { params: _ } => {
            // For app_selection, the interactive UI (ratatui) is responsible for
            // gathering the selection and invoking the actual installation logic.
            runtime.log.push_str("\n--- app_selection (handled by TUI) ---\n");
        }
    }

    // Run post_script if any.
    if let Some(post) = &step.post_script {
        runtime.log.push_str("\n--- post_script ---\n");
        let out = run_command(post)?;
        append_output(&mut runtime.log, post, &out);
        if !out.status.success() {
            runtime.status = StepStatus::Failed;
            return Ok(());
        }
    }

    if runtime.status == StepStatus::Running {
        runtime.status = StepStatus::Success;
    }

    Ok(())
}

/// Task: add text to a file.
fn run_add_text(params: &AddTextParams, log: &mut String) -> Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&params.file)
        .with_context(|| format!("Failed to open file: {}", params.file))?;

    writeln!(f, "{}", params.content).context("Failed to write to file")?;
    log.push_str(&format!(
        "Appended content to {}\n",
        params.file
    ));
    Ok(())
}

/// Task: git config (name, email, editor).
/// The ratatui layer gathers the values; this helper simply applies them.
pub fn apply_git_config(
    params: &GitConfigParams,
    name: &str,
    email: &str,
    editor: &str,
    log: &mut String,
) -> Result<()> {
    let name = name.trim();
    let email = email.trim();
    let editor = if editor.trim().is_empty() {
        params.default_editor.trim()
    } else {
        editor.trim()
    };

    if name.is_empty() {
        return Err(anyhow!("Git user.name cannot be empty"));
    }
    if email.is_empty() {
        return Err(anyhow!("Git user.email cannot be empty"));
    }

    let commands = vec![
        format!("git config --global user.name '{}'", name.replace('\'', "\\'")),
        format!(
            "git config --global user.email '{}'",
            email.replace('\'', "\\'")
        ),
        format!(
            "git config --global core.editor '{}'",
            editor.replace('\'', "\\'")
        ),
    ];

    for cmd in commands {
        let out = run_command(&cmd)?;
        append_output(log, &cmd, &out);
        if !out.status.success() {
            return Err(anyhow!("Command failed: {}", cmd));
        }
    }

    log.push_str("Git configuration updated.\n");
    Ok(())
}

/// Task: app selection and installation.
/// The ratatui layer is responsible for gathering which indices are selected; this
/// helper only performs the installations and logs the results.
pub fn apply_app_selection(
    params: &AppSelectionParams,
    selection: &[usize],
    log: &mut String,
) -> Result<()> {
    if params.apps.is_empty() {
        log.push_str("No apps defined in this step.\n");
        return Ok(());
    }

    if selection.is_empty() {
        log.push_str("No apps selected.\n");
        return Ok(());
    }

    for &idx in selection {
        if let Some(app) = params.apps.get(idx) {
            log.push_str(&format!(
                "Installing {} ({}) using: {}\n",
                app.name, app.version, app.install
            ));
            let status = run_command_streaming(&app.install)?;
            if !status.success() {
                log.push_str(&format!("Installation of {} failed.\n", app.name));
                // continue to attempt next app, but keep note the failure.
            }
        }
    }

    Ok(())
}
