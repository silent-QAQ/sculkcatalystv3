export interface WorkspaceSnapshotItem {
  id: string
}

export interface TaskSnapshotItem {
  server_id: string
}

export function filterDeletedWorkspaceSnapshot<
  Server extends WorkspaceSnapshotItem,
  Task extends TaskSnapshotItem,
  Telemetry,
>(
  servers: readonly Server[],
  tasks: readonly Task[],
  telemetry: Readonly<Record<string, Telemetry>>,
  deletedIds: ReadonlySet<string>,
) {
  const suppressedIds = new Set(deletedIds)
  return {
    servers: servers.filter(item => !suppressedIds.has(item.id)),
    tasks: tasks.filter(item => !suppressedIds.has(item.server_id)),
    telemetry: Object.fromEntries(
      Object.entries(telemetry).filter(([id]) => !suppressedIds.has(id)),
    ) as Record<string, Telemetry>,
    acknowledgedIds: [...suppressedIds].filter(
      id => !servers.some(item => item.id === id),
    ),
  }
}
