//! Launched apps run in a Job Object that kills the whole process tree when the test ends
//! (an app that starts helpers, or a launcher that starts the real app).

use async_trait::async_trait;
use raywright_core::Result;
use raywright_core::driver::{OsProcess, OutputLog, ProcessHandle, ProcessId};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};
use windows::core::PCWSTR;

struct Job(HANDLE);

// SAFETY: a job handle may be used and closed from any thread.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: closing the last handle kills every process in the job.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Puts `pid` in a new kill-on-close job; `None` if Windows refuses (then only the process
/// itself is killed when the test ends).
fn job_for(pid: u32) -> Option<Job> {
    // SAFETY: plain Win32 calls; every handle is closed (the job by `Job`'s drop).
    unsafe {
        let job = Job(CreateJobObjectW(None, PCWSTR::null()).ok()?);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, &info as *const _ as *const _, size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32).ok()?;
        let proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid).ok()?;
        let assigned = AssignProcessToJobObject(job.0, proc);
        let _ = CloseHandle(proc);
        assigned.ok()?;
        Some(job)
    }
}

pub struct JobProcess {
    inner: OsProcess,
    _job: Option<Job>,
}

impl JobProcess {
    pub fn new(inner: OsProcess) -> Self {
        let job = job_for(inner.pid().0);
        if job.is_none() {
            tracing::debug!("raywright: could not put pid {} in a job object", inner.pid());
        }
        JobProcess { inner, _job: job }
    }
}

#[async_trait]
impl ProcessHandle for JobProcess {
    fn pid(&self) -> ProcessId {
        self.inner.pid()
    }
    fn exit_status(&self) -> Option<Option<i32>> {
        self.inner.exit_status()
    }
    async fn wait(&self) -> Option<i32> {
        self.inner.wait().await
    }
    async fn kill(&self) -> Result<()> {
        self.inner.kill().await
    }
    fn output(&self) -> OutputLog {
        self.inner.output()
    }
}
