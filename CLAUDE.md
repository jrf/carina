# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

Carina is a CLI reference/paper manager written in Rust. Think Papis, but faster, opinionated, and pleasant to use. The filesystem is the source of truth — each reference is a directory containing a PDF and an `info.toml` metadata file. SQLite (with FTS5) serves as a read cache/index, fully rebuildable from the filesystem at any time.

## Build commands

- `cargo build` — debug build
- `cargo build --release` — release build
- `cargo run -- <args>` — run CLI with arguments
- `cargo test` — run all tests
- `cargo test <test_name>` — run a single test
- `cargo clippy` — lint
- `cargo fmt --check` — check formatting

## Architecture

### Filesystem layout (library)

```
~/Papers/                          # configurable via $CARINA_LIBRARY or config
  vaswani-2017-attention/
    info.toml                      # source of truth — human-editable metadata
    vaswani-2017-attention.pdf
  einstein-1905-electrodynamics/
    info.toml
    einstein-1905-electrodynamics.pdf
```

Directory naming: `{first-author}-{year}-{first-title-word}`, with `-2` suffix on collision.

### Core design principles

- **Filesystem is truth.** SQLite is a disposable index. `carina reindex` rebuilds it from scratch.
- **Opinionated defaults.** Library at `~/Papers`, `$EDITOR` for metadata editing, `open` for PDF viewing. Minimal config needed.
- **Fast.** Rust + SQLite FTS5. No Python startup tax.
- **Composable.** Output is pipe-friendly. Integrates with `fzf`, `jq`, `$EDITOR`.

### Config

`~/.config/carina/config.toml` — library path, default editor/viewer overrides.

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

### Key operations

- **Import:** copy PDF into named directory, extract metadata from PDF attributes, fetch from arXiv/CrossRef by DOI if available, write `info.toml`, index into SQLite.
- **Search:** FTS5 query across title, authors, abstract, PDF full text.
- **Export:** generate BibTeX from `info.yaml` fields.
- **Edit:** open `info.toml` in `$EDITOR`.
- **Open:** open PDF in system viewer or `$CARINA_READER`.
