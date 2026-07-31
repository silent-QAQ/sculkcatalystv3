-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

CREATE TABLE cloud_user_api_credentials (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES cloud_users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key_cipher BYTEA NOT NULL,
    api_key_nonce BYTEA NOT NULL,
    key_fingerprint CHAR(64) NOT NULL,
    key_prefix TEXT NOT NULL DEFAULT '',
    key_suffix TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, key_fingerprint)
);

CREATE INDEX cloud_user_api_credentials_user_idx
    ON cloud_user_api_credentials(user_id, updated_at DESC);
