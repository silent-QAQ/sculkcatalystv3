-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

ALTER TABLE cloud_agent_tasks
    ADD COLUMN cancel_requested_at TIMESTAMPTZ,
    ADD COLUMN cancel_requested_by UUID REFERENCES cloud_users(id) ON DELETE RESTRICT,
    ADD COLUMN cancel_acknowledged_at TIMESTAMPTZ,
    ADD CONSTRAINT cloud_agent_tasks_cancel_request_pair_check CHECK (
        (cancel_requested_at IS NULL) = (cancel_requested_by IS NULL)
    ),
    ADD CONSTRAINT cloud_agent_tasks_cancel_operation_check CHECK (
        cancel_requested_at IS NULL OR operation = 'shell.exec'
    ),
    ADD CONSTRAINT cloud_agent_tasks_cancel_ack_check CHECK (
        cancel_acknowledged_at IS NULL OR (
            cancel_requested_at IS NOT NULL
            AND cancel_acknowledged_at >= cancel_requested_at
            AND status = 'cancelled'
        )
    );

CREATE INDEX cloud_agent_tasks_pending_cancellation_idx
    ON cloud_agent_tasks(agent_id, lease_expires_at)
    WHERE status = 'running' AND cancel_requested_at IS NOT NULL
      AND cancel_acknowledged_at IS NULL;
