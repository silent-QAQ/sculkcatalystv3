use std::io;
use tokio::process::{Child, Command};

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

#[cfg(unix)]
pub(crate) fn start_kill_tree(child: &mut Child, pid: u32) -> io::Result<()> {
    match signal_process_group(pid, libc::SIGKILL) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => child.start_kill(),
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
pub(crate) fn start_kill_tree(child: &mut Child, _pid: u32) -> io::Result<()> {
    child.start_kill()
}

#[cfg(unix)]
pub(crate) fn cleanup_remaining_tree(pid: u32) -> io::Result<()> {
    match signal_process_group(pid, libc::SIGKILL) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
pub(crate) fn cleanup_remaining_tree(_pid: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: i32) -> io::Result<()> {
    let process_group = i32::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "process id is too large"))?;
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

        start_kill_tree(&mut child, pid).unwrap();
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
}
