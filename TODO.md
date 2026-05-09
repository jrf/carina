# TODO

## Now

- [ ] Project scaffold: `cargo init`, dependencies, module structure #chore
- [ ] Config: library path resolution (`$CARINA_LIBRARY`, `~/.config/carina/config.toml`, default `~/Papers`) #feature
- [ ] Storage: create reference directories, copy PDFs, write `info.toml` #feature
- [ ] Import: `carina add <file>` with PDF metadata extraction #feature

## Next

- [ ] Metadata fetch: arXiv API lookup by arXiv ID, CrossRef lookup by DOI #feature
- [ ] SQLite index: schema, indexing from filesystem, `carina reindex` #feature
- [ ] Search: FTS5 across title, authors, abstract #feature
- [ ] List/filter: `carina list`, filter by tag/year #feature
- [ ] Open: `carina open` with `$CARINA_READER` or `open` fallback #feature
- [ ] Edit: `carina edit` opens `info.toml` in `$EDITOR` #feature

## Later

- [ ] BibTeX export: single paper, selection, full library #feature
- [ ] Copy citation: BibTeX and APA formatted output #feature
- [ ] PDF full-text indexing into FTS5 #feature
- [ ] arXiv auto-detection: recognize arXiv URLs/IDs in `carina add` #improvement
- [ ] URL import: `carina add <url>` downloads PDF first #feature
- [ ] fzf integration: interactive picker for open/edit/export #improvement
- [ ] iCloud sync: library root in iCloud Drive #feature
- [ ] Shell completions: fish, bash, zsh #chore

## Scrapped

## Done
