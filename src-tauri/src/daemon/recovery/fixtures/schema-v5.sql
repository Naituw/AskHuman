-- Frozen upgrade fixture. Keep this schema unchanged when adding later migrations.
CREATE TABLE requests (
    id TEXT PRIMARY KEY,
    payload TEXT NOT NULL,
    decision TEXT,
    outcome TEXT,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE calls (
    id TEXT PRIMARY KEY,
    secret_hash TEXT NOT NULL,
    request_id TEXT NOT NULL REFERENCES requests(id),
    task_hash TEXT NOT NULL,
    caller_pid INTEGER NOT NULL,
    daemon_owned INTEGER NOT NULL DEFAULT 0,
    legacy INTEGER NOT NULL DEFAULT 0,
    cancelled INTEGER NOT NULL DEFAULT 0,
    delivered INTEGER NOT NULL DEFAULT 0,
    birth TEXT
);
CREATE INDEX calls_request ON calls(request_id);
CREATE TABLE cancelled_calls (
    id TEXT PRIMARY KEY,
    secret_hash TEXT NOT NULL,
    at_ms INTEGER NOT NULL
);
CREATE TABLE checkpoints (
    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (request_id, channel)
);
CREATE TABLE metadata (key TEXT PRIMARY KEY, payload TEXT NOT NULL);
CREATE TABLE receipts (
    id TEXT PRIMARY KEY,
    phone_id TEXT NOT NULL,
    hash TEXT NOT NULL,
    result TEXT NOT NULL,
    at_ms INTEGER NOT NULL
);
CREATE TABLE completed (
    request_id TEXT PRIMARY KEY REFERENCES requests(id) ON DELETE CASCADE,
    at_ms INTEGER NOT NULL
);
PRAGMA user_version=5;
