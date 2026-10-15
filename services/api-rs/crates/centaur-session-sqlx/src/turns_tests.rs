use super::*;
use serde_json::json;

fn new_session(key: &ThreadKey) -> NewSession<'_> {
    NewSession {
        thread_key: key,
        harness_type: &HarnessType::Codex,
        persona_id: None,
        principal_id: "prn_test",
        metadata: json!({}),
        proxy_labels: BTreeMap::new(),
    }
}

fn turn(text: &str) -> SessionTurnInput {
    SessionTurnInput {
        message: SessionMessageInput {
            client_message_id: Some(Uuid::new_v4().to_string()),
            role: MessageRole::User,
            parts: vec![json!({"type":"text", "text":text})],
            metadata: json!({}),
        },
        execution_metadata: json!({}),
        request: json!({"input_lines":[text]}),
    }
}

#[tokio::test]
async fn initial_turn_failure_rolls_back_owner_message_and_name() {
    let Some(store) = crate::tests::test_store().await else {
        return;
    };
    let key = ThreadKey::parse(format!("test:atomic-turn-{}", Uuid::new_v4())).unwrap();
    let mut input = turn("first");
    // PostgreSQL rejects NUL in jsonb. This fails the execution insert,
    // after the session and transcript inserts have already succeeded.
    input.execution_metadata = json!({"invalid":"\u{0}"});
    assert!(
        store
            .create_session_with_turn(new_session(&key), &input)
            .await
            .is_err()
    );
    assert!(matches!(
        store.get_session(&key).await,
        Err(SessionStoreError::NotFound { .. })
    ));
    assert!(store.list_messages(&key).await.unwrap().is_empty());
    assert!(
        store
            .latest_execution_for_thread(&key)
            .await
            .unwrap()
            .is_none()
    );

    input.execution_metadata = json!({});
    let (session, execution) = store
        .create_session_with_turn(new_session(&key), &input)
        .await
        .unwrap();
    assert_eq!(session.iron_control_principal.as_deref(), Some("prn_test"));
    assert_eq!(execution.status, ExecutionStatus::Queued);
    assert_eq!(store.list_messages(&key).await.unwrap().len(), 1);
    assert!(matches!(
        store
            .create_session_with_turn(new_session(&key), &turn("duplicate"))
            .await,
        Err(SessionStoreError::AlreadyExists { .. })
    ));
    assert_eq!(store.list_messages(&key).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_idle_sends_reserve_only_one_execution() {
    let Some(store) = crate::tests::test_store().await else {
        return;
    };
    let key = ThreadKey::parse(format!("test:idle-send-{}", Uuid::new_v4())).unwrap();
    store.create_owned_session(new_session(&key)).await.unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let store = store.clone();
        let key = key.clone();
        tasks.spawn(async move { store.prepare_session_turn(&key, &turn("send")).await });
    }
    let mut queued = 0;
    let mut starting = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap().unwrap() {
            PreparedSessionTurn::Queued(_) => queued += 1,
            PreparedSessionTurn::Starting => starting += 1,
            PreparedSessionTurn::Steering(_) => {
                panic!("queued input cannot be steered before dispatch")
            }
        }
    }
    assert_eq!((queued, starting), (1, 7));
    assert_eq!(store.list_messages(&key).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steering_locks_completion_and_rollback_does_not_admit_a_message() {
    let Some(store) = crate::tests::test_store().await else {
        return;
    };
    let key = ThreadKey::parse(format!("test:steer-{}", Uuid::new_v4())).unwrap();
    let (_, execution) = store
        .create_session_with_turn(new_session(&key), &turn("first"))
        .await
        .unwrap();
    // Both queued and claimed-but-not-delivered executions must finish
    // starting before accepting another message.
    assert!(matches!(
        store
            .prepare_session_turn(&key, &turn("early"))
            .await
            .unwrap(),
        PreparedSessionTurn::Starting
    ));
    store
        .mark_execution_running(&execution.execution_id)
        .await
        .unwrap();
    assert!(matches!(
        store
            .prepare_session_turn(&key, &turn("early"))
            .await
            .unwrap(),
        PreparedSessionTurn::Starting
    ));
    store
        .append_event(
            &key,
            Some(&execution.execution_id),
            "session.input_delivered",
            json!({}),
        )
        .await
        .unwrap();
    let input = turn("steer");
    let PreparedSessionTurn::Steering(pending) =
        store.prepare_session_turn(&key, &input).await.unwrap()
    else {
        panic!("expected steering");
    };
    drop(pending); // Simulate a failed stdin write.
    assert_eq!(store.list_messages(&key).await.unwrap().len(), 1);

    let PreparedSessionTurn::Steering(pending) =
        store.prepare_session_turn(&key, &input).await.unwrap()
    else {
        panic!("expected steering");
    };
    assert_eq!(pending.execution.execution_id, execution.execution_id);
    let completion_store = store.clone();
    let id = execution.execution_id.clone();
    let mut completion =
        tokio::spawn(async move { completion_store.complete_execution(&id).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut completion)
            .await
            .is_err()
    );
    pending.commit().await.unwrap();
    completion.await.unwrap().unwrap();
    assert_eq!(store.list_messages(&key).await.unwrap().len(), 2);
    // Completion won before this admission, so the next message starts a
    // new execution rather than being steered into the terminal one.
    let PreparedSessionTurn::Queued(next) = store
        .prepare_session_turn(&key, &turn("after"))
        .await
        .unwrap()
    else {
        panic!("expected a new turn");
    };
    assert_ne!(next.execution_id, execution.execution_id);
    assert_eq!(store.list_messages(&key).await.unwrap().len(), 3);
}

#[tokio::test]
async fn terminal_status_can_precede_the_cursor_marker() {
    let Some(store) = crate::tests::test_store().await else {
        return;
    };
    let key = ThreadKey::parse(format!("test:terminal-cursor-{}", Uuid::new_v4())).unwrap();
    let (_, execution) = store
        .create_session_with_turn(new_session(&key), &turn("first"))
        .await
        .unwrap();
    store
        .complete_execution(&execution.execution_id)
        .await
        .unwrap();
    assert_eq!(
        store
            .execution_terminal_event_id(&execution.execution_id)
            .await
            .unwrap(),
        None
    );
    let event = store
        .append_event(
            &key,
            Some(&execution.execution_id),
            "session.execution_completed",
            json!({"result_text":"done"}),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .execution_terminal_event_id(&execution.execution_id)
            .await
            .unwrap(),
        Some(event.event_id)
    );
}
