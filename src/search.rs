use crate::catalog::Entry;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::ops::Range;

#[derive(Debug, Clone)]
pub struct ParsedQuery {
    pub pattern: String,
}

pub fn parse_query(input: &str) -> ParsedQuery {
    ParsedQuery {
        pattern: input.to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry_index: usize,
    pub score: i64,
    /// Character indices within the combined haystack string used for matching.
    pub highlight_indices: Vec<usize>,
}

pub struct SearchEngine {
    matcher: SkimMatcherV2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchLayout {
    pub filename: Option<Range<usize>>,
    pub display_name: Option<Range<usize>>,
    pub description: Option<Range<usize>>,
    pub enriched_output: Vec<Option<Range<usize>>>,
}

pub struct HaystackLayout {
    pub haystack: String,
    pub ranges: MatchLayout,
}

const FILE_MATCH_BOOST: i64 = 10_000;

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            matcher: SkimMatcherV2::default(),
        }
    }

    /// Returns entries ranked highest score first.
    /// Empty pattern returns all entries in catalog order.
    pub fn search(&self, entries: &[Entry], query: &ParsedQuery) -> Vec<SearchResult> {
        if query.pattern.is_empty() {
            return (0..entries.len())
                .map(|i| SearchResult {
                    entry_index: i,
                    score: 0,
                    highlight_indices: vec![],
                })
                .collect();
        }

        let mut results: Vec<SearchResult> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let layout = Self::build_layout(entry);
                self.matcher
                    .fuzzy_indices(&layout.haystack, &query.pattern)
                    .map(|(score, indices)| {
                        let score = score + self.filename_match_boost(entry, &query.pattern);
                        let highlight_indices = Self::normalize_highlight_indices(
                            entry,
                            &layout.ranges,
                            &query.pattern,
                            &indices,
                        );
                        SearchResult {
                            entry_index: i,
                            score,
                            highlight_indices,
                        }
                    })
            })
            .collect();

        results.sort_by(|a, b| {
            b.score.cmp(&a.score).then_with(|| {
                let ka = format!(
                    "{}/{}",
                    entries[a.entry_index].category, entries[a.entry_index].filename
                );
                let kb = format!(
                    "{}/{}",
                    entries[b.entry_index].category, entries[b.entry_index].filename
                );
                ka.cmp(&kb)
            })
        });
        results
    }

    pub fn build_layout(entry: &Entry) -> HaystackLayout {
        let mut haystack = String::new();
        let mut next_index = 0usize;
        let mut push_part = |text: &str, slot: &mut Option<Range<usize>>| {
            if !haystack.is_empty() {
                haystack.push(' ');
                next_index += 1;
            }
            let start = next_index;
            haystack.push_str(text);
            next_index += text.chars().count();
            *slot = Some(start..next_index);
        };

        let mut filename = None;
        let mut display_name = None;
        let mut description = None;
        let mut enriched_output = vec![None; entry.enriched_output.len()];

        push_part(&entry.filename, &mut filename);
        if let Some(name) = entry.display_name.as_deref() {
            push_part(name, &mut display_name);
        }
        push_part(&entry.description, &mut description);
        for (slot, output) in enriched_output.iter_mut().zip(&entry.enriched_output) {
            if !output.is_empty() {
                push_part(output, slot);
            }
        }

        HaystackLayout {
            haystack,
            ranges: MatchLayout {
                filename,
                display_name,
                description,
                enriched_output,
            },
        }
    }

    pub fn build_layout_for_result(entry: &Entry) -> MatchLayout {
        Self::build_layout(entry).ranges
    }

    pub fn highlight_indices_for_line(
        indices: &[usize],
        range: Option<&Range<usize>>,
        line_offset: usize,
        line: &str,
    ) -> Vec<usize> {
        let mut relative = slice_highlight_indices(indices, range);
        let line_len = line.chars().count();
        relative.retain(|idx| *idx >= line_offset && *idx < line_offset + line_len);
        relative.into_iter().map(|idx| idx - line_offset).collect()
    }

    #[cfg(test)]
    pub fn direct_match_indices(text: &str, pattern: &str) -> Vec<usize> {
        if pattern.is_empty() {
            return Vec::new();
        }
        SkimMatcherV2::default()
            .fuzzy_indices(text, pattern)
            .map(|(_, indices)| indices)
            .unwrap_or_default()
    }

    fn filename_match_boost(&self, entry: &Entry, pattern: &str) -> i64 {
        let name_haystack = match entry.display_name.as_deref() {
            Some(name) => format!("{} {}", entry.filename, name),
            None => entry.filename.clone(),
        };
        if self.matcher.fuzzy_match(&name_haystack, pattern).is_some() {
            FILE_MATCH_BOOST
        } else {
            0
        }
    }

    fn normalize_highlight_indices(
        entry: &Entry,
        layout: &MatchLayout,
        pattern: &str,
        fuzzy_indices: &[usize],
    ) -> Vec<usize> {
        let direct = Self::first_direct_match_indices(entry, layout, pattern);
        if direct.is_empty() {
            fuzzy_indices.to_vec()
        } else {
            direct
        }
    }

    fn first_direct_match_indices(
        entry: &Entry,
        layout: &MatchLayout,
        pattern: &str,
    ) -> Vec<usize> {
        if pattern.is_empty() {
            return Vec::new();
        }

        if let Some(indices) =
            Self::range_adjusted_direct_match(layout.filename.as_ref(), &entry.filename, pattern)
        {
            return indices;
        }
        if let Some(display_name) = entry.display_name.as_deref() {
            if let Some(indices) = Self::range_adjusted_direct_match(
                layout.display_name.as_ref(),
                display_name,
                pattern,
            ) {
                return indices;
            }
        }
        if let Some(indices) = Self::range_adjusted_direct_match(
            layout.description.as_ref(),
            &entry.description,
            pattern,
        ) {
            return indices;
        }
        for (range, output) in layout.enriched_output.iter().zip(&entry.enriched_output) {
            if let Some(indices) =
                Self::range_adjusted_direct_match(range.as_ref(), output, pattern)
            {
                return indices;
            }
        }
        Vec::new()
    }

    fn range_adjusted_direct_match(
        range: Option<&Range<usize>>,
        text: &str,
        pattern: &str,
    ) -> Option<Vec<usize>> {
        let Some(range) = range else {
            return None;
        };
        let indices = Self::first_direct_substring_indices(text, pattern)?;
        Some(indices.into_iter().map(|idx| range.start + idx).collect())
    }

    fn first_direct_substring_indices(text: &str, pattern: &str) -> Option<Vec<usize>> {
        if pattern.is_empty() {
            return Some(Vec::new());
        }

        let text_chars: Vec<char> = text.chars().collect();
        let pattern_chars: Vec<char> = pattern.chars().collect();
        if pattern_chars.len() > text_chars.len() {
            return None;
        }

        for start in 0..=text_chars.len() - pattern_chars.len() {
            if text_chars[start..start + pattern_chars.len()]
                .iter()
                .zip(pattern_chars.iter())
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
            {
                return Some((start..start + pattern_chars.len()).collect());
            }
        }

        None
    }
}

pub fn slice_highlight_indices(indices: &[usize], range: Option<&Range<usize>>) -> Vec<usize> {
    let Some(range) = range else {
        return Vec::new();
    };
    indices
        .iter()
        .copied()
        .filter(|idx| range.contains(idx))
        .map(|idx| idx - range.start)
        .collect()
}

#[cfg(test)]
mod engine_tests {
    use super::*;
    use crate::catalog::Entry;
    use std::path::PathBuf;

    fn entry(filename: &str, display: &str, desc: &str, category: &str) -> Entry {
        Entry {
            filename: filename.to_string(),
            display_name: Some(display.to_string()),
            description: desc.to_string(),
            body_lines: desc
                .split('\n')
                .map(|line| crate::catalog::BodyLine::Text(line.to_string()))
                .collect(),
            templates: vec![],
            enrich_commands: vec![],
            enriched_output: vec![],
            enriched_status: vec![],
            category: category.to_string(),
            path: PathBuf::from(format!("{}/{}", category, filename)),
        }
    }

    #[test]
    fn empty_pattern_returns_all() {
        let entries = vec![
            entry("mc", "Midnight Commander", "file manager", "file"),
            entry("curl", "curl", "http client", "network"),
        ];
        let engine = SearchEngine::new();
        let q = parse_query("");
        assert_eq!(engine.search(&entries, &q).len(), 2);
    }

    #[test]
    fn returns_matching_entry() {
        let entries = vec![
            entry("lazygit", "LazyGit", "terminal UI for git", "dev/git"),
            entry("curl", "curl", "transfer data", "network"),
        ];
        let engine = SearchEngine::new();
        let q = parse_query("git");
        let results = engine.search(&entries, &q);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry_index, 0);
    }

    #[test]
    fn no_match_returns_empty() {
        let entries = vec![entry("curl", "curl", "transfer data", "network")];
        let engine = SearchEngine::new();
        let q = parse_query("zznotfound");
        assert!(engine.search(&entries, &q).is_empty());
    }

    #[test]
    fn filename_matches_are_ranked_above_description_only_matches() {
        let entries = vec![
            entry("git-tool", "Git Tool", "version control", "dev"),
            entry("curl", "curl", "transfers from git servers", "network"),
        ];
        let engine = SearchEngine::new();
        let q = parse_query("git");
        let results = engine.search(&entries, &q);
        assert_eq!(results[0].entry_index, 0);
    }

    #[test]
    fn enrichment_output_is_searchable() {
        let mut e = entry("tool", "Tool", "basic desc", "cat");
        e.enriched_output = vec!["extra searchable text".to_string()];
        e.enrich_commands = vec!["tool --help".to_string()];
        let entries = vec![e];
        let engine = SearchEngine::new();
        let q = parse_query("searchable");
        assert_eq!(engine.search(&entries, &q).len(), 1);
    }

    #[test]
    fn enrich_command_lines_are_not_searchable() {
        let mut e = entry("tool", "Tool", "basic desc", "cat");
        e.enrich_commands = vec!["tool --help".to_string()];
        e.enriched_output = vec![String::new()];
        let entries = vec![e];
        let engine = SearchEngine::new();
        let q = parse_query("tool --help");
        assert!(engine.search(&entries, &q).is_empty());
    }

    #[test]
    fn layout_tracks_field_ranges_in_combined_haystack() {
        let mut e = entry("tool", "Tool Name", "basic desc", "cat");
        e.enriched_output = vec!["extra text".to_string()];

        let layout = SearchEngine::build_layout(&e);

        assert_eq!(layout.haystack, "tool Tool Name basic desc extra text");
        assert_eq!(layout.ranges.filename, Some(0..4));
        assert_eq!(layout.ranges.display_name, Some(5..14));
        assert_eq!(layout.ranges.description, Some(15..25));
        assert_eq!(layout.ranges.enriched_output, vec![Some(26..36)]);
    }

    #[test]
    fn layout_skips_display_name_when_no_override_exists() {
        let mut e = entry("tool", "Tool Name", "basic desc", "cat");
        e.display_name = None;

        let layout = SearchEngine::build_layout(&e);

        assert_eq!(layout.haystack, "tool basic desc");
        assert_eq!(layout.ranges.filename, Some(0..4));
        assert_eq!(layout.ranges.display_name, None);
        assert_eq!(layout.ranges.description, Some(5..15));
    }

    #[test]
    fn highlight_indices_are_sliced_relative_to_field_range() {
        let indices = vec![4, 6, 10, 31];
        let relative = slice_highlight_indices(&indices, Some(&(4..8)));
        assert_eq!(relative, vec![0, 2]);
    }

    #[test]
    fn direct_match_indices_cover_display_name() {
        assert_eq!(
            SearchEngine::direct_match_indices("Echo arguments", "argume"),
            vec![5, 6, 7, 8, 9, 10]
        );
    }

    #[test]
    fn direct_matches_highlight_only_first_winning_occurrence() {
        let mut e = entry("cp", "Copy files", "cp copies", "file");
        e.enriched_output = vec!["cp copies".to_string()];
        let q = parse_query("cp");
        let layout = SearchEngine::build_layout(&e);
        let indices =
            SearchEngine::normalize_highlight_indices(&e, &layout.ranges, &q.pattern, &[]);

        assert_eq!(
            slice_highlight_indices(&indices, layout.ranges.filename.as_ref()),
            vec![0, 1]
        );
        assert_eq!(
            slice_highlight_indices(&indices, layout.ranges.description.as_ref()),
            Vec::<usize>::new()
        );
        assert_eq!(
            slice_highlight_indices(&indices, layout.ranges.enriched_output[0].as_ref()),
            Vec::<usize>::new()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_is_used_as_literal_input() {
        let q = parse_query("fd/git");
        assert_eq!(q.pattern, "fd/git");
    }
}
