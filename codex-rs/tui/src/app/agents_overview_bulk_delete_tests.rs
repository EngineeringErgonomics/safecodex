use super::*;
use crate::app::agents_overview_view::AgentsOverviewSelection;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn selection_tracks_identity_filters_and_visible_scope() {
    let (mut app, mut rx, _op_rx) = crate::app::tests::make_test_app_with_channels().await;
    let [alpha, beta] = [
        ThreadId::from_u128(/*value*/ 1),
        ThreadId::from_u128(/*value*/ 2),
    ];
    let threads = vec![
        overview_thread(
            alpha,
            /*parent_thread_id*/ None,
            "Alpha",
            ThreadStatus::Idle,
        ),
        overview_thread(
            beta,
            /*parent_thread_id*/ None,
            "Beta",
            ThreadStatus::Idle,
        ),
    ];
    let mut view = app.agents_overview_view(threads.clone(), Some(alpha));
    view.handle_key_event(KeyCode::Esc.into());
    view.handle_key_event(KeyCode::Insert.into());
    view.handle_key_event(KeyCode::Delete.into());
    assert!(rx.try_recv().is_err());
    view.handle_key_event(KeyCode::Char(' ').into());
    let mut reordered = threads;
    reordered.reverse();
    view = app.agents_overview_view(reordered, Some(beta));
    view.handle_key_event(KeyCode::Delete.into());
    assert!(
        matches!(rx.try_recv().unwrap(), AppEvent::ConfirmAgentsOverviewDelete { thread_ids } if thread_ids == vec![alpha])
    );
    view.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
    for character in "Beta".chars() {
        view.handle_key_event(KeyCode::Char(character).into());
    }
    view.handle_key_event(KeyCode::Delete.into());
    assert!(rx.try_recv().is_err());
    app.chat_widget.show_bottom_pane_view(Box::new(view));
    for width in [80, 36] {
        insta::assert_snapshot!(
            format!("bulk_selection_hidden_{width}"),
            render_bottom_popup(&app.chat_widget, width)
        );
    }
    app.chat_widget.handle_key_event(KeyCode::Esc.into());
    assert!(matches!(
        app.agents_overview.view_state.lock().unwrap().selection,
        AgentsOverviewSelection::Active(_)
    ));
    app.chat_widget.handle_key_event(KeyCode::Esc.into());
    app.chat_widget.handle_key_event(KeyCode::Insert.into());
    app.chat_widget
        .handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    app.chat_widget.handle_key_event(KeyCode::Delete.into());
    let AppEvent::ConfirmAgentsOverviewDelete { thread_ids } = rx.try_recv().unwrap() else {
        panic!("batch confirmation")
    };
    assert_eq!(
        thread_ids.into_iter().collect::<HashSet<_>>(),
        HashSet::from([alpha, beta])
    );
    for width in [80, 36] {
        insta::assert_snapshot!(
            format!("bulk_selection_{width}"),
            render_bottom_popup(&app.chat_widget, width)
        );
    }
}

#[tokio::test]
async fn selection_preserves_composer_metadata_and_custom_key_ownership() {
    let (mut app, mut rx, _op_rx) = crate::app::tests::make_test_app_with_channels().await;
    let id = ThreadId::new();
    let threads = vec![overview_thread(
        id,
        /*parent_thread_id*/ None,
        "Task",
        ThreadStatus::Idle,
    )];
    let mut keymap = TuiKeymap::default();
    keymap.agents.select_mode = Some(KeybindingsSpec::One(KeybindingSpec("f8".into())));
    keymap.agents.rename = Some(KeybindingsSpec::One(KeybindingSpec("insert".into())));
    keymap.agents.delete = Some(KeybindingsSpec::One(KeybindingSpec("f6".into())));
    keymap.agents.stop = Some(KeybindingsSpec::One(KeybindingSpec("ctrl-a".into())));
    keymap.list.cancel = Some(KeybindingsSpec::One(KeybindingSpec("f7".into())));
    app.keymap = RuntimeKeymap::from_config(&keymap).unwrap();
    let mut view = app.agents_overview_view(threads, Some(id));
    view.handle_key_event(KeyCode::Char(' ').into());
    assert!(matches!(
        app.agents_overview.view_state.lock().unwrap().selection,
        AgentsOverviewSelection::Inactive
    ));
    view.handle_key_event(KeyCode::Esc.into());
    view.handle_key_event(KeyCode::Insert.into());
    assert!(app.agents_overview.view_state.lock().unwrap().renaming);
    view.handle_key_event(KeyCode::Char(' ').into());
    assert_eq!(
        app.agents_overview.view_state.lock().unwrap().input,
        "Task "
    );
    view.handle_key_event(KeyCode::F(7).into());
    view.handle_key_event(KeyCode::F(8).into());
    view.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    view.handle_key_event(KeyCode::F(6).into());
    assert!(rx.try_recv().is_err());
    view.handle_key_event(KeyCode::Char(' ').into());
    view.handle_key_event(KeyCode::Enter.into());
    assert!(
        matches!(rx.try_recv().unwrap(), AppEvent::SelectAgentsOverviewThread { thread_id } if thread_id == id)
    );
    view.handle_key_event(KeyCode::F(6).into());
    assert!(
        matches!(rx.try_recv().unwrap(), AppEvent::ConfirmAgentsOverviewDelete { thread_ids } if thread_ids == vec![id])
    );
    app.chat_widget.show_bottom_pane_view(Box::new(view));
    let rendered = render_bottom_popup(&app.chat_widget, /*width*/ 240);
    assert!(rendered.contains("f7 clear selection"));
    app.chat_widget.handle_key_event(KeyCode::F(7).into());
    assert!(matches!(
        app.agents_overview.view_state.lock().unwrap().selection,
        AgentsOverviewSelection::Inactive
    ));
}

#[tokio::test]
async fn bulk_delete_cancel_subtree_partial_failure_retry_and_stale_refresh() -> Result<()> {
    for fail_second in [false, true] {
        let (mut app, mut rx, _op_rx) = crate::app::tests::make_test_app_with_channels().await;
        let mut app_server = crate::start_embedded_app_server_for_picker(&app.config).await?;
        let mut roots = Vec::new();
        for (timestamp, name) in [("12", "First"), ("13", "Second"), ("14", "Unrelated")] {
            let id = ThreadId::from_string(
                &app_test_support::create_fake_rollout(
                    &app.config.codex_home,
                    &format!("2025-01-05T{timestamp}-00-00"),
                    &format!("2025-01-05T{timestamp}:00:00Z"),
                    name,
                    Some(&app.config.model_provider_id),
                    /*git_info*/ None,
                )
                .expect("materialize root"),
            )?;
            let thread = app_server.thread_read(id, /*include_turns*/ false).await?;
            roots.push((id, thread.path.clone().unwrap(), name.to_string()));
            app.agents_overview.threads.insert(id, Some(thread));
        }
        let child = ThreadId::from_string(
            &app_test_support::create_fake_parented_rollout_with_source(
                &app.config.codex_home,
                "2025-01-05T12-01-00",
                "2025-01-05T12:01:00Z",
                "Child",
                Some(&app.config.model_provider_id),
                /*git_info*/ None,
                codex_protocol::protocol::SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                    parent_thread_id: roots[0].0,
                    depth: 1,
                    agent_path: None,
                    agent_nickname: None,
                    agent_role: None,
                }),
                codex_protocol::SessionId::from(roots[0].0),
                roots[0].0,
            )
            .expect("materialize child"),
        )?;
        let state_db = codex_state::StateRuntime::init(
            codex_state::SqliteConfig::new_for_testing(app.config.codex_home.clone()),
            app.config.model_provider_id.clone(),
        )
        .await
        .expect("initialize spawn state");
        state_db
            .upsert_thread_spawn_edge(
                roots[0].0,
                child,
                codex_state::DirectionalThreadSpawnEdgeStatus::Open,
            )
            .await
            .expect("persist spawn edge");
        let child_thread = app_server
            .thread_read(child, /*include_turns*/ false)
            .await?;
        let child_path = child_thread.path.clone().unwrap();
        app.agents_overview
            .threads
            .insert(child, Some(child_thread));
        let resumed = Box::pin(app_server.resume_thread(
            &app.local_settings,
            app.config.clone(),
            child,
            crate::app_server_session::ResumeModelSettings::PreserveExistingThread,
        ))
        .await?;
        app.enqueue_primary_thread_session(resumed.session, resumed.turns)
            .await?;
        app.app_server_target = AppServerTarget::LocalDaemon {
            endpoint: crate::RemoteAppServerEndpoint::UnixSocket {
                socket_path: app.config.codex_home.join("unused.sock"),
            },
        };
        let unrelated_bytes = std::fs::read(&roots[2].1)?;
        let mut tui = crate::tui::test_support::make_test_tui()?;
        tui.pause_events();
        let view = app.agents_overview_view(
            app.agents_overview
                .threads
                .values()
                .flatten()
                .cloned()
                .collect(),
            Some(roots[0].0),
        );
        app.chat_widget.show_bottom_pane_view(Box::new(view));
        app.chat_widget.handle_key_event(KeyCode::Esc.into());
        app.chat_widget.handle_key_event(KeyCode::Insert.into());
        app.chat_widget.handle_key_event(KeyCode::Char(' ').into());
        let view = app.agents_overview_view(
            app.agents_overview
                .threads
                .values()
                .flatten()
                .cloned()
                .collect(),
            Some(roots[1].0),
        );
        app.chat_widget
            .replace_bottom_pane_view_if_present(AGENTS_OVERVIEW_VIEW_ID, Box::new(view));
        app.chat_widget.handle_key_event(KeyCode::Char(' ').into());
        app.chat_widget.handle_key_event(KeyCode::Delete.into());
        let confirm = std::iter::from_fn(|| rx.try_recv().ok())
            .find(|event| matches!(event, AppEvent::ConfirmAgentsOverviewDelete { .. }))
            .unwrap();
        Box::pin(app.handle_event(&mut tui, &mut app_server, confirm)).await?;
        for width in [72, 36] {
            insta::assert_snapshot!(
                format!("bulk_delete_confirmation_{width}"),
                render_bottom_popup(&app.chat_widget, width)
            );
        }
        app.chat_widget.handle_key_event(KeyCode::Enter.into());
        assert!(
            !std::iter::from_fn(|| rx.try_recv().ok())
                .any(|event| matches!(event, AppEvent::RunAgentsOverviewDelete { .. }))
        );
        assert!(roots.iter().all(|(_, path, _)| path.exists()) && child_path.exists());
        // The compressed counterpart blocks cleanup after the plain rollout is deleted.
        let saved = roots[1].1.with_extension("saved");
        let blocked = roots[1].1.with_extension("jsonl.zst");
        if fail_second {
            std::fs::copy(&roots[1].1, &saved)?;
            std::fs::create_dir(&blocked)?;
        }
        app.chat_widget.handle_key_event(KeyCode::Delete.into());
        let confirm = std::iter::from_fn(|| rx.try_recv().ok())
            .find(|event| matches!(event, AppEvent::ConfirmAgentsOverviewDelete { .. }))
            .unwrap();
        Box::pin(app.handle_event(&mut tui, &mut app_server, confirm)).await?;
        app.chat_widget.handle_key_event(KeyCode::Down.into());
        app.chat_widget.handle_key_event(KeyCode::Enter.into());
        let run = std::iter::from_fn(|| rx.try_recv().ok())
            .find(|event| matches!(event, AppEvent::RunAgentsOverviewDelete { .. }))
            .unwrap();
        let request_id = Uuid::new_v4();
        app.agents_overview.request_id = Some(request_id);
        let stale_threads = app.agents_overview.threads.clone();
        Box::pin(app.handle_event(&mut tui, &mut app_server, run)).await?;
        app.apply_agents_overview_thread_refresh(
            &app_server,
            request_id,
            Ok(AgentsOverviewThreadRefresh {
                threads: stale_threads,
                last_messages: HashMap::new(),
                recent_seed_complete: true,
            }),
        );
        assert_eq!(
            (app.primary_thread_id, app.chat_widget.thread_id()),
            (None, None)
        );
        if !fail_second {
            assert_eq!(
                (
                    roots[0].1.exists(),
                    roots[1].1.exists(),
                    child_path.exists()
                ),
                (false, false, false)
            );
            assert_eq!(std::fs::read(&roots[2].1)?, unrelated_bytes);
            assert_eq!(
                app.agents_overview
                    .threads
                    .keys()
                    .copied()
                    .collect::<HashSet<_>>(),
                HashSet::from([roots[2].0])
            );
            app_server.shutdown().await?;
            continue;
        }
        assert_eq!(
            (roots[0].1.exists(), child_path.exists(), saved.exists()),
            (false, false, true)
        );
        assert_eq!(std::fs::read(&roots[2].1)?, unrelated_bytes);
        assert_eq!(
            app.agents_overview
                .threads
                .keys()
                .copied()
                .collect::<HashSet<_>>(),
            HashSet::from([roots[1].0, roots[2].0])
        );
        let failure = render_bottom_popup(&app.chat_widget, /*width*/ 80);
        assert!(failure.contains("1 deleted, 1 failed") && failure.contains("Second"));
        {
            let state = app.agents_overview.view_state.lock().unwrap();
            let AgentsOverviewSelection::Active(marked) = &state.selection else {
                panic!("failed root stays marked")
            };
            assert_eq!(marked, &HashSet::from([roots[1].0]));
        }
        std::fs::remove_dir(&blocked)?;
        std::fs::rename(&saved, &roots[1].1)?;
        app.chat_widget.handle_key_event(KeyCode::Enter.into());
        app.chat_widget.handle_key_event(KeyCode::Delete.into());
        let confirm = std::iter::from_fn(|| rx.try_recv().ok())
            .find(|event| matches!(event, AppEvent::ConfirmAgentsOverviewDelete { .. }))
            .unwrap();
        Box::pin(app.handle_event(&mut tui, &mut app_server, confirm)).await?;
        app.chat_widget.handle_key_event(KeyCode::Down.into());
        app.chat_widget.handle_key_event(KeyCode::Enter.into());
        let run = std::iter::from_fn(|| rx.try_recv().ok())
            .find(|event| matches!(event, AppEvent::RunAgentsOverviewDelete { .. }))
            .unwrap();
        Box::pin(app.handle_event(&mut tui, &mut app_server, run)).await?;
        assert_eq!(
            (
                roots[0].1.exists(),
                roots[1].1.exists(),
                child_path.exists()
            ),
            (false, false, false)
        );
        assert_eq!(std::fs::read(&roots[2].1)?, unrelated_bytes);
        app_server.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn bulk_confirmation_keeps_actions_visible_in_a_short_terminal() {
    let mut app = make_test_app().await;
    let targets: Vec<_> = (1..=25).map(ThreadId::from_u128).collect();
    for id in &targets {
        app.agents_overview.threads.insert(
            *id,
            Some(overview_thread(
                *id,
                /*parent_thread_id*/ None,
                "Repeated long task title that needs wrapping",
                ThreadStatus::Idle,
            )),
        );
    }
    app.confirm_agents_overview_delete(targets);
    let mut terminal = Terminal::new(TestBackend::new(/*width*/ 36, /*height*/ 12)).unwrap();
    terminal
        .draw(|frame| app.chat_widget.render(frame.area(), frame.buffer_mut()))
        .unwrap();
    insta::assert_snapshot!("bulk_confirmation_short", terminal.backend());
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    assert!(rendered.contains("Cancel") && rendered.contains("Permanently delete"));
}

#[tokio::test]
async fn authoritative_deletion_prunes_marks() {
    let (mut app, mut rx, _op_rx) = crate::app::tests::make_test_app_with_channels().await;
    let id = ThreadId::new();
    let thread = overview_thread(
        id,
        /*parent_thread_id*/ None,
        "Gone",
        ThreadStatus::Idle,
    );
    app.agents_overview.threads.insert(id, Some(thread.clone()));
    let mut view = app.agents_overview_view(vec![thread], Some(id));
    view.handle_key_event(KeyCode::Esc.into());
    view.handle_key_event(KeyCode::Insert.into());
    view.handle_key_event(KeyCode::Char(' ').into());
    app.track_agents_overview_notification(&ServerNotification::ThreadDeleted(
        codex_app_server_protocol::ThreadDeletedNotification {
            thread_id: id.to_string(),
        },
    ));
    view.handle_key_event(KeyCode::Delete.into());
    assert!(rx.try_recv().is_err());
    let state = app.agents_overview.view_state.lock().unwrap();
    assert!(
        matches!(&state.selection, AgentsOverviewSelection::Active(marked) if marked.is_empty())
    );
}
