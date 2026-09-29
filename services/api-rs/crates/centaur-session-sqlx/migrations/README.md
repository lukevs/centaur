# Database migration profiles

Each supported database profile has an explicit schema tree with its own SQLx
migration history and checksums.

- `paradedb/` is the default profile and may use ParadeDB extensions such as
  `pg_search`.
- `postgresql/` targets stock PostgreSQL 16, including Aurora PostgreSQL and
  RDS for PostgreSQL. Search indexes use PostgreSQL's built-in full-text search.

Set `CENTAUR_DATABASE_PROFILE` to `paradedb` or `postgresql` before the API
starts. SQLx records applied versions and checksums in `_sqlx_migrations` and
rejects an incompatible tree when an applied version is missing or has a
different checksum. Changing profiles therefore requires an explicit data
export/import into a database initialized with the new profile.

## Adding migrations

- Never edit a migration that may already have been applied.
- Add portable schema changes to both trees. The SQL may be identical, but
  each file belongs to that profile's independently checksummed history.
- Add backend-specific changes only to the relevant tree.
- Do not assume version numbers must remain aligned between profiles. Choose
  the next unused version in each tree.
- Test a fresh install and an upgrade for both profiles. A PostgreSQL-profile
  migration must run against the stock `postgres:16` image, not only against
  ParadeDB.

SQLx provides the migration ledger and checksum validation for the selected
tree.
