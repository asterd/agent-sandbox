-- Multi-tenancy/auth support for API-key mode.
--
-- NOTE: the roadmap called this migration 002, but 002 already exists in this
-- repository. It must therefore be 003 to preserve the applied migration chain.

CREATE TABLE tenants (
    id               TEXT PRIMARY KEY,
    api_key_hash     TEXT NOT NULL,
    quota_hourly     INTEGER NOT NULL DEFAULT 100,
    quota_concurrent INTEGER NOT NULL DEFAULT 10,
    enabled          INTEGER NOT NULL DEFAULT 1,
    created_at       TEXT NOT NULL
);

ALTER TABLE sandboxes ADD COLUMN tenant_id TEXT;

CREATE TABLE rate_limit_windows (
    tenant_id    TEXT NOT NULL,
    window_start TEXT NOT NULL,
    count        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, window_start)
);

CREATE INDEX idx_sandboxes_tenant_id ON sandboxes(tenant_id);
