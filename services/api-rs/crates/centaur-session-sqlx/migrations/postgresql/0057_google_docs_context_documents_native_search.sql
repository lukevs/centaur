-- no-transaction

create index concurrently if not exists idx_google_docs_context_documents_search
    on google_docs_context_documents using gin
    (to_tsvector('english', coalesce(title, '') || ' ' || coalesce(body, '')));
