-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

CREATE TABLE cloud_agents (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    platform TEXT NOT NULL,
    agent_version TEXT NOT NULL,
    workspace_label TEXT NOT NULL,
    capabilities JSONB NOT NULL DEFAULT '[]'::jsonb,
    permissions JSONB NOT NULL DEFAULT '[]'::jsonb,
    fingerprint TEXT NOT NULL,
    token_hash CHAR(64) NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'claimed'
        CHECK (status IN ('claimed', 'active', 'revoked')),
    last_seen_at TIMESTAMPTZ,
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    confirmed_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX cloud_agents_user_idx
    ON cloud_agents(user_id, created_at DESC);
CREATE INDEX cloud_agents_active_idx
    ON cloud_agents(user_id, last_seen_at DESC)
    WHERE status = 'active';

CREATE TABLE cloud_agent_pairings (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE CASCADE,
    code_hash CHAR(64) NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'claimed', 'confirmed', 'expired', 'revoked')),
    claimed_agent_id UUID UNIQUE REFERENCES cloud_agents(id) ON DELETE SET NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    claimed_at TIMESTAMPTZ,
    confirmed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX cloud_agent_pairings_user_idx
    ON cloud_agent_pairings(user_id, created_at DESC);
CREATE INDEX cloud_agent_pairings_pending_idx
    ON cloud_agent_pairings(code_hash, expires_at)
    WHERE status = 'pending';
