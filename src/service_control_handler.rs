use std::ffi::OsStr;
use std::io;
use std::os::raw::c_void;
use std::os::windows::io::{AsRawHandle, RawHandle};
use widestring::WideCString;
use windows_sys::Win32::{
    Foundation::{ERROR_CALL_NOT_IMPLEMENTED, NO_ERROR},
    System::Services,
};

use crate::service::{ServiceControl, ServiceStatus};
use crate::{Error, Result};

/// A struct that holds a unique token for updating the status of the corresponding service.
#[derive(Debug, Clone, Copy)]
pub struct ServiceStatusHandle(Services::SERVICE_STATUS_HANDLE);

impl ServiceStatusHandle {
    fn from_handle(handle: Services::SERVICE_STATUS_HANDLE) -> Self {
        ServiceStatusHandle(handle)
    }

    /// Report the new service status to the system.
    pub fn set_service_status(&self, service_status: ServiceStatus) -> crate::Result<()> {
        let raw_service_status = service_status.to_raw();
        let result = unsafe { Services::SetServiceStatus(self.0, &raw_service_status) };
        if result == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }
}

impl AsRawHandle for ServiceStatusHandle {
    /// Get access to the raw handle to use in other Windows APIs
    fn as_raw_handle(&self) -> RawHandle {
        self.0 as _
    }
}

// Underlying SERVICE_STATUS_HANDLE is thread safe.
// See remarks section for more info:
// https://msdn.microsoft.com/en-us/library/windows/desktop/ms686241(v=vs.85).aspx
unsafe impl Send for ServiceStatusHandle {}
unsafe impl Sync for ServiceStatusHandle {}

/// Abstraction over the return value of service control handler.
/// The meaning of each of variants in this enum depends on the type of received event.
///
/// See the "Return value" section of corresponding MSDN article for more info:
///
/// <https://msdn.microsoft.com/en-us/library/windows/desktop/ms683241(v=vs.85).aspx>
#[derive(Debug)]
pub enum ServiceControlHandlerResult {
    /// Either used to aknowledge the call or grant the permission in advanced events.
    NoError,
    /// The received event is not implemented.
    NotImplemented,
    /// This variant is used to deny permission and return the reason error code in advanced
    /// events.
    Other(u32),
}

impl ServiceControlHandlerResult {
    pub fn to_raw(&self) -> u32 {
        match *self {
            ServiceControlHandlerResult::NoError => NO_ERROR,
            ServiceControlHandlerResult::NotImplemented => ERROR_CALL_NOT_IMPLEMENTED,
            ServiceControlHandlerResult::Other(code) => code,
        }
    }
}

/// Register a closure for receiving service events.
///
/// Returns [`ServiceStatusHandle`] that can be used to report the service status back to the
/// system.
///
/// # Example
///
/// ```rust,no_run
/// use std::ffi::OsString;
/// use windows_service::service::ServiceControl;
/// use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
///
/// fn my_service_main(_arguments: Vec<OsString>) {
///     if let Err(_e) = run_service() {
///         // Handle errors...
///     }
/// }
///
/// fn run_service() -> windows_service::Result<()> {
///     let event_handler = move |control_event| -> ServiceControlHandlerResult {
///         match control_event {
///             ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
///             _ => ServiceControlHandlerResult::NotImplemented,
///         }
///     };
///     let status_handle = service_control_handler::register("my_service_name", event_handler)?;
///     Ok(())
/// }
///
/// # fn main() {}
/// ```
pub fn register<F>(service_name: impl AsRef<OsStr>, event_handler: F) -> Result<ServiceStatusHandle>
where
    F: FnMut(ServiceControl) -> ServiceControlHandlerResult + 'static + Send,
{
    // Move closure to heap.
    let heap_event_handler: Box<F> = Box::new(event_handler);

    // Important: leak the Box<F> which will be released in `service_control_handler`.
    let context: *mut F = Box::into_raw(heap_event_handler);

    let service_name =
        WideCString::from_os_str(service_name).map_err(|_| Error::ServiceNameHasNulByte)?;
    let status_handle = unsafe {
        Services::RegisterServiceCtrlHandlerExW(
            service_name.as_ptr(),
            Some(service_control_handler::<F>),
            context as *mut c_void,
        )
    };

    if status_handle == 0 {
        // Release the `event_handler` in case of an error.
        let _: Box<F> = unsafe { Box::from_raw(context) };
        Err(Error::Winapi(io::Error::last_os_error()))
    } else {
        Ok(ServiceStatusHandle::from_handle(status_handle))
    }
}

/// Static service control handler
#[allow(dead_code)]
extern "system" fn service_control_handler<F>(
    control: u32,
    event_type: u32,
    event_data: *mut c_void,
    context: *mut c_void,
) -> u32
where
    F: FnMut(ServiceControl) -> ServiceControlHandlerResult,
{
    // Important: cast context to &mut F without taking ownership.
    let event_handler: &mut F = unsafe { &mut *(context as *mut F) };

    match unsafe { ServiceControl::from_raw(control, event_type, event_data) } {
        Ok(service_control) => {
            let need_release = match service_control {
                ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
                    true
                }
                _ => false,
            };

            let return_code = event_handler(service_control).to_raw();

            // Important: release context upon Stop, Shutdown or Preshutdown at the end of the
            // service lifecycle.
            if need_release {
                let _: Box<F> = unsafe { Box::from_raw(context as *mut F) };
            }

            return_code
        }

        // Report all unknown control commands as unimplemented
        Err(_) => ServiceControlHandlerResult::NotImplemented.to_raw(),
    }
}
