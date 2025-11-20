use serde::Deserialize;
use anyhow::{Result, anyhow};

#[derive(Debug, Deserialize)]
pub struct StepFile {
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
pub struct Step {
    pub name: String,

    #[serde(flatten)]
    pub kind: StepKind,

    #[serde(default)]
    pub pre_script: Option<String>,

    #[serde(default)]
    pub script: Option<String>,

    #[serde(default)]
    pub post_script: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum StepKind {
    #[serde(rename = "script")]
    Script, // uses pre_script/script/post_script as-is

    #[serde(rename = "add_text")]
    AddText { params: AddTextParams },

    #[serde(rename = "git_config")]
    GitConfig { params: GitConfigParams },

    #[serde(rename = "app_selection")]
    AppSelection { params: AppSelectionParams },
}

#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub enum StepStatus {
    #[default]
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

#[derive(Debug, Deserialize, Clone)]
pub struct AddTextParams {
    pub file: String,
    pub content: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GitConfigParams {
    #[serde(default = "default_editor")]
    pub default_editor: String,
}

fn default_editor() -> String {
    "vim".into()
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppSelectionParams {
    pub apps: Vec<AppDefinition>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppDefinition {
    pub name: String,
    pub version: String,
    /// Command used to install this app (apt or custom script).
    pub install: String,
}

// ------------------ NEW: validation helpers ------------------

impl StepFile {
    pub fn validate(&self) -> Result<()> {
        if self.steps.is_empty() {
            return Err(anyhow!("YAML must contain at least one step."));
        }

        for (i, step) in self.steps.iter().enumerate() {
            if step.name.trim().is_empty() {
                return Err(anyhow!("Step {} has an empty name.", i));
            }

            match &step.kind {
                StepKind::Script => {
                    // Optional: enforce script presence if you want
                    if step.script.is_none() {
                        return Err(anyhow!(
                            "Step '{}' (script) is missing 'script' field.",
                            step.name
                        ));
                    }
                }
                StepKind::AddText { params } => {
                    if params.file.trim().is_empty() {
                        return Err(anyhow!(
                            "Step '{}' (add_text) has empty 'file' param.",
                            step.name
                        ));
                    }
                    if params.content.is_empty() {
                        return Err(anyhow!(
                            "Step '{}' (add_text) has empty 'content' param.",
                            step.name
                        ));
                    }
                }
                StepKind::GitConfig { params: _ } => {
                    // Nothing mandatory besides defaults; you could check default_editor if you want.
                }
                StepKind::AppSelection { params } => {
                    if params.apps.is_empty() {
                        return Err(anyhow!(
                            "Step '{}' (app_selection) must have at least one app.",
                            step.name
                        ));
                    }
                    for app in &params.apps {
                        if app.name.trim().is_empty() {
                            return Err(anyhow!(
                                "Step '{}' (app_selection) has an app with empty name.",
                                step.name
                            ));
                        }
                        if app.install.trim().is_empty() {
                            return Err(anyhow!(
                                "Step '{}' (app_selection) app '{}' has empty install command.",
                                step.name,
                                app.name
                            ));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
