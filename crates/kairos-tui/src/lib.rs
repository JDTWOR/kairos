use anyhow::Result;
use chrono::Local;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use kairos_core::{
    AppConfig, Approval, Message as ConversationMessage, Task, TaskEvent, TaskStatus,
    normalize_repo,
};
use kairos_provider::{Message as ProviderMessage, OpenRouter};
use kairos_runner::Runner;
use kairos_store::Store;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    io,
    process::Stdio,
    time::{Duration, Instant},
};
use tuirealm::{
    command::{Cmd, CmdResult},
    component::Component,
    props::{AttrValue, Attribute, QueryResult},
    state::{State, StateValue},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Dashboard,
    Detail,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    Help,
    Search,
    NewTask,
    Info,
    Approval,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tasks,
    Main,
    Activity,
}
struct App {
    conversation_id: Option<Uuid>,
    messages: Vec<ConversationMessage>,
    tasks: Vec<Task>,
    events: Vec<TaskEvent>,
    selected: usize,
    focused: Focus,
    route: Route,
    overlay: Option<Overlay>,
    query: String,
    last_refresh: Instant,
    should_quit: bool,
    info: String,
    pending_approval: Option<Approval>,
    composer: bool,
    prompt: PromptComponent,
}

/// Stateful text input component backed by tui-realm's component contract.
/// Kairos keeps the surrounding dashboard in Ratatui, while prompt editing
/// uses tui-realm commands so input behavior is reusable and testable.
#[derive(Clone)]
struct PromptComponent {
    value: String,
    placeholder: &'static str,
}

impl PromptComponent {
    fn new() -> Self {
        Self {
            value: String::new(),
            placeholder: "Ask Kairos to work on this repository…",
        }
    }

    fn clear(&mut self) {
        self.value.clear();
    }

    fn draw(&self, f: &mut Frame, area: Rect, focused: bool) {
        let text = if self.value.is_empty() && !focused {
            self.placeholder.to_string()
        } else {
            format!("{}{}", if focused { "› " } else { "  " }, self.value)
        };
        f.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(if focused { theme::TEXT } else { theme::MUTED }))
                .block(panel(" PROMPT ", focused)),
            area,
        );
    }
}

impl Component for PromptComponent {
    fn view(&mut self, frame: &mut Frame, area: tuirealm::ratatui::layout::Rect) {
        self.draw(frame, area, true);
    }

    fn query(&self, _: Attribute) -> Option<QueryResult<'_>> {
        None
    }

    fn attr(&mut self, _: Attribute, _: AttrValue) {}

    fn state(&self) -> State {
        State::Single(StateValue::String(self.value.clone()))
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Type(c) => self.value.push(c),
            Cmd::Delete => {
                self.value.pop();
            }
            Cmd::Cancel => self.clear(),
            Cmd::Submit
            | Cmd::Move(_)
            | Cmd::Scroll(_)
            | Cmd::GoTo(_)
            | Cmd::Toggle
            | Cmd::Change
            | Cmd::Tick
            | Cmd::Custom(_)
            | Cmd::None => return CmdResult::NoChange,
        }
        CmdResult::Changed(self.state())
    }
}

impl App {
    fn new() -> Self {
        Self {
            conversation_id: None,
            messages: vec![],
            tasks: vec![],
            events: vec![],
            selected: 0,
            focused: Focus::Tasks,
            route: Route::Dashboard,
            overlay: None,
            query: String::new(),
            last_refresh: Instant::now() - Duration::from_secs(2),
            should_quit: false,
            info: String::new(),
            pending_approval: None,
            composer: false,
            prompt: PromptComponent::new(),
        }
    }
    fn task(&self) -> Option<&Task> {
        self.tasks.get(self.selected)
    }
    async fn refresh(&mut self, store: &Store) -> Result<()> {
        if self.last_refresh.elapsed() < Duration::from_millis(700) {
            return Ok(());
        }
        self.tasks = store.list_tasks().await?;
        if self.selected >= self.tasks.len() {
            self.selected = self.tasks.len().saturating_sub(1);
        }
        self.events = if let Some(t) = self.task() {
            store.events(t.id).await?
        } else {
            vec![]
        };
        self.messages = if let Some(id) = self.conversation_id {
            store.messages(id, 48).await?
        } else {
            vec![]
        };
        self.last_refresh = Instant::now();
        Ok(())
    }
    fn filtered_tasks(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| {
                self.query.is_empty()
                    || t.title.to_lowercase().contains(&self.query.to_lowercase())
                    || t.status.to_string().contains(&self.query.to_lowercase())
            })
            .collect()
    }
    fn key(&mut self, key: KeyEvent) -> Option<UiCommand> {
        if self.overlay == Some(Overlay::Search) {
            match key.code {
                KeyCode::Esc => {
                    self.query.clear();
                    self.overlay = None
                }
                KeyCode::Enter => self.overlay = None,
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) => self.query.push(c),
                _ => {}
            }
            return None;
        }
        if self.overlay == Some(Overlay::NewTask) {
            match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Backspace => {
                    self.prompt.perform(Cmd::Delete);
                }
                KeyCode::Char(c) => {
                    self.prompt.perform(Cmd::Type(c));
                }
                KeyCode::Enter if !self.prompt.value.trim().is_empty() => {
                    return Some(UiCommand::Create(self.prompt.value.trim().to_string()));
                }
                _ => {}
            }
            return None;
        }
        if self.composer {
            match key.code {
                KeyCode::Esc => {
                    self.composer = false;
                    self.prompt.perform(Cmd::Cancel);
                }
                KeyCode::Backspace => {
                    self.prompt.perform(Cmd::Delete);
                }
                KeyCode::Enter if !self.prompt.value.trim().is_empty() => {
                    let title = self.prompt.value.trim().to_string();
                    self.prompt.perform(Cmd::Cancel);
                    self.composer = false;
                    return Some(UiCommand::Create(title));
                }
                KeyCode::Char(c) => {
                    self.prompt.perform(Cmd::Type(c));
                }
                _ => {}
            }
            return None;
        }
        if self.overlay == Some(Overlay::Approval) {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let Some(approval) = self.pending_approval.clone() {
                        return Some(UiCommand::ConfirmApprove(approval.id));
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.pending_approval = None;
                    self.overlay = None;
                }
                _ => {}
            }
            return None;
        }
        if self.overlay.is_some() {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('?') {
                self.overlay = None
            }
            return None;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.route == Route::Detail {
                    self.route = Route::Dashboard
                } else {
                    self.should_quit = true
                }
            }
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            KeyCode::Char('/') => {
                // Consume the search shortcut here. Without the early return,
                // the generic text-input handler below also inserts `/` into
                // the prompt composer.
                self.query.clear();
                self.prompt.clear();
                self.composer = false;
                self.overlay = Some(Overlay::Search);
                return None;
            }
            KeyCode::Char('n') => {
                self.prompt.clear();
                self.composer = true;
                return None;
            }
            KeyCode::Char('i') => {
                self.prompt.clear();
                self.composer = true;
                return None;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.selected = self
                    .selected
                    .saturating_add(1)
                    .min(self.tasks.len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Tab => {
                self.focused = match self.focused {
                    Focus::Tasks => Focus::Main,
                    Focus::Main => Focus::Activity,
                    Focus::Activity => Focus::Tasks,
                }
            }
            KeyCode::Enter => {
                self.route = Route::Detail;
                self.focused = Focus::Main
            }
            KeyCode::Char('r') => {
                if let Some(t) = self.task() {
                    return Some(UiCommand::Resume(t.id));
                }
            }
            KeyCode::Char('p') => {
                if let Some(t) = self.task() {
                    return Some(UiCommand::Pause(t.id));
                }
            }
            KeyCode::Char('a') => {
                if let Some(t) = self.task() {
                    return Some(UiCommand::Approve(t.id));
                }
            }
            KeyCode::Char('d') => {
                if let Some(t) = self.task() {
                    return Some(UiCommand::Diff(t.id));
                }
            }
            KeyCode::Char('l') => {
                self.route = Route::Detail;
                self.focused = Focus::Main;
            }
            KeyCode::Char('c') => return Some(UiCommand::Cost),
            _ => {}
        }
        if let KeyCode::Char(c) = key.code
            && !key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
            && !c.is_control()
        {
            self.prompt.perform(Cmd::Type(c));
            self.composer = true;
        }
        None
    }
}

enum UiCommand {
    Create(String),
    Resume(Uuid),
    Pause(Uuid),
    Approve(Uuid),
    ConfirmApprove(Uuid),
    Diff(Uuid),
    Cost,
}

fn should_create_run(prompt: &str) -> bool {
    let prompt = prompt.to_lowercase();
    [
        "implement",
        "implementa",
        "modifica",
        "cambia",
        "cambiar",
        "crea",
        "crear",
        "añade",
        "agrega",
        "elimina",
        "borra",
        "arregla",
        "corrige",
        "refactoriza",
        "migra",
        "ejecuta",
        "corre",
        "test",
        "prueba",
        "analiza el repositorio",
        "revisa el repositorio",
        "inspecciona",
        "diagnostica",
        "deploy",
        "despliega",
    ]
    .iter()
    .any(|verb| prompt.contains(verb))
}

async fn answer_direct(store: &Store, app: &mut App, prompt: &str) -> Result<()> {
    let conversation_id = app
        .conversation_id
        .ok_or_else(|| anyhow::anyhow!("conversation not initialized"))?;
    store.add_message(conversation_id, "user", prompt).await?;
    let history = store.messages(conversation_id, 48).await?;
    let mut messages = vec![ProviderMessage {
        role: "system".into(),
        content: "You are Kairos, a helpful personal terminal assistant. Answer naturally and concisely in Spanish when the user writes Spanish. This is a conversation, not an execution request. Do not invent repository changes or propose coding plans unless the user asks for work.".into(),
    }];
    messages.extend(history.into_iter().map(|message| ProviderMessage {
        role: message.role,
        content: message.content,
    }));
    let config = AppConfig::load()?;
    let provider = OpenRouter::from_env(config.model, config.fallbacks)?;
    let (answer, _) = provider
        .prompt(
            messages,
            &store
                .get_conversation(conversation_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("conversation not found"))?
                .session_id,
        )
        .await?;
    store
        .add_message(conversation_id, "assistant", &answer)
        .await?;
    app.last_refresh = Instant::now() - Duration::from_secs(2);
    Ok(())
}

pub async fn run(store: Store, focus: Option<Uuid>) -> Result<()> {
    let mut app = App::new();
    let repo = normalize_repo(std::env::current_dir()?)?;
    app.conversation_id = Some(
        store
            .get_or_create_conversation(&repo.to_string_lossy(), "Kairos conversation")
            .await?
            .id,
    );
    if let Some(id) = focus {
        app.tasks = store.list_tasks().await?;
        app.selected = app.tasks.iter().position(|t| t.id == id).unwrap_or(0);
        if let Some(task) = app.tasks.get(app.selected) {
            app.conversation_id = task.conversation_id.or(app.conversation_id);
        }
    }
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let result = run_loop(&mut terminal, &store, &mut app).await;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    result
}
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    store: &Store,
    app: &mut App,
) -> Result<()> {
    while !app.should_quit {
        app.refresh(store).await?;
        terminal.draw(|f| render(f, app))?;
        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
            && key.kind == crossterm::event::KeyEventKind::Press
            && let Some(command) = app.key(key)
        {
            execute_command(store, app, command).await?;
        }
    }
    Ok(())
}
async fn execute_command(store: &Store, app: &mut App, command: UiCommand) -> Result<()> {
    match command {
        UiCommand::Create(title) => {
            if !should_create_run(&title) {
                if let Err(error) = answer_direct(store, app, &title).await {
                    app.info = format!("Direct response failed: {error}");
                    app.overlay = Some(Overlay::Info);
                }
                return Ok(());
            }
            let config = AppConfig::load()?;
            let repo = normalize_repo(std::env::current_dir()?)?;
            let task = store
                .create_task(&title, &repo.to_string_lossy(), &config.model, None)
                .await?;
            tokio::process::Command::new(std::env::current_exe()?)
                .arg("resume")
                .arg(task.id.to_string())
                .arg("--background")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            // Keep the dashboard unobstructed after submitting a prompt. The
            // task list and activity panel are refreshed below and provide the
            // persistent status for the new background task.
            app.info = format!("Task #{} started in background", short_id(task.id));
            app.overlay = None;
            app.tasks = store.list_tasks().await?;
            if let Some(index) = app.tasks.iter().position(|item| item.id == task.id) {
                app.selected = index;
            }
            app.last_refresh = Instant::now() - Duration::from_secs(2);
        }
        UiCommand::Resume(id) => {
            tokio::process::Command::new(std::env::current_exe()?)
                .arg("resume")
                .arg(id.to_string())
                .arg("--background")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            app.info = "Resume requested in background".into();
            app.overlay = Some(Overlay::Info);
        }
        UiCommand::Pause(id) => {
            store.set_status(id, TaskStatus::Paused).await?;
            app.info = "Task paused".into();
            app.overlay = Some(Overlay::Info);
            app.last_refresh = Instant::now() - Duration::from_secs(2);
        }
        UiCommand::Approve(id) => {
            let approvals = store.approvals(id).await?;
            if let Some(a) = approvals.first() {
                app.pending_approval = Some(a.clone());
                app.overlay = Some(Overlay::Approval);
            } else {
                app.info = "No pending approval".into();
                app.overlay = Some(Overlay::Info);
            }
        }
        UiCommand::ConfirmApprove(id) => {
            store.resolve_approval(id, "approved").await?;
            app.pending_approval = None;
            app.info = "Approval granted".into();
            app.overlay = Some(Overlay::Info);
        }
        UiCommand::Diff(id) => {
            let task = store
                .get_task(id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("task not found"))?;
            app.info = Runner::new(task.worktree.unwrap_or(task.repo))
                .git_diff()
                .await?;
            app.overlay = Some(Overlay::Info);
        }
        UiCommand::Cost => {
            app.info = format!("Cost today: ${:.6}", store.cost_today().await?);
            app.overlay = Some(Overlay::Info);
        }
    }
    Ok(())
}

fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(theme::BG)), area);
    if area.width <= 80 || area.height < 16 {
        render_narrow(f, app, area)
    } else if app.route == Route::Detail {
        render_detail(f, app, area)
    } else {
        render_dashboard(f, app, area)
    }
    if let Some(overlay) = app.overlay {
        render_overlay(f, app, overlay, area)
    }
}
fn header(f: &mut Frame, area: Rect) {
    let has_key = std::env::var_os("OPENROUTER_API_KEY").is_some();
    let line = Line::from(vec![
        Span::styled(
            " KAIROS ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  personal agent", Style::default().fg(theme::MUTED)),
        Span::raw(" "),
        Span::styled("OpenRouter", Style::default().fg(theme::TEXT)),
        Span::styled(
            if has_key {
                "  ● configured "
            } else {
                "  ○ no API key "
            },
            Style::default().fg(if has_key { theme::GREEN } else { theme::AMBER }),
        ),
    ]);
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme::BG)),
        area,
    )
}
fn footer(f: &mut Frame, area: Rect, text: &str) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(text, Style::default().fg(theme::MUTED)),
            Span::styled("   [?] help", Style::default().fg(theme::VIOLET)),
        ]))
        .style(Style::default().bg(theme::SURFACE)),
        area,
    )
}
fn render_dashboard(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(area);
    header(f, rows[0]);
    let cols = Layout::horizontal([Constraint::Min(44), Constraint::Length(30)]).split(rows[1]);
    render_conversation(f, app, cols[0]);
    let sidebar =
        Layout::vertical([Constraint::Percentage(62), Constraint::Percentage(38)]).split(cols[1]);
    render_tasks(f, app, sidebar[0]);
    render_activity(f, app, sidebar[1]);
    render_composer(f, app, rows[2]);
    footer(
        f,
        rows[3],
        "[Enter] Open   [j/k] Navigate   [/] Search   [n] New   [r] Resume   [a] Approve   [q] Quit",
    )
}
fn render_conversation(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();
    if app.messages.is_empty() {
        lines.push(Line::from(Span::styled(
            "Start a conversation with Kairos",
            Style::default().fg(theme::MUTED),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(
            "Ask about this repository, request a change, or type /help.",
        ));
    } else {
        for message in &app.messages {
            let (label, color) = match message.role.as_str() {
                "user" => ("YOU", theme::CYAN),
                "assistant" => ("KAIROS", theme::VIOLET),
                _ => (message.role.as_str(), theme::MUTED),
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}  ", label),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    message
                        .created_at
                        .with_timezone(&Local)
                        .format("%H:%M")
                        .to_string(),
                    Style::default().fg(theme::MUTED),
                ),
            ]));
            for content_line in message.content.lines() {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(content_line.to_string(), Style::default().fg(theme::TEXT)),
                ]));
            }
            lines.push(Line::from(""));
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(" CONVERSATION ", app.focused == Focus::Main))
            .wrap(Wrap { trim: false }),
        area,
    );
}
fn render_composer(f: &mut Frame, app: &App, area: Rect) {
    app.prompt.draw(f, area, app.composer)
}
fn render_tasks(f: &mut Frame, app: &App, area: Rect) {
    let items = app
        .filtered_tasks()
        .iter()
        .map(|t| {
            let icon = status_icon(t.status);
            let style = if t.status == TaskStatus::Failed {
                Style::default().fg(theme::RED)
            } else {
                Style::default().fg(theme::TEXT)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", icon), status_style(t.status)),
                Span::styled(short_id(t.id), Style::default().fg(theme::MUTED)),
                Span::raw(" "),
                Span::styled(
                    truncate(&t.title, area.width.saturating_sub(20) as usize),
                    style,
                ),
            ]))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(app.selected.min(items.len().saturating_sub(1))));
    let title = if app.query.is_empty() {
        " TASKS "
    } else {
        " SEARCH "
    };
    f.render_stateful_widget(
        List::new(items)
            .block(panel(title, app.focused == Focus::Tasks))
            .highlight_style(
                Style::default()
                    .bg(theme::SELECTED)
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("› "),
        area,
        &mut state,
    )
}
#[allow(dead_code)]
fn render_active(f: &mut Frame, app: &App, area: Rect) {
    let Some(t) = app.task() else {
        f.render_widget(
            Paragraph::new("No tasks yet\n\nn  Create a task").block(panel(" ACTIVE TASK ", false)),
            area,
        );
        return;
    };
    let chunks = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(5),
        Constraint::Length(7),
    ])
    .split(area);
    let status = Line::from(vec![
        Span::styled(
            format!(" {} ", status_icon(t.status)),
            status_style(t.status),
        ),
        Span::styled(
            format!(" {}", t.status),
            status_style(t.status).add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(
            format!("${:.4}", t.cost_usd),
            Style::default().fg(theme::AMBER),
        ),
    ]);
    let summary = Text::from(vec![
        Line::from(Span::styled(
            truncate(&t.title, area.width as usize),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )),
        status,
        Line::from(Span::styled(
            format!("{}  ·  {}", t.repo.display(), t.model),
            Style::default().fg(theme::MUTED),
        )),
    ]);
    f.render_widget(
        Paragraph::new(summary).block(panel(" ACTIVE TASK ", app.focused == Focus::Main)),
        chunks[0],
    );
    let plan = t
        .plan
        .iter()
        .enumerate()
        .map(|(i, p)| {
            Line::from(vec![
                Span::styled(
                    if i == 0 { "› " } else { "✓ " },
                    if i == 0 {
                        Style::default().fg(theme::VIOLET)
                    } else {
                        Style::default().fg(theme::GREEN)
                    },
                ),
                Span::styled(p, Style::default().fg(theme::TEXT)),
            ])
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(plan).block(panel(" PLAN ", false)),
        chunks[1],
    );
    let last = app
        .events
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|e| {
            Line::from(vec![
                Span::styled(
                    e.created_at
                        .with_timezone(&Local)
                        .format("%H:%M")
                        .to_string(),
                    Style::default().fg(theme::MUTED),
                ),
                Span::raw("  "),
                Span::styled(e.kind.to_uppercase(), kind_style(&e.kind)),
                Span::raw(" "),
                Span::styled(e.message.clone(), Style::default().fg(theme::TEXT)),
            ])
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(last)
            .block(panel(" RECENT ACTIVITY ", false))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
}
fn render_activity(f: &mut Frame, app: &App, area: Rect) {
    let lines = app
        .events
        .iter()
        .rev()
        .take(8)
        .map(|e| {
            Line::from(vec![
                Span::styled(
                    e.created_at
                        .with_timezone(&Local)
                        .format("%H:%M")
                        .to_string(),
                    Style::default().fg(theme::MUTED),
                ),
                Span::raw(" "),
                Span::styled(e.kind.clone(), kind_style(&e.kind)),
            ])
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(lines).block(panel(" ACTIVITY ", app.focused == Focus::Activity)),
        area,
    )
}
fn render_detail(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(4),
        Constraint::Min(5),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(area);
    header(f, rows[0]);
    let Some(t) = app.task() else { return };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("#{}  {}", short_id(t.id), t.title),
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(t.status.to_string(), status_style(t.status)),
            Span::raw("   "),
            Span::styled(
                format!("${:.4}", t.cost_usd),
                Style::default().fg(theme::AMBER),
            ),
        ]))
        .block(panel(" TASK DETAIL ", true)),
        rows[1],
    );
    let cols = Layout::horizontal([
        Constraint::Percentage(28),
        Constraint::Percentage(52),
        Constraint::Percentage(20),
    ])
    .split(rows[2]);
    render_plan(f, t, cols[0]);
    render_logs(f, app, cols[1]);
    render_inspector(f, t, cols[2]);
    render_composer(f, app, rows[3]);
    footer(
        f,
        rows[4],
        "[j/k] Scroll   [l] Logs   [d] Diff   [p] Pause   [a] Approve   [Esc] Back",
    )
}
fn render_plan(f: &mut Frame, t: &Task, area: Rect) {
    let lines = t
        .plan
        .iter()
        .map(|p| {
            Line::from(vec![
                Span::styled("✓ ", Style::default().fg(theme::GREEN)),
                Span::styled(p.clone(), Style::default().fg(theme::TEXT)),
            ])
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(" PLAN ", false))
            .wrap(Wrap { trim: false }),
        area,
    )
}
fn render_logs(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();
    for e in &app.events {
        lines.push(Line::from(vec![
            Span::styled(
                e.created_at
                    .with_timezone(&Local)
                    .format("%H:%M:%S")
                    .to_string(),
                Style::default().fg(theme::MUTED),
            ),
            Span::raw("  "),
            Span::styled(format!("{} ", e.kind.to_uppercase()), kind_style(&e.kind)),
            Span::styled(e.message.clone(), Style::default().fg(theme::TEXT)),
        ]));
        if let Some(output) = &e.output {
            lines.push(Line::from(vec![
                Span::raw("             "),
                Span::styled(output.clone(), Style::default().fg(theme::RED)),
            ]));
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(" LOGS ", true))
            .wrap(Wrap { trim: false }),
        area,
    )
}
fn render_inspector(f: &mut Frame, t: &Task, area: Rect) {
    let text = format!(
        "provider\n  {}\n\nsession\n  {}\n\nrepo\n  {}\n\ncache\n  n/a",
        t.provider,
        truncate(&t.session_id, 16),
        truncate(&t.repo.display().to_string(), 18)
    );
    f.render_widget(
        Paragraph::new(text)
            .style(Style::default().fg(theme::TEXT))
            .block(panel(" INSPECTOR ", false)),
        area,
    )
}
fn render_narrow(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(area);
    header(f, rows[0]);
    let lines = app
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            Line::from(vec![
                Span::styled(
                    if i == app.selected { "› " } else { "  " },
                    Style::default().fg(theme::CYAN),
                ),
                Span::styled(status_icon(t.status), status_style(t.status)),
                Span::raw(" "),
                Span::styled(
                    truncate(&t.title, area.width.saturating_sub(12) as usize),
                    Style::default().fg(theme::TEXT),
                ),
            ])
        })
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(lines).block(panel(" TASKS · NARROW ", true)),
        rows[1],
    );
    render_composer(f, app, rows[2]);
    footer(
        f,
        rows[3],
        "[j/k] Navigate  [Enter] Open  [/] Search  [q] Quit",
    )
}
fn render_overlay(f: &mut Frame, app: &App, overlay: Overlay, area: Rect) {
    let size = match overlay {
        Overlay::Help => Rect::new(
            area.x + area.width / 10,
            area.y + area.height / 8,
            area.width * 8 / 10,
            area.height * 3 / 4,
        ),
        Overlay::Search | Overlay::NewTask => {
            Rect::new(area.x + area.width / 10, area.y + 2, area.width * 8 / 10, 6)
        }
        Overlay::Info => Rect::new(
            area.x + area.width / 12,
            area.y + area.height / 8,
            area.width * 5 / 6,
            area.height * 3 / 4,
        ),
        Overlay::Approval => Rect::new(
            area.x + area.width / 8,
            area.y + area.height / 4,
            area.width * 3 / 4,
            10.min(area.height.saturating_sub(2)),
        ),
    };
    f.render_widget(Clear, size);
    let content = match overlay {
        Overlay::Help => Text::from(vec![
            Line::from(Span::styled(
                "KAIROS KEYBOARD",
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("j / k       Navigate"),
            Line::from("Enter       Open task"),
            Line::from("Tab         Change focus"),
            Line::from("/           Search tasks"),
            Line::from("n           New task"),
            Line::from("r p a d l   Resume / pause / approve / diff / logs"),
            Line::from("Esc / q     Back or quit"),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close",
                Style::default().fg(theme::MUTED),
            )),
        ]),
        Overlay::Search => Text::from(vec![
            Line::from(Span::styled(
                "Search tasks",
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("/{}", app.query)),
            Line::from(Span::styled(
                "Enter apply · Esc cancel",
                Style::default().fg(theme::MUTED),
            )),
        ]),
        Overlay::NewTask => Text::from(vec![
            Line::from(Span::styled(
                "New task",
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("> {}", app.prompt.value)),
            Line::from(Span::styled(
                "Enter create · Esc cancel",
                Style::default().fg(theme::MUTED),
            )),
        ]),
        Overlay::Info => Text::from(vec![
            Line::from(Span::styled(
                "Kairos",
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(app.info.clone()),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close",
                Style::default().fg(theme::MUTED),
            )),
        ]),
        Overlay::Approval => {
            let approval = app.pending_approval.as_ref();
            Text::from(vec![
                Line::from(Span::styled(
                    "APPROVAL REQUIRED",
                    Style::default()
                        .fg(theme::AMBER)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(format!(
                    "Action   {}",
                    approval.map(|a| a.action.as_str()).unwrap_or("unknown")
                )),
                Line::from(format!(
                    "Detail   {}",
                    approval.map(|a| a.detail.as_str()).unwrap_or("unknown")
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "[y/Enter] approve once   [n/Esc] reject",
                    Style::default().fg(theme::MUTED),
                )),
            ])
        }
    };
    f.render_widget(
        Paragraph::new(content)
            .block(panel(" ", true))
            .wrap(Wrap { trim: false }),
        size,
    )
}
fn panel(title: &str, focused: bool) -> Block<'_> {
    Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(if focused { theme::CYAN } else { theme::MUTED })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused { theme::CYAN } else { theme::BORDER }))
        .style(Style::default().bg(theme::SURFACE))
}
fn status_icon(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Completed => "✓",
        TaskStatus::Running | TaskStatus::Planning | TaskStatus::Verifying => "●",
        TaskStatus::WaitingApproval => "!",
        TaskStatus::Failed => "×",
        TaskStatus::Paused => "Ⅱ",
        TaskStatus::NeedsInput => "?",
        _ => "○",
    }
}
fn status_style(s: TaskStatus) -> Style {
    Style::default().fg(match s {
        TaskStatus::Completed => theme::GREEN,
        TaskStatus::Running | TaskStatus::Planning | TaskStatus::Verifying => theme::CYAN,
        TaskStatus::WaitingApproval => theme::AMBER,
        TaskStatus::Failed => theme::RED,
        TaskStatus::NeedsInput => theme::AMBER,
        _ => theme::MUTED,
    })
}
fn kind_style(kind: &str) -> Style {
    Style::default().fg(match kind {
        "model" => theme::VIOLET,
        "error" => theme::RED,
        "verification" => theme::GREEN,
        "tool" => theme::CYAN,
        _ => theme::MUTED,
    })
}
fn short_id(id: Uuid) -> String {
    id.to_string()[..8].to_string()
}
fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!(
            "{}…",
            value
                .chars()
                .take(max.saturating_sub(1))
                .collect::<String>()
        )
    }
}
mod theme {
    use ratatui::style::Color;
    pub const BG: Color = Color::Rgb(7, 16, 25);
    pub const SURFACE: Color = Color::Rgb(15, 27, 40);
    pub const BORDER: Color = Color::Rgb(39, 58, 76);
    pub const SELECTED: Color = Color::Rgb(25, 48, 60);
    pub const TEXT: Color = Color::Rgb(220, 231, 242);
    pub const MUTED: Color = Color::Rgb(143, 164, 184);
    pub const CYAN: Color = Color::Rgb(79, 209, 197);
    pub const VIOLET: Color = Color::Rgb(167, 139, 250);
    pub const GREEN: Color = Color::Rgb(94, 224, 139);
    pub const AMBER: Color = Color::Rgb(242, 184, 75);
    pub const RED: Color = Color::Rgb(255, 107, 107);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kairos_core::AppConfig;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn routes_conversation_and_work_requests_separately() {
        assert!(!should_create_run("hola"));
        assert!(!should_create_run("¿quién eres?"));
        assert!(!should_create_run("¿cuál fue mi primera pregunta?"));
        assert!(should_create_run("implementa OAuth"));
        assert!(should_create_run("ejecuta los tests"));
        assert!(should_create_run("analiza el repositorio"));
    }

    #[test]
    fn dashboard_renders_at_supported_sizes() {
        for (width, height) in [(160, 45), (100, 30), (78, 24), (60, 16)] {
            let mut app = App::new();
            let now = chrono::Utc::now();
            app.tasks.push(Task {
                id: Uuid::new_v4(),
                conversation_id: None,
                title: "OAuth Google".into(),
                repo: ".".into(),
                status: TaskStatus::Running,
                model: AppConfig::default().model,
                provider: "openrouter".into(),
                session_id: "session".into(),
                budget_usd: Some(1.0),
                plan: vec!["Inspect auth".into(), "Add provider".into()],
                checkpoint: None,
                worktree: None,
                cost_usd: 0.031,
                created_at: now,
                updated_at: now,
            });
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            assert!(
                rendered.contains("KAIROS"),
                "missing header at {width}x{height}"
            );
            assert!(
                rendered.contains("OAuth"),
                "missing task at {width}x{height}"
            );
        }
    }
}
