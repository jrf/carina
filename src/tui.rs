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
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
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
    should_quit: bool,
    pending_output: Option<String>,
}

enum Mode {
    Browse,
    Cite { format: String },
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
            match (key.code, key.modifiers) {
                (KeyCode::Esc, _) => app.should_quit = true,
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
                (KeyCode::Char('b'), KeyModifiers::CONTROL) => app.page_up(),
                (KeyCode::Char('f'), KeyModifiers::CONTROL) => app.page_down(),

                (KeyCode::Enter, _) => {
                    app.action_select()?;
                }
                (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                    app.action_edit(terminal, tty_ctl)?;
                }
                (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                    app.action_copy_bib()?;
                }

                _ => {}
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
            },
            theme,
            mode,
            should_quit: false,
            pending_output: None,
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
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.entries.len()).collect();
        } else {
            let pattern = Pattern::parse(
                &self.filter,
                CaseMatching::Ignore,
                Normalization::Smart,
            );
            let mut matcher = Matcher::new(Config::DEFAULT);
            let mut buf = Vec::new();

            let mut scored: Vec<(usize, u32)> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    let haystack = Utf32Str::new(&e.display, &mut buf);
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

    let chunks = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ])
    .split(f.area());

    let left_chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(chunks[0]);

    // Filter input
    let prompt = " search: ";
    let filter_spans = if app.filter.is_empty() {
        vec![Span::styled(prompt, s_muted)]
    } else {
        vec![
            Span::styled(prompt, s_muted),
            Span::styled(&app.filter, s_text),
        ]
    };
    f.render_widget(
        Paragraph::new(Line::from(filter_spans)),
        left_chunks[0],
    );

    let cursor_x = left_chunks[0].x + prompt.len() as u16 + app.filter.len() as u16;
    let cursor_y = left_chunks[0].y;
    f.set_cursor_position((cursor_x, cursor_y));

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
    let mode_hint = match &app.mode {
        Mode::Browse => "  enter open  ^e edit  ^y bib  esc quit",
        Mode::Cite { .. } => "  enter select  esc cancel",
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(count, s_muted),
            Span::styled(mode_hint, s_muted),
        ])),
        left_chunks[2],
    );

    // Preview pane
    let preview_area = Layout::horizontal([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(chunks[1]);

    let sep = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(t.border));
    f.render_widget(sep, chunks[1]);

    if let Some(entry) = app.selected_entry() {
        let r = &entry.reference;
        let mut lines: Vec<Line> = Vec::new();

        lines.push(Line::from(""));

        // Title
        lines.push(Line::from(Span::styled(
            &r.title,
            s_text.add_modifier(Modifier::BOLD),
        )));

        // Year + journal
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

        // Authors
        if !r.authors.is_empty() {
            for author in r.authors.iter().take(8) {
                lines.push(Line::from(Span::styled(author.as_str(), s_accent)));
            }
            if r.authors.len() > 8 {
                lines.push(Line::from(Span::styled(
                    format!("+{} more", r.authors.len() - 8),
                    s_muted,
                )));
            }
            lines.push(Line::from(""));
        }

        // Metadata
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

        // Abstract
        if let Some(ref abs) = r.r#abstract {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(abs.as_str(), s_dim)));
        }

        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            preview_area[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(Span::styled("No selection", s_muted)),
            preview_area[1],
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
