use std::io;
use std::path::Path;

const ROOT_ENTRY_FILENAME: &str = "git";
const ROOT_ENTRY_CONTENT: &str = "= Git\nversion control system for tracking code changes\n> git --help\n@ git status\n@(History graph) git log --oneline --decorate --graph\n";

const CATEGORY_NAME: &str = "network";
const CATEGORY_ENTRY_FILENAME: &str = "curl";
const CATEGORY_ENTRY_CONTENT: &str =
    "= curl\ntransfer data to and from servers using URL syntax\n> curl --help\n@ curl\n";

pub fn ensure_demo_catalog(root: &Path) -> io::Result<()> {
    std::fs::create_dir_all(root)?;
    std::fs::write(root.join(ROOT_ENTRY_FILENAME), ROOT_ENTRY_CONTENT)?;

    let category_dir = root.join(CATEGORY_NAME);
    std::fs::create_dir_all(&category_dir)?;
    std::fs::write(
        category_dir.join(CATEGORY_ENTRY_FILENAME),
        CATEGORY_ENTRY_CONTENT,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::load_catalog;
    use tempfile::TempDir;

    #[test]
    fn creates_demo_catalog_with_root_and_nested_entries() {
        let dir = TempDir::new().unwrap();
        ensure_demo_catalog(dir.path()).unwrap();

        let entries = load_catalog(dir.path()).unwrap();
        assert_eq!(entries.len(), 2);

        let git = entries
            .iter()
            .find(|entry| entry.filename == "git")
            .unwrap();
        assert_eq!(git.category, "");
        assert_eq!(git.display_name.as_deref(), Some("Git"));
        assert_eq!(git.templates.len(), 2);
        assert_eq!(git.templates[0].value, "git status");
        assert_eq!(git.templates[1].label, "History graph");
        assert_eq!(git.enrich_commands, vec!["git --help"]);

        let curl = entries
            .iter()
            .find(|entry| entry.filename == "curl")
            .unwrap();
        assert_eq!(curl.category, "network");
        assert_eq!(curl.display_name.as_deref(), Some("curl"));
        assert_eq!(curl.templates.len(), 1);
        assert_eq!(curl.templates[0].value, "curl");
        assert_eq!(curl.enrich_commands, vec!["curl --help"]);
    }
}
