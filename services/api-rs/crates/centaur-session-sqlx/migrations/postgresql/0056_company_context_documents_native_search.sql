-- no-transaction

create index concurrently if not exists idx_company_context_documents_search
    on company_context_documents using gin
    (to_tsvector('english', coalesce(title, '') || ' ' || coalesce(body, '')));
