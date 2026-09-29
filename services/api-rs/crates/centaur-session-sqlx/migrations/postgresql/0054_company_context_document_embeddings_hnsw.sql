-- pgvector is optional on stock PostgreSQL, so its index must be conditional.
do $migration$
begin
    if to_regclass('company_context_document_embeddings') is null then
        return;
    end if;

    execute $ddl$
        create index if not exists company_context_document_embeddings_embedding_hnsw_idx
            on company_context_document_embeddings
            using hnsw (embedding vector_cosine_ops)
            where embedding is not null and not embedding_failed
    $ddl$;
end
$migration$;
