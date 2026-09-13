use super::*;

impl App {
    pub(in crate::app) fn confirm_agents_overview_delete(&mut self, thread_ids: Vec<ThreadId>) {
        let targets: Option<Vec<_>> = thread_ids
            .into_iter()
            .map(|id| {
                let thread = self.agents_overview.threads.get(&id)?.as_ref()?;
                let name = thread
                    .name
                    .as_deref()
                    .unwrap_or(&thread.preview)
                    .trim()
                    .lines()
                    .next()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Untitled task");
                Some((id, name.to_string()))
            })
            .collect();
        let Some(targets) = targets.filter(|targets| !targets.is_empty()) else {
            return;
        };
        let task_label = if targets.len() == 1 { "task" } else { "tasks" };
        let mut header = vec![
            format!("Permanently delete {} {task_label}?", targets.len()).bold().into(),
            "This stops any running work in these tasks and their child agents, then permanently deletes their history. This cannot be undone.".dim().into(),
        ];
        header.extend(targets.iter().map(|(id, name)| {
            if targets
                .iter()
                .any(|(other_id, other_name)| other_id != id && other_name == name)
            {
                format!("• {name} [{id}]").into()
            } else {
                format!("• {name}").into()
            }
        }));
        self.chat_widget.show_selection_view(SelectionViewParams {
            header: Box::new(LifecycleHeader(header)),
            items: vec![
                SelectionItem {
                    name: "Cancel".into(),
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Permanently delete selected tasks and child agents".into(),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::RunAgentsOverviewDelete {
                            targets: targets.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    require_explicit_confirmation: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
    }

    pub(in crate::app) async fn run_agents_overview_delete(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        targets: Vec<(ThreadId, String)>,
    ) -> color_eyre::Result<()> {
        if self.windows_sandbox_blocks_thread_switch() || targets.is_empty() {
            return Ok(());
        }
        let task_label = if targets.len() == 1 { "task" } else { "tasks" };
        let progress = LifecycleHeader(vec![
            format!("Deleting {} selected {task_label}…", targets.len())
                .bold()
                .into(),
            "Please wait. Task switching is unavailable until the batch finishes."
                .dim()
                .into(),
        ]);
        let events = tui.event_stream();
        let mut primary = self.primary_thread_id;
        let operation = async {
            let mut outcomes = Vec::new();
            for (id, name) in targets {
                let outcome =
                    perform_lifecycle_action(app_server, primary, id, AgentsOverviewAction::Delete)
                        .await;
                if outcome.detach_primary {
                    primary = None;
                }
                self.apply_agents_overview_outcome(id, &outcome);
                outcomes.push((format!("{name} [{id}]"), outcome));
            }
            outcomes
        };
        let outcomes = run_lifecycle_modal(tui, progress, operation, events).await?;
        let attempted = outcomes.iter().any(|(_, outcome)| outcome.attempted);
        let detach_primary = outcomes.iter().any(|(_, outcome)| outcome.detach_primary);
        self.finish_agents_overview_action(tui, app_server, attempted, detach_primary)?;
        let deleted = outcomes
            .iter()
            .filter(|(_, outcome)| outcome.result.is_ok())
            .count();
        let failures: Vec<_> = outcomes
            .into_iter()
            .filter_map(|(name, outcome)| {
                outcome
                    .result
                    .err()
                    .map(|error| format!("{name}: {error:#}"))
            })
            .collect();
        if !failures.is_empty() {
            let mut header = vec![
                format!("{deleted} deleted, {} failed", failures.len())
                    .red()
                    .bold()
                    .into(),
            ];
            header.extend(failures.into_iter().map(|failure| failure.red().into()));
            header.push("Work may have stopped. Failed tasks remain selected. Review them and confirm again to retry.".dim().into());
            self.chat_widget.show_selection_view(SelectionViewParams {
                header: Box::new(LifecycleHeader(header)),
                items: vec![SelectionItem {
                    name: "Back to agents".into(),
                    dismiss_on_select: true,
                    ..Default::default()
                }],
                ..Default::default()
            });
        }
        Ok(())
    }
}
