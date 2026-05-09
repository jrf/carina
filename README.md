# Carina

A fast, opinionated reference manager for the terminal.

## Philosophy

- **Filesystem is truth.** Each reference is a directory containing a PDF and an `info.toml` metadata file. SQLite is a disposable search index.
- **Opinionated defaults.** Works out of the box with zero configuration.
- **Composable.** Pipe-friendly output. Built-in [fzf](https://github.com/junegunn/fzf) integration for interactive browsing.
- **Fast.** Rust + SQLite FTS5. No startup tax.

## Install

Requires Rust and [fzf](https://github.com/junegunn/fzf).

```
cargo install --path .
```

Or with just:

```
just install
```

## Usage

```
carina                          # browse library in fzf
carina jepa                     # browse with "jepa" pre-filled
carina add 1706.03762           # import by arXiv ID (fetches metadata + PDF)
carina add 10.1038/nature14539  # import by DOI (fetches metadata)
carina add paper.pdf            # import local PDF
carina open attention           # open PDF matching "attention"
carina edit attention           # edit metadata in $EDITOR
carina bib attention            # output BibTeX
carina cite --format typst      # pick a reference, output @cite-key
carina list --tag ml             # list papers tagged "ml"
carina reindex                  # rebuild search index from filesystem
```

### Browse keybindings

| Key | Action |
|-----|--------|
| `enter` | Open PDF |
| `ctrl-e` | Edit metadata |
| `ctrl-y` | Copy BibTeX to clipboard |

## Library layout

```
~/Papers/
  vaswani-2017-attention/
    info.toml
    1706.03762.pdf
  lecun-2015-deep/
    info.toml
```

Directory naming: `{first-author}-{year}-{first-title-word}`.

### info.toml

```toml
title = "Attention Is All You Need"
authors = ["Ashish Vaswani", "Noam Shazeer", "Niki Parmar"]
year = 2017
arxiv = "1706.03762"
tags = ["transformers", "nlp"]
files = ["1706.03762.pdf"]
abstract = """
The dominant sequence transduction models are based on complex recurrent or
convolutional neural networks...
"""
```

## Configuration

Optional. Carina works without any config file.

`~/.config/carina/config.toml`:

```toml
library = "~/Papers"       # default
editor = "hx"              # defaults to $EDITOR
reader = "open"            # defaults to $CARINA_READER or "open"
```

Environment variables: `$CARINA_LIBRARY`, `$CARINA_READER`, `$EDITOR`.

## Helix integration

Add to `~/.config/helix/config.toml`:

```toml
[keys.normal.space.r]
r = [":insert-output carina cite", ":redraw"]
t = [":insert-output carina cite --format typst", ":redraw"]
l = [":insert-output carina cite --format latex", ":redraw"]
```

`Space r t` in normal mode opens the picker and inserts a Typst citation at the cursor.

## Smart import

`carina add` detects the input type automatically:

- **arXiv ID** (`1706.03762`) -- fetches metadata from arXiv API, downloads PDF
- **arXiv URL** (`https://arxiv.org/abs/1706.03762`) -- same
- **DOI** (`10.1038/nature14539`) -- fetches metadata from CrossRef
- **Local PDF** (`paper.pdf`) -- extracts metadata from PDF; if filename looks like an arXiv ID, fetches metadata from arXiv
