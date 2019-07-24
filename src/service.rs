use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr;
use std::time::Duration;
use std::{io, mem};

use widestring::{NulError, WideCStr, WideCString};
use winapi::shared::minwindef::DWORD;
use winapi::shared::winerror::{ERROR_SERVICE_SPECIFIC_ERROR, NO_ERROR};
use winapi::um::winbase::INFINITE;
use winapi::um::{winnt, winsvc};

use crate::sc_handle::ScHandle;
use crate::{double_nul_terminated, Error};

bitflags::bitflags! {
    /// Enum describing the types of Windows services.
    pub struct ServiceType: DWORD {
        /// File system driver service.
        const FILE_SYSTEM_DRIVER = winnt::SERVICE_FILE_SYSTEM_DRIVER;

        /// Driver service.
        const KERNEL_DRIVER = winnt::SERVICE_KERNEL_DRIVER;

        /// Service that runs in its own process.
        const OWN_PROCESS = winnt::SERVICE_WIN32_OWN_PROCESS;

        /// Service that shares a process with one or more other services.
        const SHARE_PROCESS = winnt::SERVICE_WIN32_SHARE_PROCESS;

        /// The service runs in its own process under the logged-on user account.
        const USER_OWN_PROCESS = winnt::SERVICE_USER_OWN_PROCESS;

        /// The service shares a process with one or more other services that run under the logged-on user account.
        const USER_SHARE_PROCESS = winnt::SERVICE_USER_SHARE_PROCESS;

        /// The service can be interactive.
        const INTERACTIVE_PROCESS = winnt::SERVICE_INTERACTIVE_PROCESS;
    }
}

bitflags::bitflags! {
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

        /// Can change the services configuration
        const CHANGE_CONFIG = winsvc::SERVICE_CHANGE_CONFIG;
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

    pub fn from_raw(raw: u32) -> Result<ServiceStartType, ParseRawError> {
        match raw {
            x if x == ServiceStartType::AutoStart.to_raw() => Ok(ServiceStartType::AutoStart),
            x if x == ServiceStartType::OnDemand.to_raw() => Ok(ServiceStartType::OnDemand),
            x if x == ServiceStartType::Disabled.to_raw() => Ok(ServiceStartType::Disabled),
            _ => Err(ParseRawError(raw)),
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

    pub fn from_raw(raw: u32) -> Result<ServiceErrorControl, ParseRawError> {
        match raw {
            x if x == ServiceErrorControl::Critical.to_raw() => Ok(ServiceErrorControl::Critical),
            x if x == ServiceErrorControl::Ignore.to_raw() => Ok(ServiceErrorControl::Ignore),
            x if x == ServiceErrorControl::Normal.to_raw() => Ok(ServiceErrorControl::Normal),
            x if x == ServiceErrorControl::Severe.to_raw() => Ok(ServiceErrorControl::Severe),
            _ => Err(ParseRawError(raw)),
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

    pub fn from_system_identifier<S: AsRef<OsStr>>(identifier: S) -> Self {
        let group_prefix: u16 = '+' as u16;
        let mut iter = identifier.as_ref().encode_wide().peekable();

        if iter.peek() == Some(&group_prefix) {
            let chars: Vec<u16> = iter.skip(1).collect();
            let group_name = OsString::from_wide(&chars);
            ServiceDependency::Group(group_name)
        } else {
            let chars: Vec<u16> = iter.collect();
            let service_name = OsString::from_wide(&chars);
            ServiceDependency::Service(service_name)
        }
    }
}

/// Enum describing the types of actions that the service control manager can perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceActionType {
    None = winsvc::SC_ACTION_NONE,
    Reboot = winsvc::SC_ACTION_REBOOT,
    Restart = winsvc::SC_ACTION_RESTART,
    RunCommand = winsvc::SC_ACTION_RUN_COMMAND,
}

impl ServiceActionType {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<ServiceActionType, ParseRawError> {
        match raw {
            x if x == ServiceActionType::None.to_raw() => Ok(ServiceActionType::None),
            x if x == ServiceActionType::Reboot.to_raw() => Ok(ServiceActionType::Reboot),
            x if x == ServiceActionType::Restart.to_raw() => Ok(ServiceActionType::Restart),
            x if x == ServiceActionType::RunCommand.to_raw() => Ok(ServiceActionType::RunCommand),
            _ => Err(ParseRawError(raw)),
        }
    }
}

/// Represents an action that the service control manager can perform.
///
/// See <https://docs.microsoft.com/en-us/windows/win32/api/winsvc/ns-winsvc-sc_action>
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceAction {
    /// The action to be performed.
    pub action_type: ServiceActionType,

    /// The time to wait before performing the specified action
    pub delay: Duration,
}

impl ServiceAction {
    pub fn from_raw(raw: winsvc::SC_ACTION) -> crate::Result<ServiceAction> {
        Ok(ServiceAction {
            action_type: ServiceActionType::from_raw(raw.Type)
                .map_err(Error::InvalidServiceActionType)?,
            delay: Duration::from_secs(raw.Delay as u64),
        })
    }

    pub fn to_raw(&self) -> winsvc::SC_ACTION {
        winsvc::SC_ACTION {
            Type: self.action_type.to_raw(),
            Delay: self.delay.as_secs() as DWORD,
        }
    }
}

/// A enum that representats the reset period for the failure counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceFailureResetPeriod {
    Never,
    After(Duration),
}

impl ServiceFailureResetPeriod {
    pub fn from_raw(raw: DWORD) -> ServiceFailureResetPeriod {
        match raw {
            INFINITE => ServiceFailureResetPeriod::Never,
            _ => ServiceFailureResetPeriod::After(Duration::from_secs(raw as u64)),
        }
    }

    pub fn to_raw(&self) -> DWORD {
        match self {
            ServiceFailureResetPeriod::Never => INFINITE,
            ServiceFailureResetPeriod::After(ref duration) => duration.as_secs() as DWORD,
        }
    }
}

/// A struct that describes the action that should be performed on the system service crash.
///
/// Please refer to MSDN for more info:\
/// <https://docs.microsoft.com/en-us/windows/win32/api/winsvc/ns-winsvc-_service_failure_actionsw>
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceFailureActions {
    /// The time after which to reset the failure count to zero if there are no failures, in
    /// seconds.
    pub reset_period: ServiceFailureResetPeriod,

    /// The message to be broadcast to server users before rebooting in response to the
    /// `SC_ACTION_REBOOT` service controller action.
    ///
    /// If this value is `None`, the reboot message is unchanged.
    /// If the value is an empty string, the reboot message is deleted and no message is broadcast.
    pub reboot_msg: Option<OsString>,

    /// The command line to execute in response to the `SC_ACTION_RUN_COMMAND` service controller
    /// action. This process runs under the same account as the service.
    ///
    /// If this value is `None`, the command is unchanged. If the value is an empty string, the
    /// command is deleted and no program is run when the service fails.
    pub command: Option<OsString>,

    /// The array of actions to perform.
    /// If this value is `None`, the [`ServiceFailureActions::reset_period`] member is ignored.
    pub actions: Option<Vec<ServiceAction>>,
}

impl ServiceFailureActions {
    /// Tries to parse a `SERVICE_FAILURE_ACTIONSW` into Rust [`ServiceFailureActions`].
    ///
    /// # Errors
    ///
    /// Returns an error if a field inside the `SERVICE_FAILURE_ACTIONSW` does not have a valid
    /// value.
    pub unsafe fn from_raw(
        raw: winsvc::SERVICE_FAILURE_ACTIONSW,
    ) -> crate::Result<ServiceFailureActions> {
        let reboot_msg = ptr::NonNull::new(raw.lpRebootMsg)
            .map(|wrapped_ptr| WideCStr::from_ptr_str(wrapped_ptr.as_ptr()).to_os_string());
        let command = ptr::NonNull::new(raw.lpCommand)
            .map(|wrapped_ptr| WideCStr::from_ptr_str(wrapped_ptr.as_ptr()).to_os_string());
        let reset_period = ServiceFailureResetPeriod::from_raw(raw.dwResetPeriod);

        let actions: Option<Vec<ServiceAction>> = if raw.lpsaActions.is_null() {
            None
        } else {
            Some(
                (0..raw.cActions)
                    .map(|i| {
                        let array_element_ptr: *mut winsvc::SC_ACTION =
                            raw.lpsaActions.offset(i as isize);
                        ServiceAction::from_raw(*array_element_ptr)
                    })
                    .collect::<crate::Result<Vec<ServiceAction>>>()?,
            )
        };

        Ok(ServiceFailureActions {
            reset_period,
            reboot_msg,
            command,
            actions,
        })
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
    pub dependencies: Vec<ServiceDependency>,

    /// Account to use for running the service.
    /// for example: NT Authority\System.
    ///
    /// This value can be `None` in certain cases, please refer to MSDN for more info:\
    /// <https://docs.microsoft.com/en-us/windows/desktop/api/winsvc/ns-winsvc-_query_service_configw>
    pub account_name: Option<OsString>,

    /// User-friendly service name
    pub display_name: OsString,
}

impl ServiceConfig {
    /// Tries to parse a `QUERY_SERVICE_CONFIGW` into Rust [`ServiceConfig`].
    ///
    /// # Errors
    ///
    /// Returns an error if a field inside the `QUERY_SERVICE_CONFIGW` does not have a valid value.
    pub unsafe fn from_raw(raw: winsvc::QUERY_SERVICE_CONFIGW) -> crate::Result<ServiceConfig> {
        let dependencies = double_nul_terminated::parse_str_ptr(raw.lpDependencies)
            .iter()
            .map(ServiceDependency::from_system_identifier)
            .collect();

        let load_order_group = ptr::NonNull::new(raw.lpLoadOrderGroup).and_then(|wrapped_ptr| {
            let group = WideCStr::from_ptr_str(wrapped_ptr.as_ptr()).to_os_string();
            // Return None for consistency, because lpLoadOrderGroup can be either nul or empty
            // string, which has the same meaning.
            if group.is_empty() {
                None
            } else {
                Some(group)
            }
        });

        let account_name = ptr::NonNull::new(raw.lpServiceStartName)
            .map(|wrapped_ptr| WideCStr::from_ptr_str(wrapped_ptr.as_ptr()).to_os_string());

        Ok(ServiceConfig {
            service_type: ServiceType::from_bits_truncate(raw.dwServiceType),
            start_type: ServiceStartType::from_raw(raw.dwStartType)
                .map_err(Error::InvalidServiceStartType)?,
            error_control: ServiceErrorControl::from_raw(raw.dwErrorControl)
                .map_err(Error::InvalidServiceErrorControl)?,
            executable_path: PathBuf::from(
                WideCStr::from_ptr_str(raw.lpBinaryPathName).to_os_string(),
            ),
            load_order_group,
            tag_id: raw.dwTagId,
            dependencies,
            account_name,
            display_name: WideCStr::from_ptr_str(raw.lpDisplayName).to_os_string(),
        })
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
    pub fn from_raw(raw: u32) -> Result<Self, ParseRawError> {
        match raw {
            x if x == ServiceControl::Continue.to_raw() => Ok(ServiceControl::Continue),
            x if x == ServiceControl::Interrogate.to_raw() => Ok(ServiceControl::Interrogate),
            x if x == ServiceControl::NetBindAdd.to_raw() => Ok(ServiceControl::NetBindAdd),
            x if x == ServiceControl::NetBindDisable.to_raw() => Ok(ServiceControl::NetBindDisable),
            x if x == ServiceControl::NetBindEnable.to_raw() => Ok(ServiceControl::NetBindEnable),
            x if x == ServiceControl::NetBindRemove.to_raw() => Ok(ServiceControl::NetBindRemove),
            x if x == ServiceControl::ParamChange.to_raw() => Ok(ServiceControl::ParamChange),
            x if x == ServiceControl::Pause.to_raw() => Ok(ServiceControl::Pause),
            x if x == ServiceControl::Preshutdown.to_raw() => Ok(ServiceControl::Preshutdown),
            x if x == ServiceControl::Shutdown.to_raw() => Ok(ServiceControl::Shutdown),
            x if x == ServiceControl::Stop.to_raw() => Ok(ServiceControl::Stop),
            _ => Err(ParseRawError(raw)),
        }
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
    fn from_raw(raw: u32) -> Result<Self, ParseRawError> {
        match raw {
            x if x == ServiceState::Stopped.to_raw() => Ok(ServiceState::Stopped),
            x if x == ServiceState::StartPending.to_raw() => Ok(ServiceState::StartPending),
            x if x == ServiceState::StopPending.to_raw() => Ok(ServiceState::StopPending),
            x if x == ServiceState::Running.to_raw() => Ok(ServiceState::Running),
            x if x == ServiceState::ContinuePending.to_raw() => Ok(ServiceState::ContinuePending),
            x if x == ServiceState::PausePending.to_raw() => Ok(ServiceState::PausePending),
            x if x == ServiceState::Paused.to_raw() => Ok(ServiceState::Paused),
            _ => Err(ParseRawError(raw)),
        }
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

bitflags::bitflags! {
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
        raw_status.dwServiceType = self.service_type.bits();
        raw_status.dwCurrentState = self.current_state.to_raw();
        raw_status.dwControlsAccepted = self.controls_accepted.bits();

        self.exit_code.copy_to(&mut raw_status);

        raw_status.dwCheckPoint = self.checkpoint;

        // we lose precision here but dwWaitHint should never be too big.
        raw_status.dwWaitHint = (self.wait_hint.as_secs() * 1000) as u32;

        raw_status
    }

    /// Tries to parse a `SERVICE_STATUS` into a Rust [`ServiceStatus`].
    ///
    /// # Errors
    ///
    /// Returns an error if the `dwCurrentState` field does not represent a valid [`ServiceState`].
    fn from_raw(raw: winsvc::SERVICE_STATUS) -> Result<Self, ParseRawError> {
        Ok(ServiceStatus {
            service_type: ServiceType::from_bits_truncate(raw.dwServiceType),
            current_state: ServiceState::from_raw(raw.dwCurrentState)?,
            controls_accepted: ServiceControlAccept::from_bits_truncate(raw.dwControlsAccepted),
            exit_code: ServiceExitCode::from(&raw),
            checkpoint: raw.dwCheckPoint,
            wait_hint: Duration::from_millis(raw.dwWaitHint as u64),
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
    pub fn start<S: AsRef<OsStr>>(&self, service_arguments: &[S]) -> crate::Result<()> {
        let wide_service_arguments = service_arguments
            .iter()
            .map(|s| WideCString::from_str(s).map_err(Error::InvalidStartArgument))
            .collect::<crate::Result<Vec<WideCString>>>()?;

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
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    /// Stop the service.
    pub fn stop(&self) -> crate::Result<ServiceStatus> {
        self.send_control_command(ServiceControl::Stop)
    }

    /// Get the service status from the system.
    pub fn query_status(&self) -> crate::Result<ServiceStatus> {
        let mut raw_status = unsafe { mem::zeroed::<winsvc::SERVICE_STATUS>() };
        let success = unsafe {
            winsvc::QueryServiceStatus(self.service_handle.raw_handle(), &mut raw_status)
        };
        if success == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            ServiceStatus::from_raw(raw_status).map_err(Error::InvalidServiceState)
        }
    }

    /// Delete the service from system registry.
    pub fn delete(self) -> crate::Result<()> {
        let success = unsafe { winsvc::DeleteService(self.service_handle.raw_handle()) };
        if success == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    /// Get the service config from the system.
    pub fn query_config(&self) -> crate::Result<ServiceConfig> {
        // As per docs, the maximum size of data buffer used by QueryServiceConfigW is 8K
        let mut data = [0u8; 8096];
        let mut bytes_written: u32 = 0;

        let success = unsafe {
            winsvc::QueryServiceConfigW(
                self.service_handle.raw_handle(),
                data.as_mut_ptr() as _,
                data.len() as u32,
                &mut bytes_written,
            )
        };

        if success == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            unsafe {
                let raw_config = data.as_ptr() as *const winsvc::QUERY_SERVICE_CONFIGW;
                ServiceConfig::from_raw(*raw_config)
            }
        }
    }

    /// Configure failure actions to run when the service terminates before reporting the
    /// [`ServiceState::Stopped`] back to the system or if it exits with non-zero
    /// [`ServiceExitCode`].
    ///
    /// Please refer to MSDN for more info:\
    /// <https://docs.microsoft.com/en-us/windows/win32/api/winsvc/ns-winsvc-_service_failure_actions_flag>
    pub fn set_failure_actions_on_non_crash_failures(&self, enabled: bool) -> crate::Result<()> {
        let mut raw_failure_actions_flag =
            unsafe { mem::zeroed::<winsvc::SERVICE_FAILURE_ACTIONS_FLAG>() };

        raw_failure_actions_flag.fFailureActionsOnNonCrashFailures = if enabled { 1 } else { 0 };

        unsafe {
            self.change_config2(
                winsvc::SERVICE_CONFIG_FAILURE_ACTIONS_FLAG,
                &mut raw_failure_actions_flag,
            )
            .map_err(Error::Winapi)
        }
    }

    /// Query the system for the boolean indication that the service is configured to run failure
    /// actions on non-crash failures.
    pub fn get_failure_actions_on_non_crash_failures(&self) -> crate::Result<bool> {
        let mut data = [0u8; 8096];

        let raw_failure_actions_flag: winsvc::SERVICE_FAILURE_ACTIONS_FLAG = unsafe {
            self.query_config2(winsvc::SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, &mut data)
                .map_err(Error::Winapi)?
        };

        let result = if raw_failure_actions_flag.fFailureActionsOnNonCrashFailures == 0 {
            false
        } else {
            true
        };

        Ok(result)
    }

    /// Query the configured failure actions for the service.
    pub fn get_failure_actions(&self) -> crate::Result<ServiceFailureActions> {
        unsafe {
            let mut data = [0u8; 8096];

            let raw_failure_actions: winsvc::SERVICE_FAILURE_ACTIONSW = self
                .query_config2(winsvc::SERVICE_CONFIG_FAILURE_ACTIONS, &mut data)
                .map_err(Error::Winapi)?;

            ServiceFailureActions::from_raw(raw_failure_actions)
        }
    }

    /// Update failure actions.
    ///
    /// Pass `None` for optional fields to keep the corresponding fields unchanged, or pass an empty
    /// value to reset them.
    ///
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use std::ffi::OsString;
    /// use std::time::Duration;
    /// use windows_service::service::{
    ///     ServiceAccess, ServiceAction, ServiceActionType, ServiceFailureActions,
    ///     ServiceFailureResetPeriod,
    /// };
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// # fn main() -> windows_service::Result<()> {
    /// let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    /// let my_service = manager.open_service(
    ///     "my_service",
    ///     ServiceAccess::START | ServiceAccess::CHANGE_CONFIG,
    /// )?;
    ///
    /// let actions = vec![
    ///     ServiceAction {
    ///         action_type: ServiceActionType::Restart,
    ///         delay: Duration::from_secs(5),
    ///     },
    ///     ServiceAction {
    ///         action_type: ServiceActionType::RunCommand,
    ///         delay: Duration::from_secs(10),
    ///     },
    ///     ServiceAction {
    ///         action_type: ServiceActionType::None,
    ///         delay: Duration::default(),
    ///     },
    /// ];
    ///
    /// let failure_actions = ServiceFailureActions {
    ///     reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
    ///     reboot_msg: None,
    ///     command: Some(OsString::from("ping 127.0.0.1")),
    ///     actions: Some(actions),
    /// };
    ///
    /// my_service.update_failure_actions(failure_actions)?;
    /// #    Ok(())
    /// # }
    /// ```
    pub fn update_failure_actions(&self, update: ServiceFailureActions) -> crate::Result<()> {
        let mut raw_failure_actions = unsafe { mem::zeroed::<winsvc::SERVICE_FAILURE_ACTIONSW>() };

        let mut reboot_msg = to_wide_slice(update.reboot_msg)
            .map_err(Error::InvalidServiceActionFailuresRebootMessage)?;
        let mut command =
            to_wide_slice(update.command).map_err(Error::InvalidServiceActionFailuresCommand)?;
        let mut sc_actions: Option<Vec<winsvc::SC_ACTION>> = update
            .actions
            .map(|actions| actions.iter().map(ServiceAction::to_raw).collect());

        raw_failure_actions.dwResetPeriod = update.reset_period.to_raw();
        raw_failure_actions.lpRebootMsg = reboot_msg
            .as_mut()
            .map_or(ptr::null_mut(), |s| s.as_mut_ptr());
        raw_failure_actions.lpCommand =
            command.as_mut().map_or(ptr::null_mut(), |s| s.as_mut_ptr());
        raw_failure_actions.cActions = sc_actions.as_ref().map_or(0, |v| v.len()) as u32;
        raw_failure_actions.lpsaActions = sc_actions
            .as_mut()
            .map_or(ptr::null_mut(), |actions| actions.as_mut_ptr());

        unsafe {
            self.change_config2(
                winsvc::SERVICE_CONFIG_FAILURE_ACTIONS,
                &mut raw_failure_actions,
            )
            .map_err(Error::Winapi)
        }
    }

    /// Private helper to send the control commands to the system.
    fn send_control_command(&self, command: ServiceControl) -> crate::Result<ServiceStatus> {
        let mut raw_status = unsafe { mem::zeroed::<winsvc::SERVICE_STATUS>() };
        let success = unsafe {
            winsvc::ControlService(
                self.service_handle.raw_handle(),
                command.to_raw(),
                &mut raw_status,
            )
        };

        if success == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            ServiceStatus::from_raw(raw_status).map_err(Error::InvalidServiceState)
        }
    }

    /// Private helper to query the optional configuration parameters of windows services.
    /// As per docs, the maximum size of data buffer used by QueryServiceConfig2W is 8K
    unsafe fn query_config2<T: Copy>(&self, kind: DWORD, data: &mut [u8; 8096]) -> io::Result<T> {
        let mut bytes_written: u32 = 0;

        let success = winsvc::QueryServiceConfig2W(
            self.service_handle.raw_handle(),
            kind,
            data.as_mut_ptr() as _,
            data.len() as u32,
            &mut bytes_written,
        );

        if success == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(*(data.as_ptr() as *const _))
        }
    }

    /// Private helper to update the optional configuration parameters of windows services.
    unsafe fn change_config2<T>(&self, kind: DWORD, data: &mut T) -> io::Result<()> {
        let success = winsvc::ChangeServiceConfig2W(
            self.service_handle.raw_handle(),
            kind,
            data as *mut _ as *mut _,
        );

        if success == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

fn to_wide_slice<T: AsRef<OsStr>>(
    s: Option<T>,
) -> ::std::result::Result<Option<Vec<u16>>, NulError> {
    if let Some(s) = s {
        Ok(Some(
            WideCString::from_str(s).map(|s| s.as_slice_with_nul().to_vec())?,
        ))
    } else {
        Ok(None)
    }
}


#[derive(err_derive::Error, Debug)]
#[error(display = "Invalid integer value for the target type: {}", _0)]
pub struct ParseRawError(u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_group_identifier() {
        let dependency = ServiceDependency::from_system_identifier("+network");
        assert_eq!(
            dependency,
            ServiceDependency::Group(OsString::from("network"))
        );
    }

    #[test]
    fn test_service_name_identifier() {
        let dependency = ServiceDependency::from_system_identifier("netlogon");
        assert_eq!(
            dependency,
            ServiceDependency::Service(OsString::from("netlogon"))
        );
    }
}
