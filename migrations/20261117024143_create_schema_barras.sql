-- Every table of Barras lives in this schema, and SQL names it on every table it touches
-- (`barras.users`), so that no query depends on a connection's `search_path`. sqlx keeps its
-- own history table, `_sqlx_migrations`, in `public`.
CREATE SCHEMA barras;
