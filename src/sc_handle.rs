use winapi::um::winsvc;

/// A handle holder that wraps a low level [`winsvc::SC_HANDLE`].
pub(crate) struct ScHandle(winsvc::SC_HANDLE);

impl ScHandle {
    pub(crate) unsafe fn new(handle: winsvc::SC_HANDLE) -> Self {
        ScHandle(handle)
    }

    /// Returns underlying [`winsvc::SC_HANDLE`].
    pub(crate) fn raw_handle(&self) -> winsvc::SC_HANDLE {
        self.0
    }
}

impl Drop for ScHandle {
    fn drop(&mut self) {
        unsafe { winsvc::CloseServiceHandle(self.0) };
    }
}
