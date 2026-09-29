-- no-transaction

create index concurrently if not exists idx_granola_context_documents_search
    on granola_context_documents using gin
    (to_tsvector('english', coalesce(title, '') || ' ' || coalesce(body, '')));
