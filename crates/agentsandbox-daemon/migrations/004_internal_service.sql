CREATE TABLE runtime_metadata (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE tenant_usage (
    tenant_id         TEXT PRIMARY KEY,
    concurrent_in_use INTEGER NOT NULL DEFAULT 0
);
