use super::*;
use crossterm::event::KeyModifiers;
use std::collections::HashSet;

#[derive(Default)]
pub(in crate::app) enum AgentsOverviewSelection {
    #[default]
    Inactive,
    Active(HashSet<ThreadId>),
}

impl AgentsOverviewSelection {
    pub(in crate::app) fn remove(&mut self, thread_id: &ThreadId) {
        if let Self::Active(marked) = self {
            marked.remove(thread_id);
        }
    }
}

impl AgentsOverviewView {
    fn selection_key_available(&self, key: KeyEvent) -> bool {
        self.keymap.action_for(key).is_none()
            && [
                &self.agents_keymap.resume,
                &self.agents_keymap.search,
                &self.agents_keymap.new_task,
                &self.agents_keymap.rename,
                &self.agents_keymap.stop,
                &self.agents_keymap.archive,
                &self.agents_keymap.delete,
                &self.agents_keymap.hide,
                &self.agents_keymap.toggle_grouping,
                &self.agents_keymap.select_mode,
            ]
            .into_iter()
            .all(|bindings| !bindings.is_pressed(key))
    }

    pub(super) fn handle_selection_key(&mut self, key: KeyEvent) -> bool {
        if self.state().connection_notice.is_some() {
            return false;
        }
        let active = matches!(self.state().selection, AgentsOverviewSelection::Active(_));
        if active
            && !self.state().editing_metadata()
            && self.keymap.action_for(key) == Some(ListAction::Cancel)
        {
            self.state().selection = AgentsOverviewSelection::Inactive;
            return true;
        }
        if !self.state().editing_metadata() && self.agents_keymap.select_mode.is_pressed(key) {
            self.state().selection = if active {
                AgentsOverviewSelection::Inactive
            } else {
                AgentsOverviewSelection::Active(HashSet::new())
            };
            return true;
        }
        if !active {
            return false;
        }
        let visible = self.visible_indices();
        if self.agents_keymap.delete.is_pressed(key) {
            let state = self.state();
            let AgentsOverviewSelection::Active(marked) = &state.selection else {
                unreachable!()
            };
            let thread_ids: Vec<_> = visible
                .iter()
                .map(|index| self.rows[*index].thread_id)
                .filter(|id| marked.contains(id))
                .collect();
            if !thread_ids.is_empty() && thread_ids.len() == marked.len() {
                self.app_event_tx
                    .send(AppEvent::ConfirmAgentsOverviewDelete { thread_ids });
            }
            return true;
        }
        if self.state().editing_metadata() || !self.selection_key_available(key) {
            return false;
        }
        if key.code == KeyCode::Char(' ') && key.modifiers.is_empty() {
            if let Some(row) = self.selected_row()
                && let AgentsOverviewSelection::Active(marked) = &mut self.state().selection
                && !marked.insert(row.thread_id)
            {
                marked.remove(&row.thread_id);
            }
            return true;
        }
        if key.code == KeyCode::Char('a') && key.modifiers == KeyModifiers::CONTROL {
            if let AgentsOverviewSelection::Active(marked) = &mut self.state().selection {
                marked.extend(visible.iter().map(|index| self.rows[*index].thread_id));
            }
            return true;
        }
        false
    }

    pub(super) fn selection_marker(&self, thread_id: ThreadId) -> Span<'static> {
        match &self.state().selection {
            AgentsOverviewSelection::Inactive => "".into(),
            AgentsOverviewSelection::Active(marked) if marked.contains(&thread_id) => "[x] ".cyan(),
            AgentsOverviewSelection::Active(_) => "[ ] ".dim(),
        }
    }

    pub(super) fn selection_hints(&self) -> Vec<Line<'static>> {
        let visible = self.visible_indices();
        let state = self.state();
        match &state.selection {
            AgentsOverviewSelection::Inactive => self
                .agents_keymap
                .primary_hint("select_mode", &self.agents_keymap.select_mode)
                .map(|hint| {
                    vec![
                        format!("{} select tasks", hint.display_label())
                            .dim()
                            .into(),
                    ]
                })
                .unwrap_or_default(),
            AgentsOverviewSelection::Active(marked) => {
                let hidden = marked.len()
                    - visible
                        .iter()
                        .filter(|index| marked.contains(&self.rows[**index].thread_id))
                        .count();
                let mut hints = vec![
                    format!(
                        "{} selected · {hidden} hidden{}",
                        marked.len(),
                        if hidden > 0 {
                            " · show hidden selections or clear before deleting"
                        } else if marked.is_empty() {
                            " · mark tasks to delete"
                        } else {
                            ""
                        }
                    )
                    .cyan()
                    .into(),
                ];
                if !state.editing_metadata() {
                    if self.selection_key_available(KeyCode::Char(' ').into()) {
                        hints.push("space toggle selection".dim().into());
                    }
                    if self.selection_key_available(KeyEvent::new(
                        KeyCode::Char('a'),
                        KeyModifiers::CONTROL,
                    )) {
                        hints.push("ctrl+a select all visible".dim().into());
                    }
                }
                hints
            }
        }
    }
}
