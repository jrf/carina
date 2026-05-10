use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::cursor::Show;
use crossterm::style::ResetColor;
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
    theme_popup: Option<ThemePopup>,
    layout: LayoutMode,
    flash: Option<(String, std::time::Instant)>,
    preview_scroll: u16,
    show_help: bool,
    list_height: usize,
}

struct TagPopup {
    filter: String,
    filtered_tags: Vec<String>,
    counts: std::collections::BTreeMap<String, usize>,
    total: usize,
    selected: usize,
    scroll: usize,
    prev_tag_filter: Option<String>,
}

impl TagPopup {
    fn new(all_tags: &[String], entries: &[Entry], current_tag_filter: &Option<String>) -> Self {
        let mut counts = std::collections::BTreeMap::new();
        for e in entries {
            for tag in &e.reference.tags {
                *counts.entry(tag.clone()).or_insert(0) += 1;
            }
        }
        let total = entries.len();
        let mut tags = vec!["(all)".to_string()];
        tags.extend(all_tags.iter().cloned());
        Self {
            filter: String::new(),
            filtered_tags: tags,
            counts,
            total,
            selected: 0,
            scroll: 0,
            prev_tag_filter: current_tag_filter.clone(),
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

    fn count_for(&self, tag: &str) -> usize {
        if tag == "(all)" {
            self.total
        } else {
            self.counts.get(tag).copied().unwrap_or(0)
        }
    }

    fn selected_as_filter(&self) -> Option<String> {
        match self.selected_tag() {
            Some("(all)") | None => None,
            Some(t) => Some(t.to_string()),
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

    fn page_down(&mut self) {
        if !self.filtered_tags.is_empty() {
            self.selected = (self.selected + 20).min(self.filtered_tags.len() - 1);
        }
    }

    fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(20);
    }

    fn clamp_scroll(&mut self, visible: usize) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected - visible + 1;
        }
    }

    fn selected_tag(&self) -> Option<&str> {
        self.filtered_tags.get(self.selected).map(|s| s.as_str())
    }
}

struct ThemePopup {
    names: Vec<String>,
    selected: usize,
}

impl ThemePopup {
    fn new() -> Self {
        let mut names = vec!["default".to_string()];
        let theme_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("carina")
            .join("themes");
        if let Ok(entries) = std::fs::read_dir(&theme_dir) {
            let mut found: Vec<String> = entries
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.strip_suffix(".toml").map(|s| s.to_string())
                })
                .collect();
            found.sort();
            names.extend(found);
        }
        Self { names, selected: 0 }
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self) {
        if !self.names.is_empty() {
            self.selected = (self.selected + 1).min(self.names.len() - 1);
        }
    }

    fn selected_name(&self) -> Option<&str> {
        self.names.get(self.selected).map(|s| s.as_str())
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
            let _ = f.execute(ResetColor);
            let _ = f.execute(Show);
        }
        prev_hook(info);
    }));

    tty_ctl.execute(EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;

    let backend = CrosstermBackend::new(BufWriter::new(tty.try_clone()?));
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_event_loop(&mut terminal, &mut app, &mut tty_ctl);

    terminal::disable_raw_mode()?;
    tty_ctl.execute(LeaveAlternateScreen)?;
    tty_ctl.execute(ResetColor)?;
    tty_ctl.execute(Show)?;

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

        let timeout = if app.flash.is_some() {
            std::time::Duration::from_millis(100)
        } else {
            std::time::Duration::from_secs(60)
        };
        if !event::poll(timeout)? {
            if app.flash_message().is_none() {
                app.flash = None;
            }
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if app.show_help {
                app.show_help = false;
                continue;
            }

            if app.tag_popup.is_some() {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) => {
                        let prev = app.tag_popup.as_ref().unwrap().prev_tag_filter.clone();
                        app.tag_popup = None;
                        app.tag_filter = prev;
                        app.rebuild_filter();
                    }
                    (KeyCode::Up, _) => {
                        app.tag_popup.as_mut().unwrap().move_up();
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Down, _) => {
                        app.tag_popup.as_mut().unwrap().move_down();
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                        app.tag_popup.as_mut().unwrap().page_down();
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                        app.tag_popup.as_mut().unwrap().page_up();
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Backspace, _) => {
                        let popup = app.tag_popup.as_mut().unwrap();
                        popup.filter.pop();
                        popup.rebuild(&app.all_tags);
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Char(c), _) => {
                        let popup = app.tag_popup.as_mut().unwrap();
                        popup.filter.push(c);
                        popup.rebuild(&app.all_tags);
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Enter, _) => {
                        app.tag_popup = None;
                    }
                    _ => {}
                }
                continue;
            }

            if app.theme_popup.is_some() {
                match key.code {
                    KeyCode::Esc => { app.theme_popup = None; }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.theme_popup.as_mut().unwrap().move_up();
                        if let Some(name) = app.theme_popup.as_ref().unwrap().selected_name() {
                            let theme_name = if name == "default" { None } else { Some(name) };
                            app.theme = theme::load_theme(theme_name);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.theme_popup.as_mut().unwrap().move_down();
                        if let Some(name) = app.theme_popup.as_ref().unwrap().selected_name() {
                            let theme_name = if name == "default" { None } else { Some(name) };
                            app.theme = theme::load_theme(theme_name);
                        }
                    }
                    KeyCode::Enter => {
                        let name = app.theme_popup.as_ref().unwrap().selected_name()
                            .map(|s| s.to_string());
                        app.theme_popup = None;
                        if let Some(ref n) = name {
                            app.flash = Some((format!("Theme: {}", n), std::time::Instant::now()));
                        }
                    }
                    _ => {}
                }
                continue;
            }

            match app.input_mode {
                InputMode::Insert => match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) => {
                        app.input_mode = InputMode::Normal;
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => app.should_quit = true,

                    (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                        app.filter.push(c);
                        app.rebuild_filter();
                    }
                    (KeyCode::Backspace, _) => {
                        app.filter.pop();
                        app.rebuild_filter();
                    }

                    (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => app.move_up(),
                    (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => app.move_down(),
                    (KeyCode::Char('d'), KeyModifiers::CONTROL) => app.half_page_down(),
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => app.half_page_up(),
                    (KeyCode::Char('f'), KeyModifiers::CONTROL) => app.page_down(),
                    (KeyCode::Char('b'), KeyModifiers::CONTROL) => app.page_up(),

                    (KeyCode::Tab, _) => {
                        app.tag_popup = Some(TagPopup::new(&app.all_tags, &app.entries, &app.tag_filter));
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
                    (KeyCode::Char('d'), KeyModifiers::CONTROL) => app.half_page_down(),
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => app.half_page_up(),
                    (KeyCode::Char('f'), KeyModifiers::CONTROL) => app.page_down(),
                    (KeyCode::Char('b'), KeyModifiers::CONTROL) => app.page_up(),
                    (KeyCode::Char('g'), KeyModifiers::NONE) => app.move_to_top(),
                    (KeyCode::Char('G'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.move_to_bottom();
                    }

                    (KeyCode::Char('t'), KeyModifiers::NONE) => {
                        app.tag_popup = Some(TagPopup::new(&app.all_tags, &app.entries, &app.tag_filter));
                    }
                    (KeyCode::Char('T'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.theme_popup = Some(ThemePopup::new());
                    }
                    (KeyCode::Char('c'), KeyModifiers::NONE) => {
                        app.filter.clear();
                        app.tag_filter = None;
                        app.rebuild_filter();
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
                    (KeyCode::Char('o'), KeyModifiers::NONE) => {
                        app.action_open_url();
                    }

                    (KeyCode::Char('J'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.preview_scroll = app.preview_scroll.saturating_add(3);
                    }
                    (KeyCode::Char('K'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.preview_scroll = app.preview_scroll.saturating_sub(3);
                    }
                    (KeyCode::Char('?'), _) => {
                        app.show_help = true;
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
            theme_popup: None,
            layout: LayoutMode::from_config(config.layout.as_deref()),
            flash: None,
            preview_scroll: 0,
            show_help: false,
            list_height: 20,
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
        self.preview_scroll = 0;
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
        self.preview_scroll = 0;
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
        self.preview_scroll = 0;
    }

    fn scroll_up(&mut self, lines: usize) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some(i.saturating_sub(lines)));
        self.preview_scroll = 0;
    }

    fn scroll_down(&mut self, lines: usize) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let max = self.filtered_indices.len() - 1;
        self.list_state.select(Some((i + lines).min(max)));
        self.preview_scroll = 0;
    }

    fn half_page_up(&mut self) {
        self.scroll_up(self.list_height / 2);
    }

    fn half_page_down(&mut self) {
        self.scroll_down(self.list_height / 2);
    }

    fn page_up(&mut self) {
        self.scroll_up(self.list_height);
    }

    fn page_down(&mut self) {
        self.scroll_down(self.list_height);
    }

    fn move_to_top(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.list_state.select(Some(0));
            self.preview_scroll = 0;
        }
    }

    fn move_to_bottom(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.list_state.select(Some(self.filtered_indices.len() - 1));
            self.preview_scroll = 0;
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

    fn action_copy_bib(&mut self) -> Result<()> {
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

        self.flash = Some(("Copied BibTeX".to_string(), std::time::Instant::now()));
        Ok(())
    }

    fn flash_message(&self) -> Option<&str> {
        self.flash.as_ref().and_then(|(msg, t)| {
            if t.elapsed().as_secs() < 2 { Some(msg.as_str()) } else { None }
        })
    }

    fn action_open_url(&mut self) {
        let entry = match self.selected_entry() {
            Some(e) => e,
            None => return,
        };
        let r = &entry.reference;
        let url = if let Some(ref doi) = r.doi {
            format!("https://doi.org/{}", doi)
        } else if let Some(ref arxiv) = r.arxiv {
            format!("https://arxiv.org/abs/{}", arxiv)
        } else {
            self.flash = Some(("No DOI or arXiv ID".to_string(), std::time::Instant::now()));
            return;
        };
        let _ = std::process::Command::new("open").arg(&url).spawn();
        self.flash = Some(("Opened in browser".to_string(), std::time::Instant::now()));
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
        filter_spans.push(Span::styled(format!("[{}] ", tag), s_accent));
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
    app.list_height = left_chunks[1].height as usize;
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
        InputMode::Normal => Span::styled(
            " NOR ",
            Style::default().fg(t.status_fg).bg(t.normal_bg).add_modifier(Modifier::BOLD),
        ),
        InputMode::Insert => Span::styled(
            " INS ",
            Style::default().fg(t.status_fg).bg(t.insert_bg).add_modifier(Modifier::BOLD),
        ),
    };
    let mode_hint = match (app.input_mode, &app.mode) {
        (InputMode::Insert, _) => "  esc normal",
        (InputMode::Normal, Mode::Browse) => "  / search  c clear  q quit",
        (InputMode::Normal, Mode::Cite { .. }) => "  / search  c clear  q quit",
    };
    let right_status = if let Some(flash) = app.flash_message() {
        Span::styled(format!("  {}", flash), s_warm)
    } else {
        Span::styled(mode_hint, s_muted)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            mode_indicator,
            Span::styled(count, s_muted),
            right_status,
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
        let styles = Styles { text: s_text, dim: s_dim, muted: s_muted, accent: s_accent };
        draw_preview(f, app, content_area, &styles);
    }

    // Tag picker popup
    if let Some(ref mut popup) = app.tag_popup {
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
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
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

        popup.clamp_scroll(max_visible);

        let inner_width = popup_chunks[1].width as usize;
        let lines: Vec<Line> = popup.filtered_tags.iter()
            .enumerate()
            .skip(popup.scroll)
            .take(max_visible)
            .map(|(i, tag)| {
                let is_selected = i == popup.selected;
                let prefix = if is_selected { " > " } else { "   " };
                let style = if is_selected {
                    Style::default().fg(t.text).bg(t.selection).add_modifier(Modifier::BOLD)
                } else {
                    s_dim
                };
                let count = popup.count_for(tag);
                let count_str = format!("{} ", count);
                let label = format!("{}{}", prefix, tag);
                let pad = inner_width.saturating_sub(label.len() + count_str.len());
                let count_style = if is_selected {
                    Style::default().fg(t.text_dim).bg(t.selection)
                } else {
                    s_muted
                };
                Line::from(vec![
                    Span::styled(label, style),
                    Span::styled(" ".repeat(pad), style),
                    Span::styled(count_str, count_style),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines), popup_chunks[1]);

        let hint = Line::from(Span::styled(" enter select  esc cancel", s_muted));
        f.render_widget(Paragraph::new(hint), popup_chunks[2]);
    }

    // Theme picker popup
    if let Some(ref popup) = app.theme_popup {
        let area = f.area();
        let max_visible = 12.min(popup.names.len());
        let height = max_visible as u16 + 3;
        let width = 30.min(area.width.saturating_sub(4));
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
            .title(" Theme ")
            .title_style(s_accent.add_modifier(Modifier::BOLD));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        let popup_chunks = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

        let scroll = popup.selected.saturating_sub(max_visible - 1);

        let lines: Vec<Line> = popup.names.iter()
            .enumerate()
            .skip(scroll)
            .take(max_visible)
            .map(|(i, name)| {
                let is_selected = i == popup.selected;
                let prefix = if is_selected { " > " } else { "   " };
                let style = if is_selected {
                    Style::default().fg(t.text).bg(t.selection).add_modifier(Modifier::BOLD)
                } else {
                    s_dim
                };
                Line::from(Span::styled(format!("{}{}", prefix, name), style))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), popup_chunks[0]);

        let hint = Line::from(Span::styled(" j/k preview  enter select  esc cancel", s_muted));
        f.render_widget(Paragraph::new(hint), popup_chunks[1]);
    }

    // Help popup
    if app.show_help {
        let help_lines = vec![
            ("", "Normal mode"),
            ("j / k", "Move down / up"),
            ("g / G", "Jump to top / bottom"),
            ("^d / ^u", "Half-page down / up"),
            ("^f / ^b", "Page down / up"),
            ("J / K", "Scroll preview down / up"),
            ("/ or i", "Enter search (insert mode)"),
            ("enter", "Open PDF"),
            ("e", "Edit info.toml"),
            ("y", "Copy BibTeX"),
            ("o", "Open DOI / arXiv in browser"),
            ("c", "Clear search and tag filter"),
            ("t", "Browse tags"),
            ("T", "Switch theme"),
            ("q / esc", "Quit"),
            ("", ""),
            ("", "Insert mode"),
            ("esc", "Return to normal mode"),
            ("^p / ^n", "Move up / down"),
            ("^d / ^u", "Half-page down / up"),
            ("^f / ^b", "Page down / up"),
            ("enter", "Open PDF"),
            ("tab", "Browse tags"),
        ];

        let height = help_lines.len() as u16 + 4;
        let width = 56.min(area.width.saturating_sub(4));
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
            .title(" Help ")
            .title_style(s_accent.add_modifier(Modifier::BOLD));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        let popup_chunks = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

        let key_width = 10;
        let lines: Vec<Line> = help_lines.iter()
            .map(|(key, desc)| {
                if key.is_empty() {
                    Line::from(Span::styled(
                        format!(" {}", desc),
                        s_accent.add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(vec![
                        Span::styled(format!(" {:>width$}  ", key, width = key_width), s_accent),
                        Span::styled(*desc, s_dim),
                    ])
                }
            })
            .collect();
        f.render_widget(Paragraph::new(lines), popup_chunks[0]);

        let hint = Line::from(Span::styled(" press any key to close", s_muted));
        f.render_widget(Paragraph::new(hint), popup_chunks[1]);
    }
}

struct Styles {
    text: Style,
    dim: Style,
    muted: Style,
    accent: Style,
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
                parts.push(Span::styled(year.to_string(), s_dim));
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
                Span::styled(r.tags.join(", "), s_dim),
            ]));
        }

        if let Some(ref abs) = r.r#abstract {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(abs.as_str(), s_dim)));
        }

        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.preview_scroll, 0)),
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
