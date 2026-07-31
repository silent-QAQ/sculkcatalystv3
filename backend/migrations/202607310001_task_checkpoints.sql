-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

ALTER TABLE cloud_agent_tasks
    ADD COLUMN lineage_id UUID,
    ADD COLUMN attempt_no INTEGER,
    ADD COLUMN retry_of_task_id UUID REFERENCES cloud_agent_tasks(id) ON DELETE RESTRICT,
    ADD COLUMN execution_mode TEXT,
    ADD COLUMN resume_checkpoint_id UUID,
    ADD COLUMN rollback_source_task_id UUID REFERENCES cloud_agent_tasks(id) ON DELETE RESTRICT,
    ADD COLUMN retry_request_key TEXT;

UPDATE cloud_agent_tasks
SET lineage_id = id,
    attempt_no = 1,
    execution_mode = 'original',
    rollback_source_task_id = CASE
        WHEN rollback_available AND operation != 'task.rollback' THEN id
        ELSE NULL
    END;

ALTER TABLE cloud_agent_tasks
    ALTER COLUMN lineage_id SET NOT NULL,
    ALTER COLUMN attempt_no SET NOT NULL,
    ALTER COLUMN execution_mode SET NOT NULL,
    ADD CONSTRAINT cloud_agent_tasks_lineage_fk
        FOREIGN KEY (lineage_id) REFERENCES cloud_agent_tasks(id) ON DELETE RESTRICT,
    ADD CONSTRAINT cloud_agent_tasks_attempt_positive CHECK (attempt_no > 0),
    ADD CONSTRAINT cloud_agent_tasks_execution_mode_check
        CHECK (execution_mode IN ('original', 'restart', 'resume')),
    ADD CONSTRAINT cloud_agent_tasks_retry_shape_check CHECK (
        (execution_mode = 'original' AND retry_of_task_id IS NULL AND attempt_no = 1) OR
        (execution_mode IN ('restart', 'resume') AND retry_of_task_id IS NOT NULL AND attempt_no > 1)
    ),
    ADD CONSTRAINT cloud_agent_tasks_resume_pointer_check CHECK (
        execution_mode = 'resume' OR resume_checkpoint_id IS NULL
    ),
    ADD CONSTRAINT cloud_agent_tasks_retry_key_check CHECK (
        (retry_of_task_id IS NULL) = (retry_request_key IS NULL)
    );

CREATE UNIQUE INDEX cloud_agent_tasks_lineage_attempt_idx
    ON cloud_agent_tasks(lineage_id, attempt_no);
CREATE UNIQUE INDEX cloud_agent_tasks_retry_idempotency_idx
    ON cloud_agent_tasks(retry_of_task_id, retry_request_key)
    WHERE retry_of_task_id IS NOT NULL;
CREATE INDEX cloud_agent_tasks_lineage_idx
    ON cloud_agent_tasks(lineage_id, attempt_no DESC);

CREATE TABLE cloud_agent_task_checkpoints (
    id UUID PRIMARY KEY,
    task_id UUID NOT NULL REFERENCES cloud_agent_tasks(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL CHECK (seq > 0),
    checkpoint_key TEXT NOT NULL CHECK (char_length(checkpoint_key) BETWEEN 1 AND 128),
    kind TEXT NOT NULL CHECK (kind IN ('progress', 'result')),
    resumable BOOLEAN NOT NULL DEFAULT FALSE,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (task_id, seq),
    UNIQUE (task_id, checkpoint_key),
    CHECK (pg_column_size(payload) <= 1200000)
);

CREATE INDEX cloud_agent_task_checkpoints_task_idx
    ON cloud_agent_task_checkpoints(task_id, seq DESC);
CREATE INDEX cloud_agent_task_checkpoints_resumable_idx
    ON cloud_agent_task_checkpoints(task_id, seq DESC)
    WHERE kind = 'result' AND resumable;

ALTER TABLE cloud_agent_tasks
    ADD CONSTRAINT cloud_agent_tasks_resume_checkpoint_fk
        FOREIGN KEY (resume_checkpoint_id)
        REFERENCES cloud_agent_task_checkpoints(id) ON DELETE SET NULL;
