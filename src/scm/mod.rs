use windows_sys::Win32::{Security, System::Services};

/// A handle holder that wraps a low level [`Security::SC_HANDLE`].
pub(crate) struct ScHandle(Security::SC_HANDLE);

impl ScHandle {
    pub(crate) unsafe fn new(handle: Security::SC_HANDLE) -> Self {
        ScHandle(handle)
    }

    /// Returns underlying [`Security::SC_HANDLE`].
    pub(crate) fn raw_handle(&self) -> Security::SC_HANDLE {
        self.0
    }
}

impl Drop for ScHandle {
    fn drop(&mut self) {
        unsafe { Services::CloseServiceHandle(self.0) };
    }
}
