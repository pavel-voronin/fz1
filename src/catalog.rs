use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyLine {
    DisplayName(String),
    Text(String),
    Template(usize),
    Command(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub value: String,
    pub label: String,
    pub raw_line: String,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub filename: String,
    /// Optional `= ` override shown alongside the filename in the left pane.
    pub display_name: Option<String>,
    /// Static description lines (non-`> cmd` lines), joined by `\n`.
    pub description: String,
    /// Original body layout, preserving interleaving of blank lines and directives.
    pub body_lines: Vec<BodyLine>,
    /// Templates declared by `@ ` lines inside the file body.
    pub templates: Vec<Template>,
    /// Commands from `> cmd` lines, with `"> "` prefix stripped.
    pub enrich_commands: Vec<String>,
    /// One slot per enrich_command. Starts empty; filled by enrichment threads.
    pub enriched_output: Vec<String>,
    /// Readable status for failed enrichment commands, shown inline with `> cmd`.
    pub enriched_status: Vec<Option<String>>,
    pub category: String, // e.g. "file/manager"
    pub path: PathBuf,
}

pub fn load_catalog(root: &Path) -> std::io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    collect_entries(root, root, &mut entries)?;
    Ok(entries)
}

fn collect_entries(root: &Path, dir: &Path, out: &mut Vec<Entry>) -> std::io::Result<()> {
    let mut children: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|e| e.path());
    for child in children {
        let path = child.path();
        if path.is_dir() {
            collect_entries(root, &path, out)?;
        } else if path.is_file() {
            if let Ok(entry) = parse_entry(&path, root) {
                out.push(entry);
            }
        }
    }
    Ok(())
}

pub fn parse_entry(path: &Path, root: &Path) -> std::io::Result<Entry> {
    let content = std::fs::read_to_string(path)?;
    let lines = content.lines();
    let mut description_lines: Vec<String> = Vec::new();
    let mut body_lines: Vec<BodyLine> = Vec::new();
    let mut templates: Vec<Template> = Vec::new();
    let mut enrich_commands: Vec<String> = Vec::new();
    let mut display_name = None;
    for line in lines {
        if let Some(cmd) = line.strip_prefix("> ") {
            body_lines.push(BodyLine::Command(enrich_commands.len()));
            enrich_commands.push(cmd.to_string());
        } else if let Some(name) = parse_display_name_line(line) {
            description_lines.push(line.to_string());
            body_lines.push(BodyLine::DisplayName(line.to_string()));
            display_name = Some(name);
        } else if let Some(template) = parse_template_line(line) {
            description_lines.push(template.raw_line.clone());
            body_lines.push(BodyLine::Template(templates.len()));
            templates.push(template);
        } else {
            description_lines.push(line.to_string());
            body_lines.push(BodyLine::Text(line.to_string()));
        }
    }
    let enriched_output = vec![String::new(); enrich_commands.len()];
    let enriched_status = vec![None; enrich_commands.len()];
    let description = description_lines.join("\n");
    let parent = path.parent().unwrap_or(root);
    let rel = parent.strip_prefix(root).unwrap_or(Path::new(""));
    let category = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let filename = path.file_name().unwrap().to_string_lossy().into_owned();
    Ok(Entry {
        filename,
        display_name,
        description,
        body_lines,
        templates,
        enrich_commands,
        enriched_output,
        enriched_status,
        category,
        path: path.to_path_buf(),
    })
}

fn parse_template_line(line: &str) -> Option<Template> {
    let (label, value) = if let Some(named_rest) = line.strip_prefix("@(") {
        if let Some(close_idx) = named_rest.find(") ") {
            let label = named_rest[..close_idx].trim();
            let value = named_rest[close_idx + 2..].trim();
            if !label.is_empty() && !value.is_empty() {
                (label.to_string(), value.to_string())
            } else {
                let value = line.strip_prefix("@(").unwrap_or(line).trim();
                if value.is_empty() {
                    return None;
                }
                (value.to_string(), value.to_string())
            }
        } else {
            let value = line.strip_prefix("@(").unwrap_or(line).trim();
            if value.is_empty() {
                return None;
            }
            (value.to_string(), value.to_string())
        }
    } else if let Some(rest) = line.strip_prefix("@ ") {
        let value = rest.trim();
        if value.is_empty() {
            return None;
        }
        (value.to_string(), value.to_string())
    } else if let Some(named_rest) = line.strip_prefix('@') {
        let named_rest = named_rest.trim_end();
        let split_idx = named_rest.find(char::is_whitespace)?;
        let label = named_rest[..split_idx].trim();
        let value = named_rest[split_idx..].trim();
        if label.is_empty() || value.is_empty() {
            return None;
        }
        (label.to_string(), value.to_string())
    } else {
        return None;
    };

    Some(Template {
        value,
        label,
        raw_line: line.to_string(),
    })
}

fn parse_display_name_line(line: &str) -> Option<String> {
    let name = line.strip_prefix("= ")?.trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_catalog() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("file/manager")).unwrap();
        fs::create_dir_all(root.join("network/http")).unwrap();
        fs::write(
            root.join("file/manager/mc"),
            "= Midnight Commander\ndual panel file manager\n> mc -h\n",
        )
        .unwrap();
        fs::write(
            root.join("file/manager/yazi"),
            "blazing fast file manager\n",
        )
        .unwrap();
        fs::write(
            root.join("network/http/curl"),
            "= curl\ntransfer data with URLs\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn test_load_count() {
        let dir = make_catalog();
        let entries = load_catalog(dir.path()).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_entry_fields() {
        let dir = make_catalog();
        let entries = load_catalog(dir.path()).unwrap();
        let mc = entries.iter().find(|e| e.filename == "mc").unwrap();
        assert_eq!(mc.display_name.as_deref(), Some("Midnight Commander"));
        assert_eq!(
            mc.description,
            "= Midnight Commander\ndual panel file manager"
        );
        assert_eq!(
            mc.body_lines,
            vec![
                BodyLine::DisplayName("= Midnight Commander".to_string()),
                BodyLine::Text("dual panel file manager".to_string()),
                BodyLine::Command(0)
            ]
        );
        assert!(mc.templates.is_empty());
        assert_eq!(mc.enrich_commands, vec!["mc -h"]);
        assert_eq!(mc.enriched_output.len(), 1);
        assert_eq!(mc.enriched_output[0], "");
        assert_eq!(mc.enriched_status, vec![None]);
        assert_eq!(mc.category, "file/manager");
    }

    #[test]
    fn test_no_enrich_commands() {
        let dir = make_catalog();
        let entries = load_catalog(dir.path()).unwrap();
        let yazi = entries.iter().find(|e| e.filename == "yazi").unwrap();
        assert_eq!(yazi.display_name, None);
        assert_eq!(yazi.description, "blazing fast file manager");
        assert!(yazi.enrich_commands.is_empty());
        assert!(yazi.enriched_output.is_empty());
        assert!(yazi.enriched_status.is_empty());
        assert!(yazi.templates.is_empty());
    }

    #[test]
    fn preserves_blank_line_before_enrichment_commands() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join("tool"), "first line\n\n> tool --help\n").unwrap();

        let entry = parse_entry(&root.join("tool"), root).unwrap();

        assert_eq!(entry.description, "first line\n");
        assert_eq!(
            entry.body_lines,
            vec![
                BodyLine::Text("first line".to_string()),
                BodyLine::Text(String::new()),
                BodyLine::Command(0)
            ]
        );
    }

    #[test]
    fn parses_templates_and_keeps_raw_lines_in_description() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(
            root.join("tool"),
            "@ cargo test\n@(Watch mode) cargo watch -x test\n@fast cargo test -- --watch\n@ (Literal) cargo test\n",
        )
        .unwrap();

        let entry = parse_entry(&root.join("tool"), root).unwrap();

        assert_eq!(
            entry.description,
            "@ cargo test\n@(Watch mode) cargo watch -x test\n@fast cargo test -- --watch\n@ (Literal) cargo test"
        );
        assert_eq!(
            entry.body_lines,
            vec![
                BodyLine::Template(0),
                BodyLine::Template(1),
                BodyLine::Template(2),
                BodyLine::Template(3)
            ]
        );
        assert_eq!(
            entry.templates,
            vec![
                Template {
                    value: "cargo test".to_string(),
                    label: "cargo test".to_string(),
                    raw_line: "@ cargo test".to_string()
                },
                Template {
                    value: "cargo watch -x test".to_string(),
                    label: "Watch mode".to_string(),
                    raw_line: "@(Watch mode) cargo watch -x test".to_string()
                },
                Template {
                    value: "cargo test -- --watch".to_string(),
                    label: "fast".to_string(),
                    raw_line: "@fast cargo test -- --watch".to_string()
                },
                Template {
                    value: "(Literal) cargo test".to_string(),
                    label: "(Literal) cargo test".to_string(),
                    raw_line: "@ (Literal) cargo test".to_string()
                }
            ]
        );
    }

    #[test]
    fn parses_display_name_directive_and_keeps_raw_line_in_description() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join("tool"), "= Tool Name\nplain text\n").unwrap();

        let entry = parse_entry(&root.join("tool"), root).unwrap();

        assert_eq!(entry.display_name.as_deref(), Some("Tool Name"));
        assert_eq!(entry.description, "= Tool Name\nplain text");
        assert_eq!(
            entry.body_lines,
            vec![
                BodyLine::DisplayName("= Tool Name".to_string()),
                BodyLine::Text("plain text".to_string())
            ]
        );
    }

    #[test]
    fn test_entries_sorted_by_path() {
        let dir = make_catalog();
        let entries = load_catalog(dir.path()).unwrap();
        // file/manager/mc < file/manager/yazi < network/http/curl
        assert_eq!(entries[0].filename, "mc");
        assert_eq!(entries[1].filename, "yazi");
        assert_eq!(entries[2].filename, "curl");
    }
}
