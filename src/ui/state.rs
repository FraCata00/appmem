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

impl RowId {
    /// The group this row belongs to.
    pub fn group_key(&self) -> &Bucket {
        match self {
            RowId::Group(key) | RowId::Proc(key, _) => key,
        }
    }
}

pub enum Row<'a> {
    Group(&'a Group, bool),
    Proc(&'a Group, &'a ProcInfo),
    More { count: usize, total: u64 },
}

impl Row<'_> {
    fn id(&self) -> Option<RowId> {
        match self {
            Row::Group(g, _) => Some(RowId::Group(g.key.clone())),
            Row::Proc(g, p) => Some(RowId::Proc(g.key.clone(), p.pid)),
            Row::More { .. } => None,
        }
    }

    /// Whether this row is `id`, without cloning keys.
    pub fn is(&self, id: &RowId) -> bool {
        match (self, id) {
            (Row::Group(g, _), RowId::Group(key)) => &g.key == key,
            (Row::Proc(g, p), RowId::Proc(key, pid)) => &g.key == key && p.pid == *pid,
            _ => false,
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
                    count: hidden.len(),
                    total,
                });
            }
        }
        rows
    }

    /// Index of the selected row in `rows`.
    pub fn selected_in(&self, rows: &[Row]) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        rows.iter().position(|r| r.is(selected))
    }

    /// Index of the selected row in `rows(groups)`.
    pub fn selected_index(&self, groups: &[Group]) -> Option<usize> {
        self.selected_in(&self.rows(groups))
    }

    /// The group the selection is in.
    pub fn selected_group<'a>(&self, groups: &'a [Group]) -> Option<&'a Group> {
        let key = self.selected.as_ref()?.group_key();
        groups.iter().find(|g| &g.key == key)
    }

    /// Moves by `delta` selectable rows, skipping "… N more", clamped at both ends.
    pub fn move_selection(&mut self, groups: &[Group], delta: i32) {
        let rows = self.rows(groups);
        let selectable: Vec<&Row> = rows.iter().filter(|r| !matches!(r, Row::More { .. })).collect();
        if selectable.is_empty() {
            self.selected = None;
            return;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|s| selectable.iter().position(|r| r.is(s)))
            .unwrap_or(0);
        let target = (current as i64 + i64::from(delta)).clamp(0, selectable.len() as i64 - 1) as usize;
        self.selected = selectable[target].id();
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

    /// Keeps the selection meaningful after a refresh: a row that is still
    /// visible stays selected, a process that went away selects its app.
    pub fn reconcile(&mut self, groups: &[Group]) {
        if self.selected_index(groups).is_some() {
            return;
        }
        self.selected = match self.selected.take() {
            Some(RowId::Proc(key, _)) if groups.iter().any(|g| g.key == key) => Some(RowId::Group(key)),
            _ => groups.first().map(|g| RowId::Group(g.key.clone())),
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
