use crate::{
    AppState, PersistedState, TaskArtifact, TaskEvent, TaskInfo, TaskRollback, broadcast_line,
    download, ensure_provision_workspace, new_task_record, perform_server_action, persist, runtime,
    server_operation_lock,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::Response,
};
use chrono::Local;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::{fs, time::Duration};
use uuid::Uuid;

const MAX_EVENTS: usize = 200;
const MAX_SERVER_LOGS: usize = 1000;
const PROVISION_CANCELLED: &str = "server provisioning cancelled";

pub(crate) fn is_executable_kind(kind: &str) -> bool {
    matches!(
        kind,
        "diagnostic"
            | "server_start"
            | "server_stop"
            | "rollback_server_state"
            | "server_provision"
    )
}

pub(crate) fn normalize_requested_kind(kind: &str) -> Option<&'static str> {
    match kind.trim() {
        "diagnostic" | "repair" => Some("diagnostic"),
        "server_start" | "start" => Some("server_start"),
        "server_stop" | "stop" => Some("server_stop"),
        _ => None,
    }
}

pub(crate) fn reconcile_interrupted_tasks(state: &mut PersistedState) -> bool {
    let now = Local::now().to_rfc3339();
    let mut changed = false;
    for task in &mut state.tasks {
        if task.kind == "bootstrap" {
            task.kind = "server_provision".into();
            if task.status != "completed" {
                task.status = "queued".into();
                task.progress = 0;
                task.started_at = None;
                task.finished_at = None;
                task.error = None;
            }
            task.updated_at = now.clone();
            task.events.push(TaskEvent {
                at: now.clone(),
                level: "warn".into(),
                message: "Legacy bootstrap task migrated to server_provision".into(),
            });
            changed = true;
            continue;
        }
        if task.kind == "server_provision"
            && matches!(task.status.as_str(), "running" | "cancelling")
        {
            task.status = "queued".into();
            task.progress = 0;
            task.started_at = None;
            task.finished_at = None;
            task.error = None;
            task.updated_at = now.clone();
            task.events.push(TaskEvent {
                at: now.clone(),
                level: "warn".into(),
                message: "Interrupted provisioning task safely returned to queue".into(),
            });
            changed = true;
            continue;
        }
        if task.kind == "download" && matches!(task.status.as_str(), "running" | "cancelling") {
            task.status = "interrupted".into();
            task.error = Some(
                "Backend exited during core download; final result requires inspection".into(),
            );
            task.finished_at = Some(now.clone());
            task.updated_at = now.clone();
            task.events.push(TaskEvent {
                at: now.clone(),
                level: "error".into(),
                message: "Core download was interrupted by backend restart".into(),
            });
            changed = true;
            continue;
        }
        if matches!(task.status.as_str(), "running" | "cancelling")
            && is_executable_kind(&task.kind)
        {
            task.status = "interrupted".into();
            task.error = Some("后端在任务执行期间退出；为避免重复副作用，任务未自动重放。".into());
            task.finished_at = Some(now.clone());
            task.updated_at = now.clone();
            task.events.push(TaskEvent {
                at: now.clone(),
                level: "error".into(),
                message: "检测到未正常收尾的执行，已标记为中断。".into(),
            });
            changed = true;
        }
    }
    changed
}

pub(crate) fn resume_queued(state: AppState) {
    tokio::spawn(async move {
        let ids: Vec<Uuid> = state
            .inner
            .read()
            .await
            .tasks
            .iter()
            .filter(|task| task.status == "queued" && is_executable_kind(&task.kind))
            .map(|task| task.id)
            .collect();
        for id in ids {
            spawn(state.clone(), id).await;
        }
    });
}

pub(crate) async fn spawn(state: AppState, id: Uuid) {
    let cancellation = {
        let mut controls = state.task_controls.write().await;
        if controls.contains_key(&id) {
            return;
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        controls.insert(id, cancellation.clone());
        cancellation
    };
    tokio::spawn(async move {
        run_task(state.clone(), id, cancellation).await;
        state.task_controls.write().await.remove(&id);
    });
}

pub(crate) async fn request_cancel(
    state: &AppState,
    id: Uuid,
) -> Result<TaskInfo, (StatusCode, String)> {
    let cancellation = state.task_controls.read().await.get(&id).cloned();
    let mut data = state.inner.write().await;
    let task_index = data
        .tasks
        .iter()
        .position(|task| task.id == id)
        .ok_or((StatusCode::NOT_FOUND, "task not found".into()))?;
    let previous = data.tasks[task_index].clone();
    let task = &mut data.tasks[task_index];
    let mut signal_executor = false;
    let mut settle_cancelled_provision = false;
    match task.status.as_str() {
        "awaiting_approval" | "queued" => {
            settle_cancelled_provision = task.kind == "server_provision";
            task.status = "cancelled".into();
            task.progress = 0;
            task.finished_at = Some(Local::now().to_rfc3339());
            push_event(task, "info", "任务已在执行前取消。")
        }
        "running" => {
            if cancellation.is_none() {
                return Err((
                    StatusCode::CONFLICT,
                    "任务缺少活动执行器，无法确认取消；请刷新任务状态".into(),
                ));
            }
            signal_executor = true;
            task.status = "cancelling".into();
            push_event(task, "warn", "已请求取消；当前安全步骤结束后将执行补偿。")
        }
        "cancelling" => {}
        _ => {
            return Err((
                StatusCode::CONFLICT,
                format!("任务当前状态 {} 不允许取消", task.status),
            ));
        }
    }
    let result = task.clone();
    if let Err(error) = persist(state, &data).await {
        data.tasks[task_index] = previous;
        return Err(crate::internal(error));
    }
    if signal_executor {
        cancellation
            .expect("running task cancellation was checked")
            .store(true, Ordering::Release);
    }
    drop(data);
    if settle_cancelled_provision {
        settle_provision_server(state, &result.server_id, false, Some(PROVISION_CANCELLED)).await;
    }
    Ok(result)
}

pub(crate) async fn schedule_rollback(
    state: &AppState,
    id: Uuid,
) -> Result<TaskInfo, (StatusCode, String)> {
    let mut data = state.inner.write().await;
    let original_index = data
        .tasks
        .iter()
        .position(|task| task.id == id)
        .ok_or((StatusCode::NOT_FOUND, "task not found".into()))?;
    let previous_tasks = data.tasks.clone();
    let original = data.tasks[original_index].clone();
    if original.status != "completed" {
        return Err((StatusCode::CONFLICT, "仅已完成任务可以手动回滚".into()));
    }
    let rollback = original
        .rollback
        .as_ref()
        .filter(|rollback| rollback.status == "available")
        .ok_or((StatusCode::CONFLICT, "该任务没有可用的补偿操作".into()))?;
    let mut task = new_task_record(
        original.server_id.clone(),
        format!("回滚：{}", original.title),
        "rollback_server_state".into(),
        "queued".into(),
        0,
        "high".into(),
        Some("user".into()),
    );
    task.parent_task_id = Some(original.id);
    task.rollback = Some(TaskRollback {
        status: "planned".into(),
        previous_server_status: rollback.previous_server_status.clone(),
        summary: Some("恢复到原任务执行前的服务器运行状态。".into()),
    });
    push_event(&mut task, "info", "用户已确认创建补偿任务。");
    if let Some(original_rollback) = data.tasks[original_index].rollback.as_mut() {
        original_rollback.status = "scheduled".into();
        original_rollback.summary = Some(format!("补偿任务 {} 已创建", task.id));
    }
    data.tasks[original_index].updated_at = Local::now().to_rfc3339();
    data.tasks.insert(0, task.clone());
    if let Err(error) = persist(state, &data).await {
        data.tasks = previous_tasks;
        return Err(crate::internal(error));
    }
    drop(data);
    spawn(state.clone(), task.id).await;
    Ok(task)
}

async fn run_task(state: AppState, id: Uuid, cancellation: Arc<AtomicBool>) {
    let task = match claim_task(&state, id).await {
        Ok(Some(task)) => task,
        Ok(None) => return,
        Err(error) => {
            eprintln!("[task {id}] 无法领取任务：{error}");
            return;
        }
    };
    let previous_status = state
        .inner
        .read()
        .await
        .servers
        .iter()
        .find(|server| server.id == task.server_id)
        .map(|server| server.status.clone());
    let Some(previous_status) = previous_status else {
        finish_failed(&state, id, "目标服务器不存在", None).await;
        return;
    };

    if matches!(task.kind.as_str(), "server_start" | "server_stop") {
        if let Err(error) = prepare_compensation(&state, id, &previous_status).await {
            finish_failed(&state, id, &error, None).await;
            return;
        }
    }
    let outcome = match task.kind.as_str() {
        "diagnostic" => execute_diagnostic(&state, &task, &cancellation).await,
        "server_start" => execute_server_target(&state, &task, true, &cancellation).await,
        "server_stop" => execute_server_target(&state, &task, false, &cancellation).await,
        "rollback_server_state" => execute_scheduled_rollback(&state, &task, &cancellation).await,
        "server_provision" => execute_server_provision(&state, &task, &cancellation).await,
        _ => Err(format!("任务类型 {} 未接入执行器", task.kind)),
    };
    match outcome {
        Ok(summary) if cancellation.load(Ordering::Acquire) => {
            if task.kind == "server_provision" {
                settle_provision_server(&state, &task.server_id, false, Some(PROVISION_CANCELLED))
                    .await;
            }
            let rollback = if task_changes_server(&task.kind) {
                compensate_to(&state, &task.server_id, &previous_status).await
            } else {
                "无需补偿：只读任务未修改服务器状态".into()
            };
            finish_cancelled(&state, id, &summary, rollback).await;
        }
        Ok(summary) => {
            if let Err(error) = write_execution_report(&state, id, &summary).await {
                let rollback = if task_changes_server(&task.kind) {
                    Some(compensate_to(&state, &task.server_id, &previous_status).await)
                } else {
                    None
                };
                finish_failed(&state, id, &format!("审计产物写入失败：{error}"), rollback).await;
            } else {
                finish_completed(&state, id, &summary).await;
            }
        }
        Err(error) if task.kind == "server_provision" && error == PROVISION_CANCELLED => {
            settle_provision_server(&state, &task.server_id, false, Some(PROVISION_CANCELLED))
                .await;
            finish_cancelled(
                &state,
                id,
                PROVISION_CANCELLED,
                "No runtime state compensation was required".into(),
            )
            .await;
        }
        Err(error) => {
            if task.kind == "server_provision" {
                settle_provision_server(&state, &task.server_id, false, Some(&error)).await;
            }
            let rollback = if matches!(task.kind.as_str(), "server_start" | "server_stop") {
                Some(compensate_to(&state, &task.server_id, &previous_status).await)
            } else if task.kind == "rollback_server_state" && error.starts_with("补偿失败") {
                Some(error.clone())
            } else {
                None
            };
            finish_failed(&state, id, &error, rollback).await;
        }
    }
}

fn task_changes_server(kind: &str) -> bool {
    matches!(
        kind,
        "server_start" | "server_stop" | "rollback_server_state"
    )
}

async fn claim_task(state: &AppState, id: Uuid) -> Result<Option<TaskInfo>, String> {
    let mut data = state.inner.write().await;
    let Some(task) = data.tasks.iter_mut().find(|task| task.id == id) else {
        return Ok(None);
    };
    if task.status != "queued" || !is_executable_kind(&task.kind) {
        return Ok(None);
    }
    task.status = "running".into();
    task.progress = 5;
    task.started_at = Some(Local::now().to_rfc3339());
    task.finished_at = None;
    task.error = None;
    push_event(task, "info", "执行器已领取任务。");
    let result = task.clone();
    if let Err(error) = persist(state, &data).await {
        if let Some(task) = data.tasks.iter_mut().find(|task| task.id == id) {
            task.status = "queued".into();
            task.progress = 0;
            task.started_at = None;
            task.events.pop();
        }
        return Err(error);
    }
    Ok(Some(result))
}

async fn prepare_compensation(
    state: &AppState,
    id: Uuid,
    previous_status: &str,
) -> Result<(), String> {
    if !matches!(previous_status, "online" | "stopped") {
        return Err(format!("服务器当前状态 {previous_status} 不允许安全执行"));
    }
    update_task(state, id, |task| {
        task.rollback = Some(TaskRollback {
            status: "prepared".into(),
            previous_server_status: previous_status.into(),
            summary: Some("已记录执行前的服务器运行状态。".into()),
        });
        push_event(
            task,
            "info",
            &format!("执行前状态已记录：{previous_status}"),
        );
    })
    .await
    .map(|_| ())
}

async fn execute_server_provision(
    state: &AppState,
    task: &TaskInfo,
    cancellation: &Arc<AtomicBool>,
) -> Result<String, String> {
    let operation = server_operation_lock(state, &task.server_id).await;
    let _guard = operation.lock().await;
    if cancellation.load(Ordering::Acquire) {
        return Err(PROVISION_CANCELLED.into());
    }
    record_event(
        state,
        task.id,
        "info",
        "Checking server workspace and EULA",
        10,
    )
    .await?;
    ensure_provision_workspace(state, &task.server_id).await?;
    if cancellation.load(Ordering::Acquire) {
        return Err(PROVISION_CANCELLED.into());
    }
    record_event(
        state,
        task.id,
        "info",
        "Resolving and verifying the server core",
        20,
    )
    .await?;
    match download::provision_core(state, &task.server_id, task.id, cancellation.clone()).await? {
        download::ProvisionCoreOutcome::Cancelled => return Err(PROVISION_CANCELLED.into()),
        download::ProvisionCoreOutcome::AlreadyReady => {
            record_event(
                state,
                task.id,
                "info",
                "Existing atomically installed server.jar reused",
                85,
            )
            .await?;
        }
        download::ProvisionCoreOutcome::Installed => {
            record_event(
                state,
                task.id,
                "info",
                "Server core downloaded, verified, and installed",
                85,
            )
            .await?;
        }
    }
    if cancellation.load(Ordering::Acquire) {
        return Err(PROVISION_CANCELLED.into());
    }
    record_event(state, task.id, "info", "Checking the Java runtime", 90).await?;
    let java = runtime::detect_java(&runtime::data_root()).await;
    if !java.java_installed {
        return Err(format!(
            "Java is not installed; install managed Java {} and retry provisioning",
            runtime::RECOMMENDED_JAVA
        ));
    }
    if !java.java_compatible {
        return Err(format!(
            "Detected Java {:?}, but Java {} or newer is required",
            java.java_major,
            runtime::RECOMMENDED_JAVA
        ));
    }
    if cancellation.load(Ordering::Acquire) {
        return Err(PROVISION_CANCELLED.into());
    }
    settle_provision_server(state, &task.server_id, true, None).await;
    record_event(
        state,
        task.id,
        "info",
        "Server provisioning completed and the server is ready to start",
        98,
    )
    .await?;
    Ok("Server workspace, core, EULA, and Java runtime are ready".into())
}

async fn settle_provision_server(
    state: &AppState,
    server_id: &str,
    ready: bool,
    error: Option<&str>,
) {
    if error == Some(PROVISION_CANCELLED) {
        let _ = fs::remove_file(runtime::server_directory(server_id).join("server.jar.part")).await;
    }
    let core_ready = fs::metadata(runtime::server_directory(server_id).join("server.jar"))
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
    let mut data = state.inner.write().await;
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == server_id)
    {
        server.operation_state = "idle".into();
        server.core_ready = core_ready;
        server.last_error = error.map(str::to_string);
        server.task = if ready {
            "Ready to start".into()
        } else if error == Some(PROVISION_CANCELLED) {
            "Provisioning cancelled".into()
        } else {
            "Provisioning failed".into()
        };
    }
    if let Err(persist_error) = persist(state, &data).await {
        eprintln!("failed to persist provision state for {server_id}: {persist_error}");
    }
}

async fn execute_server_target(
    state: &AppState,
    task: &TaskInfo,
    online: bool,
    cancellation: &AtomicBool,
) -> Result<String, String> {
    if cancellation.load(Ordering::Acquire) {
        return Ok("任务在服务器操作前收到取消请求。".into());
    }
    record_event(
        state,
        task.id,
        "info",
        if online {
            "正在启动服务器。"
        } else {
            "正在安全停止服务器。"
        },
        25,
    )
    .await?;
    let action = if online { "start" } else { "stop" };
    let _ = perform_server_action(state.clone(), task.server_id.clone(), action)
        .await
        .map_err(|(_, error)| error)?;
    if online {
        wait_for_server_online(state, &task.server_id, cancellation).await?;
    }
    let current = state
        .inner
        .read()
        .await
        .servers
        .iter()
        .find(|server| server.id == task.server_id)
        .map(|server| server.status.clone())
        .ok_or_else(|| "目标服务器不存在".to_string())?;
    let expected = if online { "online" } else { "stopped" };
    if current != expected {
        return Err(format!("操作结束后状态为 {current}，预期 {expected}"));
    }
    record_event(
        state,
        task.id,
        "info",
        &format!("服务器已达到目标状态：{expected}"),
        90,
    )
    .await?;
    Ok(if online {
        "服务器已启动并通过就绪标记确认。"
    } else {
        "服务器进程已安全退出。"
    }
    .into())
}

async fn wait_for_server_online(
    state: &AppState,
    server_id: &str,
    cancellation: &AtomicBool,
) -> Result<(), String> {
    for _ in 0..120 {
        if cancellation.load(Ordering::Acquire) {
            return Ok(());
        }
        let status = {
            let data = state.inner.read().await;
            data.servers
                .iter()
                .find(|server| server.id == server_id)
                .map(|server| server.status.clone())
                .ok_or_else(|| "目标服务器不存在".to_string())?
        };
        let managed = state.processes.read().await.contains_key(server_id);
        if status == "online" {
            return Ok(());
        }
        if !managed {
            return Err(format!("服务器进程在就绪前退出，当前状态 {status}"));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err("等待服务器就绪超时".into())
}

async fn execute_diagnostic(
    state: &AppState,
    task: &TaskInfo,
    cancellation: &AtomicBool,
) -> Result<String, String> {
    record_event(
        state,
        task.id,
        "info",
        "正在读取最近 500 条服务器日志。",
        20,
    )
    .await?;
    if cancellation.load(Ordering::Acquire) {
        return Ok("诊断在分析前收到取消请求。".into());
    }
    let lines = state
        .inner
        .read()
        .await
        .logs
        .get(&task.server_id)
        .cloned()
        .unwrap_or_default();
    let recent: Vec<String> = lines
        .into_iter()
        .rev()
        .take(500)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let errors: Vec<&String> = recent
        .iter()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("error") || lower.contains("exception") || lower.contains("failed")
        })
        .collect();
    let warnings = recent
        .iter()
        .filter(|line| line.to_ascii_lowercase().contains("warn"))
        .count();
    let report = format!(
        "# 服务器日志诊断\n\n- 服务器：{}\n- 分析日志：{} 条\n- 错误特征：{} 条\n- 警告特征：{} 条\n- 生成时间：{}\n\n## 最近的错误特征\n\n{}\n",
        task.server_id,
        recent.len(),
        errors.len(),
        warnings,
        Local::now().to_rfc3339(),
        if errors.is_empty() {
            "未在最近日志中发现 ERROR、Exception 或 failed 特征。".into()
        } else {
            errors
                .iter()
                .rev()
                .take(30)
                .rev()
                .map(|line| format!("- `{}`", line.replace('`', "'")))
                .collect::<Vec<_>>()
                .join("\n")
        }
    );
    write_artifact(
        state,
        task.id,
        "diagnostic.md",
        "diagnostic",
        report.as_bytes(),
    )
    .await?;
    record_event(state, task.id, "info", "诊断报告已生成。", 90).await?;
    Ok(format!(
        "已分析 {} 条日志，发现 {} 条错误特征和 {} 条警告特征。",
        recent.len(),
        errors.len(),
        warnings
    ))
}

async fn execute_scheduled_rollback(
    state: &AppState,
    task: &TaskInfo,
    cancellation: &AtomicBool,
) -> Result<String, String> {
    let target = task
        .rollback
        .as_ref()
        .map(|rollback| rollback.previous_server_status.as_str())
        .ok_or_else(|| "补偿任务缺少目标状态".to_string())?;
    if cancellation.load(Ordering::Acquire) {
        return Ok("补偿任务在执行前收到取消请求。".into());
    }
    let result = compensate_to(state, &task.server_id, target).await;
    if result.starts_with("补偿失败") {
        Err(result)
    } else {
        record_event(state, task.id, "info", &result, 90).await?;
        Ok(result)
    }
}

async fn compensate_to(state: &AppState, server_id: &str, target: &str) -> String {
    let current = state
        .inner
        .read()
        .await
        .servers
        .iter()
        .find(|server| server.id == server_id)
        .map(|server| server.status.clone());
    let Some(current) = current else {
        return "补偿失败：目标服务器不存在".into();
    };
    if current == target {
        return format!("无需补偿：服务器已经处于 {target} 状态");
    }
    let action = match target {
        "online" => "start",
        "stopped" => "stop",
        _ => return format!("补偿失败：不支持恢复到 {target} 状态"),
    };
    if let Err((_, error)) =
        perform_server_action(state.clone(), server_id.to_string(), action).await
    {
        return format!("补偿失败：{error}");
    }
    if target == "online" {
        let never_cancel = AtomicBool::new(false);
        if let Err(error) = wait_for_server_online(state, server_id, &never_cancel).await {
            return format!("补偿失败：{error}");
        }
    }
    format!("补偿完成：服务器已恢复到 {target} 状态")
}

async fn write_execution_report(state: &AppState, id: Uuid, summary: &str) -> Result<(), String> {
    let task = state
        .inner
        .read()
        .await
        .tasks
        .iter()
        .find(|task| task.id == id)
        .cloned()
        .ok_or_else(|| "task not found".to_string())?;
    let bytes = serde_json::to_vec_pretty(&json!({
        "task_id": task.id,
        "server_id": task.server_id,
        "kind": task.kind,
        "started_at": task.started_at,
        "finished_at": Local::now().to_rfc3339(),
        "summary": summary,
        "events": task.events,
        "rollback": task.rollback,
    }))
    .map_err(|error| error.to_string())?;
    write_artifact(state, id, "execution.json", "audit", &bytes).await
}

async fn write_artifact(
    state: &AppState,
    task_id: Uuid,
    name: &str,
    kind: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let directory = runtime::data_root().join("tasks").join(task_id.to_string());
    fs::create_dir_all(&directory)
        .await
        .map_err(|error| error.to_string())?;
    let target = directory.join(name);
    let temp = directory.join(format!(".{name}.{}.tmp", Uuid::new_v4().simple()));
    fs::write(&temp, bytes)
        .await
        .map_err(|error| error.to_string())?;
    fs::rename(&temp, &target)
        .await
        .map_err(|error| error.to_string())?;
    let relative_path = format!("tasks/{task_id}/{name}");
    let artifact = TaskArtifact {
        id: Uuid::new_v4().simple().to_string(),
        name: name.into(),
        kind: kind.into(),
        size: bytes.len() as u64,
        created_at: Local::now().to_rfc3339(),
        relative_path,
    };
    if let Err(error) = update_task(state, task_id, |task| task.artifacts.push(artifact)).await {
        let _ = fs::remove_file(&target).await;
        return Err(error);
    }
    Ok(())
}

async fn finish_completed(state: &AppState, id: Uuid, summary: &str) {
    commit_terminal(state, id, "完成", |task| mark_completed(task, summary)).await;
}

fn mark_completed(task: &mut TaskInfo, summary: &str) {
    task.status = "completed".into();
    task.progress = 100;
    task.summary = Some(summary.into());
    task.error = None;
    task.finished_at = Some(Local::now().to_rfc3339());
    let is_compensation = task.kind == "rollback_server_state";
    if let Some(rollback) = task.rollback.as_mut() {
        if is_compensation {
            rollback.status = "completed".into();
            rollback.summary = Some("服务器已恢复到补偿任务指定的状态。".into());
        } else {
            rollback.status = "available".into();
            rollback.summary = Some("可创建补偿任务恢复执行前的服务器状态。".into());
        }
    }
    push_event(task, "info", "任务执行完成。")
}

async fn finish_failed(state: &AppState, id: Uuid, error: &str, rollback: Option<String>) {
    let committed = commit_terminal(state, id, "失败", |task| {
        task.status = if rollback
            .as_deref()
            .is_some_and(|value| value.starts_with("补偿失败"))
        {
            "rollback_failed".into()
        } else {
            "failed".into()
        };
        task.error = Some(error.into());
        task.finished_at = Some(Local::now().to_rfc3339());
        if let (Some(metadata), Some(summary)) = (task.rollback.as_mut(), rollback.as_ref()) {
            metadata.status = if summary.starts_with("补偿失败") {
                "failed"
            } else {
                "completed"
            }
            .into();
            metadata.summary = Some(summary.clone());
        }
        push_event(task, "error", error)
    })
    .await;
    if committed
        && let Err(report_error) =
            write_execution_report(state, id, &format!("执行失败：{error}")).await
    {
        eprintln!("[task {id}] 失败审计产物写入失败：{report_error}");
    }
}

async fn finish_cancelled(state: &AppState, id: Uuid, summary: &str, rollback: String) {
    let committed = commit_terminal(state, id, "取消", |task| {
        task.status = if rollback.starts_with("补偿失败") {
            "rollback_failed"
        } else {
            "cancelled"
        }
        .into();
        task.summary = Some(summary.into());
        task.finished_at = Some(Local::now().to_rfc3339());
        if let Some(metadata) = task.rollback.as_mut() {
            metadata.status = if rollback.starts_with("补偿失败") {
                "failed"
            } else {
                "completed"
            }
            .into();
            metadata.summary = Some(rollback.clone());
        }
        push_event(task, "warn", "任务已取消并完成安全收尾。")
    })
    .await;
    if committed
        && let Err(error) =
            write_execution_report(state, id, &format!("任务已取消：{summary}")).await
    {
        eprintln!("[task {id}] 取消审计产物写入失败：{error}");
    }
}

async fn commit_terminal<F>(state: &AppState, id: Uuid, transition: &str, update: F) -> bool
where
    F: Fn(&mut TaskInfo),
{
    for attempt in 1..=3 {
        match update_task(state, id, |task| update(task)).await {
            Ok(_) => return true,
            Err(error) => {
                eprintln!("[task {id}] {transition}终态持久化失败（第 {attempt} 次）：{error}");
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_millis(100 * attempt)).await;
                }
            }
        }
    }
    false
}

async fn record_event(
    state: &AppState,
    id: Uuid,
    level: &str,
    message: &str,
    progress: u8,
) -> Result<(), String> {
    let (server_id, line) = {
        let mut data = state.inner.write().await;
        let task_index = data
            .tasks
            .iter()
            .position(|task| task.id == id)
            .ok_or_else(|| "task not found".to_string())?;
        let previous_task = data.tasks[task_index].clone();
        let task = &mut data.tasks[task_index];
        task.progress = progress;
        push_event(task, level, message);
        let server_id = task.server_id.clone();
        let line = format!(
            "[{} TASK {}]: {}",
            Local::now().format("%H:%M:%S"),
            id,
            message
        );
        let previous_logs = data.logs.get(&server_id).cloned();
        let logs = data.logs.entry(server_id.clone()).or_default();
        logs.push(line.clone());
        if logs.len() > MAX_SERVER_LOGS {
            logs.drain(..logs.len() - MAX_SERVER_LOGS);
        }
        if let Err(error) = persist(state, &data).await {
            data.tasks[task_index] = previous_task;
            if let Some(previous_logs) = previous_logs {
                data.logs.insert(server_id, previous_logs);
            } else {
                data.logs.remove(&server_id);
            }
            return Err(error);
        }
        (server_id, line)
    };
    broadcast_line(state, &server_id, &line).await;
    Ok(())
}

async fn update_task<F>(state: &AppState, id: Uuid, update: F) -> Result<TaskInfo, String>
where
    F: FnOnce(&mut TaskInfo),
{
    let mut data = state.inner.write().await;
    let index = data
        .tasks
        .iter()
        .position(|task| task.id == id)
        .ok_or_else(|| "task not found".to_string())?;
    let previous = data.tasks[index].clone();
    let task = &mut data.tasks[index];
    update(task);
    task.updated_at = Local::now().to_rfc3339();
    let result = task.clone();
    if let Err(error) = persist(state, &data).await {
        data.tasks[index] = previous;
        return Err(error);
    }
    Ok(result)
}

fn push_event(task: &mut TaskInfo, level: &str, message: &str) {
    task.updated_at = Local::now().to_rfc3339();
    task.events.push(TaskEvent {
        at: task.updated_at.clone(),
        level: level.into(),
        message: message.into(),
    });
    if task.events.len() > MAX_EVENTS {
        task.events.drain(..task.events.len() - MAX_EVENTS);
    }
}

pub(crate) async fn get_artifact(
    Path((id, artifact_id)): Path<(Uuid, String)>,
    State(state): State<AppState>,
) -> Result<Response, (StatusCode, String)> {
    let artifact = state
        .inner
        .read()
        .await
        .tasks
        .iter()
        .find(|task| task.id == id)
        .and_then(|task| {
            task.artifacts
                .iter()
                .find(|artifact| artifact.id == artifact_id)
        })
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "artifact not found".into()))?;
    let root = fs::canonicalize(runtime::data_root())
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let target = fs::canonicalize(runtime::data_root().join(&artifact.relative_path))
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "artifact file not found".into()))?;
    if !target.starts_with(&root) {
        return Err((StatusCode::FORBIDDEN, "invalid artifact path".into()));
    }
    let bytes = fs::read(target)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            if artifact.name.ends_with(".json") {
                "application/json; charset=utf-8"
            } else {
                "text/markdown; charset=utf-8"
            },
        )
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "attachment; filename=\"{}\"",
                artifact.name.replace('"', "")
            ),
        )
        .body(Body::from(bytes))
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_structured_safe_kinds_are_executable() {
        assert_eq!(normalize_requested_kind("start"), Some("server_start"));
        assert_eq!(normalize_requested_kind("repair"), Some("diagnostic"));
        assert_eq!(normalize_requested_kind("economy"), None);
        assert!(!is_executable_kind("general"));
        assert!(is_executable_kind("server_provision"));
    }

    #[test]
    fn restart_reconciliation_marks_inflight_tasks_interrupted() {
        let mut state = crate::initial_state();
        let mut task = new_task_record(
            "sculk".into(),
            "启动".into(),
            "server_start".into(),
            "running".into(),
            20,
            "medium".into(),
            Some("user".into()),
        );
        task.started_at = Some(Local::now().to_rfc3339());
        state.tasks.push(task);
        assert!(reconcile_interrupted_tasks(&mut state));
        assert_eq!(state.tasks[0].status, "interrupted");
        assert!(state.tasks[0].error.is_some());
    }

    #[test]
    fn restart_reconciliation_requeues_only_recoverable_provisioning() {
        let mut state = crate::initial_state();
        let mut provision = new_task_record(
            "sculk".into(),
            "初始化".into(),
            "server_provision".into(),
            "running".into(),
            72,
            "low".into(),
            None,
        );
        provision.started_at = Some(Local::now().to_rfc3339());
        let bootstrap = new_task_record(
            "legacy".into(),
            "旧初始化".into(),
            "bootstrap".into(),
            "cancelling".into(),
            30,
            "low".into(),
            None,
        );
        state.tasks.extend([provision, bootstrap]);

        assert!(reconcile_interrupted_tasks(&mut state));
        for task in &state.tasks {
            assert_eq!(task.kind, "server_provision");
            assert_eq!(task.status, "queued");
            assert_eq!(task.progress, 0);
            assert!(task.started_at.is_none());
        }
    }

    #[test]
    fn completed_compensation_does_not_offer_the_same_rollback_again() {
        let mut task = new_task_record(
            "sculk".into(),
            "恢复运行状态".into(),
            "rollback_server_state".into(),
            "running".into(),
            90,
            "high".into(),
            Some("user".into()),
        );
        task.rollback = Some(TaskRollback {
            status: "planned".into(),
            previous_server_status: "online".into(),
            summary: None,
        });

        mark_completed(&mut task, "补偿完成");

        assert_eq!(task.status, "completed");
        assert_eq!(task.rollback.as_ref().unwrap().status, "completed");
    }

    #[test]
    fn diagnostic_is_not_treated_as_a_server_mutation() {
        assert!(!task_changes_server("diagnostic"));
        assert!(task_changes_server("server_start"));
        assert!(task_changes_server("server_stop"));
        assert!(task_changes_server("rollback_server_state"));
    }
}
