use crate::executor::{run_step, start_sudo_session, StepRuntime, StepStatus};
use crate::model::Step;
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
    text::Span,
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
}

impl<'a> App<'a> {
    pub fn new(steps: &'a [Step]) -> Self {
        Self {
            steps,
            runtimes: vec![StepRuntime::default(); steps.len()],
            current: 0,
            global_log: String::new(),
            log_scroll: 0,
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
        terminal.draw(|f| ui(f, &app))?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('n') => {
                        if app.current + 1 < app.steps.len() {
                            app.current += 1;
                            app.reset_scroll(); // NEW
                        }
                    }
                    KeyCode::Char('p') => {
                        if app.current > 0 {
                            app.current -= 1;
                            app.reset_scroll(); // NEW
                        }
                    }
                    KeyCode::Char('s') => {
                        let rt = app.current_runtime_mut();
                        rt.status = StepStatus::Skipped;
                        rt.log.push_str("Step manually skipped.\n");
                    }
                    KeyCode::Enter => {
                        // Temporarily leave TUI raw mode for dialoguer prompts in some tasks.
                        disable_raw_mode()?;
                        let step = &app.steps[app.current];
                        let rt = app.current_runtime_mut();
                        let res = run_step(step, rt);
                        enable_raw_mode()?;
                        app.reset_scroll(); // start at top of new log
                        if let Err(e) = res {
                            let rt = app.current_runtime_mut();
                            rt.status = StepStatus::Failed;
                            rt.log.push_str(&format!("\n[ERROR] {}\n", e));
                        }
                    }

                    // NEW: scrolling the log
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
    let help = Paragraph::new(vec![
        Span::from("Keys: Enter=Run | n=Next | p=Prev | s=Skip | Up/Down/PgUp/PgDn=Scroll | q=Quit"),
    ])
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

    let status_text = Span::from(vec![
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
