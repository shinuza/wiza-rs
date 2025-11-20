use crate::executor::{apply_app_selection, apply_git_config, run_step, start_sudo_session};
use crate::model::{Step, StepKind, StepRuntime, StepStatus};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    execute,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use std::io::{stdout, Stdout};

pub struct App<'a> {
    pub steps: &'a [Step],
    pub runtimes: Vec<StepRuntime>,
    pub current: usize,
    pub global_log: String,
    // NEW: vertical scroll offset for the current step's log
    pub log_scroll: u16,
    pub mode: InteractiveMode,
}

impl<'a> App<'a> {
    pub fn new(steps: &'a [Step]) -> Self {
        Self {
            steps,
            runtimes: vec![StepRuntime::default(); steps.len()],
            current: 0,
            global_log: String::new(),
            log_scroll: 0,
            mode: InteractiveMode::None,
        }
    }

    pub fn current_runtime_mut(&mut self) -> &mut StepRuntime {
        &mut self.runtimes[self.current]
    }

    pub fn current_runtime(&self) -> &StepRuntime {
        &self.runtimes[self.current]
    }

    fn reset_scroll(&mut self) {
        self.log_scroll = 0;
    }
}

#[derive(Debug, Clone)]
pub enum InteractiveMode {
    None,
    AppSelection(AppSelectionState),
    GitConfig(GitConfigState),
}

#[derive(Debug, Clone)]
pub struct AppSelectionState {
    pub cursor: usize,
    pub selected: Vec<bool>,
}

#[derive(Debug, Clone, Copy)]
pub enum GitField {
    Name,
    Email,
    Editor,
}

#[derive(Debug, Clone)]
pub struct GitConfigState {
    pub field: GitField,
    pub name: String,
    pub email: String,
    pub editor: String,
}

pub fn run_tui(steps: &[Step]) -> Result<()> {
    // Initialize TUI.
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run(&mut terminal, steps);

    // Restore terminal.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
    terminal.show_cursor()?;

    res
}

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, steps: &[Step]) -> Result<()> {
    let mut app = App::new(steps);

    // Start sudo at the very beginning.
    {
        // Temporarily leave raw mode to let sudo prompt if needed
        disable_raw_mode()?;
        let mut dummy_runtime = StepRuntime::default();
        start_sudo_session(&mut dummy_runtime.log)?;
        enable_raw_mode()?;
        app.global_log.push_str(&dummy_runtime.log);
    }

    loop {
        terminal.draw(|f| match &app.mode {
            InteractiveMode::None => ui(f, &app),
            InteractiveMode::AppSelection(state) => ui_app_selection(f, &app, state),
            InteractiveMode::GitConfig(state) => ui_git_config(f, &app, state),
        })?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match &mut app.mode {
                    InteractiveMode::None => match code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('n') => {
                            if app.current + 1 < app.steps.len() {
                                app.current += 1;
                                app.reset_scroll();
                            }
                        }
                        KeyCode::Char('p') => {
                            if app.current > 0 {
                                app.current -= 1;
                                app.reset_scroll();
                            }
                        }
                        KeyCode::Char('s') => {
                            let rt = app.current_runtime_mut();
                            rt.status = StepStatus::Skipped;
                            rt.log.push_str("Step manually skipped.\n");
                        }
                        KeyCode::Enter => {
                            let step = &app.steps[app.current];
                            match &step.kind {
                                StepKind::AppSelection { params } => {
                                    // Enter interactive app selection mode.
                                    let rt = app.current_runtime_mut();
                                    rt.status = StepStatus::Running;
                                    rt.log.push_str(&format!(
                                        "== Running step: {} (app selection) ==\n",
                                        step.name
                                    ));

                                    // Initialise selection state.
                                    let state = AppSelectionState {
                                        cursor: 0,
                                        selected: vec![false; params.apps.len()],
                                    };
                                    app.mode = InteractiveMode::AppSelection(state);
                                    app.reset_scroll();
                                }
                                StepKind::GitConfig { params } => {
                                    let rt = app.current_runtime_mut();
                                    rt.status = StepStatus::Running;
                                    rt.log.push_str(&format!(
                                        "== Running step: {} (git config) ==\n",
                                        step.name
                                    ));

                                    let state = GitConfigState {
                                        field: GitField::Name,
                                        name: String::new(),
                                        email: String::new(),
                                        editor: params.default_editor.clone(),
                                    };
                                    app.mode = InteractiveMode::GitConfig(state);
                                    app.reset_scroll();
                                }
                                _ => {
                                    // Non-interactive steps use the existing executor flow.
                                    disable_raw_mode()?;
                                    let rt = app.current_runtime_mut();
                                    let res = run_step(step, rt);
                                    enable_raw_mode()?;
                                    app.reset_scroll();
                                    if let Err(e) = res {
                                        let rt = app.current_runtime_mut();
                                        rt.status = StepStatus::Failed;
                                        rt.log.push_str(&format!("\n[ERROR] {}\n", e));
                                    }
                                }
                            }
                        }

                        KeyCode::Up => {
                            app.log_scroll = app.log_scroll.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            app.log_scroll = app.log_scroll.saturating_add(1);
                        }
                        KeyCode::PageUp => {
                            app.log_scroll = app.log_scroll.saturating_sub(10);
                        }
                        KeyCode::PageDown => {
                            app.log_scroll = app.log_scroll.saturating_add(10);
                        }

                        _ => {}
                    },
                    InteractiveMode::AppSelection(state) => match code {
                        KeyCode::Esc => {
                            // Cancel selection, leave step Pending.
                            let rt = app.current_runtime_mut();
                            rt.status = StepStatus::Pending;
                            rt.log.push_str("App selection cancelled.\n");
                            app.mode = InteractiveMode::None;
                            app.reset_scroll();
                        }
                        KeyCode::Up => {
                            if state.cursor > 0 {
                                state.cursor -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if state.cursor + 1 < state.selected.len() {
                                state.cursor += 1;
                            }
                        }
                        KeyCode::Char(' ') => {
                            if let Some(sel) = state.selected.get_mut(state.cursor) {
                                *sel = !*sel;
                            }
                        }
                        KeyCode::Enter => {
                            // Confirm selection and run installations.
                            let step_index = app.current;
                            let step = &app.steps[step_index];
                            if let StepKind::AppSelection { params } = &step.kind {
                                let selected_indices: Vec<usize> = state
                                    .selected
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(idx, &sel)| if sel { Some(idx) } else { None })
                                    .collect();

                                let rt = app.current_runtime_mut();
                                if let Err(e) = apply_app_selection(params, &selected_indices, &mut rt.log)
                                {
                                    rt.status = StepStatus::Failed;
                                    rt.log.push_str(&format!("\n[ERROR] {}\n", e));
                                } else if rt.status == StepStatus::Running {
                                    rt.status = StepStatus::Success;
                                }
                            }

                            app.mode = InteractiveMode::None;
                            app.reset_scroll();
                        }
                        _ => {}
                    },
                    InteractiveMode::GitConfig(state) => {
                        enum GitAction {
                            None,
                            Cancel,
                            Apply { name: String, email: String, editor: String },
                        }

                        let mut action = GitAction::None;

                        match code {
                            KeyCode::Esc => {
                                action = GitAction::Cancel;
                            }
                            KeyCode::Tab => {
                                state.field = match state.field {
                                    GitField::Name => GitField::Email,
                                    GitField::Email => GitField::Editor,
                                    GitField::Editor => GitField::Name,
                                };
                            }
                            KeyCode::BackTab => {
                                state.field = match state.field {
                                    GitField::Name => GitField::Editor,
                                    GitField::Email => GitField::Name,
                                    GitField::Editor => GitField::Email,
                                };
                            }
                            KeyCode::Backspace => {
                                let buf = match state.field {
                                    GitField::Name => &mut state.name,
                                    GitField::Email => &mut state.email,
                                    GitField::Editor => &mut state.editor,
                                };
                                buf.pop();
                            }
                            KeyCode::Char(c) => {
                                let buf = match state.field {
                                    GitField::Name => &mut state.name,
                                    GitField::Email => &mut state.email,
                                    GitField::Editor => &mut state.editor,
                                };
                                buf.push(c);
                            }
                            KeyCode::Enter => {
                                action = GitAction::Apply {
                                    name: state.name.clone(),
                                    email: state.email.clone(),
                                    editor: state.editor.clone(),
                                };
                            }
                            _ => {}
                        }

                        // End of &mut state borrow here.
                        match action {
                            GitAction::None => {}
                            GitAction::Cancel => {
                                let rt = app.current_runtime_mut();
                                rt.status = StepStatus::Pending;
                                rt.log.push_str("Git config cancelled.\n");
                                app.mode = InteractiveMode::None;
                                app.reset_scroll();
                            }
                            GitAction::Apply { name, email, editor } => {
                                let step_index = app.current;
                                let step = &app.steps[step_index];
                                if let StepKind::GitConfig { params } = &step.kind {
                                    let rt = app.current_runtime_mut();
                                    if let Err(e) = apply_git_config(
                                        params,
                                        &name,
                                        &email,
                                        &editor,
                                        &mut rt.log,
                                    ) {
                                        rt.status = StepStatus::Failed;
                                        rt.log.push_str(&format!("\n[ERROR] {}\n", e));
                                    } else if rt.status == StepStatus::Running {
                                        rt.status = StepStatus::Success;
                                    }
                                }

                                app.mode = InteractiveMode::None;
                                app.reset_scroll();
                            }
                        }
                    },
                }
            }
        }
    }

    Ok(())
}

fn ui(f: &mut ratatui::Frame<>, app: &App) {
    let size = f.area();

    // NEW: split vertically into main body + 1-line status bar
    let root_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(3),      // main content
                Constraint::Length(1),   // status bar
            ]
            .as_ref(),
        )
        .split(size);

    let body_area = root_chunks[0];
    let status_area = root_chunks[1];

    // Existing layout now applied to body_area
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)].as_ref())
        .split(body_area);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(5),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(chunks[0]);

    // Steps list.
    let items: Vec<ListItem> = app
        .steps
        .iter()
        .enumerate()
        .map(|(idx, step)| {
            let rt = &app.runtimes[idx];
            let status_str = match rt.status {
                StepStatus::Pending => "[ ]",
                StepStatus::Running => "[>]",
                StepStatus::Skipped => "[-]",
                StepStatus::Success => "[✓]",
                StepStatus::Failed => "[✗]",
            };
            let prefix = if idx == app.current { "➤" } else { " " };
            let content = format!("{} {} {}", prefix, status_str, step.name);
            ListItem::new(content)
        })
        .collect();

    let steps_list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Steps"));

    f.render_widget(steps_list, left_chunks[0]);

    // Help box.
    let help = Paragraph::new(
        "Keys: Enter=Run | n=Next | p=Prev | s=Skip | Up/Down/PgUp/PgDn=Scroll | q=Quit"
    )
    .block(Block::default().borders(Borders::ALL).title("Help"));

    f.render_widget(help, left_chunks[1]);

    // Right side: log of current step, with scroll.
    let log = &app.current_runtime().log;
    let log_widget = Paragraph::new(log.clone())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Step log: {}", app.steps[app.current].name)),
        )
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((app.log_scroll, 0)); // NEW: apply scroll

    f.render_widget(log_widget, chunks[1]);

    // NEW: status bar at the bottom
    let total = app.steps.len();
    let current_idx = app.current + 1;
    let current_status = match app.current_runtime().status {
        StepStatus::Pending => "Pending",
        StepStatus::Running => "Running",
        StepStatus::Skipped => "Skipped",
        StepStatus::Success => "Success",
        StepStatus::Failed => "Failed",
    };

    let status_text = Line::from(vec![
        Span::raw(format!(" Step {}/{} ", current_idx, total)),
        Span::raw("| "),
        Span::styled(
            format!("Status: {}", current_status),
            Style::default().fg(match current_status {
                "Success" => Color::Green,
                "Failed" => Color::Red,
                "Running" => Color::Yellow,
                "Skipped" => Color::Blue,
                _ => Color::White,
            }),
        ),
        Span::raw(" | "),
        Span::raw("Press 'q' to quit."),
    ]);

    let status = Paragraph::new(status_text);
    f.render_widget(status, status_area);
}

fn ui_app_selection(
    f: &mut ratatui::Frame<>,
    app: &App,
    state: &AppSelectionState,
) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(5),      // top: interactive selection
                Constraint::Percentage(50), // bottom: log
            ]
            .as_ref(),
        )
        .split(size);

    // Top: checklist of apps for the current AppSelection step.
    let step = &app.steps[app.current];
    let items: Vec<ListItem> = if let StepKind::AppSelection { params } = &step.kind {
        params
            .apps
            .iter()
            .enumerate()
            .map(|(idx, app_def)| {
                let checked = state
                    .selected
                    .get(idx)
                    .copied()
                    .unwrap_or(false);
                let mark = if checked { "[x]" } else { "[ ]" };
                let cursor = if idx == state.cursor { "➤" } else { " " };
                let text = format!(
                    "{} {} {} ({}) - {}",
                    cursor, mark, app_def.name, app_def.version, app_def.install
                );
                ListItem::new(text)
            })
            .collect()
    } else {
        Vec::new()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Select apps (Space=toggle, Enter=confirm, Esc=cancel)"),
    );

    f.render_widget(list, chunks[0]);

    // Bottom: log for current step.
    let log = &app.current_runtime().log;
    let log_widget = Paragraph::new(log.clone())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Step log: {}", step.name)),
        )
        .wrap(ratatui::widgets::Wrap { trim: false });

    f.render_widget(log_widget, chunks[1]);
}

fn ui_git_config(
    f: &mut ratatui::Frame<>,
    app: &App,
    state: &GitConfigState,
) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(5),      // top: interactive fields + script preview
                Constraint::Percentage(50), // bottom: log
            ]
            .as_ref(),
        )
        .split(size);

    let step = &app.steps[app.current];

    // Build a textual representation of fields and resulting commands.
    let field_marker = |f: GitField| match (state.field, f) {
        (GitField::Name, GitField::Name)
        | (GitField::Email, GitField::Email)
        | (GitField::Editor, GitField::Editor) => ">",
        _ => " ",
    };

    let name = &state.name;
    let email = &state.email;
    let editor = &state.editor;

    let preview = format!(
        "Git configuration (Tab/Shift+Tab to move, type to edit, Enter=apply, Esc=cancel)\n\
{} user.name: {}\n\
{} user.email: {}\n\
{} editor: {}\n\n\
Commands to run:\n\
  git config --global user.name '{}'\n\
  git config --global user.email '{}'\n\
  git config --global core.editor '{}'\n",
        field_marker(GitField::Name),
        name,
        field_marker(GitField::Email),
        email,
        field_marker(GitField::Editor),
        editor,
        name.replace('\'', "\\'"),
        email.replace('\'', "\\'"),
        editor.replace('\'', "\\'"),
    );

    let top = Paragraph::new(preview).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Interactive Git config"),
    );

    f.render_widget(top, chunks[0]);

    // Bottom: log for current step.
    let log = &app.current_runtime().log;
    let log_widget = Paragraph::new(log.clone())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Step log: {}", step.name)),
        )
        .wrap(ratatui::widgets::Wrap { trim: false });

    f.render_widget(log_widget, chunks[1]);
}
