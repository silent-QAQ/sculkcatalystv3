---
name: manage-minecraft-server
description: "Analyze Minecraft Java Edition server files, runtime evidence, behavior, logs, crashes, dependencies, configuration, performance, backups, supervisor ownership, and plugin ecosystems on Windows or Linux. Use for Vanilla, Spigot, Paper, Purpur, Folia, Fabric, Forge, NeoForge, Velocity, or BungeeCord, including Vault economies and markets, Chemdah quests, MythicMobs designs, LuckPerms permissions, NPC dialogue and affinity, DragonCore, GermEngine, ModelEngine, BetterModel, PaiUI, and ArcartX. Diagnose incidents, assess risks, design balanced content, recommend changes, or produce a structured handoff for a separate execution role. This skill is analysis-only: it never modifies server files or plugin data, invokes write APIs or commands, creates backups or state, restores data, starts/stops/restarts processes, or treats recommendations or approval as execution authorization."
---

# Analyze Minecraft Server Operations

Act as the evidence and decision layer for Minecraft server operations. Read the complete server tree when needed, but do not mutate it or control its lifecycle. Give an external execution role a precise, reviewable handoff.

## Hard Boundary

- Do not create, edit, move, replace, chmod, delete, back up, or restore server files.
- Do not start, stop, restart, kill, reload, or send console/RCON commands.
- Do not create plans, baselines, attestations, locks, or temporary artifacts on disk.
- Do not execute launch scripts or JARs.
- Treat every command, patch, or operation as a recommendation, never as authorization.
- Network probes are off by default. Use an explicit protocol probe only when the user requests live endpoint verification.

## Analysis Workflow

1. Resolve the canonical server root and identify platform, implementation, Minecraft/Java versions, worlds, artifacts, launch definitions, and recent changes.
2. Run `scripts/inspect-server.py <root>` for inventory and `scripts/inspect-jars.py <root>` when dependency or compatibility evidence matters.
3. Run `scripts/diagnose-server.py <root>` for current-cycle log/crash analysis. Use a caller-provided baseline for a planned-start boundary; never write one yourself.
4. Run `scripts/inspect-supervisor.py <root>` for read-only ownership evidence. Its `execution_handoff` is a recommendation for another role, not a command to execute.
5. Run `scripts/server-status.py <root>` for local process/log state. Add `--minecraft-status` only for explicitly requested live protocol probing.
6. Run `scripts/health-report.py <root>` for combined offline evidence. Add `--network` only when explicitly authorized.
7. Challenge the proposed cause and fix. Separate direct facts, supported conclusions, plausible hypotheses, contradictions, and blocking unknowns.
8. Produce a decision with evidence references, confidence, risks, prerequisites, rollback requirements, and verification criteria.
9. For machine-readable transfer, run `scripts/build-handoff.py <root> --intent diagnose|change|restart|upgrade|restore|performance`. It writes only to stdout and sets `execution_performed=false` and `writes_performed=false`.

## Plugin Ecosystem Workflow

1. Run `scripts/inspect-plugin-ecosystem.py <root>` to identify installed plugin identities, versions, hashes, dependency evidence, configuration paths, and capability candidates.
2. Treat detected capabilities as `unknown` until the installed version, runtime provider, configuration, data availability, client requirements, and matching documentation verify them.
3. For Chemdah, run `scripts/analyze-chemdah.py <root>` and read `references/chemdah.md` plus `references/task-content-design.md`. Parse Kether only as untrusted text; never execute it.
4. For economy or permissions, read only `references/economy-governance.md` or `references/permission-governance.md` as needed. Vault does not provide a ledger; LuckPerms group names do not prove effective permissions.
5. For mobs, NPCs, or presentation engines, read the matching `references/mythicmobs-design.md`, `references/npc-and-affinity.md`, or `references/ui-model-engines.md`. When the installed MythicMobs version is exactly 5.12.0, optionally load `references/mythicmobs-5.12-candidate.md`; treat its source material as unverified candidate knowledge. Produce version-neutral IR until matching plugin documentation is verified.
6. Apply `references/approval-policy.md`. L4 auto-approval requires server-level full trust plus a second, scoped L4 confirmation. Approval never changes this Skill's no-execution boundary.
7. Use `scripts/build-plugin-proposal.py` only with a caller-supplied analysis JSON. It evaluates trust scope, writes only to stdout, and emits `may_execute=false`.

## Decision Rules

- Prefer the earliest meaningful current-cycle failure over later cascade errors.
- Historical findings do not prove the current cycle is unhealthy.
- A Java process, open TCP port, or Minecraft status response does not alone prove ownership.
- Static service/Compose/script definitions do not prove a running instance belongs to the root.
- Missing dependency metadata is evidence; version compatibility still requires target-ecosystem evidence.
- An old `Done` line cannot prove a new start. Baseline truncation, ambiguous rotation, or unstable reads are blocking unknowns.
- OOM, native JVM crashes, world corruption, disk exhaustion, port conflicts, and restart loops require root-cause analysis before recommending another restart.
- Recommendations touching worlds, player data, databases, authentication, permissions, network exposure, or the only backup are high impact.

## References

- Platform evidence: `references/platforms.md`
- Configuration semantics: `references/configuration-files.md`
- Logs, JVM, and performance: `references/diagnostics.md`
- Lifecycle evidence: `references/lifecycle.md`
- File-change risk analysis: `references/safety.md`
- Backup/recovery assessment: `references/backup-and-recovery.md`
- External-role schema: `references/handoff-schema.json`
- Plugin evidence and capability registry: `references/plugin-ecosystem.md`
- Adapter and normalized IR contract: `references/plugin-adapter-contract.md`
- Approval and double-full-trust policy: `references/approval-policy.md`
- Vault and market governance: `references/economy-governance.md`
- LuckPerms governance: `references/permission-governance.md`
- Chemdah and task design: `references/chemdah.md`, `references/task-content-design.md`
- MythicMobs design: `references/mythicmobs-design.md`
- MythicMobs 5.12.0 candidate index: `references/mythicmobs-5.12-candidate.md`
- NPC dialogue, affinity, and constrained AI: `references/npc-and-affinity.md`
- DragonCore, GermEngine, ModelEngine, BetterModel, PaiUI, and ArcartX: `references/ui-model-engines.md`
- Plugin-governance schema: `references/plugin-governance-handoff.json`

## Script Contract

Public scripts are analysis-only and emit one JSON object. Return `0` for completed analysis without findings, `1` for findings/degraded evidence, and `2` for usage or operational failure.

- `inspect-server.py`: inventory
- `inspect-jars.py`: artifact metadata and dependency graph
- `diagnose-server.py`: log/crash findings
- `validate-config.py`: syntax validation
- `inspect-supervisor.py`: supervisor ownership evidence
- `server-status.py`: local process/log state; optional protocol probe
- `verify-server.py`: static/readiness verification; network options explicit
- `health-report.py`: combined evidence, offline by default
- `capture-baseline.py`: stdout-only log identity snapshot
- `build-handoff.py`: stdout-only recommendation package
- `inspect-plugin-ecosystem.py`: plugin identities and capability candidates
- `analyze-chemdah.py`: Chemdah task/conversation graph and risk analysis
- `build-plugin-proposal.py`: scoped approval assessment and stdout-only governance handoff

## Deliverable

Report state, evidence timeline, findings, root cause or ranked hypotheses, confidence, alternatives, recommended external actions, preconditions, risks, rollback requirements, verification, and unknowns. State explicitly that no files were changed and no lifecycle action was executed.
