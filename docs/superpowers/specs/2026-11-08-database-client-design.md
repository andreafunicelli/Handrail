# Database client (MySQL/MariaDB, PostgreSQL)

Decided with the owner on 2026-11-08: MySQL/MariaDB and PostgreSQL in the first version; schema
browsing, an SQL editor and result grid, editing rows in the grid; the AI agent can read the schema
and run queries, with approvals like terminal commands.

## Connections

- `ConnectionKind::Mysql` (already reserved, serialized `"mysql"`) and a new `Postgres`
  (`"postgres"`). Host, port (3306 / 5432), user, password (`SshAuth::Password` from the vault, or
  `Prompt`), jump hosts, environment, approval mode: the fields connections already have.
- New optional fields: `database` (the default database to open) and `db_tls` (`prefer` by
  default; `disable`, `require`, `verify`). Optional fields only: older vault items still load.
- Through jump hosts the database is reached over an SSH local forward to `127.0.0.1:<port>`, the
  way RDP already does (`rdp.rs::tunnel`). TLS verification of the host name is not meaningful
  through a tunnel; `verify` checks the certificate chain only there.

## Backend: `crates/aht-db`

sqlx 0.8 (`mysql`, `postgres`, `runtime-tokio`, rustls with the system roots), no macros.

- `DbClient::connect(DbParams) -> DbClient`: a small pool (2 connections: one for statements,
  one for the schema and for cancelling).
- Schema: databases (MySQL) / databases + schemas (PostgreSQL), tables and views, columns (name,
  type, nullable, default, primary key), from `information_schema` / `pg_catalog`.
- `query(sql, max_rows)`: runs the text as given with the simple (text) protocol (`raw_sql`), so
  several statements work and every value arrives as text; returns one result per statement:
  columns (name, type), rows of `Option<String>` (binary values hex-encoded with a marker),
  rows affected, whether rows were cut at `max_rows`, elapsed time. A cut result closes its
  connection instead of draining the rest.
- `cancel()`: `KILL QUERY <id>` / `pg_cancel_backend(<pid>)` on the second connection.
- Edits (phase 3): `update`, `insert`, `delete` of one row of one table, identified by its
  primary key. Identifiers are quoted by the backend (backticks / double quotes), values are
  bound parameters (PostgreSQL casts `$n::text` to the column's catalog type). Tables without a
  primary key are read-only in the grid.
- SQL risk classification for the agent (phase 4): read (`SELECT`, `SHOW`, `EXPLAIN`,
  `DESCRIBE`, `WITH` without data-changing statements), write (other DML), destructive
  (`DROP`, `TRUNCATE`, `DELETE`/`UPDATE` without `WHERE`, `ALTER ... DROP`).

Tests run against real MySQL 8 and PostgreSQL 16 containers in CI (ubuntu), skipped where the
`HANDRAIL_MYSQL_TEST` / `HANDRAIL_PG_TEST` URLs are unset, like the SSH and S3 tests.

## App and UI (phase 2, 3)

- Commands `db_open`, `db_close`, `db_schema`, `db_query`, `db_cancel`, later `db_edit`.
- Sidebar: the "MySQL (soon)" tab becomes "Databases".
- A database pane in a tab: schema tree on the left; on the right, an SQL editor (CodeMirror,
  MySQL / PostgreSQL dialect, completion from the schema) over a virtualised result grid with
  CSV export. Opening a table shows its rows (`SELECT * ... LIMIT n`) in an editable grid:
  edited cells are marked, then saved together or discarded.

## Agent (phase 4)

MCP tools `db_schema` and `db_query` on the database tabs the user has open. Reads run directly;
writes need approval under confirm-writes, destructive statements always; on `prod` connections
every write is confirmed. Results are sent to the agent with a row and size cap.
