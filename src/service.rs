use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Duration;
use std::{io, mem};

use widestring::{WideCStr, WideCString};
use winapi::shared::ntdef::LPWSTR;
use winapi::shared::winerror::{ERROR_INSUFFICIENT_BUFFER, ERROR_SERVICE_SPECIFIC_ERROR, NO_ERROR};
use winapi::um::{winnt, winsvc};

use sc_handle::ScHandle;
use {ErrorKind, Result, ResultExt};

/// Enum describing the types of Windows services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceType {
    /// File system driver service.
    FileSystemDriver = winnt::SERVICE_FILE_SYSTEM_DRIVER,

    /// Driver service.
    KernelDriver = winnt::SERVICE_KERNEL_DRIVER,

    /// Service that runs in its own process.
    OwnProcess = winnt::SERVICE_WIN32_OWN_PROCESS,

    /// Service that shares a process with other services.
    ShareProcess = winnt::SERVICE_WIN32_SHARE_PROCESS,
}

impl ServiceType {
    pub fn from_raw(raw_value: u32) -> Result<Self> {
        let service_type = match raw_value {
            x if x == ServiceType::FileSystemDriver.to_raw() => ServiceType::FileSystemDriver,
            x if x == ServiceType::KernelDriver.to_raw() => ServiceType::KernelDriver,
            x if x == ServiceType::OwnProcess.to_raw() => ServiceType::OwnProcess,
            x if x == ServiceType::ShareProcess.to_raw() => ServiceType::ShareProcess,
            _ => Err(ErrorKind::InvalidServiceType(raw_value))?,
        };
        Ok(service_type)
    }

    pub fn to_raw(&self) -> u32 {
        *self as u32
    }
}

bitflags! {
    /// Flags describing the access permissions when working with services
    pub struct ServiceAccess: u32 {
        /// Can query the service status
        const QUERY_STATUS = winsvc::SERVICE_QUERY_STATUS;

        /// Can start the service
        const START = winsvc::SERVICE_START;

        /// Can stop the service
        const STOP = winsvc::SERVICE_STOP;

        /// Can pause or continue the service execution
        const PAUSE_CONTINUE = winsvc::SERVICE_PAUSE_CONTINUE;

        /// Can ask the service to report its status
        const INTERROGATE = winsvc::SERVICE_INTERROGATE;

        /// Can delete the service
        const DELETE = winnt::DELETE;

        /// Can query the services configuration
        const QUERY_CONFIG = winsvc::SERVICE_QUERY_CONFIG;
    }
}

/// Enum describing the start options for windows services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceStartType {
    /// Autostart on system startup
    AutoStart = winnt::SERVICE_AUTO_START,
    /// Service is enabled, can be started manually
    OnDemand = winnt::SERVICE_DEMAND_START,
    /// Disabled service
    Disabled = winnt::SERVICE_DISABLED,
}

impl ServiceStartType {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<ServiceStartType> {
        match raw {
            x if x == ServiceStartType::AutoStart.to_raw() => Ok(ServiceStartType::AutoStart),
            x if x == ServiceStartType::OnDemand.to_raw() => Ok(ServiceStartType::OnDemand),
            x if x == ServiceStartType::Disabled.to_raw() => Ok(ServiceStartType::Disabled),
            _ => Err(ErrorKind::InvalidServiceStartType(raw))?,
        }
    }
}

/// Error handling strategy for service failures.
///
/// See <https://msdn.microsoft.com/en-us/library/windows/desktop/ms682450(v=vs.85).aspx>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceErrorControl {
    Critical = winnt::SERVICE_ERROR_CRITICAL,
    Ignore = winnt::SERVICE_ERROR_IGNORE,
    Normal = winnt::SERVICE_ERROR_NORMAL,
    Severe = winnt::SERVICE_ERROR_SEVERE,
}

impl ServiceErrorControl {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<ServiceErrorControl> {
        match raw {
            x if x == ServiceErrorControl::Critical.to_raw() => Ok(ServiceErrorControl::Critical),
            x if x == ServiceErrorControl::Ignore.to_raw() => Ok(ServiceErrorControl::Ignore),
            x if x == ServiceErrorControl::Normal.to_raw() => Ok(ServiceErrorControl::Normal),
            x if x == ServiceErrorControl::Severe.to_raw() => Ok(ServiceErrorControl::Severe),
            _ => Err(ErrorKind::InvalidServiceErrorControl(raw))?,
        }
    }
}

/// Service dependency descriptor
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ServiceDependency {
    Service(OsString),
    Group(OsString),
}

impl ServiceDependency {
    pub fn to_system_identifier(&self) -> OsString {
        match *self {
            ServiceDependency::Service(ref name) => name.to_owned(),
            ServiceDependency::Group(ref name) => {
                // since services and service groups share the same namespace the group identifiers
                // should be prefixed with '+' (SC_GROUP_IDENTIFIER)
                let mut group_identifier = OsString::new();
                group_identifier.push("+");
                group_identifier.push(name);
                group_identifier
            }
        }
    }
}

/// A struct that describes the service.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceInfo {
    /// Service name
    pub name: OsString,

    /// User-friendly service name
    pub display_name: OsString,

    /// The service type
    pub service_type: ServiceType,

    /// The service startup options
    pub start_type: ServiceStartType,

    /// The severity of the error, and action taken, if this service fails to start.
    pub error_control: ServiceErrorControl,

    /// Path to the service binary
    pub executable_path: PathBuf,

    /// Launch arguments passed to `main` when system starts the service.
    /// This is not the same as arguments passed to `service_main`.
    pub launch_arguments: Vec<OsString>,

    /// Service dependencies
    pub dependencies: Vec<ServiceDependency>,

    /// Account to use for running the service.
    /// for example: NT Authority\System.
    /// use `None` to run as LocalSystem.
    pub account_name: Option<OsString>,

    /// Account password.
    /// For system accounts this should normally be `None`.
    pub account_password: Option<OsString>,
}

/// A struct that describes the service.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceConfig {
    /// The service type
    pub service_type: ServiceType,

    /// The service startup options
    pub start_type: ServiceStartType,

    /// The severity of the error, and action taken, if this service fails to start.
    pub error_control: ServiceErrorControl,

    /// Path to the service binary
    pub executable_path: PathBuf,

    /// Path to the service binary
    pub load_order_group: Option<OsString>,

    /// A unique tag value for this service in the group specified by the load_order_group
    /// parameter.
    pub tag_id: u32,

    /// Service dependencies
    pub dependencies: Option<Vec<ServiceDependency>>,

    /// Account to use for running the service.
    /// for example: NT Authority\System.
    /// use `None` to run as LocalSystem.
    pub account_name: Option<OsString>,

    /// User-friendly service name
    pub display_name: OsString,
}

impl ServiceConfig {
    pub fn from_raw(raw: winsvc::QUERY_SERVICE_CONFIGW) -> Result<ServiceConfig> {
        Ok(ServiceConfig {
            service_type: ServiceType::from_raw(raw.dwServiceType)?,
            start_type: ServiceStartType::from_raw(raw.dwStartType)?,
            error_control: ServiceErrorControl::from_raw(raw.dwErrorControl)?,
            executable_path: PathBuf::from(
                unsafe { WideCStr::from_ptr_str(raw.lpBinaryPathName) }.to_os_string(),
            ),
            load_order_group: {
                match raw.lpLoadOrderGroup {
                    i if i == ::std::ptr::null_mut() => None,
                    _ => {
                        let value =
                            unsafe { WideCStr::from_ptr_str(raw.lpLoadOrderGroup) }.to_os_string();
                        match value.len() {
                            0 => None,
                            _ => Some(value),
                        }
                    }
                }
            },
            tag_id: raw.dwTagId,
            dependencies: ServiceConfig::array_string_to_vec(raw.lpDependencies),
            account_name: Some(
                unsafe { WideCStr::from_ptr_str(raw.lpServiceStartName) }.to_os_string(),
            ),
            display_name: unsafe { WideCStr::from_ptr_str(raw.lpDisplayName) }.to_os_string(),
        })
    }

    fn array_string_to_vec(input: LPWSTR) -> Option<Vec<ServiceDependency>> {
        let mut next = input;
        let mut deps = Vec::new();
        while {
            match next {
                i if i == ::std::ptr::null_mut() => false,
                _ => match unsafe { WideCStr::from_ptr_str(next) }.to_string() {
                    Ok(value) => match value.len() {
                        i if i > 0 => {
                            next = (next as usize + (value.len() * 2 + 2)) as LPWSTR;
                            match value.starts_with("+") {
                                true => deps.push(ServiceDependency::Group(value.into())),
                                _ => deps.push(ServiceDependency::Service(value.into())),
                            }
                            true
                        }
                        _ => false,
                    },
                    Err(_) => false,
                },
            }
        } {}
        match deps.len() {
            0 => None,
            _ => Some(deps),
        }
    }
}

/// Enum describing the service control operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceControl {
    Continue = winsvc::SERVICE_CONTROL_CONTINUE,
    Interrogate = winsvc::SERVICE_CONTROL_INTERROGATE,
    NetBindAdd = winsvc::SERVICE_CONTROL_NETBINDADD,
    NetBindDisable = winsvc::SERVICE_CONTROL_NETBINDDISABLE,
    NetBindEnable = winsvc::SERVICE_CONTROL_NETBINDENABLE,
    NetBindRemove = winsvc::SERVICE_CONTROL_NETBINDREMOVE,
    ParamChange = winsvc::SERVICE_CONTROL_PARAMCHANGE,
    Pause = winsvc::SERVICE_CONTROL_PAUSE,
    Preshutdown = winsvc::SERVICE_CONTROL_PRESHUTDOWN,
    Shutdown = winsvc::SERVICE_CONTROL_SHUTDOWN,
    Stop = winsvc::SERVICE_CONTROL_STOP,
}

impl ServiceControl {
    pub fn from_raw(raw_value: u32) -> Result<Self> {
        let service_control = match raw_value {
            x if x == ServiceControl::Continue.to_raw() => ServiceControl::Continue,
            x if x == ServiceControl::Interrogate.to_raw() => ServiceControl::Interrogate,
            x if x == ServiceControl::NetBindAdd.to_raw() => ServiceControl::NetBindAdd,
            x if x == ServiceControl::NetBindDisable.to_raw() => ServiceControl::NetBindDisable,
            x if x == ServiceControl::NetBindEnable.to_raw() => ServiceControl::NetBindEnable,
            x if x == ServiceControl::NetBindRemove.to_raw() => ServiceControl::NetBindRemove,
            x if x == ServiceControl::ParamChange.to_raw() => ServiceControl::ParamChange,
            x if x == ServiceControl::Pause.to_raw() => ServiceControl::Pause,
            x if x == ServiceControl::Preshutdown.to_raw() => ServiceControl::Preshutdown,
            x if x == ServiceControl::Shutdown.to_raw() => ServiceControl::Shutdown,
            x if x == ServiceControl::Stop.to_raw() => ServiceControl::Stop,
            other => Err(ErrorKind::InvalidServiceControl(other))?,
        };
        Ok(service_control)
    }

    pub fn to_raw(&self) -> u32 {
        *self as u32
    }
}

/// Service state returned as a part of [`ServiceStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceState {
    Stopped = winsvc::SERVICE_STOPPED,
    StartPending = winsvc::SERVICE_START_PENDING,
    StopPending = winsvc::SERVICE_STOP_PENDING,
    Running = winsvc::SERVICE_RUNNING,
    ContinuePending = winsvc::SERVICE_CONTINUE_PENDING,
    PausePending = winsvc::SERVICE_PAUSE_PENDING,
    Paused = winsvc::SERVICE_PAUSED,
}

impl ServiceState {
    fn from_raw(raw_state: u32) -> Result<Self> {
        let service_state = match raw_state {
            x if x == ServiceState::Stopped.to_raw() => ServiceState::Stopped,
            x if x == ServiceState::StartPending.to_raw() => ServiceState::StartPending,
            x if x == ServiceState::StopPending.to_raw() => ServiceState::StopPending,
            x if x == ServiceState::Running.to_raw() => ServiceState::Running,
            x if x == ServiceState::ContinuePending.to_raw() => ServiceState::ContinuePending,
            x if x == ServiceState::PausePending.to_raw() => ServiceState::PausePending,
            x if x == ServiceState::Paused.to_raw() => ServiceState::Paused,
            other => Err(ErrorKind::InvalidServiceState(other))?,
        };
        Ok(service_state)
    }

    fn to_raw(&self) -> u32 {
        *self as u32
    }
}

/// Service exit code abstraction.
///
/// This struct provides a logic around the relationship between [`dwWin32ExitCode`] and
/// [`dwServiceSpecificExitCode`].
///
/// The service can either return a win32 error code or a custom error code. In case of custom
/// error, [`dwWin32ExitCode`] has to be set to [`ERROR_SERVICE_SPECIFIC_ERROR`] and the
/// [`dwServiceSpecificExitCode`] assigned with custom error code.
///
/// Refer to the corresponding MSDN article for more info:\
/// <https://msdn.microsoft.com/en-us/library/windows/desktop/ms685996(v=vs.85).aspx>
///
/// [`dwWin32ExitCode`]: winsvc::SERVICE_STATUS::dwWin32ExitCode
/// [`dwServiceSpecificExitCode`]: winsvc::SERVICE_STATUS::dwServiceSpecificExitCode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceExitCode {
    Win32(u32),
    ServiceSpecific(u32),
}

impl ServiceExitCode {
    /// A `ServiceExitCode` indicating success, no errors.
    pub const NO_ERROR: Self = ServiceExitCode::Win32(NO_ERROR);

    fn copy_to(&self, raw_service_status: &mut winsvc::SERVICE_STATUS) {
        match *self {
            ServiceExitCode::Win32(win32_error_code) => {
                raw_service_status.dwWin32ExitCode = win32_error_code;
                raw_service_status.dwServiceSpecificExitCode = 0;
            }
            ServiceExitCode::ServiceSpecific(service_error_code) => {
                raw_service_status.dwWin32ExitCode = ERROR_SERVICE_SPECIFIC_ERROR;
                raw_service_status.dwServiceSpecificExitCode = service_error_code;
            }
        }
    }
}

impl Default for ServiceExitCode {
    fn default() -> Self {
        Self::NO_ERROR
    }
}

impl<'a> From<&'a winsvc::SERVICE_STATUS> for ServiceExitCode {
    fn from(service_status: &'a winsvc::SERVICE_STATUS) -> Self {
        if service_status.dwWin32ExitCode == ERROR_SERVICE_SPECIFIC_ERROR {
            ServiceExitCode::ServiceSpecific(service_status.dwServiceSpecificExitCode)
        } else {
            ServiceExitCode::Win32(service_status.dwWin32ExitCode)
        }
    }
}

bitflags! {
    /// Flags describing accepted types of service control events.
    pub struct ServiceControlAccept: u32 {
        /// The service is a network component that can accept changes in its binding without being
        /// stopped and restarted. This allows service to receive `ServiceControl::Netbind*`
        /// family of events.
        const NETBIND_CHANGE = winsvc::SERVICE_ACCEPT_NETBINDCHANGE;

        /// The service can reread its startup parameters without being stopped and restarted.
        const PARAM_CHANGE = winsvc::SERVICE_ACCEPT_PARAMCHANGE;

        /// The service can be paused and continued.
        const PAUSE_CONTINUE = winsvc::SERVICE_ACCEPT_PAUSE_CONTINUE;

        /// The service can perform preshutdown tasks.
        /// Mutually exclusive with shutdown.
        const PRESHUTDOWN = winsvc::SERVICE_ACCEPT_PRESHUTDOWN;

        /// The service is notified when system shutdown occurs.
        /// Mutually exclusive with preshutdown.
        const SHUTDOWN = winsvc::SERVICE_ACCEPT_SHUTDOWN;

        /// The service can be stopped.
        const STOP = winsvc::SERVICE_ACCEPT_STOP;
    }
}

/// Service status.
///
/// This struct wraps the lower level [`SERVICE_STATUS`] providing a few convenience types to fill
/// in the service status information. However it doesn't fully guard the developer from producing
/// an invalid `ServiceStatus`, therefore please refer to the corresponding MSDN article and in
/// particular how to fill in the `exit_code`, `checkpoint`, `wait_hint` fields:\
/// <https://msdn.microsoft.com/en-us/library/windows/desktop/ms685996(v=vs.85).aspx>
///
/// [`SERVICE_STATUS`]: winsvc::SERVICE_STATUS
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceStatus {
    /// Type of service.
    pub service_type: ServiceType,

    /// Current state of the service.
    pub current_state: ServiceState,

    /// Control commands that service accepts.
    pub controls_accepted: ServiceControlAccept,

    /// The error code the service uses to report an error that occurs when it is starting or
    /// stopping.
    pub exit_code: ServiceExitCode,

    /// Service initialization progress value that should be increased during a lengthy start,
    /// stop, pause or continue operations. For example the service should increment the value as
    /// it completes each step of initialization.
    /// This value must be zero if the service does not have any pending start, stop, pause or
    /// continue operations.
    pub checkpoint: u32,

    /// Estimated time for pending operation.
    /// This basically works as a timeout until the system assumes that the service hung.
    /// This could be either circumvented by updating the [`ServiceStatus::current_state`] or
    /// incrementing a [`ServiceStatus::checkpoint`] value.
    pub wait_hint: Duration,
}

impl ServiceStatus {
    pub(crate) fn to_raw(&self) -> winsvc::SERVICE_STATUS {
        let mut raw_status = unsafe { mem::zeroed::<winsvc::SERVICE_STATUS>() };
        raw_status.dwServiceType = self.service_type.to_raw();
        raw_status.dwCurrentState = self.current_state.to_raw();
        raw_status.dwControlsAccepted = self.controls_accepted.bits();

        self.exit_code.copy_to(&mut raw_status);

        raw_status.dwCheckPoint = self.checkpoint;

        // we lose precision here but dwWaitHint should never be too big.
        raw_status.dwWaitHint = (self.wait_hint.as_secs() * 1000) as u32;

        raw_status
    }

    fn from_raw(raw_status: winsvc::SERVICE_STATUS) -> Result<Self> {
        Ok(ServiceStatus {
            service_type: ServiceType::from_raw(raw_status.dwServiceType)?,
            current_state: ServiceState::from_raw(raw_status.dwCurrentState)?,
            controls_accepted: ServiceControlAccept::from_bits_truncate(
                raw_status.dwControlsAccepted,
            ),
            exit_code: ServiceExitCode::from(&raw_status),
            checkpoint: raw_status.dwCheckPoint,
            wait_hint: Duration::from_millis(raw_status.dwWaitHint as u64),
        })
    }
}

/// A struct that represents a system service.
///
/// The instances of the [`Service`] can be obtained via [`ServiceManager`].
///
/// [`ServiceManager`]: super::service_manager::ServiceManager
pub struct Service {
    service_handle: ScHandle,
}

impl Service {
    pub(crate) fn new(service_handle: ScHandle) -> Self {
        Service { service_handle }
    }

    /// Start the service.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use std::ffi::OsStr;
    /// use windows_service::service::ServiceAccess;
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// # fn main() -> windows_service::Result<()> {
    /// let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    /// let my_service = manager.open_service("my_service", ServiceAccess::START)?;
    /// my_service.start(&[OsStr::new("Started from Rust!")])?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn start<S: AsRef<OsStr>>(&self, service_arguments: &[S]) -> Result<()> {
        let wide_service_arguments = service_arguments
            .iter()
            .map(|s| WideCString::from_str(s).chain_err(|| ErrorKind::InvalidStartArgument))
            .collect::<Result<Vec<WideCString>>>()?;
        let mut raw_service_arguments: Vec<*const u16> =
            wide_service_arguments.iter().map(|s| s.as_ptr()).collect();

        let success = unsafe {
            winsvc::StartServiceW(
                self.service_handle.raw_handle(),
                raw_service_arguments.len() as u32,
                raw_service_arguments.as_mut_ptr(),
            )
        };

        if success == 0 {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(())
        }
    }

    /// Stop the service.
    pub fn stop(&self) -> Result<ServiceStatus> {
        self.send_control_command(ServiceControl::Stop)
    }

    /// Get the service status from the system.
    pub fn query_status(&self) -> Result<ServiceStatus> {
        let mut raw_status = unsafe { mem::zeroed::<winsvc::SERVICE_STATUS>() };
        let success = unsafe {
            winsvc::QueryServiceStatus(self.service_handle.raw_handle(), &mut raw_status)
        };
        if success == 0 {
            Err(io::Error::last_os_error().into())
        } else {
            ServiceStatus::from_raw(raw_status)
        }
    }

    /// Delete the service from system registry.
    pub fn delete(self) -> io::Result<()> {
        let success = unsafe { winsvc::DeleteService(self.service_handle.raw_handle()) };
        if success == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Private helper to send the control commands to the system.
    fn send_control_command(&self, command: ServiceControl) -> Result<ServiceStatus> {
        let mut raw_status = unsafe { mem::zeroed::<winsvc::SERVICE_STATUS>() };
        let success = unsafe {
            winsvc::ControlService(
                self.service_handle.raw_handle(),
                command.to_raw(),
                &mut raw_status,
            )
        };

        if success == 0 {
            Err(io::Error::last_os_error().into())
        } else {
            ServiceStatus::from_raw(raw_status).map_err(|err| err.into())
        }
    }

    /// Query service config and convert to ServiceConfig
    fn query_service_config(&self, mut size: u32) -> Result<ServiceConfig> {
        let mut data = vec![0; size as usize];
        match unsafe {
            winsvc::QueryServiceConfigW(
                self.service_handle.raw_handle(),
                data.as_mut_ptr() as _,
                data.len() as u32,
                &mut size,
            )
        } {
            0 => Err(io::Error::last_os_error().into()),
            _ => {
                let raw_config: winsvc::QUERY_SERVICE_CONFIGW =
                    unsafe { ::std::ptr::read(data.as_mut_ptr() as _) };
                ServiceConfig::from_raw(raw_config)
            }
        }
    }

    /// Get the service config from the system.
    pub fn query_config(&self) -> Result<ServiceConfig> {
        let mut bytes_needed: u32 = 0;
        match unsafe {
            winsvc::QueryServiceConfigW(
                self.service_handle.raw_handle(),
                ::std::ptr::null_mut() as _,
                0,
                &mut bytes_needed,
            )
        } {
            0 => match io::Error::last_os_error().raw_os_error() {
                Some(e) => match e {
                    i if i == ERROR_INSUFFICIENT_BUFFER as i32 => {
                        self.query_service_config(bytes_needed)
                    }
                    _ => Err(io::Error::last_os_error().into()),
                },
                None => Err(io::Error::last_os_error().into()),
            },
            _ => self.query_service_config(bytes_needed),
        }
    }
}
