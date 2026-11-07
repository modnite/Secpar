//! Crash containment: the engine child must not outlive its controller, even if
//! the controller is killed without running destructors.
//!
//! Windows: the child is assigned to a job object that kills its members when the
//! last handle closes (the handle closes when the controller dies).
//! Linux: the child asks the kernel for SIGKILL when its parent thread exits.
use tokio::process::{Child, Command};

#[cfg(unix)]
pub fn prepare(command: &mut Command) {
    // SAFETY: only async-signal-safe calls run between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may already be gone between fork and prctl.
            if libc::getppid() == 1 {
                libc::_exit(1);
            }
            Ok(())
        });
    }
}

#[cfg(unix)]
pub struct Containment;

#[cfg(unix)]
impl Containment {
    pub fn attach(_child: &Child) -> std::io::Result<Self> {
        Ok(Self)
    }
}

#[cfg(windows)]
pub fn prepare(_command: &mut Command) {}

#[cfg(windows)]
pub struct Containment(windows_sys::Win32::Foundation::HANDLE);

// The handle is only closed on drop and never shared.
#[cfg(windows)]
unsafe impl Send for Containment {}
#[cfg(windows)]
unsafe impl Sync for Containment {}

#[cfg(windows)]
impl Containment {
    pub fn attach(child: &Child) -> std::io::Result<Self> {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
        };
        let process = child
            .raw_handle()
            .ok_or_else(|| std::io::Error::other("The engine process already exited"))?;
        // SAFETY: plain Win32 calls with valid, owned arguments.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 || AssignProcessToJobObject(job, process as _) == 0 {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }
            Ok(Self(job))
        }
    }
}

#[cfg(windows)]
impl Drop for Containment {
    fn drop(&mut self) {
        // SAFETY: the handle was created by attach and is closed exactly once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}
