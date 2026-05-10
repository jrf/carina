# TODO

## Now

- [ ] PDF full-text indexing into FTS5 #feature
- [ ] URL import: `carina add <url>` downloads PDF from arbitrary URLs #feature
- [ ] `carina tag` command for quick tagging without opening editor #improvement

## Next

- [ ] `carina related` via Semantic Scholar API #feature

## Later

- [ ] APA/Chicago formatted citation output #feature
- [ ] iCloud sync documentation and testing #docs
- [ ] Batch operations: `carina tag --all`, `carina bib --all` #improvement
- [ ] Configurable browse keybindings #improvement

## Scrapped

## Done

- [x] Project scaffold: `cargo init`, dependencies, module structure #chore
- [x] Config: library path resolution (`$CARINA_LIBRARY`, config.toml, `~/Papers`) #feature
- [x] Storage: create reference directories, copy PDFs, write `info.toml` #feature
- [x] Import: `carina add <file>` with PDF metadata extraction #feature
- [x] Metadata fetch: arXiv API lookup by arXiv ID, CrossRef lookup by DOI #feature
- [x] arXiv auto-detection: recognize arXiv IDs and URLs in `carina add` #feature
- [x] List/filter: `carina list`, filter by tag #feature
- [x] Open: `carina open` with `$CARINA_READER` or `open` fallback, `--reader` flag #feature
- [x] Edit: `carina edit` opens `info.toml` in `$EDITOR` #feature
- [x] BibTeX export: `carina bib <query>` #feature
- [x] fzf browser: `carina` launches interactive picker with preview and keybindings #feature
- [x] Import from Polaris: `carina import-polaris` with `--force` flag #feature
- [x] SQLite FTS5 index: schema, indexing, `carina reindex` #feature
- [x] Picker-agnostic: configurable via `$CARINA_PICKER` or config.toml #improvement
- [x] Shell completions: `carina completions {fish,bash,zsh}` #chore
