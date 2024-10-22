use windows_sys::Win32::System::Services;

/// A handle holder that wraps a low level [`Security::SC_HANDLE`].
pub(crate) struct ScHandle(Services::SC_HANDLE);

impl ScHandle {
    pub(crate) unsafe fn new(handle: Services::SC_HANDLE) -> Self {
        ScHandle(handle)
    }

    /// Returns underlying [`Security::SC_HANDLE`].
    pub(crate) fn raw_handle(&self) -> Services::SC_HANDLE {
        self.0
    }
}

impl Drop for ScHandle {
    fn drop(&mut self) {
        unsafe { Services::CloseServiceHandle(self.0) };
    }
}
