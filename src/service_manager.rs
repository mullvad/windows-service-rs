use std::alloc::{alloc_zeroed, dealloc, handle_alloc_error};
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStringExt;
use std::ptr::NonNull;
use std::{io, ptr};

use widestring::WideCString;
use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
use windows_sys::Win32::System::Services::{self, EnumServicesStatusExW, SC_ENUM_PROCESS_INFO};

use crate::sc_handle::ScHandle;
use crate::service::{to_wide, RawServiceInfo, Service, ServiceAccess, ServiceInfo};
use crate::service_enum::{EnumServiceState, EnumServiceStatus, EnumServiceType, RawEnumServices};
use crate::{Error, Result};

bitflags::bitflags! {
    /// Flags describing access permissions for [`ServiceManager`].
    #[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Copy, Clone, Hash)]
    pub struct ServiceManagerAccess: u32 {
        /// Can connect to service control manager.
        const CONNECT = Services::SC_MANAGER_CONNECT;

        /// Can create services.
        const CREATE_SERVICE = Services::SC_MANAGER_CREATE_SERVICE;

        /// Can enumerate services or receive notifications.
        const ENUMERATE_SERVICE = Services::SC_MANAGER_ENUMERATE_SERVICE;

        /// Includes all possible access rights.
        const ALL_ACCESS = Services::SC_MANAGER_ALL_ACCESS;
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
        let machine_name =
            to_wide(machine).map_err(|_| Error::ArgumentHasNulByte("machine name"))?;
        let database_name =
            to_wide(database).map_err(|_| Error::ArgumentHasNulByte("database name"))?;
        let handle = unsafe {
            Services::OpenSCManagerW(
                machine_name.map_or(ptr::null(), |s| s.as_ptr()),
                database_name.map_or(ptr::null(), |s| s.as_ptr()),
                request_access.bits(),
            )
        };

        if handle.is_null() {
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

        if service_handle.is_null() {
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
        let service_name = WideCString::from_os_str(name)
            .map_err(|_| Error::ArgumentHasNulByte("service name"))?;
        let service_handle = unsafe {
            Services::OpenServiceW(
                self.manager_handle.raw_handle(),
                service_name.as_ptr(),
                request_access.bits(),
            )
        };

        if service_handle.is_null() {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            Ok(Service::new(unsafe { ScHandle::new(service_handle) }))
        }
    }

    /// Return the service name given a service display name.
    ///
    /// # Arguments
    ///
    /// * `name` - A service display name.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// # fn main() -> windows_service::Result<()> {
    /// let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    /// let my_service_name = manager.service_name_from_display_name("My Service Display Name")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn service_name_from_display_name(
        &self,
        display_name: impl AsRef<OsStr>,
    ) -> Result<OsString> {
        let service_display_name = WideCString::from_os_str(display_name)
            .map_err(|_| Error::ArgumentHasNulByte("display name"))?;

        // As per docs, the maximum size of data buffer used by GetServiceKeyNameW is 4k bytes,
        // which is 2k wchars
        let mut buffer = [0u16; 2 * 1024];
        let mut buffer_len = u32::try_from(buffer.len()).expect("size must fit in u32");

        let result = unsafe {
            Services::GetServiceKeyNameW(
                self.manager_handle.raw_handle(),
                service_display_name.as_ptr(),
                buffer.as_mut_ptr(),
                &mut buffer_len,
            )
        };

        if result == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            Ok(OsString::from_wide(
                &buffer[..usize::try_from(buffer_len).unwrap()],
            ))
        }
    }

    /// Enumerates the services in the manager database.
    ///
    /// # Arguments
    ///
    /// * `service_type` - The type of services to be enumerated.
    /// * `service_state` - The state of the services to be enumerated.
    /// * `group_name` - The load-order group name.
    fn enum_services_status_(
        &self,
        service_type: EnumServiceType,
        service_state: EnumServiceState,
        group_name: Option<impl AsRef<OsStr>>,
    ) -> Result<RawEnumServices> {
        let mut required_buf_size: u32 = 0;
        let mut returned_services: u32 = 0;
        let mut resume_handle: u32 = 0;

        let group_name = group_name
            .map(|name| {
                WideCString::from_os_str(name.as_ref())
                    .map_err(|_| Error::ArgumentHasNulByte("options.group_name"))
            })
            .transpose()?;

        let group_name = group_name.as_ref().map_or(ptr::null(), |s| s.as_ptr());

        let result = unsafe {
            EnumServicesStatusExW(
                self.manager_handle.raw_handle(),
                SC_ENUM_PROCESS_INFO,
                service_type.bits(),
                service_state.bits(),
                ptr::null_mut(),
                0,
                &mut required_buf_size,
                &mut returned_services,
                &mut resume_handle,
                group_name,
            )
        };

        if result == 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(ERROR_MORE_DATA as i32) {
                return Err(Error::Winapi(err));
            }
        }

        let buffer_size = required_buf_size as usize;
        assert!(buffer_size != 0);
        let buffer_layout = RawEnumServices::layout(buffer_size);
        // SAFETY: the layout size is non-zero.
        let Some(buffer) = NonNull::new(unsafe { alloc_zeroed(buffer_layout) }) else {
            handle_alloc_error(buffer_layout);
        };

        let result = unsafe {
            EnumServicesStatusExW(
                self.manager_handle.raw_handle(),
                SC_ENUM_PROCESS_INFO,
                service_type.bits(),
                service_state.bits(),
                buffer.as_ptr(),
                buffer_size as u32,
                &mut required_buf_size,
                &mut returned_services,
                &mut resume_handle,
                group_name,
            )
        };

        if result == 0 {
            // SAFETY: `buffer` has been allocated with the global allocator with this layout
            unsafe { dealloc(buffer.as_ptr(), buffer_layout) };

            return Err(Error::Winapi(io::Error::last_os_error()));
        }

        // SAFETY: `buffer` has been successfully been initialized
        Ok(unsafe { RawEnumServices::from_parts(buffer, buffer_size, returned_services as usize) })
    }

    /// Enumerates the services in the manager database.
    ///
    /// # Arguments
    ///
    /// * `service_type` - The type of services to be enumerated.
    /// * `service_state` - The state of the services to be enumerated.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// # fn main() -> windows_service::Result<()> {
    /// let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::ENUMERATE_SERVICE)?;
    /// let services = service_manager.enum_services_status_raw(EnumServiceType::WIN32_OWN_PROCESS, EnumServiceState::STATE_ALL)?;
    /// for service in &services {
    ///     println!("{service:?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn enum_services_status_raw(
        &self,
        service_type: EnumServiceType,
        service_state: EnumServiceState,
    ) -> Result<RawEnumServices> {
        self.enum_services_status_(service_type, service_state, None::<&str>)
    }

    /// Enumerates the services in the manager database.
    ///
    /// # Arguments
    ///
    /// * `service_type` - The type of services to be enumerated.
    /// * `service_state` - The state of the services to be enumerated.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// # fn main() -> windows_service::Result<()> {
    /// let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::ENUMERATE_SERVICE)?;
    /// let services = service_manager.enum_services_status(EnumServiceType::WIN32_OWN_PROCESS, EnumServiceState::STATE_ALL)?;
    /// for service in &services {
    ///     println!("{service:?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn enum_services_status(
        &self,
        service_type: EnumServiceType,
        service_state: EnumServiceState,
    ) -> Result<Vec<EnumServiceStatus>> {
        self.enum_services_status_raw(service_type, service_state)
            .and_then(|services| services.to_parsed())
    }

    /// Enumerates the services in the manager database that belong to the group that has the name specified by `group`.
    ///
    /// If `group` is an empty string, only services that do not belong to any group are enumerated.
    ///
    /// # Arguments
    ///
    /// * `service_type` - The type of services to be enumerated.
    /// * `service_state` - The state of the services to be enumerated.
    /// * `group_name` - The load-order group name.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// # fn main() -> windows_service::Result<()> {
    /// let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::ENUMERATE_SERVICE)?;
    /// let services = service_manager.enum_services_status_with_group_raw(EnumServiceType::WIN32_OWN_PROCESS, EnumServiceState::STATE_ALL, "Boot File System")?;
    /// for service in &services {
    ///     println!("{service:?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn enum_services_status_with_group_raw(
        &self,
        service_type: EnumServiceType,
        service_state: EnumServiceState,
        group: impl AsRef<OsStr>,
    ) -> Result<RawEnumServices> {
        self.enum_services_status_(service_type, service_state, Some(group))
    }

    /// Enumerates the services in the manager database that belong to the group that has the name specified by `group`.
    ///
    /// If `group` is an empty string, only services that do not belong to any group are enumerated.
    ///
    /// # Arguments
    ///
    /// * `service_type` - The type of services to be enumerated.
    /// * `service_state` - The state of the services to be enumerated.
    /// * `group_name` - The load-order group name.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// # fn main() -> windows_service::Result<()> {
    /// let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::ENUMERATE_SERVICE)?;
    /// let services = service_manager.enum_services_status_with_group(EnumServiceType::WIN32_OWN_PROCESS, EnumServiceState::STATE_ALL, "Boot File System")?;
    /// for service in &services {
    ///     println!("{service:?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn enum_services_status_with_group(
        &self,
        service_type: EnumServiceType,
        service_state: EnumServiceState,
        group: impl AsRef<OsStr>,
    ) -> Result<Vec<EnumServiceStatus>> {
        self.enum_services_status_with_group_raw(service_type, service_state, group)
            .and_then(|services| services.to_parsed())
    }
}
