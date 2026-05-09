mod config;
mod fetch;
mod import_polaris;
mod index;
mod metadata;
mod model;
mod storage;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use config::Config;

#[derive(Parser)]
#[command(name = "carina", about = "A fast, opinionated reference manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Search/browse the library (default)
    Search {
        /// Search query (pre-fills fzf filter)
        query: Vec<String>,
    },
    /// Import a PDF into the library
    Add {
        /// Path to PDF file, DOI, or arXiv ID
        path: String,
    },
    /// List references in the library
    List {
        /// Filter by tag
        #[arg(short, long)]
        tag: Option<String>,
    },
    /// Show metadata for a reference
    Show {
        /// Reference directory name
        name: String,
    },
    /// Open a reference's PDF
    Open {
        /// Open with a specific macOS app (e.g. "Polaris", "Skim")
        #[arg(short, long)]
        reader: Option<String>,
        /// Search query to find the reference
        query: Vec<String>,
    },
    /// Edit a reference's metadata in $EDITOR
    Edit {
        /// Search query to find the reference
        query: Vec<String>,
    },
    /// Output BibTeX for a reference
    Bib {
        /// Search query to find the reference
        query: Vec<String>,
    },
    /// Pick a reference and output its citation key
    Cite {
        /// Output format: plain (default), latex, typst
        #[arg(short, long, default_value = "plain")]
        format: String,
    },
    /// Rebuild the search index from filesystem
    Reindex,
    /// Import papers from a Polaris library
    ImportPolaris {
        /// Overwrite metadata for existing entries
        #[arg(short, long)]
        force: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;
    let library = config.library_dir();

    match cli.command {
        None | Some(Command::Search { .. }) => {
            let query = match cli.command {
                Some(Command::Search { query }) => query,
                _ => vec![],
            };
            cmd_search(&config, &library, &query)
        }
        Some(Command::Add { path }) => cmd_add(&config, &library, &path),
        Some(Command::List { tag }) => cmd_list(&library, tag.as_deref()),
        Some(Command::Show { name }) => cmd_show(&library, &name),
        Some(Command::Open { reader, query }) => cmd_open(&config, &library, &query, reader.as_deref()),
        Some(Command::Edit { query }) => cmd_edit(&config, &library, &query),
        Some(Command::Bib { query }) => cmd_bib(&library, &query),
        Some(Command::Cite { format }) => cmd_cite(&library, &format),
        Some(Command::Reindex) => cmd_reindex(&library),
        Some(Command::ImportPolaris { force }) => import_polaris::run(&library, force),
    }
}

fn launch_fzf(library: &Path, entries: &[(String, String)], initial_query: Option<&str>) -> Result<()> {
    if entries.is_empty() {
        println!("No results.");
        return Ok(());
    }

    let carina_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("carina"));
    let library_str = library.to_string_lossy();
    let bin_str = carina_bin.to_string_lossy();

    let input = entries
        .iter()
        .map(|(name, display)| format!("{}\t{}", name, display))
        .collect::<Vec<_>>()
        .join("\n");

    let preview_cmd = format!(
        "CARINA_LIBRARY={} {} show {{1}}",
        shell_escape(&library_str), shell_escape(&bin_str)
    );
    let open_cmd = format!(
        "CARINA_LIBRARY={} {} open {{1}}",
        shell_escape(&library_str), shell_escape(&bin_str)
    );
    let edit_cmd = format!(
        "CARINA_LIBRARY={} {} edit {{1}}",
        shell_escape(&library_str), shell_escape(&bin_str)
    );
    let bib_cmd = format!(
        "CARINA_LIBRARY={} {} bib {{1}} | pbcopy",
        shell_escape(&library_str), shell_escape(&bin_str)
    );

    let mut args = vec![
        "--delimiter=\t".to_string(),
        "--with-nth=2..".to_string(),
        "--height=100%".to_string(),
        "--preview".to_string(), preview_cmd,
        "--preview-window=right:40%:wrap".to_string(),
        "--preview-wrap-sign=".to_string(),
        format!("--bind=enter:execute({})+abort", open_cmd),
        format!("--bind=ctrl-e:execute({})", edit_cmd),
        format!("--bind=ctrl-y:execute-silent({})", bib_cmd),
        "--header=enter: open │ ctrl-e: edit │ ctrl-y: copy bib".to_string(),
        "--no-mouse".to_string(),
    ];

    if let Some(q) = initial_query {
        args.push(format!("--query={}", q));
    }

    let status = std::process::Command::new("fzf")
        .args(&args)
        .env("CARINA_LIBRARY", library.as_os_str())
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.take() {
                let mut stdin = stdin;
                let _ = stdin.write_all(input.as_bytes());
                drop(stdin);
            }
            child.wait()
        });

    match status {
        Ok(s) if s.success() || s.code() == Some(130) => Ok(()),
        Ok(s) => anyhow::bail!("fzf exited with status {}", s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("fzf not found — install it with `brew install fzf`")
        }
        Err(e) => Err(e).context("Failed to launch fzf"),
    }
}

fn cmd_add(_config: &Config, library: &Path, input: &str) -> Result<()> {
    std::fs::create_dir_all(library)?;

    let path = PathBuf::from(input);
    if path.exists() {
        return add_from_file(library, input);
    }

    if let Some(arxiv_id) = fetch::detect_arxiv_id(input) {
        return add_from_arxiv(library, &arxiv_id);
    }

    if let Some(doi) = fetch::detect_doi(input) {
        return add_from_doi(library, &doi);
    }

    anyhow::bail!("Not a file, arXiv ID, or DOI: {}", input)
}

fn index_reference(library: &Path, ref_dir: &Path, reference: &crate::model::Reference) {
    if let Ok(idx) = index::Index::open(library) {
        let dir_name = ref_dir.file_name().unwrap_or_default().to_string_lossy().to_string();
        let _ = idx.upsert(&dir_name, reference);
    }
}

fn add_from_arxiv(library: &Path, arxiv_id: &str) -> Result<()> {
    println!("Fetching metadata from arXiv: {}", arxiv_id);
    let mut reference = fetch::fetch_arxiv(arxiv_id)?;

    let ref_dir = storage::create_ref_dir(library, &reference)?;
    let pdf_filename = format!("{}.pdf", arxiv_id);
    let pdf_path = ref_dir.join(&pdf_filename);

    println!("Downloading PDF...");
    fetch::download_arxiv_pdf(arxiv_id, &pdf_path)?;

    reference.files = vec![pdf_filename];
    metadata::write_info(&ref_dir, &reference)?;
    index_reference(library, &ref_dir, &reference);

    println!("Added: {}", reference.title);
    println!("  → {}", ref_dir.display());
    Ok(())
}

fn add_from_doi(library: &Path, doi: &str) -> Result<()> {
    println!("Fetching metadata from CrossRef: {}", doi);
    let reference = fetch::fetch_crossref(doi)?;

    let ref_dir = storage::create_ref_dir(library, &reference)?;
    metadata::write_info(&ref_dir, &reference)?;
    index_reference(library, &ref_dir, &reference);

    println!("Added: {}", reference.title);
    println!("  → {}", ref_dir.display());
    println!("  (no PDF — add one manually to the directory)");
    Ok(())
}

fn add_from_file(library: &Path, path: &str) -> Result<()> {
    let path = PathBuf::from(path)
        .canonicalize()
        .with_context(|| format!("File not found: {}", path))?;

    anyhow::ensure!(
        path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")),
        "Not a PDF file and not a recognized arXiv ID or DOI"
    );

    let mut reference = metadata::extract_from_pdf(&path)?;

    let arxiv_id = path.file_stem()
        .and_then(|s| fetch::detect_arxiv_id(&s.to_string_lossy()));
    if let Some(ref id) = arxiv_id {
        println!("Detected arXiv ID: {} — fetching metadata...", id);
        if let Ok(fetched) = fetch::fetch_arxiv(id) {
            reference.title = fetched.title;
            reference.authors = fetched.authors;
            reference.year = fetched.year;
            reference.doi = fetched.doi;
            reference.arxiv = fetched.arxiv;
            reference.r#abstract = fetched.r#abstract;
        }
    }

    let ref_dir = storage::create_ref_dir(library, &reference)?;
    let filename = storage::copy_pdf(&path, &ref_dir)?;
    reference.files = vec![filename];
    metadata::write_info(&ref_dir, &reference)?;
    index_reference(library, &ref_dir, &reference);

    println!("Added: {}", reference.title);
    println!("  → {}", ref_dir.display());
    Ok(())
}

fn cmd_search(_config: &Config, library: &Path, query: &[String]) -> Result<()> {
    let dirs = storage::list_ref_dirs(library)?;
    if dirs.is_empty() {
        println!("Library is empty. Use `carina add <file.pdf>` to import a paper.");
        return Ok(());
    }

    let entries: Vec<(String, String)> = dirs
        .iter()
        .filter_map(|dir| {
            let dir_name = dir.file_name()?.to_string_lossy().to_string();
            let r = metadata::read_info(dir).ok()?;
            let authors = if r.authors.is_empty() {
                String::new()
            } else if r.authors.len() == 1 {
                r.authors[0].clone()
            } else {
                format!("{} et al.", r.authors[0])
            };
            let year = r.year.map(|y| format!("({})", y)).unwrap_or_default();
            let display = [authors, year, r.title]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("  ");
            Some((dir_name, display))
        })
        .collect();

    let initial = if query.is_empty() {
        None
    } else {
        Some(query.join(" "))
    };

    launch_fzf(library, &entries, initial.as_deref())
}

fn cmd_cite(library: &Path, format: &str) -> Result<()> {
    let dirs = storage::list_ref_dirs(library)?;
    if dirs.is_empty() {
        anyhow::bail!("Library is empty");
    }

    let mut lines = Vec::new();
    for dir in &dirs {
        let dir_name = match dir.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        let r = match metadata::read_info(dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let authors = if r.authors.is_empty() {
            String::new()
        } else if r.authors.len() == 1 {
            r.authors[0].clone()
        } else {
            format!("{} et al.", r.authors[0])
        };
        let year = r.year.map(|y| format!("({})", y)).unwrap_or_default();
        let display = [authors, year, r.title]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("  ");
        lines.push(format!("{}\t{}", dir_name, display));
    }

    let input = lines.join("\n");

    let output = std::process::Command::new("fzf")
        .args([
            "--delimiter=\t",
            "--with-nth=2..",
            "--height=100%",
            "--no-mouse",
            "--header=Pick a reference",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.take() {
                let mut stdin = stdin;
                let _ = stdin.write_all(input.as_bytes());
                drop(stdin);
            }
            child.wait_with_output()
        })
        .context("Failed to launch fzf")?;

    if !output.status.success() {
        return Ok(());
    }

    let selected = String::from_utf8_lossy(&output.stdout);
    let key = selected.split('\t').next().unwrap_or("").trim();
    if key.is_empty() {
        return Ok(());
    }

    match format {
        "latex" => print!("\\cite{{{}}}", key),
        "typst" => print!("@{}", key),
        _ => print!("{}", key),
    }

    Ok(())
}

fn cmd_reindex(library: &Path) -> Result<()> {
    let idx = index::Index::open(library)?;
    let count = idx.reindex(library)?;
    println!("Indexed {} references.", count);
    Ok(())
}

fn cmd_list(library: &Path, tag_filter: Option<&str>) -> Result<()> {
    let dirs = storage::list_ref_dirs(library)?;
    if dirs.is_empty() {
        println!("Library is empty. Use `carina add <file.pdf>` to import a paper.");
        return Ok(());
    }

    for dir in &dirs {
        let reference = match metadata::read_info(dir) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if let Some(tag) = tag_filter {
            if !reference.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                continue;
            }
        }

        let authors = if reference.authors.is_empty() {
            String::new()
        } else {
            format!(" — {}", reference.authors.join(", "))
        };

        let year = reference
            .year
            .map(|y| format!(" ({})", y))
            .unwrap_or_default();

        println!("{}{}{}", reference.title, authors, year);
    }

    Ok(())
}

fn cmd_show(library: &Path, name: &str) -> Result<()> {
    let dir = library.join(name);
    anyhow::ensure!(dir.join("info.toml").exists(), "Reference not found: {}", name);

    let r = metadata::read_info(&dir)?;

    println!("{}", r.title);

    if r.year.is_some() || r.journal.is_some() {
        let mut parts = Vec::new();
        if let Some(year) = r.year {
            parts.push(year.to_string());
        }
        if let Some(ref journal) = r.journal {
            parts.push(journal.clone());
        }
        println!("{}", parts.join(" · "));
    }

    println!();

    if !r.authors.is_empty() {
        let shown: Vec<_> = r.authors.iter().take(5).collect();
        for author in &shown {
            println!("  {}", author);
        }
        if r.authors.len() > 5 {
            println!("  +{} more", r.authors.len() - 5);
        }
        println!();
    }

    if let Some(ref doi) = r.doi {
        println!("DOI:   {}", doi);
    }
    if let Some(ref arxiv) = r.arxiv {
        println!("arXiv: {}", arxiv);
    }
    if !r.tags.is_empty() {
        println!("Tags:  {}", r.tags.join(", "));
    }

    if let Some(ref abs) = r.r#abstract {
        println!();
        println!("{}", abs);
    }

    Ok(())
}

fn cmd_open(config: &Config, library: &Path, query: &[String], reader: Option<&str>) -> Result<()> {
    let dir = find_reference(library, query)?;
    let reference = metadata::read_info(&dir)?;
    let pdf = reference
        .files
        .first()
        .context("No files associated with this reference")?;
    let pdf_path = dir.join(pdf);

    if let Some(app) = reader {
        std::process::Command::new("open")
            .args(["-a", app])
            .arg(&pdf_path)
            .spawn()
            .with_context(|| format!("Failed to open with {}", app))?;
    } else {
        std::process::Command::new(config.reader())
            .arg(&pdf_path)
            .spawn()
            .context("Failed to open PDF")?;
    }
    Ok(())
}

fn cmd_bib(library: &Path, query: &[String]) -> Result<()> {
    let dir = find_reference(library, query)?;
    let reference = metadata::read_info(&dir)?;

    let cite_key = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let authors_bib = reference
        .authors
        .iter()
        .map(|a| a.as_str())
        .collect::<Vec<_>>()
        .join(" and ");

    println!("@article{{{},", cite_key);
    println!("  title = {{{}}},", reference.title);
    if !authors_bib.is_empty() {
        println!("  author = {{{}}},", authors_bib);
    }
    if let Some(year) = reference.year {
        println!("  year = {{{}}},", year);
    }
    if let Some(ref journal) = reference.journal {
        println!("  journal = {{{}}},", journal);
    }
    if let Some(ref doi) = reference.doi {
        println!("  doi = {{{}}},", doi);
    }
    if let Some(ref arxiv) = reference.arxiv {
        println!("  eprint = {{{}}},", arxiv);
        println!("  archiveprefix = {{arXiv}},");
    }
    println!("}}");

    Ok(())
}

fn cmd_edit(config: &Config, library: &Path, query: &[String]) -> Result<()> {
    let dir = find_reference(library, query)?;
    let info_path = dir.join("info.toml");

    std::process::Command::new(config.editor())
        .arg(&info_path)
        .status()
        .context("Failed to open editor")?;
    Ok(())
}

fn find_reference(library: &Path, query: &[String]) -> Result<PathBuf> {
    let query_str = query.join(" ");
    let dirs = storage::list_ref_dirs(library)?;

    anyhow::ensure!(!dirs.is_empty(), "Library is empty");
    anyhow::ensure!(!query_str.is_empty(), "No search query provided");

    // First try exact directory name match
    let exact = library.join(&query_str);
    if exact.join("info.toml").exists() {
        return Ok(exact);
    }

    // Fuzzy search over titles and authors
    let query_lower = query_str.to_lowercase();
    let mut matches = Vec::new();
    for dir in &dirs {
        let reference = match metadata::read_info(dir) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let haystack = format!(
            "{} {} {}",
            reference.title.to_lowercase(),
            reference.authors.join(" ").to_lowercase(),
            dir.file_name().unwrap_or_default().to_string_lossy().to_lowercase()
        );

        if query_lower.split_whitespace().all(|q| haystack.contains(q)) {
            matches.push(dir.clone());
        }
    }

    match matches.len() {
        0 => anyhow::bail!("No matching reference found"),
        1 => Ok(matches.into_iter().next().unwrap()),
        n => {
            eprintln!("Found {} matches:", n);
            for (i, dir) in matches.iter().enumerate() {
                if let Ok(r) = metadata::read_info(dir) {
                    eprintln!("  [{}] {}", i + 1, r.title);
                }
            }
            anyhow::bail!("Multiple matches — refine your query")
        }
    }
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
