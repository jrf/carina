use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::metadata;
use crate::storage;

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(library: &Path) -> Result<Self> {
        let db_path = library.join(".carina.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open index at {}", db_path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS reference (
                dir_name TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                authors TEXT NOT NULL DEFAULT '',
                year INTEGER,
                doi TEXT,
                arxiv TEXT,
                journal TEXT,
                tags TEXT NOT NULL DEFAULT '',
                abstract_text TEXT,
                files TEXT NOT NULL DEFAULT ''
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS reference_fts USING fts5(
                title, authors, abstract_text, tags,
                content='reference',
                content_rowid='rowid'
            );

            CREATE TRIGGER IF NOT EXISTS reference_ai AFTER INSERT ON reference BEGIN
                INSERT INTO reference_fts(rowid, title, authors, abstract_text, tags)
                VALUES (new.rowid, new.title, new.authors, new.abstract_text, new.tags);
            END;

            CREATE TRIGGER IF NOT EXISTS reference_ad AFTER DELETE ON reference BEGIN
                INSERT INTO reference_fts(reference_fts, rowid, title, authors, abstract_text, tags)
                VALUES ('delete', old.rowid, old.title, old.authors, old.abstract_text, old.tags);
            END;

            CREATE TRIGGER IF NOT EXISTS reference_au AFTER UPDATE ON reference BEGIN
                INSERT INTO reference_fts(reference_fts, rowid, title, authors, abstract_text, tags)
                VALUES ('delete', old.rowid, old.title, old.authors, old.abstract_text, old.tags);
                INSERT INTO reference_fts(rowid, title, authors, abstract_text, tags)
                VALUES (new.rowid, new.title, new.authors, new.abstract_text, new.tags);
            END;"
        )?;

        Ok(Self { conn })
    }

    pub fn reindex(&self, library: &Path) -> Result<usize> {
        self.conn.execute("DELETE FROM reference", [])?;

        let dirs = storage::list_ref_dirs(library)?;
        let mut count = 0;

        for dir in &dirs {
            let reference = match metadata::read_info(dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let dir_name = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
            self.upsert(&dir_name, &reference)?;
            count += 1;
        }

        Ok(count)
    }

    pub fn upsert(&self, dir_name: &str, r: &crate::model::Reference) -> Result<()> {
        self.conn.execute(
            "INSERT INTO reference (dir_name, title, authors, year, doi, arxiv, journal, tags, abstract_text, files)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(dir_name) DO UPDATE SET
                title=?2, authors=?3, year=?4, doi=?5, arxiv=?6, journal=?7, tags=?8, abstract_text=?9, files=?10",
            params![
                dir_name,
                r.title,
                r.authors.join("; "),
                r.year,
                r.doi,
                r.arxiv,
                r.journal,
                r.tags.join(", "),
                r.r#abstract,
                r.files.join(", "),
            ],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let fts_query = query
            .split_whitespace()
            .map(|w| format!("\"{}\"", w.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" ");

        let mut stmt = self.conn.prepare(
            "SELECT r.dir_name, r.title, r.authors, r.year,
                    snippet(reference_fts, 0, '{{', '}}', '...', 20) as snippet
             FROM reference_fts fts
             JOIN reference r ON r.rowid = fts.rowid
             WHERE reference_fts MATCH ?1
             ORDER BY rank
             LIMIT 50"
        )?;

        let hits = stmt.query_map(params![fts_query], |row| {
            Ok(SearchHit {
                dir_name: row.get(0)?,
                title: row.get(1)?,
                authors: row.get(2)?,
                year: row.get(3)?,
                snippet: row.get(4)?,
            })
        })?;

        let mut results = Vec::new();
        for hit in hits {
            results.push(hit?);
        }
        Ok(results)
    }

}

#[allow(dead_code)]
pub struct SearchHit {
    pub dir_name: String,
    pub title: String,
    pub authors: String,
    pub year: Option<u16>,
    #[allow(dead_code)]
    pub snippet: String,
}
