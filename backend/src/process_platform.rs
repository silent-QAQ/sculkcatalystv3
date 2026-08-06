use std::io;
use tokio::process::{Child, Command};

#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};

#[cfg(windows)]
use std::mem::size_of;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
    },
};

#[derive(Debug)]
pub(crate) struct ProcessGuard {
    #[cfg(windows)]
    job: HANDLE,
    #[cfg(unix)]
    process_group: AtomicI32,
}

#[cfg(windows)]
// Windows kernel handles are process-wide and can be closed from the actor thread.
unsafe impl Send for ProcessGuard {}

#[cfg(windows)]
unsafe impl Sync for ProcessGuard {}

#[cfg(windows)]
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        if !self.job.is_null() {
            unsafe {
                // KILL_ON_JOB_CLOSE is the final backstop, but terminating first makes the
                // intended tree shutdown synchronous even when this drop happens during abort.
                let _ = TerminateJobObject(self.job, 1);
                CloseHandle(self.job);
            }
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let process_group = self.process_group.swap(0, Ordering::AcqRel);
        if process_group > 0 {
            let _ = signal_process_group(process_group, libc::SIGKILL);
        }
    }
}

#[cfg(all(not(windows), not(unix)))]
impl Drop for ProcessGuard {
    fn drop(&mut self) {}
}

pub(crate) fn create_process_guard() -> io::Result<ProcessGuard> {
    #[cfg(windows)]
    {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            unsafe {
                CloseHandle(job);
            }
            return Err(io::Error::last_os_error());
        }
        Ok(ProcessGuard { job })
    }

    #[cfg(unix)]
    {
        Ok(ProcessGuard {
            process_group: AtomicI32::new(0),
        })
    }

    #[cfg(all(not(windows), not(unix)))]
    {
        Ok(ProcessGuard {})
    }
}

pub(crate) fn bind_process_to_guard(guard: &ProcessGuard, pid: u32) -> io::Result<()> {
    #[cfg(windows)]
    {
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let assigned = unsafe { AssignProcessToJobObject(guard.job, process) };
        unsafe {
            CloseHandle(process);
        }
        if assigned == 0 {
            return Err(io::Error::last_os_error());
        }
    }

    #[cfg(unix)]
    {
        guard
            .process_group
            .store(process_group_from_pid(pid)?, Ordering::Release);
    }

    #[cfg(all(not(windows), not(unix)))]
    {
        let _ = (guard, pid);
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn configure_managed_command(command: &mut Command) -> io::Result<()> {
    command.process_group(0);

    #[cfg(target_os = "linux")]
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() == 1 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "backend exited before the managed Java process was started",
                ));
            }
            Ok(())
        });
    }

    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn configure_managed_command(_command: &mut Command) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn terminate_job(guard: &ProcessGuard) -> io::Result<()> {
    let result = unsafe { TerminateJobObject(guard.job, 1) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn start_kill_tree(
    child: &mut Child,
    pid: u32,
    guard: Option<&ProcessGuard>,
) -> io::Result<()> {
    let result = match signal_process_group(process_group_from_pid(pid)?, libc::SIGKILL) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => child.start_kill(),
        Err(error) => Err(error),
    };
    if result.is_ok()
        && let Some(guard) = guard
    {
        guard.disarm_process_group(pid);
    }
    result
}

#[cfg(not(unix))]
pub(crate) fn start_kill_tree(
    child: &mut Child,
    _pid: u32,
    guard: Option<&ProcessGuard>,
) -> io::Result<()> {
    #[cfg(windows)]
    if let Some(guard) = guard {
        return terminate_job(guard);
    }
    child.start_kill()
}

#[cfg(unix)]
pub(crate) fn cleanup_remaining_tree(pid: u32, guard: Option<&ProcessGuard>) -> io::Result<()> {
    let result = match signal_process_group(process_group_from_pid(pid)?, libc::SIGKILL) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
        Err(error) => Err(error),
    };
    if result.is_ok()
        && let Some(guard) = guard
    {
        guard.disarm_process_group(pid);
    }
    result
}

#[cfg(not(unix))]
pub(crate) fn cleanup_remaining_tree(_pid: u32, guard: Option<&ProcessGuard>) -> io::Result<()> {
    #[cfg(windows)]
    if let Some(guard) = guard {
        return terminate_job(guard);
    }
    Ok(())
}

#[cfg(unix)]
fn process_group_from_pid(pid: u32) -> io::Result<i32> {
    i32::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "process id is too large"))
}

#[cfg(unix)]
impl ProcessGuard {
    fn disarm_process_group(&self, pid: u32) {
        let Ok(process_group) = process_group_from_pid(pid) else {
            return;
        };
        let _ = self.process_group.compare_exchange(
            process_group,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

#[cfg(unix)]
fn signal_process_group(process_group: i32, signal: i32) -> io::Result<()> {
    let result = unsafe { libc::kill(-process_group, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Stdio;
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        time::{Duration, Instant},
    };

    #[tokio::test]
    async fn force_kill_terminates_the_managed_process_group() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "sleep 60 & child=$!; printf '%s\\n' \"$child\"; wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        configure_managed_command(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        let mut descendant_line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut descendant_line)
            .await
            .unwrap();
        let descendant = descendant_line.trim().parse::<i32>().unwrap();

        start_kill_tree(&mut child, pid, None).unwrap();
        child.wait().await.unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let exists = unsafe { libc::kill(descendant, 0) } == 0;
            if !exists {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "descendant process {descendant} survived process-group termination"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn dropping_process_guard_terminates_the_bound_process_group() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "sleep 60 & child=$!; printf '%s\\n' \"$child\"; wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        configure_managed_command(&mut command).unwrap();

        let guard = create_process_guard().unwrap();
        let mut child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        bind_process_to_guard(&guard, pid).unwrap();
        let mut descendant_line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut descendant_line)
            .await
            .unwrap();
        let descendant = descendant_line.trim().parse::<i32>().unwrap();

        drop(guard);
        child.wait().await.unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let exists = unsafe { libc::kill(descendant, 0) } == 0;
            if !exists {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "descendant process {descendant} survived ProcessGuard drop"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn cleanup_remaining_tree_terminates_and_disarms_the_bound_guard() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "sleep 60 & child=$!; printf '%s\\n' \"$child\"; wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        configure_managed_command(&mut command).unwrap();

        let guard = create_process_guard().unwrap();
        let mut child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        bind_process_to_guard(&guard, pid).unwrap();
        let mut descendant_line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut descendant_line)
            .await
            .unwrap();
        let descendant = descendant_line.trim().parse::<i32>().unwrap();

        cleanup_remaining_tree(pid, Some(&guard)).unwrap();
        assert_eq!(guard.process_group.load(Ordering::Acquire), 0);
        child.wait().await.unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let exists = unsafe { libc::kill(descendant, 0) } == 0;
            if !exists {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "descendant process {descendant} survived normal completion cleanup"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
