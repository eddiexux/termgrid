use crate::git::GitContext;

#[cfg(test)]
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct TabEntry {
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabFilter {
    All,
    Project(String),
    Other,
}

impl TabFilter {
    pub fn matches(&self, git_context: &Option<GitContext>) -> bool {
        match self {
            TabFilter::All => true,
            TabFilter::Project(name) => git_context
                .as_ref()
                .is_some_and(|g| g.project_name == *name),
            TabFilter::Other => git_context.is_none(),
        }
    }
}

/// Aggregate tiles by project name.
///
/// - Counts tiles per project name (from GitContext)
/// - Counts non-git tiles as "Other"
/// - Sorts by count descending, then name ascending
/// - Appends "Other" at the end if count > 0
/// - Does NOT include "ALL" — caller prepends that
pub fn aggregate_tabs(contexts: &[Option<GitContext>]) -> Vec<TabEntry> {
    use std::collections::HashMap;

    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut other_count = 0;

    for ctx in contexts {
        match ctx {
            Some(git_ctx) => {
                *counts.entry(git_ctx.project_name.clone()).or_insert(0) += 1;
            }
            None => {
                other_count += 1;
            }
        }
    }

    // Sort projects by count descending, then name ascending
    let mut entries: Vec<(String, usize)> = counts.into_iter().collect();
    entries.sort_by(|a, b| match b.1.cmp(&a.1) {
        std::cmp::Ordering::Equal => a.0.cmp(&b.0),
        other => other,
    });

    let mut result: Vec<TabEntry> = entries
        .into_iter()
        .map(|(label, count)| TabEntry { label, count })
        .collect();

    // Append "Other" at the end if count > 0
    if other_count > 0 {
        result.push(TabEntry {
            label: "Other".to_string(),
            count: other_count,
        });
    }

    result
}

/// Cycle to the next tab.
///
/// Cycle: ALL → first project → ... → Other → ALL
/// Maps "Other" label to TabFilter::Other
pub fn next_tab(current: &TabFilter, tabs: &[TabEntry]) -> TabFilter {
    match current {
        TabFilter::All => {
            // Move to first project if available, else Other if available, else ALL
            if let Some(first) = tabs.first() {
                if first.label == "Other" {
                    // No projects, only Other
                    if tabs.len() > 1 {
                        // There are projects before Other (shouldn't happen with our sort order)
                        TabFilter::Project(first.label.clone())
                    } else {
                        TabFilter::Other
                    }
                } else {
                    TabFilter::Project(first.label.clone())
                }
            } else {
                // No tabs, stay at ALL
                TabFilter::All
            }
        }
        TabFilter::Project(name) => {
            // Find current project in tabs and move to next
            if let Some(pos) = tabs.iter().position(|t| t.label == *name) {
                if pos + 1 < tabs.len() {
                    let next_label = &tabs[pos + 1].label;
                    if next_label == "Other" {
                        TabFilter::Other
                    } else {
                        TabFilter::Project(next_label.clone())
                    }
                } else {
                    // At last tab, cycle back to ALL
                    TabFilter::All
                }
            } else {
                // Current project not found, go to ALL
                TabFilter::All
            }
        }
        TabFilter::Other => {
            // Cycle back to ALL
            TabFilter::All
        }
    }
}

/// Cycle to the previous tab.
///
/// Reverse of next_tab.
pub fn prev_tab(current: &TabFilter, tabs: &[TabEntry]) -> TabFilter {
    match current {
        TabFilter::All => {
            // Move to last tab (Other if present, else last project)
            if let Some(last) = tabs.last() {
                if last.label == "Other" {
                    TabFilter::Other
                } else {
                    TabFilter::Project(last.label.clone())
                }
            } else {
                // No tabs, stay at ALL
                TabFilter::All
            }
        }
        TabFilter::Project(name) => {
            // Find current project and move to previous
            if let Some(pos) = tabs.iter().position(|t| t.label == *name) {
                if pos > 0 {
                    let prev_label = &tabs[pos - 1].label;
                    if prev_label == "Other" {
                        TabFilter::Other
                    } else {
                        TabFilter::Project(prev_label.clone())
                    }
                } else {
                    // At first tab, cycle back to ALL
                    TabFilter::All
                }
            } else {
                // Current project not found, go to ALL
                TabFilter::All
            }
        }
        TabFilter::Other => {
            // Move to last project if available, else ALL
            if let Some(last) = tabs.iter().rev().find(|t| t.label != "Other") {
                TabFilter::Project(last.label.clone())
            } else {
                TabFilter::All
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_ctx(name: &str) -> Option<GitContext> {
        Some(GitContext {
            project_name: name.into(),
            branch: Some("main".into()),
            is_worktree: false,
            worktree_name: None,
            repo_root: PathBuf::from("/tmp"),
        })
    }

    #[test]
    fn test_aggregate_tabs() {
        let contexts = vec![
            git_ctx("alpha"),
            git_ctx("beta"),
            git_ctx("alpha"),
            None,
            git_ctx("alpha"),
        ];

        let result = aggregate_tabs(&contexts);

        assert_eq!(
            result,
            vec![
                TabEntry {
                    label: "alpha".to_string(),
                    count: 3
                },
                TabEntry {
                    label: "beta".to_string(),
                    count: 1
                },
                TabEntry {
                    label: "Other".to_string(),
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn test_aggregate_empty() {
        let contexts: Vec<Option<GitContext>> = vec![];
        let result = aggregate_tabs(&contexts);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_aggregate_all_non_git() {
        let contexts: Vec<Option<GitContext>> = vec![None, None];
        let result = aggregate_tabs(&contexts);
        assert_eq!(
            result,
            vec![TabEntry {
                label: "Other".to_string(),
                count: 2
            }]
        );
    }

    #[test]
    fn test_filter_matches_all() {
        let filter = TabFilter::All;
        assert!(filter.matches(&git_ctx("any")));
        assert!(filter.matches(&None));
    }

    #[test]
    fn test_filter_matches_project() {
        let filter = TabFilter::Project("alpha".to_string());
        assert!(filter.matches(&git_ctx("alpha")));
        assert!(!filter.matches(&git_ctx("beta")));
        assert!(!filter.matches(&None));
    }

    #[test]
    fn test_filter_matches_other() {
        let filter = TabFilter::Other;
        assert!(filter.matches(&None));
        assert!(!filter.matches(&git_ctx("any")));
    }

    #[test]
    fn test_tab_cycling_forward() {
        let tabs = vec![
            TabEntry {
                label: "alpha".to_string(),
                count: 3,
            },
            TabEntry {
                label: "beta".to_string(),
                count: 1,
            },
            TabEntry {
                label: "Other".to_string(),
                count: 1,
            },
        ];

        // ALL → alpha
        let tab = next_tab(&TabFilter::All, &tabs);
        assert_eq!(tab, TabFilter::Project("alpha".to_string()));

        // alpha → beta
        let tab = next_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::Project("beta".to_string()));

        // beta → Other
        let tab = next_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::Other);

        // Other → ALL
        let tab = next_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::All);
    }

    #[test]
    fn test_tab_cycling_backward() {
        let tabs = vec![
            TabEntry {
                label: "alpha".to_string(),
                count: 3,
            },
            TabEntry {
                label: "beta".to_string(),
                count: 1,
            },
            TabEntry {
                label: "Other".to_string(),
                count: 1,
            },
        ];

        // ALL → Other
        let tab = prev_tab(&TabFilter::All, &tabs);
        assert_eq!(tab, TabFilter::Other);

        // Other → beta
        let tab = prev_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::Project("beta".to_string()));

        // beta → alpha
        let tab = prev_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::Project("alpha".to_string()));

        // alpha → ALL
        let tab = prev_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::All);
    }

    #[test]
    fn test_prev_tab_at_first_project() {
        let tabs = vec![
            TabEntry {
                label: "alpha".to_string(),
                count: 3,
            },
            TabEntry {
                label: "beta".to_string(),
                count: 1,
            },
        ];

        // alpha → ALL
        let tab = prev_tab(&TabFilter::Project("alpha".to_string()), &tabs);
        assert_eq!(tab, TabFilter::All);
    }

    #[test]
    fn test_next_tab_empty_tabs() {
        let tabs: Vec<TabEntry> = vec![];

        // ALL stays ALL when no tabs
        let tab = next_tab(&TabFilter::All, &tabs);
        assert_eq!(tab, TabFilter::All);
    }

    #[test]
    fn test_prev_tab_empty_tabs() {
        let tabs: Vec<TabEntry> = vec![];

        // ALL stays ALL when no tabs
        let tab = prev_tab(&TabFilter::All, &tabs);
        assert_eq!(tab, TabFilter::All);
    }

    #[test]
    fn test_aggregate_sorting_multiple_projects() {
        // Test sorting: higher count first, then alphabetical for ties
        let contexts = vec![
            git_ctx("zebra"),
            git_ctx("alpha"),
            git_ctx("zebra"),
            git_ctx("beta"),
            git_ctx("alpha"),
            git_ctx("alpha"),
        ];

        let result = aggregate_tabs(&contexts);

        assert_eq!(
            result,
            vec![
                TabEntry {
                    label: "alpha".to_string(),
                    count: 3
                },
                TabEntry {
                    label: "zebra".to_string(),
                    count: 2
                },
                TabEntry {
                    label: "beta".to_string(),
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn test_next_tab_only_other() {
        let tabs = vec![TabEntry {
            label: "Other".to_string(),
            count: 2,
        }];

        // ALL → Other
        let tab = next_tab(&TabFilter::All, &tabs);
        assert_eq!(tab, TabFilter::Other);

        // Other → ALL
        let tab = next_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::All);
    }

    #[test]
    fn test_prev_tab_only_other() {
        let tabs = vec![TabEntry {
            label: "Other".to_string(),
            count: 2,
        }];

        // ALL → Other
        let tab = prev_tab(&TabFilter::All, &tabs);
        assert_eq!(tab, TabFilter::Other);

        // Other → ALL
        let tab = prev_tab(&tab, &tabs);
        assert_eq!(tab, TabFilter::All);
    }
}
