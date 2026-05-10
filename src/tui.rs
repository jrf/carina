use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::config::Config as AppConfig;
use crate::metadata;
use crate::model::Reference;
use crate::storage;
use crate::theme::{self, Theme};

struct Entry {
    dir: PathBuf,
    dir_name: String,
    reference: Reference,
    display: String,
}

pub struct App {
    entries: Vec<Entry>,
    filtered_indices: Vec<usize>,
    filter: String,
    list_state: ListState,
    config: AppConfig,
    theme: Theme,
    mode: Mode,
    input_mode: InputMode,
    should_quit: bool,
    pending_output: Option<String>,
    tag_filter: Option<String>,
    all_tags: Vec<String>,
    tag_popup: Option<TagPopup>,
    layout: LayoutMode,
    last_insert_char: Option<std::time::Instant>,
}

struct TagPopup {
    filter: String,
    filtered_tags: Vec<String>,
    selected: usize,
}

impl TagPopup {
    fn new(all_tags: &[String]) -> Self {
        let mut tags = vec!["(all)".to_string()];
        tags.extend(all_tags.iter().cloned());
        Self {
            filter: String::new(),
            filtered_tags: tags,
            selected: 0,
        }
    }

    fn rebuild(&mut self, all_tags: &[String]) {
        let mut tags = vec!["(all)".to_string()];
        tags.extend(all_tags.iter().cloned());
        if self.filter.is_empty() {
            self.filtered_tags = tags;
        } else {
            let f = self.filter.to_lowercase();
            self.filtered_tags = tags.into_iter().filter(|t| t.to_lowercase().contains(&f)).collect();
        }
        if self.selected >= self.filtered_tags.len() {
            self.selected = self.filtered_tags.len().saturating_sub(1);
        }
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self) {
        if !self.filtered_tags.is_empty() {
            self.selected = (self.selected + 1).min(self.filtered_tags.len() - 1);
        }
    }

    fn selected_tag(&self) -> Option<&str> {
        self.filtered_tags.get(self.selected).map(|s| s.as_str())
    }
}

#[derive(Clone, Copy)]
enum LayoutMode {
    Wide,
    Tall,
    Auto,
}

impl LayoutMode {
    fn from_config(s: Option<&str>) -> Self {
        match s {
            Some("wide") => Self::Wide,
            Some("tall") => Self::Tall,
            _ => Self::Auto,
        }
    }

    fn resolve(self, width: u16, _height: u16) -> ResolvedLayout {
        match self {
            Self::Wide => ResolvedLayout::Wide,
            Self::Tall => ResolvedLayout::Tall,
            Self::Auto => {
                if width >= 100 {
                    ResolvedLayout::Wide
                } else {
                    ResolvedLayout::Tall
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ResolvedLayout {
    Wide,
    Tall,
}

enum Mode {
    Browse,
    Cite { format: String },
}

#[derive(Clone, Copy, PartialEq)]
enum InputMode {
    Normal,
    Insert,
}

pub fn browse(config: &AppConfig, library: &Path, initial_query: Option<&str>) -> Result<()> {
    let app = App::new(config, library, Mode::Browse, initial_query)?;
    if app.entries.is_empty() {
        println!("Library is empty. Use `carina add <file.pdf>` to import a paper.");
        return Ok(());
    }
    run_app(app)
}

pub fn cite(config: &AppConfig, library: &Path, format: &str) -> Result<()> {
    let app = App::new(config, library, Mode::Cite { format: format.to_string() }, None)?;
    if app.entries.is_empty() {
        anyhow::bail!("Library is empty");
    }
    run_app(app)
}

fn run_app(mut app: App) -> Result<()> {
    let tty = File::options().read(true).write(true).open("/dev/tty")?;
    let mut tty_ctl = tty.try_clone()?;

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        if let Ok(mut f) = File::options().write(true).open("/dev/tty") {
            let _ = f.execute(LeaveAlternateScreen);
        }
        prev_hook(info);
    }));

    tty_ctl.execute(EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;

    let backend = CrosstermBackend::new(BufWriter::new(tty.try_clone()?));
    let mut terminal = Terminal::new(backend)?;

    let result = run_event_loop(&mut terminal, &mut app, &mut tty_ctl);

    terminal::disable_raw_mode()?;
    tty_ctl.execute(LeaveAlternateScreen)?;

    if let Some(output) = app.pending_output.take() {
        print!("{}", output);
    }
    result
}

type Term = Terminal<CrosstermBackend<BufWriter<File>>>;

fn run_event_loop(terminal: &mut Term, app: &mut App, tty_ctl: &mut File) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if app.should_quit {
            return Ok(());
        }

        if let Event::Key(key) = event::read()? {
            if app.tag_popup.is_some() {
                match key.code {
                    KeyCode::Esc => { app.tag_popup = None; }
                    KeyCode::Up => { app.tag_popup.as_mut().unwrap().move_up(); }
                    KeyCode::Down => { app.tag_popup.as_mut().unwrap().move_down(); }
                    KeyCode::Backspace => {
                        let popup = app.tag_popup.as_mut().unwrap();
                        popup.filter.pop();
                        popup.rebuild(&app.all_tags);
                    }
                    KeyCode::Char(c) => {
                        let popup = app.tag_popup.as_mut().unwrap();
                        popup.filter.push(c);
                        popup.rebuild(&app.all_tags);
                    }
                    KeyCode::Enter => {
                        let tag = app.tag_popup.as_ref().unwrap().selected_tag()
                            .map(|s| s.to_string());
                        app.tag_popup = None;
                        match tag.as_deref() {
                            Some("(all)") | None => { app.tag_filter = None; }
                            Some(t) => { app.tag_filter = Some(t.to_string()); }
                        }
                        app.rebuild_filter();
                    }
                    _ => {}
                }
                continue;
            }

            match app.input_mode {
                InputMode::Insert => match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) => {
                        app.input_mode = InputMode::Normal;
                        app.last_insert_char = None;
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => app.should_quit = true,

                    (KeyCode::Char('k'), KeyModifiers::NONE)
                        if app.last_insert_char.is_some_and(|t| t.elapsed().as_millis() < 500)
                            && app.filter.ends_with('j') =>
                    {
                        app.filter.pop();
                        app.rebuild_filter();
                        app.input_mode = InputMode::Normal;
                        app.last_insert_char = None;
                    }
                    (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                        app.last_insert_char = Some(std::time::Instant::now());
                        app.filter.push(c);
                        app.rebuild_filter();
                    }
                    (KeyCode::Backspace, _) => {
                        app.filter.pop();
                        app.rebuild_filter();
                        app.last_insert_char = None;
                    }

                    (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => app.move_up(),
                    (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => app.move_down(),
                    (KeyCode::Char('b'), KeyModifiers::CONTROL) => app.page_up(),
                    (KeyCode::Char('f'), KeyModifiers::CONTROL) => app.page_down(),

                    (KeyCode::Tab, _) => {
                        app.tag_popup = Some(TagPopup::new(&app.all_tags));
                    }

                    (KeyCode::Enter, _) => {
                        app.action_select()?;
                    }

                    _ => {}
                },
                InputMode::Normal => match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                        app.should_quit = true;
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => app.should_quit = true,

                    (KeyCode::Char('/'), KeyModifiers::NONE)
                    | (KeyCode::Char('i'), KeyModifiers::NONE) => {
                        app.input_mode = InputMode::Insert;
                    }

                    (KeyCode::Char('j'), KeyModifiers::NONE)
                    | (KeyCode::Down, _)
                    | (KeyCode::Char('n'), KeyModifiers::CONTROL) => app.move_down(),
                    (KeyCode::Char('k'), KeyModifiers::NONE)
                    | (KeyCode::Up, _)
                    | (KeyCode::Char('p'), KeyModifiers::CONTROL) => app.move_up(),
                    (KeyCode::Char('b'), KeyModifiers::CONTROL) => app.page_up(),
                    (KeyCode::Char('f'), KeyModifiers::CONTROL) => app.page_down(),
                    (KeyCode::Char('g'), KeyModifiers::NONE) => app.move_to_top(),
                    (KeyCode::Char('G'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.move_to_bottom();
                    }

                    (KeyCode::Tab, _) => {
                        app.tag_popup = Some(TagPopup::new(&app.all_tags));
                    }

                    (KeyCode::Enter, _) => {
                        app.action_select()?;
                    }
                    (KeyCode::Char('e'), KeyModifiers::NONE) => {
                        app.action_edit(terminal, tty_ctl)?;
                    }
                    (KeyCode::Char('y'), KeyModifiers::NONE) => {
                        app.action_copy_bib()?;
                    }

                    _ => {}
                },
            }
        }
    }
}

impl App {
    fn new(config: &AppConfig, library: &Path, mode: Mode, initial_query: Option<&str>) -> Result<Self> {
        let dirs = storage::list_ref_dirs(library)?;
        let entries: Vec<Entry> = dirs
            .into_iter()
            .filter_map(|dir| {
                let dir_name = dir.file_name()?.to_string_lossy().to_string();
                let reference = metadata::read_info(&dir).ok()?;
                let authors = if reference.authors.is_empty() {
                    String::new()
                } else if reference.authors.len() == 1 {
                    reference.authors[0].clone()
                } else {
                    format!("{} et al.", reference.authors[0])
                };
                let year = reference.year.map(|y| format!("({})", y)).unwrap_or_default();
                let display = [authors, year, reference.title.clone()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("  ");
                Some(Entry { dir, dir_name, reference, display })
            })
            .collect();

        let filter = initial_query.unwrap_or("").to_string();
        let filtered_indices: Vec<usize> = (0..entries.len()).collect();

        let theme = theme::load_theme(config.theme.as_deref());

        let mut tag_set = std::collections::BTreeSet::new();
        for e in &entries {
            for tag in &e.reference.tags {
                tag_set.insert(tag.clone());
            }
        }
        let all_tags: Vec<String> = tag_set.into_iter().collect();

        let mut app = App {
            entries,
            filtered_indices,
            filter,
            list_state: ListState::default(),
            config: AppConfig {
                library: config.library.clone(),
                editor: config.editor.clone(),
                reader: config.reader.clone(),
                picker: config.picker.clone(),
                theme: config.theme.clone(),
                layout: config.layout.clone(),
            },
            theme,
            mode,
            input_mode: InputMode::Insert,
            should_quit: false,
            pending_output: None,
            tag_filter: None,
            all_tags,
            tag_popup: None,
            layout: LayoutMode::from_config(config.layout.as_deref()),
            last_insert_char: None,
        };

        if !app.filter.is_empty() {
            app.rebuild_filter();
        }
        if !app.filtered_indices.is_empty() {
            app.list_state.select(Some(0));
        }

        Ok(app)
    }

    fn rebuild_filter(&mut self) {
        let tag_filtered: Vec<usize> = if let Some(ref tag) = self.tag_filter {
            self.entries.iter().enumerate()
                .filter(|(_, e)| e.reference.tags.iter().any(|t| t == tag))
                .map(|(i, _)| i)
                .collect()
        } else {
            (0..self.entries.len()).collect()
        };

        if self.filter.is_empty() {
            self.filtered_indices = tag_filtered;
        } else {
            let pattern = Pattern::parse(
                &self.filter,
                CaseMatching::Ignore,
                Normalization::Smart,
            );
            let mut matcher = Matcher::new(Config::DEFAULT);
            let mut buf = Vec::new();

            let mut scored: Vec<(usize, u32)> = tag_filtered
                .into_iter()
                .filter_map(|i| {
                    let haystack = Utf32Str::new(&self.entries[i].display, &mut buf);
                    pattern.score(haystack, &mut matcher).map(|s| (i, s))
                })
                .collect();
            scored.sort_by_key(|&(_, s)| std::cmp::Reverse(s));
            self.filtered_indices = scored.into_iter().map(|(i, _)| i).collect();
        }

        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let selected = self.list_state.selected()?;
        let &idx = self.filtered_indices.get(selected)?;
        self.entries.get(idx)
    }

    fn move_up(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(0) | None => 0,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(i));
    }

    fn move_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) if i >= self.filtered_indices.len() - 1 => self.filtered_indices.len() - 1,
            Some(i) => i + 1,
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn page_up(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some(i.saturating_sub(20)));
    }

    fn page_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let max = self.filtered_indices.len() - 1;
        self.list_state.select(Some((i + 20).min(max)));
    }

    fn move_to_top(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    fn move_to_bottom(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.list_state.select(Some(self.filtered_indices.len() - 1));
        }
    }

    fn action_select(&mut self) -> Result<()> {
        let entry = match self.selected_entry() {
            Some(e) => e,
            None => return Ok(()),
        };

        match &self.mode {
            Mode::Browse => {
                let pdf = if let Some(f) = entry.reference.files.first() {
                    entry.dir.join(f)
                } else {
                    // Fall back to first PDF found in directory
                    match std::fs::read_dir(&entry.dir)
                        .ok()
                        .and_then(|rd| rd.flatten().find(|e| {
                            e.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
                        }))
                    {
                        Some(e) => e.path(),
                        None => return Ok(()),
                    }
                };
                std::process::Command::new(self.config.reader())
                    .arg(&pdf)
                    .spawn()?;
            }
            Mode::Cite { format } => {
                let key = entry.dir_name.clone();
                let output = match format.as_str() {
                    "latex" => format!("\\cite{{{}}}", key),
                    "typst" => format!("@{}", key),
                    _ => key,
                };
                self.pending_output = Some(output);
                self.should_quit = true;
            }
        }
        Ok(())
    }

    fn action_edit(&mut self, terminal: &mut Term, tty_ctl: &mut File) -> Result<()> {
        let entry = match self.selected_entry() {
            Some(e) => e,
            None => return Ok(()),
        };
        let info_path = entry.dir.join("info.toml");

        terminal::disable_raw_mode()?;
        tty_ctl.execute(LeaveAlternateScreen)?;

        std::process::Command::new(self.config.editor())
            .arg(&info_path)
            .status()?;

        tty_ctl.execute(EnterAlternateScreen)?;
        terminal::enable_raw_mode()?;
        terminal.clear()?;

        let idx = self.filtered_indices[self.list_state.selected().unwrap_or(0)];
        if let Ok(r) = metadata::read_info(&self.entries[idx].dir) {
            let authors = if r.authors.is_empty() {
                String::new()
            } else if r.authors.len() == 1 {
                r.authors[0].clone()
            } else {
                format!("{} et al.", r.authors[0])
            };
            let year = r.year.map(|y| format!("({})", y)).unwrap_or_default();
            let display = [authors, year, r.title.clone()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("  ");
            self.entries[idx].reference = r;
            self.entries[idx].display = display;
        }
        Ok(())
    }

    fn action_copy_bib(&self) -> Result<()> {
        let entry = match self.selected_entry() {
            Some(e) => e,
            None => return Ok(()),
        };
        let r = &entry.reference;
        let cite_key = &entry.dir_name;
        let authors_bib = r.authors.join(" and ");

        let mut bib = format!("@article{{{},\n", cite_key);
        bib.push_str(&format!("  title = {{{}}},\n", r.title));
        if !authors_bib.is_empty() {
            bib.push_str(&format!("  author = {{{}}},\n", authors_bib));
        }
        if let Some(year) = r.year {
            bib.push_str(&format!("  year = {{{}}},\n", year));
        }
        if let Some(ref journal) = r.journal {
            bib.push_str(&format!("  journal = {{{}}},\n", journal));
        }
        if let Some(ref doi) = r.doi {
            bib.push_str(&format!("  doi = {{{}}},\n", doi));
        }
        if let Some(ref arxiv) = r.arxiv {
            bib.push_str(&format!("  eprint = {{{}}},\n", arxiv));
            bib.push_str("  archiveprefix = {arXiv},\n");
        }
        bib.push('}');

        std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(bib.as_bytes());
                }
                child.wait()
            })?;

        Ok(())
    }
}

fn draw(f: &mut Frame, app: &mut App) {
    let t = &app.theme;
    let s_text = Style::default().fg(t.text);
    let s_dim = Style::default().fg(t.text_dim);
    let s_muted = Style::default().fg(t.text_muted);
    let s_accent = Style::default().fg(t.accent);
    let s_warm = Style::default().fg(t.warm);

    let area = f.area();
    let resolved = app.layout.resolve(area.width, area.height);

    let (list_area, preview_area) = match resolved {
        ResolvedLayout::Wide => {
            let chunks = Layout::horizontal([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(area);
            (chunks[0], Some(chunks[1]))
        }
        ResolvedLayout::Tall => {
            let chunks = Layout::vertical([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(area);
            (chunks[0], Some(chunks[1]))
        }
    };

    let left_chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(list_area);

    // Filter input
    let prompt = " search: ";
    let mut filter_spans = vec![Span::styled(prompt, s_muted)];
    if let Some(ref tag) = app.tag_filter {
        filter_spans.push(Span::styled(format!("[{}] ", tag), s_warm));
    }
    if !app.filter.is_empty() {
        filter_spans.push(Span::styled(&app.filter, s_text));
    }
    f.render_widget(
        Paragraph::new(Line::from(filter_spans)),
        left_chunks[0],
    );

    if app.input_mode == InputMode::Insert {
        let tag_label_len = app.tag_filter.as_ref().map(|t| t.len() + 3).unwrap_or(0);
        let cursor_x = left_chunks[0].x + prompt.len() as u16 + tag_label_len as u16 + app.filter.len() as u16;
        let cursor_y = left_chunks[0].y;
        f.set_cursor_position((cursor_x, cursor_y));
    }

    // Paper list — year + author + title
    let list_width = left_chunks[1].width as usize;
    let prefix_width = 3 + 6 + 14; // highlight_symbol + year + author
    let title_max = list_width.saturating_sub(prefix_width);

    let items: Vec<ListItem> = app
        .filtered_indices
        .iter()
        .map(|&idx| {
            let r = &app.entries[idx].reference;

            let year_str = r.year
                .map(|y| format!(" {} ", y))
                .unwrap_or_else(|| "      ".to_string());

            let author_str = r.authors.first()
                .map(|a| {
                    let last = if let Some((last, _)) = a.rsplit_once(',') {
                        last.trim()
                    } else {
                        a.split_whitespace().last().unwrap_or(a)
                    };
                    format!("{:>12}  ", truncate_str(last, 12))
                })
                .unwrap_or_else(|| "              ".to_string());

            let title = truncate_ellipsis(&r.title, title_max);

            ListItem::new(Line::from(vec![
                Span::styled(year_str, s_muted),
                Span::styled(author_str, s_dim),
                Span::styled(title, s_text),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(t.selection).add_modifier(Modifier::BOLD))
        .highlight_symbol(" > ");

    f.render_stateful_widget(list, left_chunks[1], &mut app.list_state);

    // Status bar
    let count = format!("  {}/{}", app.filtered_indices.len(), app.entries.len());
    let mode_indicator = match app.input_mode {
        InputMode::Normal => Span::styled(" NOR ", s_accent.add_modifier(Modifier::BOLD)),
        InputMode::Insert => Span::styled(" INS ", s_warm.add_modifier(Modifier::BOLD)),
    };
    let mode_hint = match (app.input_mode, &app.mode) {
        (InputMode::Insert, _) => "  esc normal",
        (InputMode::Normal, Mode::Browse) => "  / search  e edit  y bib  tab tags  q quit",
        (InputMode::Normal, Mode::Cite { .. }) => "  / search  enter select  tab tags  q quit",
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            mode_indicator,
            Span::styled(count, s_muted),
            Span::styled(mode_hint, s_muted),
        ])),
        left_chunks[2],
    );

    // Preview pane
    if let Some(pane_area) = preview_area {
        let (_sep_border, content_area) = if resolved == ResolvedLayout::Wide {
            let inner = Layout::horizontal([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(pane_area);
            let sep = Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(t.border));
            f.render_widget(sep, pane_area);
            ((), inner[1])
        } else {
            let inner = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(pane_area);
            let sep = Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(t.border));
            f.render_widget(sep, inner[0]);
            let content = Layout::horizontal([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner[1]);
            ((), content[1])
        };
        let styles = Styles { text: s_text, dim: s_dim, muted: s_muted, accent: s_accent, warm: s_warm };
        draw_preview(f, app, content_area, &styles);
    }

    // Tag picker popup
    if let Some(ref popup) = app.tag_popup {
        let area = f.area();
        let max_visible = 20.min(popup.filtered_tags.len());
        let height = max_visible as u16 + 4;
        let width = 36.min(area.width.saturating_sub(4));
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .title(" Tags ")
            .title_style(s_accent.add_modifier(Modifier::BOLD));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        let popup_chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

        let filter_line = if popup.filter.is_empty() {
            Line::from(Span::styled(" type to filter...", s_muted))
        } else {
            Line::from(vec![
                Span::styled(" > ", s_accent),
                Span::styled(&popup.filter, s_text),
            ])
        };
        f.render_widget(Paragraph::new(filter_line), popup_chunks[0]);

        let scroll = if popup.selected >= max_visible {
            popup.selected - max_visible + 1
        } else {
            0
        };

        let lines: Vec<Line> = popup.filtered_tags.iter()
            .enumerate()
            .skip(scroll)
            .take(max_visible)
            .map(|(i, tag)| {
                let is_selected = i == popup.selected;
                let prefix = if is_selected { " > " } else { "   " };
                let style = if is_selected {
                    Style::default().fg(t.text).bg(t.selection).add_modifier(Modifier::BOLD)
                } else if tag == "(all)" {
                    s_dim
                } else {
                    Style::default().fg(t.warm)
                };
                Line::from(Span::styled(format!("{}{}", prefix, tag), style))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), popup_chunks[1]);

        let hint = Line::from(Span::styled(" enter select  esc cancel", s_muted));
        f.render_widget(Paragraph::new(hint), popup_chunks[2]);
    }
}

struct Styles {
    text: Style,
    dim: Style,
    muted: Style,
    accent: Style,
    warm: Style,
}

fn draw_preview(
    f: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    s: &Styles,
) {
    let s_text = s.text;
    let s_dim = s.dim;
    let s_muted = s.muted;
    let s_accent = s.accent;
    let s_warm = s.warm;
    if let Some(entry) = app.selected_entry() {
        let r = &entry.reference;
        let mut lines: Vec<Line> = Vec::new();

        lines.push(Line::from(""));

        lines.push(Line::from(Span::styled(
            &r.title,
            s_text.add_modifier(Modifier::BOLD),
        )));

        if r.year.is_some() || r.journal.is_some() {
            let mut parts = Vec::new();
            if let Some(year) = r.year {
                parts.push(Span::styled(year.to_string(), s_warm));
            }
            if let Some(ref journal) = r.journal {
                if r.year.is_some() {
                    parts.push(Span::styled(" · ", s_muted));
                }
                parts.push(Span::styled(journal.as_str(), s_dim));
            }
            lines.push(Line::from(parts));
        }

        lines.push(Line::from(""));

        if !r.authors.is_empty() {
            let author_text = r.authors.join(", ");
            lines.push(Line::from(Span::styled(author_text, s_accent)));
            lines.push(Line::from(""));
        }

        if let Some(ref doi) = r.doi {
            lines.push(Line::from(vec![
                Span::styled("doi   ", s_muted),
                Span::styled(doi.as_str(), s_dim),
            ]));
        }
        if let Some(ref arxiv) = r.arxiv {
            lines.push(Line::from(vec![
                Span::styled("arxiv ", s_muted),
                Span::styled(arxiv.as_str(), s_dim),
            ]));
        }
        if !r.tags.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("tags  ", s_muted),
                Span::styled(r.tags.join(", "), s_warm),
            ]));
        }

        if let Some(ref abs) = r.r#abstract {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(abs.as_str(), s_dim)));
        }

        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            area,
        );
    } else {
        f.render_widget(
            Paragraph::new(Span::styled("No selection", s_muted)),
            area,
        );
    }
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}

fn truncate_ellipsis(s: &str, max: usize) -> String {
    if max < 2 || s.len() <= max {
        s.to_string()
    } else {
        let end = s.floor_char_boundary(max - 1);
        format!("{}…", &s[..end])
    }
}
