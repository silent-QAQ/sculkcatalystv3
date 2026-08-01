-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

ALTER TABLE cloud_terminal_sessions
    ADD COLUMN terminal_redaction_pending BOOLEAN NOT NULL DEFAULT FALSE;

-- Historical terminal output was stored without a server-side redaction
-- boundary. It cannot be safely classified after the fact, so remove all
-- retained content rather than risk exposing credentials to a later reader.
UPDATE cloud_terminal_events
SET data_base64 = 'W1JFREFDVEVEXQ=='
WHERE kind = 'output';

-- Input must be available to the agent only until it acknowledges delivery.
-- New writes use encrypted-v1; this also clears plaintext retained by older
-- releases after their command was acknowledged.
UPDATE cloud_terminal_commands
SET payload = jsonb_build_object('format', 'redacted-v1')
WHERE kind = 'input' AND acknowledged_at IS NOT NULL;
