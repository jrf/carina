# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

Grimoire is a CLI/TUI reference manager for academic papers, written in Rust. The filesystem is the source of truth — each reference is a directory containing a PDF and an `info.toml` metadata file. SQLite (with FTS5) serves as a disposable read cache/index, fully rebuildable from the filesystem at any time via `grimoire reindex`.

## Build commands

- `cargo build` — debug build
- `cargo build --release` — release build
- `cargo run -- <args>` — run CLI with arguments
- `cargo test` — run all tests
- `cargo test <test_name>` — run a single test
- `cargo clippy` — lint
- `cargo fmt --check` — check formatting
- `just install` — release build + copy to `~/.local/bin/grimoire` + codesign on macOS

## Architecture

### Module overview

- **main.rs** — CLI entry point (clap). Dispatches commands: bare invocation opens TUI, subcommands handle `add`, `cite`, `reindex`, `validate`, `import-polaris`. Contains the smart-add logic that auto-detects input type (arXiv ID, DOI, URL, local PDF).
- **model.rs** — `Reference` struct. The core data type used by every other module.
- **tui.rs** — Interactive TUI (ratatui). Browse/Search modes, fuzzy filtering (nucleo), tag/theme popups, metadata enrichment with background threads, dedup workflow, preview pane. This is the largest file.
- **storage.rs** — Filesystem operations: create reference directories (`{last-name}-{year}-{first-title-word}`), copy PDFs, list reference dirs.
- **metadata.rs** — Read/write `info.toml`, extract metadata and full text from PDFs (lopdf).
- **index.rs** — SQLite FTS5 index. Schema with triggers, upsert with full-text, ranked search.
- **fetch.rs** — External API calls: arXiv API (XML), CrossRef API (JSON), PDF download. Regex-based arXiv ID and DOI detection.
- **config.rs** — Load `~/.config/grimoire/config.toml`. Resolution order: env var > config file > default.
- **theme.rs** — Color theme system. Loads from `~/.config/grimoire/themes/{name}.toml`, defaults to Tokyo Night Moon.
- **validate.rs** — Library integrity checks with optional auto-fix (remove junk files, rename temp files).
- **import_polaris.rs** — One-time import from Polaris PDF reader's SQLite database.

### Core design principles

- **Filesystem is truth.** SQLite is disposable. `grimoire reindex` rebuilds from scratch.
- **Defaults over config.** Library at `~/Papers`, `$EDITOR` for editing, `open` for PDFs. Config is optional.
- **Composable.** Output is pipe-friendly. `grimoire cite` outputs citation keys for editor integration.

### Filesystem layout (library)

```
~/Papers/                          # configurable via $GRIM_LIBRARY or config
  vaswani-2017-attention/
    info.toml                      # source of truth — human-editable metadata
    vaswani-2017-attention.pdf
```

Directory naming: `{first-author}-{year}-{first-title-word}`, with `-2` suffix on collision.

### TUI modes

The TUI has two input modes: **Browse** (single-key shortcuts for actions) and **Search** (typing filters the list). The `LayoutMode` auto-detects terminal aspect ratio — side-by-side when wider than tall, stacked when taller than wide — with manual `wide`/`tall` override in config.

### Metadata schema (info.toml)

```toml
title = "Paper Title"
authors = ["First Author", "Second Author"]
year = 2024
doi = "10.xxxx/xxxxx"
arxiv = "2401.00000"
journal = "Venue"
tags = ["tag1", "tag2"]
files = ["filename.pdf"]
abstract = """
The dominant sequence transduction models...
"""
```
