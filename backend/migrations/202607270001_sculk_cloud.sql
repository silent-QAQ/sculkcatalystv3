-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

CREATE TABLE cloud_users (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    nickname TEXT NOT NULL,
    avatar_url TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user', 'admin')),
    plan TEXT NOT NULL DEFAULT 'free' CHECK (plan IN ('free', 'pro', 'team')),
    locale TEXT NOT NULL DEFAULT 'zh-CN',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX cloud_users_email_unique ON cloud_users (LOWER(email));

CREATE TABLE cloud_devices (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    platform TEXT NOT NULL DEFAULT 'unknown',
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX cloud_devices_user_idx ON cloud_devices(user_id, last_seen_at DESC);

CREATE TABLE cloud_sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE CASCADE,
    device_id UUID NOT NULL REFERENCES cloud_devices(id) ON DELETE CASCADE,
    token_hash CHAR(64) NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX cloud_sessions_active_idx ON cloud_sessions(user_id, expires_at DESC) WHERE revoked_at IS NULL;

CREATE TABLE cloud_settings (
    user_id UUID PRIMARY KEY REFERENCES cloud_users(id) ON DELETE CASCADE,
    version BIGINT NOT NULL DEFAULT 1,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_by_device UUID REFERENCES cloud_devices(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE cloud_teams (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    owner_id UUID NOT NULL REFERENCES cloud_users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE cloud_team_members (
    team_id UUID NOT NULL REFERENCES cloud_teams(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'approver', 'member')),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (team_id, user_id)
);
CREATE INDEX cloud_team_members_user_idx ON cloud_team_members(user_id);

CREATE TABLE cloud_team_invitations (
    id UUID PRIMARY KEY,
    team_id UUID NOT NULL REFERENCES cloud_teams(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'approver', 'member')),
    token_hash CHAR(64) NOT NULL UNIQUE,
    invited_by UUID NOT NULL REFERENCES cloud_users(id),
    expires_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX cloud_team_invites_email_idx ON cloud_team_invitations(LOWER(email), expires_at DESC);

CREATE TABLE cloud_approvals (
    id UUID PRIMARY KEY,
    team_id UUID NOT NULL REFERENCES cloud_teams(id) ON DELETE CASCADE,
    requested_by UUID NOT NULL REFERENCES cloud_users(id),
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    risk TEXT NOT NULL CHECK (risk IN ('low', 'medium', 'high')),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected', 'cancelled')),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    decision_comment TEXT NOT NULL DEFAULT '',
    decided_by UUID REFERENCES cloud_users(id),
    decided_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX cloud_approvals_team_idx ON cloud_approvals(team_id, status, created_at DESC);

CREATE TABLE cloud_relay_provider (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key_cipher BYTEA NOT NULL,
    api_key_nonce BYTEA NOT NULL,
    default_model TEXT NOT NULL DEFAULT '',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    updated_by UUID REFERENCES cloud_users(id),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE cloud_api_tokens (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    token_prefix TEXT NOT NULL,
    token_hash CHAR(64) NOT NULL UNIQUE,
    last_used_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX cloud_api_tokens_user_idx ON cloud_api_tokens(user_id, created_at DESC);

CREATE TABLE cloud_api_usage (
    id UUID PRIMARY KEY,
    token_id UUID REFERENCES cloud_api_tokens(id) ON DELETE SET NULL,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE CASCADE,
    endpoint TEXT NOT NULL,
    model TEXT NOT NULL DEFAULT '',
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    status_code INTEGER NOT NULL,
    latency_ms INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX cloud_api_usage_user_time_idx ON cloud_api_usage(user_id, created_at DESC);
