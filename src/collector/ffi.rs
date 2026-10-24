//! macOS symbols that `libc` doesn't expose (or only exposes as deprecated).

use std::ffi::{CStr, c_void};
use std::mem::{MaybeUninit, size_of};

unsafe extern "C" {
    pub fn mach_host_self() -> libc::mach_port_t;
}

pub type ResponsibleFn = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;

/// Looks up the private `responsibility_get_pid_responsible_for_pid`, if this macOS has it.
pub fn responsible_fn() -> Option<ResponsibleFn> {
    let name = c"responsibility_get_pid_responsible_for_pid";
    // SAFETY: dlsym with RTLD_DEFAULT and a NUL-terminated name only reads.
    let sym = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if sym.is_null() {
        return None;
    }
    // SAFETY: the symbol has had this signature since macOS 10.14.
    Some(unsafe { std::mem::transmute::<*mut c_void, ResponsibleFn>(sym) })
}

/// Reads a fixed-size sysctl value by name.
pub fn sysctl<T: Copy>(name: &CStr) -> Option<T> {
    let mut value = MaybeUninit::<T>::uninit();
    let mut len = size_of::<T>();
    // SAFETY: the buffer is exactly `len` bytes and the kernel writes at most `len`.
    let rc = unsafe {
        libc::sysctlbyname(name.as_ptr(), value.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0)
    };
    // SAFETY: on success with the full size written, the value is initialized.
    (rc == 0 && len == size_of::<T>()).then(|| unsafe { value.assume_init() })
}
