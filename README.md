# wiza-rs

`wiza-rs` is a small TUI helper that walks you through a sequence of provisioning or setup steps defined in a YAML file. It is designed for things like bootstrapping a fresh WSL / Linux environment: running checks, appending config, tweaking git, and installing common tools.

The tool does **not** hard-code any logic about your setup. Instead, you describe steps in a `steps.yaml` file, and `wiza-rs` executes them in order while showing output in a simple terminal UI.

---

## Requirements

- Rust toolchain (to build from source): https://rustup.rs
- A Unix-like shell (`bash`) available in PATH
- For some steps you may also need:
  - `sudo` (for installation steps that require root)
  - Network access (for package installation, connectivity checks, etc.)

This project is primarily intended for use inside WSL or a similar Linux environment.

---

## Building

Clone the repository and build the binary:

```bash
git clone https://github.com/shinuza/wiza-rs.git
cd wiza-rs
cargo build --release
```

The resulting binary will be at:

```bash
target/release/wiza-rs
```

You can copy this somewhere on your PATH if you like.

---

## Running

By default, `wiza-rs` looks for a file named `steps.yaml` in the current working directory:

```bash
./target/release/wiza-rs
```

You can also pass an explicit path to a YAML file:

```bash
./target/release/wiza-rs /path/to/your-steps.yaml
```

If parsing or validating the YAML fails, the program will exit with an error message describing what went wrong.

---

## Terminal UI

When you run `wiza-rs`, it opens a small TUI:

- **Left pane**
  - Shows the list of steps with their statuses: Pending, Running, Skipped, Success, Failed.
- **Right pane**
  - Shows logs for the currently selected step: output from pre-scripts, main scripts, post-scripts, and any helper actions.

### Key bindings

- `Enter` — Run the currently selected step
- `n` — Move to the next step
- `p` — Move to the previous step
- `s` — Skip the current step (mark as Skipped)
- Arrow `Up` / `Down` — Scroll within the log for the selected step
- `PageUp` / `PageDown` — Faster log scrolling (if supported by your terminal)
- `q` — Quit the wizard

The exact set of keys is also shown in a small Help box in the UI.

---

## Step file format (`steps.yaml`)

At a high level, your YAML file looks like this:

```yaml
steps:
  - name: "Description of step"
    type: <step-type>
    # optional: shell commands executed before/after the main action
    pre_script: "..."
    post_script: "..."
    params: ... # depends on the step type
```

Each step has:

- **`name`** (string) — Human-friendly label shown in the UI.
- **`type`** (string) — One of the supported step kinds:
  - `script`
  - `add_text`
  - `git_config`
  - `app_selection`
- **`pre_script`** (optional, string) — Shell command run before the main action. If it fails, the step will be marked as failed.
- **`post_script`** (optional, string) — Shell command run after the main action.
- **`params`** — A nested object whose shape depends on `type` (see below).

### `script` step

Runs arbitrary shell commands. Uses `pre_script`, `script`, and `post_script` as-is.

Example:

```yaml
- name: "Check internet connectivity"
  type: script
  pre_script: "ping -c 1 google.com >/dev/null 2>&1"
  script: "echo 'Internet looks good!'"
  post_script: "echo 'Step complete.'"
```

### `add_text` step

Appends some text to a file, optionally gated by a `pre_script`.

Params:

- `file` — Path to the file to modify.
- `content` — Text to append (a newline is usually added if needed).

Example:

```yaml
- name: "Append custom line to .bashrc"
  type: add_text
  pre_script: "test -f ~/.bashrc"
  post_script: "echo 'Remember to reload your shell later.'"
  params:
    file: "/home/$USER/.bashrc"
    content: "export PATH=\"$HOME/.local/bin:$PATH\""
```

### `git_config` step

Configures some opinionated git settings. The exact behavior is controlled by code, but you can specify defaults.

Params:

- `default_editor` — Editor to set as the default (e.g. `vim`, `nvim`, `code --wait`).

Example:

```yaml
- name: "Configure Git"
  type: git_config
  params:
    default_editor: "vim"
```

### `app_selection` step

Shows an interactive checklist (via `dialoguer::MultiSelect`) where you can choose which apps to install.

Params:

- `apps` — List of applications:
  - `name` — Display name of the app.
  - `version` — Version string for display only.
  - `install` — Shell command used to install the app.

Example:

```yaml
- name: "Install common dev tools"
  type: app_selection
  params:
    apps:
      - name: "Neovim"
        version: "0.9"
        install: "sudo apt update && sudo apt install -y neovim"
      - name: "Git"
        version: "2.x"
        install: "sudo apt update && sudo apt install -y git"
      - name: "htop"
        version: "latest"
        install: "sudo apt update && sudo apt install -y htop"
```

---

## Error handling and validation

On startup, `wiza-rs` parses your YAML into an internal model and runs a validation pass. If anything is wrong (missing fields, wrong types, unknown step kinds), it will:

- Print a clear error message describing the issue.
- Exit with a non-zero status code.

Fix the YAML as indicated and re-run the tool.

At runtime, each step accumulates its own log. If a `pre_script`, main script, or installation command fails, the step status becomes `Failed`, and the error output is shown in the log pane.

---

## Tips

- Keep your `steps.yaml` in version control so you can share it across machines.
- Start with simple `echo` and `script` steps to verify your flow before adding destructive commands.
- Prefer idempotent operations where possible (e.g. `apt install -y`, `mkdir -p`, checking for existing config before appending).

---

## License

This project is licensed under the MIT License.

See the `LICENSE` file for full details.
