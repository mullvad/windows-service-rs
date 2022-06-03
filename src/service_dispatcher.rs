use std::ffi::{OsStr, OsString};
use std::{io, ptr};

use widestring::{WideCStr, WideCString};
use windows_sys::Win32::System::Services;

use crate::{Error, Result};

/// A macro to generate an entry point function (aka "service_main") for Windows service.
///
/// The `$function_name` function parses service arguments provided by the system
/// and passes them with a call to `$service_main_handler`.
///
/// `$function_name` - name of the "service_main" callback.
///
/// `$service_main_handler` - function with a signature `fn(Vec<OsString>)` that's called from
/// generated `$function_name`. Accepts parsed service arguments as `Vec<OsString>`. Its
/// responsibility is to create a `ServiceControlHandler`, start processing control events and
/// report the service status to the system.
///
/// # Example
///
/// ```rust,no_run
/// #[macro_use]
/// extern crate windows_service;
///
/// use std::ffi::OsString;
///
/// define_windows_service!(ffi_service_main, my_service_main);
///
/// fn my_service_main(arguments: Vec<OsString>) {
///     // Service entry point
/// }
///
/// # fn main() {}
/// ```
#[macro_export]
macro_rules! define_windows_service {
    ($function_name:ident, $service_main_handler:ident) => {
        /// Static callback used by the system to bootstrap the service.
        /// Do not call it directly.
        extern "system" fn $function_name(
            num_service_arguments: u32,
            service_arguments: *mut *mut u16,
        ) {
            let arguments = unsafe {
                $crate::service_dispatcher::parse_service_arguments(
                    num_service_arguments,
                    service_arguments,
                )
            };

            $service_main_handler(arguments);
        }
    };
}

/// Start service control dispatcher.
///
/// Once started the service control dispatcher blocks the current thread execution
/// until the service is stopped.
///
/// Upon successful initialization, system calls the `service_main` on background thread.
///
/// On failure: immediately returns an error, no threads are spawned.
///
/// # Example
///
/// ```rust,no_run
/// #[macro_use]
/// extern crate windows_service;
///
/// use std::ffi::OsString;
/// use windows_service::service_dispatcher;
///
/// define_windows_service!(ffi_service_main, my_service_main);
///
/// fn my_service_main(arguments: Vec<OsString>) {
///     // The entry point where execution will start on a background thread after a call to
///     // `service_dispatcher::start` from `main`.
/// }
///
/// fn main() -> windows_service::Result<()> {
///     // Register generated `ffi_service_main` with the system and start the service, blocking
///     // this thread until the service is stopped.
///     service_dispatcher::start("myservice", ffi_service_main)?;
///     Ok(())
/// }
/// ```
pub fn start(
    service_name: impl AsRef<OsStr>,
    service_main: extern "system" fn(u32, *mut *mut u16),
) -> Result<()> {
    let service_name =
        WideCString::from_os_str(service_name).map_err(|_| Error::ServiceNameHasNulByte)?;
    let service_table: &[Services::SERVICE_TABLE_ENTRYW] = &[
        Services::SERVICE_TABLE_ENTRYW {
            lpServiceName: service_name.as_ptr() as _,
            lpServiceProc: Some(service_main),
        },
        // the last item has to be { null, null }
        Services::SERVICE_TABLE_ENTRYW {
            lpServiceName: ptr::null_mut(),
            lpServiceProc: None,
        },
    ];

    let result = unsafe { Services::StartServiceCtrlDispatcherW(service_table.as_ptr()) };
    if result == 0 {
        Err(Error::Winapi(io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

/// Parse raw arguments received in `service_main` into `Vec<OsString>`.
///
/// This is an implementation detail and *should not* be called directly!
#[doc(hidden)]
pub unsafe fn parse_service_arguments(argc: u32, argv: *mut *mut u16) -> Vec<OsString> {
    (0..argc)
        .map(|i| {
            let array_element_ptr: *mut *mut u16 = argv.offset(i as isize);
            WideCStr::from_ptr_str(*array_element_ptr).to_os_string()
        })
        .collect()
}
