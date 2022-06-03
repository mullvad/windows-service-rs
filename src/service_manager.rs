use std::ffi::OsStr;
use std::{io, ptr};

use widestring::WideCString;
use windows_sys::Win32::System::Services;

use crate::sc_handle::ScHandle;
use crate::service::{to_wide, RawServiceInfo, Service, ServiceAccess, ServiceInfo};
use crate::{Error, Result};

bitflags::bitflags! {
    /// Flags describing access permissions for [`ServiceManager`].
    pub struct ServiceManagerAccess: u32 {
        /// Can connect to service control manager.
        const CONNECT = Services::SC_MANAGER_CONNECT;

        /// Can create services.
        const CREATE_SERVICE = Services::SC_MANAGER_CREATE_SERVICE;

        /// Can enumerate services or receive notifications.
        const ENUMERATE_SERVICE = Services::SC_MANAGER_ENUMERATE_SERVICE;
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
    /// * `machine` - The name of machine. Pass `None` to connect to local machine.
    /// * `database` - The name of database to connect to. Pass `None` to connect to active
    ///   database.
    fn new(
        machine: Option<impl AsRef<OsStr>>,
        database: Option<impl AsRef<OsStr>>,
        request_access: ServiceManagerAccess,
    ) -> Result<Self> {
        let machine_name = to_wide(machine).map_err(|_| Error::MachineNameHasNulByte)?;
        let database_name = to_wide(database).map_err(|_| Error::DatabaseNameHasNulByte)?;
        let handle = unsafe {
            Services::OpenSCManagerW(
                machine_name.map_or(ptr::null(), |s| s.as_ptr()),
                database_name.map_or(ptr::null(), |s| s.as_ptr()),
                request_access.bits(),
            )
        };

        if handle == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
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
    /// * `database` - The name of database to connect to. Pass `None` to connect to active
    ///   database.
    /// * `request_access` - Desired access permissions.
    pub fn local_computer(
        database: Option<impl AsRef<OsStr>>,
        request_access: ServiceManagerAccess,
    ) -> Result<Self> {
        ServiceManager::new(None::<&OsStr>, database, request_access)
    }

    /// Connect to remote services database.
    ///
    /// # Arguments
    ///
    /// * `machine` - The name of remote machine.
    /// * `database` - The name of database to connect to. Pass `None` to connect to active
    ///   database.
    /// * `request_access` - desired access permissions.
    pub fn remote_computer(
        machine: impl AsRef<OsStr>,
        database: Option<impl AsRef<OsStr>>,
        request_access: ServiceManagerAccess,
    ) -> Result<Self> {
        ServiceManager::new(Some(machine), database, request_access)
    }

    /// Create a service.
    ///
    /// # Arguments
    ///
    /// * `service_info` - The service information that will be saved to the system services
    ///   registry.
    /// * `service_access` - Desired access permissions for the returned [`Service`] instance.
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
    ///         service_type: ServiceType::OWN_PROCESS,
    ///         start_type: ServiceStartType::OnDemand,
    ///         error_control: ServiceErrorControl::Normal,
    ///         executable_path: PathBuf::from(r"C:\path\to\my\service.exe"),
    ///         launch_arguments: vec![],
    ///         dependencies: vec![],
    ///         account_name: None, // run as System
    ///         account_password: None,
    ///     };
    ///
    ///     let my_service = manager.create_service(&my_service_info, ServiceAccess::QUERY_STATUS)?;
    ///     Ok(())
    /// }
    /// ```
    pub fn create_service(
        &self,
        service_info: &ServiceInfo,
        service_access: ServiceAccess,
    ) -> Result<Service> {
        let raw_info = RawServiceInfo::new(service_info)?;
        let service_handle = unsafe {
            Services::CreateServiceW(
                self.manager_handle.raw_handle(),
                raw_info.name.as_ptr(),
                raw_info.display_name.as_ptr(),
                service_access.bits(),
                raw_info.service_type,
                raw_info.start_type,
                raw_info.error_control,
                raw_info.launch_command.as_ptr(),
                ptr::null(),     // load ordering group
                ptr::null_mut(), // tag id within the load ordering group
                raw_info
                    .dependencies
                    .as_ref()
                    .map_or(ptr::null(), |s| s.as_ptr()),
                raw_info
                    .account_name
                    .as_ref()
                    .map_or(ptr::null(), |s| s.as_ptr()),
                raw_info
                    .account_password
                    .as_ref()
                    .map_or(ptr::null(), |s| s.as_ptr()),
            )
        };

        if service_handle == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            Ok(Service::new(unsafe { ScHandle::new(service_handle) }))
        }
    }

    /// Open an existing service.
    ///
    /// # Arguments
    ///
    /// * `name` - The service name.
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
    pub fn open_service(
        &self,
        name: impl AsRef<OsStr>,
        request_access: ServiceAccess,
    ) -> Result<Service> {
        let service_name =
            WideCString::from_os_str(name).map_err(|_| Error::ServiceNameHasNulByte)?;
        let service_handle = unsafe {
            Services::OpenServiceW(
                self.manager_handle.raw_handle(),
                service_name.as_ptr(),
                request_access.bits(),
            )
        };

        if service_handle == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            Ok(Service::new(unsafe { ScHandle::new(service_handle) }))
        }
    }
}
