//! Reads processes and memory statistics from macOS.

mod bundle;
mod ffi;

use std::ffi::{CStr, c_int};
use std::io;
use std::mem::{MaybeUninit, size_of};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

pub use bundle::PlistBundles;

use crate::model::{Pressure, ProcInfo, Snapshot, SystemStats};

pub trait Collector {
    fn snapshot(&mut self) -> io::Result<Snapshot>;
}

pub struct MacCollector {
    responsible: Option<ffi::ResponsibleFn>,
    pids: Vec<libc::pid_t>,
}

impl MacCollector {
    pub fn new() -> MacCollector {
        MacCollector {
            responsible: ffi::responsible_fn(),
            pids: Vec::new(),
        }
    }

    fn list_pids(&mut self) -> io::Result<()> {
        // SAFETY: a null buffer asks for the number of pids only.
        let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return Err(io::Error::last_os_error());
        }
        self.pids.clear();
        self.pids.resize(count as usize + count as usize / 5, 0);
        let bytes = (self.pids.len() * size_of::<libc::pid_t>()) as c_int;
        // SAFETY: the buffer holds `bytes` bytes; the call returns how many pids it wrote.
        let written = unsafe { libc::proc_listallpids(self.pids.as_mut_ptr().cast(), bytes) };
        if written <= 0 {
            return Err(io::Error::last_os_error());
        }
        self.pids.truncate(written as usize);
        self.pids.retain(|&pid| pid > 0);
        Ok(())
    }

    fn read_proc(&self, pid: libc::pid_t) -> Option<ProcInfo> {
        let info = bsd_info(pid)?;
        let name = c_name(&info.pbi_name)
            .filter(|n| !n.is_empty())
            .or_else(|| c_name(&info.pbi_comm))?;
        let responsible_pid = self.responsible.and_then(|f| {
            // SAFETY: the function takes any pid and returns -1 on failure.
            let r = unsafe { f(pid) };
            (r > 0).then_some(r)
        });
        Some(ProcInfo {
            pid,
            ppid: info.pbi_ppid as i32,
            uid: info.pbi_uid,
            name,
            path: proc_path(pid),
            footprint: footprint(pid),
            responsible_pid,
        })
    }
}

impl Default for MacCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for MacCollector {
    fn snapshot(&mut self) -> io::Result<Snapshot> {
        self.list_pids()?;
        let procs = self.pids.iter().filter_map(|&pid| self.read_proc(pid)).collect();
        Ok(Snapshot {
            procs,
            stats: system_stats()?,
            // SAFETY: getuid never fails.
            current_uid: effective_user(unsafe { libc::getuid() }, std::env::var("SUDO_UID").ok().as_deref()),
            self_pid: std::process::id() as i32,
        })
    }
}

/// Under `sudo`, the user who ran sudo, so root processes never count as "yours".
fn effective_user(uid: u32, sudo_uid: Option<&str>) -> u32 {
    match (uid, sudo_uid.and_then(|s| s.parse().ok())) {
        (0, Some(real)) => real,
        _ => uid,
    }
}

/// Executable path of a process, or `None` if it exited or can't be read.
pub fn proc_path(pid: i32) -> Option<PathBuf> {
    let mut buf = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is PROC_PIDPATHINFO_MAXSIZE bytes as the call requires.
    let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (len > 0).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..len as usize])))
}

fn bsd_info(pid: libc::pid_t) -> Option<libc::proc_bsdinfo> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = size_of::<libc::proc_bsdinfo>() as c_int;
    // SAFETY: the buffer is exactly one proc_bsdinfo; the call returns bytes written.
    let written = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    // SAFETY: a full-size write initializes the struct.
    (written == size).then(|| unsafe { info.assume_init() })
}

fn footprint(pid: libc::pid_t) -> Option<u64> {
    let mut usage = MaybeUninit::<libc::rusage_info_v4>::uninit();
    // SAFETY: RUSAGE_INFO_V4 writes one rusage_info_v4 into the buffer.
    let rc = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, usage.as_mut_ptr().cast()) };
    // SAFETY: rc == 0 means the struct was filled.
    (rc == 0).then(|| unsafe { usage.assume_init() }.ri_phys_footprint)
}

fn c_name(raw: &[libc::c_char]) -> Option<String> {
    let bytes: Vec<u8> = raw.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn system_stats() -> io::Result<SystemStats> {
    let missing = |name: &CStr| io::Error::other(format!("sysctl {name:?} unavailable"));
    let total: u64 = ffi::sysctl(c"hw.memsize").ok_or_else(|| missing(c"hw.memsize"))?;
    let page: u64 = ffi::sysctl::<u64>(c"hw.pagesize")
        .or_else(|| ffi::sysctl::<u32>(c"hw.pagesize").map(u64::from))
        .ok_or_else(|| missing(c"hw.pagesize"))?;
    let pressure = ffi::sysctl::<i32>(c"kern.memorystatus_vm_pressure_level").unwrap_or(1);
    let swap_used = ffi::sysctl::<libc::xsw_usage>(c"vm.swapusage").map_or(0, |s| s.xsu_used);

    let mut vm = MaybeUninit::<libc::vm_statistics64>::zeroed();
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: `count` is the struct size in natural_t units; the kernel writes at most that.
    let rc = unsafe {
        libc::host_statistics64(
            ffi::mach_host_self(),
            libc::HOST_VM_INFO64,
            vm.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if rc != libc::KERN_SUCCESS {
        return Err(io::Error::other(format!("host_statistics64 failed: {rc}")));
    }
    // SAFETY: zero-initialized and partly overwritten by the kernel; all fields are integers.
    let vm = unsafe { vm.assume_init() };
    let app = u64::from(vm.internal_page_count).saturating_sub(u64::from(vm.purgeable_count));
    let wired = u64::from(vm.wire_count) * page;
    let used = (app + u64::from(vm.wire_count) + u64::from(vm.compressor_page_count)) * page;
    let app_footprint = (app + vm.total_uncompressed_pages_in_compressor) * page;

    Ok(SystemStats {
        total,
        used: used.min(total),
        wired,
        swap_used,
        app_footprint,
        pressure: Pressure::from_level(pressure),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sudo_user_is_the_real_user() {
        assert_eq!(effective_user(0, Some("501")), 501);
    }

    #[test]
    fn plain_root_stays_root() {
        assert_eq!(effective_user(0, None), 0);
        assert_eq!(effective_user(0, Some("garbage")), 0);
    }

    #[test]
    fn normal_user_ignores_sudo_uid() {
        assert_eq!(effective_user(501, Some("0")), 501);
    }
}
