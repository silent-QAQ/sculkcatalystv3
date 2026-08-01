-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

ALTER TABLE cloud_terminal_sessions
    ADD COLUMN team_id UUID REFERENCES cloud_teams(id) ON DELETE RESTRICT,
    ADD COLUMN approval_id UUID REFERENCES cloud_approvals(id) ON DELETE RESTRICT,
    ADD COLUMN approval_enforced BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE cloud_approvals
    ADD COLUMN terminal_session_id UUID;

ALTER TABLE cloud_approvals
    ADD CONSTRAINT cloud_approvals_terminal_session_fk
    FOREIGN KEY (terminal_session_id) REFERENCES cloud_terminal_sessions(id)
    ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE cloud_terminal_sessions
    ADD CONSTRAINT cloud_terminal_approval_team_fk
    FOREIGN KEY (approval_id, team_id) REFERENCES cloud_approvals(id, team_id)
    ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE cloud_approvals
    ADD CONSTRAINT cloud_approvals_single_resource_check
    CHECK (NOT (agent_task_id IS NOT NULL AND terminal_session_id IS NOT NULL))
    NOT VALID;

UPDATE cloud_terminal_sessions
SET approval_enforced = FALSE
WHERE status IN ('awaiting_approval', 'pending', 'starting', 'running', 'terminating')
  AND (team_id IS NULL OR approval_id IS NULL);

ALTER TABLE cloud_terminal_sessions
    ADD CONSTRAINT cloud_terminal_active_approval_check
    CHECK (
        NOT approval_enforced
        OR status NOT IN ('awaiting_approval', 'pending', 'starting', 'running', 'terminating')
        OR (team_id IS NOT NULL AND approval_id IS NOT NULL)
    )
    NOT VALID;

CREATE UNIQUE INDEX cloud_approvals_terminal_session_unique
    ON cloud_approvals(terminal_session_id)
    WHERE terminal_session_id IS NOT NULL;

CREATE INDEX cloud_terminal_sessions_approval_idx
    ON cloud_terminal_sessions(approval_id)
    WHERE approval_id IS NOT NULL;

-- Sessions created by older releases had no independently verifiable approval.
-- Stop only those not yet started; active shells are left for explicit user termination.
UPDATE cloud_terminal_sessions
SET status = 'cancelled',
    approval_enforced = FALSE,
    approved_by = NULL,
    approved_at = NULL,
    exited_at = COALESCE(exited_at, NOW()),
    error = COALESCE(error, 'Legacy terminal session cancelled because it has no linked team approval'),
    updated_at = NOW()
WHERE status IN ('awaiting_approval', 'pending')
  AND (team_id IS NULL OR approval_id IS NULL);

ALTER TABLE cloud_approvals
    VALIDATE CONSTRAINT cloud_approvals_single_resource_check;
ALTER TABLE cloud_terminal_sessions
    VALIDATE CONSTRAINT cloud_terminal_active_approval_check;
