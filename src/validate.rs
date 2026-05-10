use std::path::Path;

use anyhow::Result;

use crate::model::Reference;
use crate::storage;

#[derive(Debug)]
enum Issue {
    MissingPdf { dir: String },
    FileListed { dir: String, file: String, problem: &'static str },
    TempName { dir: String, file: String },
    NoMetadata { dir: String, field: &'static str },
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Issue::MissingPdf { dir } => write!(f, "  no PDF: {}", dir),
            Issue::FileListed { dir, file, problem } => {
                write!(f, "  {}: {}/{}", problem, dir, file)
            }
            Issue::TempName { dir, file } => write!(f, "  temp name: {}/{}", dir, file),
            Issue::NoMetadata { dir, field } => write!(f, "  missing {}: {}", field, dir),
        }
    }
}

pub struct ValidateResult {
    pub total: usize,
    pub fixed: usize,
    pub issues: Vec<String>,
}

impl ValidateResult {
    pub fn summary(&self) -> String {
        if self.issues.is_empty() && self.fixed == 0 {
            format!("Library OK — {} references", self.total)
        } else if self.fixed > 0 && self.issues.is_empty() {
            format!("Fixed {} issues", self.fixed)
        } else if self.fixed > 0 {
            format!("{} issues, fixed {}", self.issues.len() + self.fixed, self.fixed)
        } else {
            format!("{} issues found", self.issues.len())
        }
    }
}

pub fn validate(library: &Path, fix: bool) -> Result<ValidateResult> {
    let dirs = storage::list_ref_dirs(library)?;
    let total = dirs.len();
    let mut issues: Vec<Issue> = Vec::new();
    let mut fixed = 0u32;

    for ref_dir in &dirs {
        let dir_name = ref_dir.file_name().unwrap_or_default().to_string_lossy().to_string();

        let toml_path = ref_dir.join("info.toml");
        let content = std::fs::read_to_string(&toml_path)?;
        let reference: Reference = toml::from_str(&content)?;

        if reference.title.is_empty() {
            issues.push(Issue::NoMetadata { dir: dir_name.clone(), field: "title" });
        }
        if reference.authors.is_empty() {
            issues.push(Issue::NoMetadata { dir: dir_name.clone(), field: "authors" });
        }
        if reference.year.is_none() {
            issues.push(Issue::NoMetadata { dir: dir_name.clone(), field: "year" });
        }

        for listed_file in &reference.files {
            let file_path = ref_dir.join(listed_file);
            if !file_path.exists() {
                issues.push(Issue::FileListed {
                    dir: dir_name.clone(),
                    file: listed_file.clone(),
                    problem: "listed but missing",
                });
            } else if !is_pdf(&file_path) {
                if fix {
                    std::fs::remove_file(&file_path)?;
                    let new_content = content.replace(
                        &format!("\"{}\"", listed_file),
                        "",
                    );
                    let new_content = new_content.replace("files = [\n    \n]", "files = []");
                    std::fs::write(&toml_path, &new_content)?;
                    fixed += 1;
                } else {
                    issues.push(Issue::FileListed {
                        dir: dir_name.clone(),
                        file: listed_file.clone(),
                        problem: "not a PDF",
                    });
                }
            }
        }

        let has_pdf = std::fs::read_dir(ref_dir)?
            .filter_map(|e| e.ok())
            .any(|e| {
                let p = e.path();
                p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
                    && is_pdf(&p)
            });

        if !has_pdf && reference.files.is_empty() {
            issues.push(Issue::MissingPdf { dir: dir_name.clone() });
        }

        for entry in std::fs::read_dir(ref_dir)?.filter_map(|e| e.ok()) {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.starts_with("tmp") && fname != "info.toml" {
                if fix && fname.ends_with(".pdf") && is_pdf(&entry.path()) {
                    let proper = format!("{}.pdf", dir_name);
                    let new_path = ref_dir.join(&proper);
                    std::fs::rename(entry.path(), &new_path)?;
                    let new_content = content.replace(
                        &format!("\"{}\"", fname),
                        &format!("\"{}\"", proper),
                    );
                    std::fs::write(&toml_path, &new_content)?;
                    fixed += 1;
                } else {
                    issues.push(Issue::TempName { dir: dir_name.clone(), file: fname });
                }
            }
        }
    }

    let issue_strings: Vec<String> = issues.iter().map(|i| i.to_string()).collect();

    Ok(ValidateResult {
        total,
        fixed: fixed as usize,
        issues: issue_strings,
    })
}

pub fn run(library: &Path, fix: bool) -> Result<()> {
    let result = validate(library, fix)?;
    println!("{}", result.summary());
    Ok(())
}

fn is_pdf(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut buf = [0u8; 4];
    use std::io::Read;
    if file.read_exact(&mut buf).is_err() {
        return false;
    }
    &buf == b"%PDF"
}
