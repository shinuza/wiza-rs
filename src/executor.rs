use crate::model::*;
use anyhow::{anyhow, Context, Result};
use std::process::{Command, Output};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Skipped,
    Success,
    Failed,
}

#[derive(Debug, Default, Clone)]
pub struct StepRuntime {
    pub status: StepStatus,
    pub log: String,
}

/// Run a command through `bash -c` and capture output.
pub fn run_command(cmd: &str) -> Result<Output> {
    let output = Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .output()
        .with_context(|| format!("Failed to execute command: {}", cmd))?;
    Ok(output)
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
        StepKind::GitConfig { params } => {
            runtime.log.push_str("\n--- git_config ---\n");
            run_git_config(params, &mut runtime.log)?;
        }
        StepKind::AppSelection { params } => {
            runtime.log.push_str("\n--- app_selection ---\n");
            run_app_selection(params, &mut runtime.log)?;
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
fn run_git_config(params: &GitConfigParams, log: &mut String) -> Result<()> {
    use dialoguer::Input;

    // We temporarily drop raw mode when used from TUI.
    log.push_str("Entering interactive git configuration prompts...\n");

    // Ask for user data in the normal terminal.
    let name: String = Input::new()
        .with_prompt("Git user.name")
        .interact_text()
        .context("Failed to read git user.name")?;

    let email: String = Input::new()
        .with_prompt("Git user.email")
        .interact_text()
        .context("Failed to read git user.email")?;

    let editor: String = Input::new()
        .with_prompt("Preferred editor")
        .default(params.default_editor.clone())
        .interact_text()
        .context("Failed to read editor")?;

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
/// Uses dialoguer::MultiSelect for simple TUI-ish selection.
fn run_app_selection(params: &AppSelectionParams, log: &mut String) -> Result<()> {
    use dialoguer::MultiSelect;

    let mut items = Vec::new();
    for app in &params.apps {
        items.push(format!("{} ({})", app.name, app.version));
    }

    if items.is_empty() {
        log.push_str("No apps defined in this step.\n");
        return Ok(());
    }

    log.push_str("Opening app selection menu in terminal...\n");
    let selection = MultiSelect::new()
        .with_prompt("Select apps to install (space to select, enter to confirm)")
        .items(&items)
        .interact()
        .context("Failed during app selection")?;

    if selection.is_empty() {
        log.push_str("No apps selected.\n");
        return Ok(());
    }

    for idx in selection {
        let app = &params.apps[idx];
        log.push_str(&format!(
            "Installing {} ({}) using: {}\n",
            app.name, app.version, app.install
        ));
        let out = run_command(&app.install)?;
        append_output(log, &app.install, &out);
        if !out.status.success() {
            log.push_str(&format!("Installation of {} failed.\n", app.name));
            // continue to attempt next app, but keep note the failure.
        }
    }

    Ok(())
}
