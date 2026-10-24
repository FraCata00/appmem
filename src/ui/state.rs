//! Which rows are visible, which one is selected, and which groups are open.

use std::collections::HashSet;

use crate::attribution::Bucket;
use crate::grouping::Group;
use crate::model::ProcInfo;

/// Processes shown under an expanded group before the "… N more" row.
pub const EXPANDED_LIMIT: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowId {
    Group(Bucket),
    Proc(Bucket, i32),
}

pub enum Row<'a> {
    Group(&'a Group, bool),
    Proc(&'a Group, &'a ProcInfo),
    More { group: &'a Group, count: usize, total: u64 },
}

impl Row<'_> {
    fn id(&self) -> Option<RowId> {
        match self {
            Row::Group(g, _) => Some(RowId::Group(g.key.clone())),
            Row::Proc(g, p) => Some(RowId::Proc(g.key.clone(), p.pid)),
            Row::More { .. } => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct UiState {
    pub expanded: HashSet<Bucket>,
    pub selected: Option<RowId>,
}

impl UiState {
    pub fn rows<'a>(&self, groups: &'a [Group]) -> Vec<Row<'a>> {
        let mut rows = Vec::new();
        for group in groups {
            let open = self.expanded.contains(&group.key);
            rows.push(Row::Group(group, open));
            if !open {
                continue;
            }
            rows.extend(group.procs.iter().take(EXPANDED_LIMIT).map(|p| Row::Proc(group, p)));
            let hidden = &group.procs[group.procs.len().min(EXPANDED_LIMIT)..];
            if !hidden.is_empty() {
                let total = hidden.iter().filter_map(|p| p.footprint).sum();
                rows.push(Row::More {
                    group,
                    count: hidden.len(),
                    total,
                });
            }
        }
        rows
    }

    /// Index of the selected row in `rows(groups)`.
    pub fn selected_index(&self, groups: &[Group]) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.rows(groups).iter().position(|r| r.id().as_ref() == Some(selected))
    }

    /// Moves by `delta` selectable rows, skipping "… N more", clamped at both ends.
    pub fn move_selection(&mut self, groups: &[Group], delta: i32) {
        let ids: Vec<RowId> = self.rows(groups).iter().filter_map(Row::id).collect();
        if ids.is_empty() {
            self.selected = None;
            return;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|s| ids.iter().position(|id| id == s))
            .unwrap_or(0);
        let target = (current as i64 + i64::from(delta)).clamp(0, ids.len() as i64 - 1) as usize;
        self.selected = Some(ids[target].clone());
    }

    pub fn toggle_expand(&mut self) {
        match self.selected.clone() {
            Some(RowId::Group(key)) => {
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key);
                }
            }
            Some(RowId::Proc(key, _)) => {
                self.expanded.remove(&key);
                self.selected = Some(RowId::Group(key));
            }
            None => {}
        }
    }

    /// Keeps the selection meaningful after a refresh.
    pub fn reconcile(&mut self, groups: &[Group]) {
        let first = groups.first().map(|g| RowId::Group(g.key.clone()));
        self.selected = match self.selected.take() {
            Some(RowId::Group(key)) if groups.iter().any(|g| g.key == key) => Some(RowId::Group(key)),
            Some(RowId::Proc(key, pid)) => match groups.iter().find(|g| g.key == key) {
                Some(g)
                    if self.expanded.contains(&key) && g.procs.iter().take(EXPANDED_LIMIT).any(|p| p.pid == pid) =>
                {
                    Some(RowId::Proc(key, pid))
                }
                Some(_) => Some(RowId::Group(key)),
                None => first,
            },
            _ => first,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProcInfo;

    fn app(name: &str) -> Bucket {
        Bucket::App(format!("/Applications/{name}.app").into())
    }

    fn group(name: &str, pids: &[i32]) -> Group {
        let procs = pids
            .iter()
            .map(|&pid| ProcInfo {
                pid,
                ppid: 1,
                uid: 501,
                name: format!("p{pid}"),
                path: None,
                footprint: Some(1),
                responsible_pid: None,
            })
            .collect();
        Group {
            key: app(name),
            name: name.into(),
            bundle_id: None,
            procs,
            total: 1,
            estimated: false,
        }
    }

    fn ids(state: &UiState, groups: &[Group]) -> Vec<String> {
        state
            .rows(groups)
            .iter()
            .map(|r| match r {
                Row::Group(g, _) => g.name.clone(),
                Row::Proc(_, p) => p.pid.to_string(),
                Row::More { count, .. } => format!("+{count}"),
            })
            .collect()
    }

    #[test]
    fn collapsed_shows_one_row_per_group() {
        let groups = vec![group("Chrome", &[1, 2]), group("Xcode", &[3])];
        assert_eq!(ids(&UiState::default(), &groups), vec!["Chrome", "Xcode"]);
    }

    #[test]
    fn expanded_shows_top_six_and_more() {
        let groups = vec![group("Chrome", &[1, 2, 3, 4, 5, 6, 7, 8, 9]), group("Xcode", &[10])];
        let mut state = UiState::default();
        state.reconcile(&groups);
        state.toggle_expand();
        assert_eq!(
            ids(&state, &groups),
            vec!["Chrome", "1", "2", "3", "4", "5", "6", "+3", "Xcode"]
        );
    }

    #[test]
    fn move_skips_more_row() {
        let groups = vec![group("Chrome", &[1, 2, 3, 4, 5, 6, 7]), group("Xcode", &[10])];
        let mut state = UiState::default();
        state.reconcile(&groups);
        state.toggle_expand();
        state.move_selection(&groups, 6);
        assert_eq!(state.selected, Some(RowId::Proc(app("Chrome"), 6)));
        state.move_selection(&groups, 1);
        assert_eq!(state.selected, Some(RowId::Group(app("Xcode"))));
        state.move_selection(&groups, -1);
        assert_eq!(state.selected, Some(RowId::Proc(app("Chrome"), 6)));
    }

    #[test]
    fn move_clamps_at_ends() {
        let groups = vec![group("Chrome", &[1]), group("Xcode", &[2])];
        let mut state = UiState::default();
        state.reconcile(&groups);
        state.move_selection(&groups, -1);
        assert_eq!(state.selected, Some(RowId::Group(app("Chrome"))));
        state.move_selection(&groups, 5);
        assert_eq!(state.selected, Some(RowId::Group(app("Xcode"))));
    }

    #[test]
    fn selection_follows_app_when_order_changes() {
        let mut state = UiState::default();
        let before = vec![group("Chrome", &[1]), group("Xcode", &[2])];
        state.reconcile(&before);
        let after = vec![group("Xcode", &[2]), group("Chrome", &[1])];
        state.reconcile(&after);
        assert_eq!(state.selected, Some(RowId::Group(app("Chrome"))));
        assert_eq!(state.selected_index(&after), Some(1));
    }

    #[test]
    fn dead_process_selects_its_group() {
        let mut state = UiState::default();
        let before = vec![group("Chrome", &[1, 2])];
        state.reconcile(&before);
        state.toggle_expand();
        state.move_selection(&before, 2);
        assert_eq!(state.selected, Some(RowId::Proc(app("Chrome"), 2)));
        state.reconcile(&[group("Chrome", &[1])]);
        assert_eq!(state.selected, Some(RowId::Group(app("Chrome"))));
    }

    #[test]
    fn vanished_group_selects_first() {
        let mut state = UiState::default();
        let before = vec![group("Chrome", &[1]), group("Xcode", &[2])];
        state.reconcile(&before);
        state.move_selection(&before, 1);
        state.reconcile(&[group("Chrome", &[1])]);
        assert_eq!(state.selected, Some(RowId::Group(app("Chrome"))));
    }

    #[test]
    fn toggle_on_proc_collapses_group() {
        let groups = vec![group("Chrome", &[1, 2])];
        let mut state = UiState::default();
        state.reconcile(&groups);
        state.toggle_expand();
        state.move_selection(&groups, 1);
        state.toggle_expand();
        assert_eq!(state.selected, Some(RowId::Group(app("Chrome"))));
        assert_eq!(ids(&state, &groups), vec!["Chrome"]);
    }
}
