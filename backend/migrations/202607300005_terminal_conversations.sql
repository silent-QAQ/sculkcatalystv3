-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

CREATE TABLE cloud_terminal_sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE RESTRICT,
    agent_id UUID NOT NULL REFERENCES cloud_agents(id) ON DELETE RESTRICT,
    title TEXT NOT NULL,
    cwd TEXT,
    cols INTEGER NOT NULL CHECK (cols BETWEEN 20 AND 400),
    rows INTEGER NOT NULL CHECK (rows BETWEEN 5 AND 200),
    status TEXT NOT NULL CHECK (status IN (
        'awaiting_approval', 'pending', 'starting', 'running',
        'terminating', 'exited', 'failed', 'cancelled'
    )),
    approved_by UUID REFERENCES cloud_users(id) ON DELETE SET NULL,
    approved_at TIMESTAMPTZ,
    instance_id TEXT,
    lease_expires_at TIMESTAMPTZ,
    exit_code INTEGER,
    error TEXT,
    next_command_seq BIGINT NOT NULL DEFAULT 0 CHECK (next_command_seq >= 0),
    last_event_seq BIGINT NOT NULL DEFAULT 0 CHECK (last_event_seq >= 0),
    event_count INTEGER NOT NULL DEFAULT 0 CHECK (event_count BETWEEN 0 AND 20000),
    output_bytes BIGINT NOT NULL DEFAULT 0 CHECK (output_bytes BETWEEN 0 AND 8388608),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    last_seen_at TIMESTAMPTZ,
    exited_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK ((approved_by IS NULL) = (approved_at IS NULL)),
    CHECK ((instance_id IS NULL) = (lease_expires_at IS NULL)),
    CHECK ((status IN ('starting', 'running', 'terminating')) = (instance_id IS NOT NULL)),
    CHECK (status NOT IN ('exited', 'failed', 'cancelled') OR exited_at IS NOT NULL)
);

CREATE INDEX cloud_terminal_sessions_user_idx
    ON cloud_terminal_sessions(user_id, created_at DESC);
CREATE INDEX cloud_terminal_sessions_agent_active_idx
    ON cloud_terminal_sessions(agent_id, status, lease_expires_at)
    WHERE status IN ('pending', 'starting', 'running', 'terminating');

CREATE TABLE cloud_terminal_commands (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES cloud_terminal_sessions(id) ON DELETE RESTRICT,
    seq BIGINT NOT NULL CHECK (seq > 0),
    kind TEXT NOT NULL CHECK (kind IN ('start', 'input', 'resize', 'terminate')),
    payload JSONB NOT NULL,
    idempotency_key TEXT,
    lease_instance_id TEXT,
    lease_expires_at TIMESTAMPTZ,
    acknowledged_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, seq),
    CHECK ((lease_instance_id IS NULL) = (lease_expires_at IS NULL)),
    CHECK (acknowledged_at IS NULL OR lease_instance_id IS NULL)
);

CREATE INDEX cloud_terminal_commands_pending_idx
    ON cloud_terminal_commands(session_id, seq, lease_expires_at)
    WHERE acknowledged_at IS NULL;
CREATE UNIQUE INDEX cloud_terminal_commands_idempotency_idx
    ON cloud_terminal_commands(session_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE TABLE cloud_terminal_events (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL REFERENCES cloud_terminal_sessions(id) ON DELETE RESTRICT,
    seq BIGINT NOT NULL CHECK (seq > 0),
    kind TEXT NOT NULL CHECK (kind IN ('started', 'output', 'keepalive', 'exit', 'error')),
    data_base64 TEXT,
    data JSONB,
    output_bytes INTEGER NOT NULL DEFAULT 0 CHECK (output_bytes BETWEEN 0 AND 16384),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, seq),
    CHECK ((kind = 'output') = (data_base64 IS NOT NULL)),
    CHECK (kind = 'output' OR output_bytes = 0)
);

CREATE INDEX cloud_terminal_events_session_idx
    ON cloud_terminal_events(session_id, seq);

CREATE TABLE cloud_conversations (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE RESTRICT,
    title TEXT NOT NULL,
    agent_id UUID REFERENCES cloud_agents(id) ON DELETE RESTRICT,
    next_message_seq BIGINT NOT NULL DEFAULT 0 CHECK (next_message_seq >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX cloud_conversations_user_idx
    ON cloud_conversations(user_id, updated_at DESC);

CREATE TABLE cloud_conversation_messages (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES cloud_conversations(id) ON DELETE RESTRICT,
    seq BIGINT NOT NULL CHECK (seq > 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content TEXT NOT NULL CHECK (char_length(content) BETWEEN 1 AND 20000),
    kind TEXT NOT NULL CHECK (kind IN ('text', 'plan', 'system')),
    linked_task_id UUID REFERENCES cloud_agent_tasks(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK ((kind = 'plan') = (linked_task_id IS NOT NULL)),
    CHECK (kind != 'plan' OR role = 'assistant'),
    CHECK (kind != 'system' OR role = 'system'),
    UNIQUE (conversation_id, seq)
);

CREATE INDEX cloud_conversation_messages_conversation_idx
    ON cloud_conversation_messages(conversation_id, seq);
CREATE UNIQUE INDEX cloud_conversation_plan_task_idx
    ON cloud_conversation_messages(linked_task_id)
    WHERE kind = 'plan';
