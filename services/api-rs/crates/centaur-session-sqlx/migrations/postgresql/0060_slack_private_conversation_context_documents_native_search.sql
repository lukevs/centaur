-- no-transaction

create index concurrently if not exists idx_slack_private_conversation_context_documents_search
    on slack_private_conversation_context_documents using gin
    (to_tsvector('english', coalesce(title, '') || ' ' || coalesce(body, '')));
