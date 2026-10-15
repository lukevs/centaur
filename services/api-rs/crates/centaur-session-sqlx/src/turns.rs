//! Atomic admission for callers that submit a transcript message and a turn together.

use super::*;

pub struct NewSession<'a> {
    pub thread_key: &'a ThreadKey,
    pub harness_type: &'a HarnessType,
    pub persona_id: Option<&'a str>,
    pub principal_id: &'a str,
    pub metadata: Value,
    pub proxy_labels: BTreeMap<String, String>,
}

#[cfg(test)]
#[path = "turns_tests.rs"]
mod tests;

pub struct SessionTurnInput {
    pub message: SessionMessageInput,
    pub execution_metadata: Value,
    pub request: Value,
}

/// A running turn stays locked until the input write finishes. Completion and
/// another sender cannot change the routing decision midway through that write.
pub struct SteeringSessionTurn {
    pub execution: SessionExecution,
    pub message_ids: Vec<String>,
    transaction: sqlx::Transaction<'static, sqlx::Postgres>,
}

impl SteeringSessionTurn {
    pub async fn commit(self) -> Result<SessionExecution, SessionStoreError> {
        self.transaction.commit().await?;
        Ok(self.execution)
    }
}

pub enum PreparedSessionTurn {
    Queued(SessionExecution),
    Steering(SteeringSessionTurn),
    /// The execution is queued or its driver has not written its initial input.
    /// No message was appended; retry after allowing the driver to progress.
    Starting,
}

impl PgSessionStore {
    async fn insert_owned_session(
        conn: &mut sqlx::PgConnection,
        input: NewSession<'_>,
    ) -> Result<Session, SessionStoreError> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"insert into sessions
                (thread_key, harness_type, persona_id, iron_control_principal, status, metadata, proxy_labels)
            values ($1, $2, $3, $4, 'idle', $5, $6)
            on conflict (thread_key) do nothing
            returning thread_key, title, sandbox_id, sandbox_repo_cache_enabled, sandbox_repo_cache_access, sandbox_observability_enabled, harness_type, harness_thread_id, persona_id, status, iron_control_principal, proxy_labels, sandbox_last_active_at, created_at, updated_at"#,
        )
        .bind(input.thread_key.as_str())
        .bind(input.harness_type.as_ref())
        .bind(input.persona_id)
        .bind(input.principal_id)
        .bind(input.metadata)
        .bind(Json(input.proxy_labels))
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| SessionStoreError::AlreadyExists { thread_key: input.thread_key.to_string() })?;
        row.try_into()
    }

    /// A pure create never exposes a row without its immutable owner.
    pub async fn create_owned_session(
        &self,
        input: NewSession<'_>,
    ) -> Result<Session, SessionStoreError> {
        Self::insert_owned_session(&mut *self.pool.acquire().await?, input).await
    }

    /// The owner, first transcript message, and replayable execution request are
    /// one commit. Failure at any insert leaves the name available for retry.
    pub async fn create_session_with_turn(
        &self,
        input: NewSession<'_>,
        turn: &SessionTurnInput,
    ) -> Result<(Session, SessionExecution), SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let thread_key = input.thread_key.clone();
        let session = Self::insert_owned_session(&mut tx, input).await?;
        Self::append_messages_on(&mut tx, &thread_key, std::slice::from_ref(&turn.message)).await?;
        let execution = Self::create_execution_on(
            &mut tx,
            &thread_key,
            turn.message.client_message_id.as_deref(),
            turn.execution_metadata.clone(),
            turn.request.clone(),
        )
        .await?;
        tx.commit().await?;
        Ok((session, execution.execution))
    }

    /// Choose exactly one route under a transaction. Starting executions must
    /// finish their initial input write before accepting steering. The lock serializes
    /// idle senders, and the execution lock serializes claims and completion.
    pub async fn prepare_session_turn(
        &self,
        thread_key: &ThreadKey,
        turn: &SessionTurnInput,
    ) -> Result<PreparedSessionTurn, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "select pg_advisory_xact_lock(hashtext('session.turn.admission'), hashtext($1))",
        )
        .bind(thread_key.as_str())
        .execute(&mut *tx)
        .await?;
        let active = sqlx::query_as::<_, SessionExecutionRow>(
            r#"select execution_id, idempotency_key, thread_key, status, metadata, error, created_at, updated_at, started_at, completed_at
            from session_executions where thread_key = $1 and status in ('queued', 'running')
            for update"#,
        ).bind(thread_key.as_str()).fetch_optional(&mut *tx).await?;
        let Some(active) = active else {
            Self::append_messages_on(&mut tx, thread_key, std::slice::from_ref(&turn.message))
                .await?;
            let execution = Self::create_execution_on(
                &mut tx,
                thread_key,
                turn.message.client_message_id.as_deref(),
                turn.execution_metadata.clone(),
                turn.request.clone(),
            )
            .await?;
            tx.commit().await?;
            return Ok(PreparedSessionTurn::Queued(execution.execution));
        };
        let execution: SessionExecution = active.try_into()?;
        if execution.status == ExecutionStatus::Queued {
            return Ok(PreparedSessionTurn::Starting);
        }
        if execution.status == ExecutionStatus::Running {
            let ready = sqlx::query_scalar::<_, bool>(
                "select exists(select 1 from session_events where execution_id = $1 and event_type = 'session.input_delivered')",
            ).bind(&execution.execution_id).fetch_one(&mut *tx).await?;
            if !ready {
                return Ok(PreparedSessionTurn::Starting);
            }
        }
        let message_ids =
            Self::append_messages_on(&mut tx, thread_key, std::slice::from_ref(&turn.message))
                .await?;
        Ok(PreparedSessionTurn::Steering(SteeringSessionTurn {
            execution,
            message_ids,
            transaction: tx,
        }))
    }

    pub async fn execution_terminal_event_id(
        &self,
        execution_id: &str,
    ) -> Result<Option<i64>, SessionStoreError> {
        Ok(sqlx::query_scalar(
            "select max(event_id) from session_events where execution_id = $1 and event_type in ('session.execution_completed', 'session.execution_failed', 'session.execution_cancelled')",
        ).bind(execution_id).fetch_one(&self.pool).await?)
    }
}
