-- Initial schema for AgentSandbox daemon.
--
-- `spec_json`  : exact JSON the client submitted, for audit/debug.
-- `ir_json`    : compiled IR with secret_env stripped. Never persist secrets.
-- `lease_token`: opaque, per-sandbox bearer used to authorise exec/destroy.
-- `status`     : mirrors `SandboxStatus::as_str()` (creating|running|stopped|error).
-- `backend`    : adapter backend name (e.g. "docker").

CREATE TABLE sandboxes (
    id            TEXT PRIMARY KEY,
    lease_token   TEXT NOT NULL UNIQUE,
    status        TEXT NOT NULL,
    backend       TEXT NOT NULL,
    spec_json     TEXT NOT NULL,
    ir_json       TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    expires_at    TEXT NOT NULL,
    error_message TEXT
);

CREATE TABLE audit_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    sandbox_id TEXT NOT NULL,
    event      TEXT NOT NULL,
    detail     TEXT,
    ts         TEXT NOT NULL
);

CREATE INDEX idx_sandboxes_status ON sandboxes(status);
CREATE INDEX idx_sandboxes_expires ON sandboxes(expires_at);
CREATE INDEX idx_audit_sandbox ON audit_log(sandbox_id);
