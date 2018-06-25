use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::{io, ptr};

use widestring::{NulError, WideCString, WideString};
use winapi::um::winsvc;

use sc_handle::ScHandle;
use service::{Service, ServiceAccess, ServiceInfo};
use shell_escape;

use {ErrorKind, Result, ResultExt};

bitflags! {
    /// Flags describing access permissions for [`ServiceManager`].
    pub struct ServiceManagerAccess: u32 {
        /// Can connect to service control manager.
        const CONNECT = winsvc::SC_MANAGER_CONNECT;

        /// Can create services.
        const CREATE_SERVICE = winsvc::SC_MANAGER_CREATE_SERVICE;

        /// Can enumerate services or receive notifications.
        const ENUMERATE_SERVICE = winsvc::SC_MANAGER_ENUMERATE_SERVICE;
    }
}

/// Service manager.
pub struct ServiceManager {
    manager_handle: ScHandle,
}

impl ServiceManager {
    /// Private initializer.
    ///
    /// # Arguments
    ///
    /// * `machine`  - The name of machine.
    ///                Pass `None` to connect to local machine.
    /// * `database` - The name of database to connect to.
    ///                Pass `None` to connect to active database.
    ///
    fn new<M: AsRef<OsStr>, D: AsRef<OsStr>>(
        machine: Option<M>,
        database: Option<D>,
        request_access: ServiceManagerAccess,
    ) -> Result<Self> {
        let machine_name = to_wide(machine).chain_err(|| ErrorKind::InvalidMachineName)?;
        let database_name = to_wide(database).chain_err(|| ErrorKind::InvalidDatabaseName)?;
        let handle = unsafe {
            winsvc::OpenSCManagerW(
                machine_name.map_or(ptr::null(), |s| s.as_ptr()),
                database_name.map_or(ptr::null(), |s| s.as_ptr()),
                request_access.bits(),
            )
        };

        if handle.is_null() {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(ServiceManager {
                manager_handle: unsafe { ScHandle::new(handle) },
            })
        }
    }

    /// Connect to local services database.
    ///
    /// # Arguments
    ///
    /// * `database`       - The name of database to connect to.
    ///                      Pass `None` to connect to active database.
    /// * `request_access` - Desired access permissions.
    ///
    pub fn local_computer<D: AsRef<OsStr>>(
        database: Option<D>,
        request_access: ServiceManagerAccess,
    ) -> Result<Self> {
        ServiceManager::new(None::<&OsStr>, database, request_access)
    }

    /// Connect to remote services database.
    ///
    /// # Arguments
    ///
    /// * `machine`        - The name of remote machine.
    /// * `database`       - The name of database to connect to.
    ///                      Pass `None` to connect to active database.
    /// * `request_access` - desired access permissions.
    ///
    pub fn remote_computer<M: AsRef<OsStr>, D: AsRef<OsStr>>(
        machine: M,
        database: Option<D>,
        request_access: ServiceManagerAccess,
    ) -> Result<Self> {
        ServiceManager::new(Some(machine), database, request_access)
    }

    /// Create a service.
    ///
    /// # Arguments
    ///
    /// * `service_info`   - The service information that will be saved to the system services
    ///                      registry.
    /// * `service_access` - Desired access permissions for the returned [`Service`]
    ///                      instance.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use std::ffi::OsString;
    /// use std::path::PathBuf;
    /// use windows_service::service::{
    ///     ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType,
    /// };
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// fn main() -> windows_service::Result<()> {
    ///     let manager =
    ///         ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)?;
    ///
    ///     let my_service_info = ServiceInfo {
    ///         name: OsString::from("my_service"),
    ///         display_name: OsString::from("My service"),
    ///         service_type: ServiceType::OwnProcess,
    ///         start_type: ServiceStartType::OnDemand,
    ///         error_control: ServiceErrorControl::Normal,
    ///         executable_path: PathBuf::from(r"C:\path\to\my\service.exe"),
    ///         launch_arguments: vec![],
    ///         dependencies: vec![],
    ///         account_name: None, // run as System
    ///         account_password: None,
    ///     };
    ///
    ///     let my_service = manager.create_service(my_service_info, ServiceAccess::QUERY_STATUS)?;
    ///     Ok(())
    /// }
    /// ```
    pub fn create_service(
        &self,
        service_info: ServiceInfo,
        service_access: ServiceAccess,
    ) -> Result<Service> {
        let service_name =
            WideCString::from_str(service_info.name).chain_err(|| ErrorKind::InvalidServiceName)?;
        let display_name = WideCString::from_str(service_info.display_name)
            .chain_err(|| ErrorKind::InvalidDisplayName)?;
        let account_name =
            to_wide(service_info.account_name).chain_err(|| ErrorKind::InvalidAccountName)?;
        let account_password =
            to_wide(service_info.account_password).chain_err(|| ErrorKind::InvalidAccountPassword)?;

        // escape executable path and arguments and combine them into single command
        let executable_path = escape_wide(service_info.executable_path)
            .chain_err(|| ErrorKind::InvalidExecutablePath)?;

        let mut launch_command_buffer = WideString::new();
        launch_command_buffer.push(executable_path);

        for launch_argument in service_info.launch_arguments.iter() {
            let wide = escape_wide(launch_argument).chain_err(|| ErrorKind::InvalidLaunchArgument)?;

            launch_command_buffer.push_str(" ");
            launch_command_buffer.push(wide);
        }

        let launch_command = WideCString::from_wide_str(launch_command_buffer).unwrap();

        let dependency_identifiers: Vec<OsString> = service_info
            .dependencies
            .iter()
            .map(|dependency| dependency.to_system_identifier())
            .collect();
        let joined_dependencies = to_double_nul_terminated(&dependency_identifiers)
            .chain_err(|| ErrorKind::InvalidDependency)?;

        let service_handle = unsafe {
            winsvc::CreateServiceW(
                self.manager_handle.raw_handle(),
                service_name.as_ptr(),
                display_name.as_ptr(),
                service_access.bits(),
                service_info.service_type.to_raw(),
                service_info.start_type.to_raw(),
                service_info.error_control.to_raw(),
                launch_command.as_ptr(),
                ptr::null(),     // load ordering group
                ptr::null_mut(), // tag id within the load ordering group
                joined_dependencies.map_or(ptr::null(), |s| s.as_ptr()),
                account_name.map_or(ptr::null(), |s| s.as_ptr()),
                account_password.map_or(ptr::null(), |s| s.as_ptr()),
            )
        };

        if service_handle.is_null() {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(Service::new(unsafe { ScHandle::new(service_handle) }))
        }
    }

    /// Open an existing service.
    ///
    /// # Arguments
    ///
    /// * `name`           - The service name.
    /// * `request_access` - Desired permissions for the returned [`Service`] instance.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use windows_service::service::ServiceAccess;
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// # fn main() -> windows_service::Result<()> {
    /// let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    /// let my_service = manager.open_service("my_service", ServiceAccess::QUERY_STATUS)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    pub fn open_service<T: AsRef<OsStr>>(
        &self,
        name: T,
        request_access: ServiceAccess,
    ) -> Result<Service> {
        let service_name = WideCString::from_str(name).chain_err(|| ErrorKind::InvalidServiceName)?;
        let service_handle = unsafe {
            winsvc::OpenServiceW(
                self.manager_handle.raw_handle(),
                service_name.as_ptr(),
                request_access.bits(),
            )
        };

        if service_handle.is_null() {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(Service::new(unsafe { ScHandle::new(service_handle) }))
        }
    }
}

fn to_wide<T: AsRef<OsStr>>(s: Option<T>) -> ::std::result::Result<Option<WideCString>, NulError> {
    if let Some(s) = s {
        Ok(Some(WideCString::from_str(s)?))
    } else {
        Ok(None)
    }
}

/// A helper to join a collection of `OsStr` into a nul-separated `WideString` ending with two nul
/// bytes.
///
/// Sample output:
/// "item one\0item two\0\0"
///
/// Returns None if the source collection is empty.
fn to_double_nul_terminated<T: AsRef<OsStr>>(
    source: &[T],
) -> ::std::result::Result<Option<WideString>, NulError> {
    if source.len() > 0 {
        let mut wide = WideString::new();
        for s in source {
            let checked_str = WideCString::from_str(s)?;
            wide.push_slice(checked_str);
            wide.push_slice(&[0]);
        }
        wide.push_slice(&[0]);
        Ok(Some(wide))
    } else {
        Ok(None)
    }
}

fn escape_wide<T: AsRef<OsStr>>(s: T) -> ::std::result::Result<WideString, NulError> {
    let escaped = shell_escape::escape(Cow::Borrowed(s.as_ref()));
    let wide = WideCString::from_str(escaped)?;
    Ok(wide.to_wide_string())
}
