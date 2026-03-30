use crate::catalog::Entry;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct TreeItem {
    pub indent: usize,
    pub ancestor_has_next_sibling: Vec<bool>,
    pub has_next_sibling: bool,
    pub kind: TreeItemKind,
}

#[derive(Debug, Clone)]
pub enum TreeItemKind {
    Category {
        path: String, // full path e.g. "file/manager"
        name: String, // last segment e.g. "manager"
        collapsed: bool,
    },
    Entry {
        entry_index: usize,
    },
}

pub struct TreeState {
    pub collapsed: HashSet<String>,
    pub cursor: usize,
    pub selected_entry_index: Option<usize>,
}

impl TreeState {
    pub fn new(entries: &[Entry]) -> Self {
        let mut s = Self {
            collapsed: HashSet::new(),
            cursor: 0,
            selected_entry_index: None,
        };
        s.update_selected(entries);
        s
    }

    pub fn visible_items(&self, entries: &[Entry]) -> Vec<TreeItem> {
        let mut out = Vec::new();
        self.build_level(entries, "", 0, &[], &mut out);
        out
    }

    fn build_level(
        &self,
        entries: &[Entry],
        prefix: &str,
        indent: usize,
        ancestor_has_next_sibling: &[bool],
        out: &mut Vec<TreeItem>,
    ) {
        // Collect immediate child category segments at this level (in order of first appearance)
        let mut child_cats: Vec<String> = Vec::new();
        let mut direct_entries: Vec<usize> = Vec::new();

        for (i, entry) in entries.iter().enumerate() {
            let cat = &entry.category;
            if prefix.is_empty() {
                if cat.is_empty() {
                    direct_entries.push(i);
                } else {
                    let seg = cat.split('/').next().unwrap_or("");
                    if !child_cats.iter().any(|c| c == seg) {
                        child_cats.push(seg.to_string());
                    }
                }
            } else {
                if cat == prefix {
                    direct_entries.push(i);
                } else if cat.starts_with(&format!("{}/", prefix)) {
                    let rest = &cat[prefix.len() + 1..];
                    let seg = rest.split('/').next().unwrap_or("");
                    let full = format!("{}/{}", prefix, seg);
                    if !child_cats.contains(&full) {
                        child_cats.push(full);
                    }
                }
            }
        }

        let total_items = child_cats.len() + direct_entries.len();
        for (index, cat_path) in child_cats.iter().enumerate() {
            let name = cat_path.split('/').last().unwrap_or(cat_path).to_string();
            let collapsed = self.collapsed.contains(cat_path);
            let has_next_sibling = index + 1 < total_items;
            out.push(TreeItem {
                indent,
                ancestor_has_next_sibling: ancestor_has_next_sibling.to_vec(),
                has_next_sibling,
                kind: TreeItemKind::Category {
                    path: cat_path.clone(),
                    name,
                    collapsed,
                },
            });
            if !collapsed {
                let mut child_ancestor_has_next_sibling = ancestor_has_next_sibling.to_vec();
                child_ancestor_has_next_sibling.push(has_next_sibling);
                self.build_level(
                    entries,
                    cat_path,
                    indent + 1,
                    &child_ancestor_has_next_sibling,
                    out,
                );
            }
        }

        for (index, &i) in direct_entries.iter().enumerate() {
            let has_next_sibling = child_cats.len() + index + 1 < total_items;
            out.push(TreeItem {
                indent,
                ancestor_has_next_sibling: ancestor_has_next_sibling.to_vec(),
                has_next_sibling,
                kind: TreeItemKind::Entry { entry_index: i },
            });
        }
    }

    pub fn move_down(&mut self, entries: &[Entry]) {
        let len = self.visible_items(entries).len();
        if self.cursor + 1 < len {
            self.cursor += 1;
        }
        self.update_selected(entries);
    }

    pub fn move_up(&mut self, entries: &[Entry]) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        self.update_selected(entries);
    }

    pub fn move_left(&mut self, entries: &[Entry]) {
        let items = self.visible_items(entries);
        if items.is_empty() {
            return;
        }
        let item = items[self.cursor].clone();
        match item.kind {
            TreeItemKind::Category {
                path,
                collapsed: false,
                ..
            } => {
                self.collapsed.insert(path);
            }
            TreeItemKind::Category {
                path,
                collapsed: true,
                ..
            } => {
                if let Some(slash) = path.rfind('/') {
                    let parent = path[..slash].to_string();
                    for (i, it) in items.iter().enumerate() {
                        if let TreeItemKind::Category { path: p, .. } = &it.kind {
                            if *p == parent {
                                self.cursor = i;
                                break;
                            }
                        }
                    }
                }
            }
            TreeItemKind::Entry { entry_index } => {
                let cat = entries[entry_index].category.clone();
                if !cat.is_empty() {
                    for (i, it) in items.iter().enumerate() {
                        if let TreeItemKind::Category { path, .. } = &it.kind {
                            if *path == cat {
                                self.cursor = i;
                                break;
                            }
                        }
                    }
                }
            }
        }
        self.update_selected(entries);
    }

    pub fn move_right(&mut self, entries: &[Entry]) {
        let items = self.visible_items(entries);
        if items.is_empty() {
            return;
        }
        let item = items[self.cursor].clone();
        match item.kind {
            TreeItemKind::Category {
                path,
                collapsed: true,
                ..
            } => {
                self.collapsed.remove(&path);
            }
            TreeItemKind::Category {
                path,
                collapsed: false,
                ..
            } => {
                let target_indent = item.indent + 1;
                for (i, it) in items.iter().enumerate().skip(self.cursor + 1) {
                    if it.indent <= item.indent {
                        break;
                    }
                    if it.indent == target_indent {
                        match &it.kind {
                            TreeItemKind::Category {
                                path: child_path, ..
                            } if Self::is_direct_child(&path, child_path) => {
                                self.cursor = i;
                                break;
                            }
                            TreeItemKind::Entry { entry_index }
                                if entries[*entry_index].category == path =>
                            {
                                self.cursor = i;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
            TreeItemKind::Entry { .. } => {}
        }
        self.update_selected(entries);
    }

    pub fn toggle_collapse(&mut self, entries: &[Entry]) {
        let items = self.visible_items(entries);
        if let Some(item) = items.get(self.cursor).cloned() {
            if let TreeItemKind::Category {
                path, collapsed, ..
            } = item.kind
            {
                if collapsed {
                    self.collapsed.remove(&path);
                } else {
                    self.collapsed.insert(path);
                }
            }
        }
        // clamp cursor in case it's now past end
        let len = self.visible_items(entries).len();
        if self.cursor >= len {
            self.cursor = len.saturating_sub(1);
        }
        self.update_selected(entries);
    }

    /// Expand ancestors and move cursor to entry_index.
    pub fn focus_entry(&mut self, entry_index: usize, entries: &[Entry]) {
        let cat = entries[entry_index].category.clone();
        // Expand all ancestor categories
        let parts: Vec<&str> = cat.split('/').collect();
        for i in 0..parts.len() {
            self.collapsed.remove(&parts[..=i].join("/"));
        }
        let items = self.visible_items(entries);
        for (i, item) in items.iter().enumerate() {
            if let TreeItemKind::Entry { entry_index: ei } = item.kind {
                if ei == entry_index {
                    self.cursor = i;
                    break;
                }
            }
        }
        self.update_selected(entries);
    }

    fn update_selected(&mut self, entries: &[Entry]) {
        let items = self.visible_items(entries);
        self.selected_entry_index = items.get(self.cursor).and_then(|item| {
            if let TreeItemKind::Entry { entry_index } = item.kind {
                Some(entry_index)
            } else {
                None
            }
        });
    }

    fn parent_category(path: &str) -> Option<String> {
        path.rfind('/').map(|slash| path[..slash].to_string())
    }

    fn is_direct_child(parent: &str, candidate: &str) -> bool {
        Self::parent_category(candidate).as_deref() == Some(parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn e(filename: &str, category: &str) -> Entry {
        Entry {
            filename: filename.to_string(),
            display_name: None,
            description: String::new(),
            body_lines: vec![],
            templates: vec![],
            enrich_commands: vec![],
            enriched_output: vec![],
            enriched_status: vec![],
            category: category.to_string(),
            path: PathBuf::from(format!("{}/{}", category, filename)),
        }
    }

    fn catalog() -> Vec<Entry> {
        vec![
            e("mc", "file/manager"),
            e("yazi", "file/manager"),
            e("curl", "network/http"),
        ]
    }

    #[test]
    fn all_expanded_shows_7_items() {
        // file, file/manager, mc, yazi, network, network/http, curl
        let entries = catalog();
        let state = TreeState::new(&entries);
        assert_eq!(state.visible_items(&entries).len(), 7);
    }

    #[test]
    fn collapse_leaf_category_hides_entries() {
        let entries = catalog();
        let mut state = TreeState::new(&entries);
        state.collapsed.insert("file/manager".to_string());
        // file, file/manager[+], network, network/http, curl = 5
        assert_eq!(state.visible_items(&entries).len(), 5);
    }

    #[test]
    fn collapse_parent_hides_subtree() {
        let entries = catalog();
        let mut state = TreeState::new(&entries);
        state.collapsed.insert("file".to_string());
        // file[+], network, network/http, curl = 4
        assert_eq!(state.visible_items(&entries).len(), 4);
    }

    #[test]
    fn move_down_advances_cursor() {
        let entries = catalog();
        let mut state = TreeState::new(&entries);
        state.cursor = 0;
        state.move_down(&entries);
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn move_down_clamps_at_end() {
        let entries = catalog();
        let mut state = TreeState::new(&entries);
        let len = state.visible_items(&entries).len();
        state.cursor = len - 1;
        state.move_down(&entries);
        assert_eq!(state.cursor, len - 1);
    }

    #[test]
    fn focus_entry_expands_ancestors_and_places_cursor() {
        let entries = catalog();
        let mut state = TreeState::new(&entries);
        state.collapsed.insert("network".to_string());
        state.focus_entry(2, &entries); // curl is index 2
        let items = state.visible_items(&entries);
        assert!(matches!(
            items[state.cursor].kind,
            TreeItemKind::Entry { entry_index: 2 }
        ));
    }

    #[test]
    fn move_right_expands_collapsed_category() {
        let entries = catalog();
        let mut state = TreeState::new(&entries);
        state.collapsed.insert("file".to_string());
        state.move_right(&entries);
        assert!(!state.collapsed.contains("file"));
    }

    #[test]
    fn move_right_enters_expanded_category() {
        let entries = catalog();
        let mut state = TreeState::new(&entries);
        state.cursor = 0; // file
        state.move_right(&entries);
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn move_left_from_entry_goes_to_containing_category() {
        let entries = catalog();
        let mut state = TreeState::new(&entries);
        state.focus_entry(0, &entries); // mc in file/manager
        state.move_left(&entries);
        let items = state.visible_items(&entries);
        match &items[state.cursor].kind {
            TreeItemKind::Category { path, .. } => assert_eq!(path, "file/manager"),
            _ => panic!("expected category"),
        }
    }

    #[test]
    fn visible_items_track_tree_guides() {
        let entries = catalog();
        let state = TreeState::new(&entries);
        let items = state.visible_items(&entries);

        assert_eq!(items[0].ancestor_has_next_sibling, Vec::<bool>::new());
        assert!(items[0].has_next_sibling);

        assert_eq!(items[1].ancestor_has_next_sibling, vec![true]);
        assert!(!items[1].has_next_sibling);

        assert_eq!(items[2].ancestor_has_next_sibling, vec![true, false]);
        assert!(items[2].has_next_sibling);

        assert_eq!(items[3].ancestor_has_next_sibling, vec![true, false]);
        assert!(!items[3].has_next_sibling);

        assert_eq!(items[4].ancestor_has_next_sibling, Vec::<bool>::new());
        assert!(!items[4].has_next_sibling);
    }
}
