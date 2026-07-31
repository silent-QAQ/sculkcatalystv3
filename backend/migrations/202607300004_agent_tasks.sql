-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

CREATE TABLE cloud_agent_tasks (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE RESTRICT,
    agent_id UUID NOT NULL REFERENCES cloud_agents(id) ON DELETE RESTRICT,
    operation TEXT NOT NULL CHECK (operation IN (
        'host.inspect',
        'workspace.list',
        'log.tail',
        'workspace.create_directory',
        'server.properties.update',
        'shell.exec',
        'task.rollback'
    )),
    required_permission TEXT NOT NULL CHECK (required_permission IN ('read', 'write', 'full')),
    risk TEXT NOT NULL CHECK (risk IN ('low', 'high', 'critical')),
    input JSONB NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'awaiting_approval', 'queued', 'leased', 'running',
        'succeeded', 'failed', 'cancelled'
    )),
    idempotency_key TEXT,
    source_task_id UUID REFERENCES cloud_agent_tasks(id) ON DELETE RESTRICT,
    approved_by UUID REFERENCES cloud_users(id) ON DELETE SET NULL,
    approved_at TIMESTAMPTZ,
    lease_token_hash CHAR(64),
    lease_expires_at TIMESTAMPTZ,
    leased_at TIMESTAMPTZ,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    output JSONB,
    error TEXT,
    artifacts JSONB,
    rollback_available BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, idempotency_key),
    CHECK ((operation = 'task.rollback') = (source_task_id IS NOT NULL)),
    CHECK (operation != 'task.rollback' OR required_permission = 'write'),
    CHECK (operation != 'shell.exec' OR (required_permission = 'full' AND risk = 'critical')),
    CHECK (
        (required_permission = 'read' AND risk = 'low') OR
        (required_permission = 'write' AND risk = 'high') OR
        (required_permission = 'full' AND risk = 'critical')
    ),
    CHECK (rollback_available = FALSE OR (status = 'succeeded' AND required_permission = 'write')),
    CHECK ((approved_by IS NULL) = (approved_at IS NULL)),
    CHECK ((lease_token_hash IS NULL) = (lease_expires_at IS NULL)),
    CHECK ((status IN ('leased', 'running')) = (lease_token_hash IS NOT NULL)),
    CHECK (risk = 'low' OR status IN ('awaiting_approval', 'cancelled') OR approved_by IS NOT NULL)
);

CREATE INDEX cloud_agent_tasks_user_idx
    ON cloud_agent_tasks(user_id, created_at DESC);
CREATE INDEX cloud_agent_tasks_agent_queue_idx
    ON cloud_agent_tasks(agent_id, status, created_at)
    WHERE status IN ('queued', 'leased', 'running');
CREATE INDEX cloud_agent_tasks_lease_expiry_idx
    ON cloud_agent_tasks(lease_expires_at)
    WHERE status IN ('leased', 'running');
CREATE UNIQUE INDEX cloud_agent_tasks_active_rollback_idx
    ON cloud_agent_tasks(source_task_id)
    WHERE operation = 'task.rollback'
      AND status NOT IN ('failed', 'cancelled');

CREATE TABLE cloud_agent_task_events (
    id UUID PRIMARY KEY,
    task_id UUID NOT NULL REFERENCES cloud_agent_tasks(id) ON DELETE RESTRICT,
    seq INTEGER NOT NULL CHECK (seq > 0),
    level TEXT NOT NULL CHECK (level IN ('info', 'warn', 'error')),
    message TEXT NOT NULL CHECK (char_length(message) BETWEEN 1 AND 2000),
    data JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (task_id, seq)
);

CREATE INDEX cloud_agent_task_events_task_idx
    ON cloud_agent_task_events(task_id, seq);
