-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

ALTER TABLE cloud_agent_tasks
    ADD COLUMN team_id UUID REFERENCES cloud_teams(id) ON DELETE RESTRICT,
    ADD COLUMN approval_id UUID REFERENCES cloud_approvals(id) ON DELETE RESTRICT,
    ADD COLUMN approval_enforced BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE cloud_approvals
    ADD COLUMN agent_task_id UUID;

ALTER TABLE cloud_approvals
    ADD CONSTRAINT cloud_approvals_agent_task_fk
    FOREIGN KEY (agent_task_id) REFERENCES cloud_agent_tasks(id)
    ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE cloud_approvals
    ADD CONSTRAINT cloud_approvals_id_team_key UNIQUE (id, team_id);

ALTER TABLE cloud_agent_tasks
    ADD CONSTRAINT cloud_agent_tasks_approval_team_fk
    FOREIGN KEY (approval_id, team_id) REFERENCES cloud_approvals(id, team_id)
    ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED;

-- Tasks created before this migration have no independently verifiable approval.
-- Do not leave old queued work in a state that the new lease predicate can never
-- execute. Active leases/runs are grandfathered for termination/reconciliation;
-- only work that has not started is cancelled here.
UPDATE cloud_agent_tasks
SET status = 'cancelled',
    approval_enforced = FALSE,
    approved_by = NULL,
    approved_at = NULL,
    lease_token_hash = NULL,
    lease_expires_at = NULL,
    cancelled_at = COALESCE(cancelled_at, NOW()),
    completed_at = COALESCE(completed_at, NOW()),
    error = COALESCE(error, 'Legacy high-risk task cancelled because it has no linked team approval'),
    updated_at = NOW()
WHERE risk <> 'low'
  AND (team_id IS NULL OR approval_id IS NULL)
  AND status IN ('awaiting_approval', 'queued');

UPDATE cloud_agent_tasks
SET approval_enforced = FALSE
WHERE risk <> 'low' AND (team_id IS NULL OR approval_id IS NULL);

ALTER TABLE cloud_agent_tasks
    ADD CONSTRAINT cloud_agent_tasks_high_risk_approval_check
    CHECK (
        NOT approval_enforced
        OR risk = 'low'
        OR (team_id IS NOT NULL AND approval_id IS NOT NULL)
    )
    NOT VALID;

ALTER TABLE cloud_agent_tasks
    VALIDATE CONSTRAINT cloud_agent_tasks_high_risk_approval_check;

CREATE INDEX cloud_agent_tasks_approval_idx
    ON cloud_agent_tasks(approval_id)
    WHERE approval_id IS NOT NULL;

CREATE UNIQUE INDEX cloud_approvals_agent_task_unique
    ON cloud_approvals(agent_task_id)
    WHERE agent_task_id IS NOT NULL;
