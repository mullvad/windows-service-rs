use std::borrow::Cow;
use std::convert::TryFrom;
use std::ffi::{OsStr, OsString};
use std::os::raw::c_void;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr;
use std::time::Duration;
use std::{io, mem};

use widestring::{error::ContainsNul, WideCStr, WideCString, WideString};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{ERROR_SERVICE_SPECIFIC_ERROR, NO_ERROR},
        Storage::FileSystem,
        System::{Power, RemoteDesktop, Services, SystemServices, WindowsProgramming::INFINITE},
        UI::WindowsAndMessaging,
    },
};

use crate::sc_handle::ScHandle;
use crate::shell_escape;
use crate::{double_nul_terminated, Error};

bitflags::bitflags! {
    /// Flags describing the access permissions when working with services
    pub struct ServiceAccess: u32 {
        /// Can query the service status
        const QUERY_STATUS = Services::SERVICE_QUERY_STATUS;

        /// Can start the service
        const START = Services::SERVICE_START;

        /// Can stop the service
        const STOP = Services::SERVICE_STOP;

        /// Can pause or continue the service execution
        const PAUSE_CONTINUE = Services::SERVICE_PAUSE_CONTINUE;

        /// Can ask the service to report its status
        const INTERROGATE = Services::SERVICE_INTERROGATE;

        /// Can delete the service
        const DELETE = FileSystem::DELETE;

        /// Can query the services configuration
        const QUERY_CONFIG = Services::SERVICE_QUERY_CONFIG;

        /// Can change the services configuration
        const CHANGE_CONFIG = Services::SERVICE_CHANGE_CONFIG;
    }
}

/// Enum describing the start options for windows services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceStartType {
    /// Autostart on system startup
    AutoStart = Services::SERVICE_AUTO_START,
    /// Service is enabled, can be started manually
    OnDemand = Services::SERVICE_DEMAND_START,
    /// Disabled service
    Disabled = Services::SERVICE_DISABLED,
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
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }
}

/// Error handling strategy for service failures.
///
/// See <https://msdn.microsoft.com/en-us/library/windows/desktop/ms682450(v=vs.85).aspx>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceErrorControl {
    Critical = Services::SERVICE_ERROR_CRITICAL,
    Ignore = Services::SERVICE_ERROR_IGNORE,
    Normal = Services::SERVICE_ERROR_NORMAL,
    Severe = Services::SERVICE_ERROR_SEVERE,
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
            _ => Err(ParseRawError::InvalidInteger(raw)),
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

    pub fn from_system_identifier(identifier: impl AsRef<OsStr>) -> Self {
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
#[repr(i32)]
pub enum ServiceActionType {
    None = Services::SC_ACTION_NONE,
    Reboot = Services::SC_ACTION_REBOOT,
    Restart = Services::SC_ACTION_RESTART,
    RunCommand = Services::SC_ACTION_RUN_COMMAND,
}

impl ServiceActionType {
    pub fn to_raw(&self) -> i32 {
        *self as i32
    }

    pub fn from_raw(raw: i32) -> Result<ServiceActionType, ParseRawError> {
        match raw {
            x if x == ServiceActionType::None.to_raw() => Ok(ServiceActionType::None),
            x if x == ServiceActionType::Reboot.to_raw() => Ok(ServiceActionType::Reboot),
            x if x == ServiceActionType::Restart.to_raw() => Ok(ServiceActionType::Restart),
            x if x == ServiceActionType::RunCommand.to_raw() => Ok(ServiceActionType::RunCommand),
            _ => Err(ParseRawError::InvalidIntegerSigned(raw)),
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
    ///
    /// # Panics
    ///
    /// Converting this to the FFI form will panic if the delay is too large to fit as milliseconds
    /// in a `u32`.
    pub delay: Duration,
}

impl ServiceAction {
    pub fn from_raw(raw: Services::SC_ACTION) -> crate::Result<ServiceAction> {
        Ok(ServiceAction {
            action_type: ServiceActionType::from_raw(raw.Type)
                .map_err(|e| Error::ParseValue("service action type", e))?,
            delay: Duration::from_millis(raw.Delay as u64),
        })
    }

    pub fn to_raw(&self) -> Services::SC_ACTION {
        Services::SC_ACTION {
            Type: self.action_type.to_raw(),
            Delay: u32::try_from(self.delay.as_millis()).expect("Too long delay"),
        }
    }
}

/// A enum that represents the reset period for the failure counter.
///
/// # Panics
///
/// Converting this to the FFI form will panic if the period is too large to fit as seconds in a
/// `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceFailureResetPeriod {
    Never,
    After(Duration),
}

impl ServiceFailureResetPeriod {
    pub fn from_raw(raw: u32) -> ServiceFailureResetPeriod {
        match raw {
            INFINITE => ServiceFailureResetPeriod::Never,
            _ => ServiceFailureResetPeriod::After(Duration::from_secs(raw as u64)),
        }
    }

    pub fn to_raw(&self) -> u32 {
        match self {
            ServiceFailureResetPeriod::Never => INFINITE,
            ServiceFailureResetPeriod::After(duration) => {
                u32::try_from(duration.as_secs()).expect("Too long reset period")
            }
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
    /// Returns an error if any of the `SC_ACTION`s pointed to by `lpsaActions` does not
    /// successfully convert into a [`ServiceAction`].
    ///
    /// # Safety
    ///
    /// The `SERVICE_FAILURE_ACTIONSW` fields `lpRebootMsg`, `lpCommand` must be either null
    /// or proper null terminated wide C strings.
    /// `lpsaActions` must be either null or an array with `cActions` number of of `SC_ACTION`s.
    pub unsafe fn from_raw(
        raw: Services::SERVICE_FAILURE_ACTIONSW,
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
                        let array_element_ptr: *mut Services::SC_ACTION =
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

/// Same as `ServiceInfo` but with fields that are compatible with the Windows API.
pub(crate) struct RawServiceInfo {
    /// Service name
    pub name: WideCString,

    /// User-friendly service name
    pub display_name: WideCString,

    /// The service type
    pub service_type: u32,

    /// The service startup options
    pub start_type: u32,

    /// The severity of the error, and action taken, if this service fails to start.
    pub error_control: u32,

    /// Path to the service binary with arguments appended
    pub launch_command: WideCString,

    /// Service dependencies
    pub dependencies: Option<WideString>,

    /// Account to use for running the service.
    /// for example: NT Authority\System.
    /// use `None` to run as LocalSystem.
    pub account_name: Option<WideCString>,

    /// Account password.
    /// For system accounts this should normally be `None`.
    pub account_password: Option<WideCString>,
}

impl RawServiceInfo {
    pub fn new(service_info: &ServiceInfo) -> crate::Result<Self> {
        let service_name = WideCString::from_os_str(&service_info.name)
            .map_err(|_| Error::ArgumentHasNulByte("service name"))?;
        let display_name = WideCString::from_os_str(&service_info.display_name)
            .map_err(|_| Error::ArgumentHasNulByte("display name"))?;
        let account_name = to_wide(service_info.account_name.as_ref())
            .map_err(|_| Error::ArgumentHasNulByte("account name"))?;
        let account_password = to_wide(service_info.account_password.as_ref())
            .map_err(|_| Error::ArgumentHasNulByte("account password"))?;

        // escape executable path and arguments and combine them into a single command
        let mut launch_command_buffer = WideString::new();
        if service_info
            .service_type
            .intersects(ServiceType::KERNEL_DRIVER | ServiceType::FILE_SYSTEM_DRIVER)
        {
            // drivers do not support launch arguments
            if !service_info.launch_arguments.is_empty() {
                return Err(Error::LaunchArgumentsNotSupported);
            }

            // also the path must not be quoted even if it contains spaces
            let executable_path = WideCString::from_os_str(&service_info.executable_path)
                .map_err(|_| Error::ArgumentHasNulByte("executable path"))?;
            launch_command_buffer.push(executable_path.to_ustring());
        } else {
            let executable_path = escape_wide(&service_info.executable_path)
                .map_err(|_| Error::ArgumentHasNulByte("executable path"))?;
            launch_command_buffer.push(executable_path);

            for (i, launch_argument) in service_info.launch_arguments.iter().enumerate() {
                let wide = escape_wide(launch_argument)
                    .map_err(|_| Error::ArgumentArrayElementHasNulByte("launch argument", i))?;

                launch_command_buffer.push_str(" ");
                launch_command_buffer.push(wide);
            }
        }

        // Safety: We are sure launch_command_buffer does not contain nulls
        let launch_command = unsafe { WideCString::from_ustr_unchecked(launch_command_buffer) };

        let dependency_identifiers: Vec<OsString> = service_info
            .dependencies
            .iter()
            .map(|dependency| dependency.to_system_identifier())
            .collect();
        let joined_dependencies = double_nul_terminated::from_slice(&dependency_identifiers)
            .map_err(|_| Error::ArgumentHasNulByte("dependency"))?;

        Ok(Self {
            name: service_name,
            display_name,
            service_type: service_info.service_type.bits(),
            start_type: service_info.start_type.to_raw(),
            error_control: service_info.error_control.to_raw(),
            launch_command,
            dependencies: joined_dependencies,
            account_name,
            account_password,
        })
    }
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
    /// Returns an error if `dwStartType` does not successfully convert into a
    /// [`ServiceStartType`], or `dwErrorControl` does not successfully convert
    /// into a [`ServiceErrorControl`].
    ///
    /// # Safety
    ///
    /// `lpDependencies` must contain a wide string where each dependency is delimited with a NUL
    /// and the entire string ends in two NULs.
    ///
    /// `lpLoadOrderGroup`, `lpServiceStartName`, `lpBinaryPathName` and `lpDisplayName` must be
    /// either null or proper null terminated wide C strings.
    pub unsafe fn from_raw(raw: Services::QUERY_SERVICE_CONFIGW) -> crate::Result<ServiceConfig> {
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
                .map_err(|e| Error::ParseValue("service start type", e))?,
            error_control: ServiceErrorControl::from_raw(raw.dwErrorControl)
                .map_err(|e| Error::ParseValue("service error control", e))?,
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

/// This controls how the service SID is added to the service process token.
/// <https://docs.microsoft.com/en-us/windows/win32/api/winsvc/ns-winsvc-service_sid_info>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceSidType {
    None = 0,
    Restricted = 3,
    Unrestricted = 1,
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
            .map(|s| {
                WideCString::from_os_str(s).map_err(|_| Error::ArgumentHasNulByte("start argument"))
            })
            .collect::<crate::Result<Vec<WideCString>>>()?;

        let raw_service_arguments: Vec<*const u16> = wide_service_arguments
            .iter()
            .map(|s| s.as_ptr() as _)
            .collect();

        let success = unsafe {
            Services::StartServiceW(
                self.service_handle.raw_handle(),
                raw_service_arguments.len() as u32,
                raw_service_arguments.as_ptr(),
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

    /// Pause the service.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use windows_service::service::ServiceAccess;
    /// use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    ///
    /// # fn main() -> windows_service::Result<()> {
    /// let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    /// let my_service = manager.open_service("my_service", ServiceAccess::PAUSE_CONTINUE)?;
    /// my_service.pause()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn pause(&self) -> crate::Result<ServiceStatus> {
        self.send_control_command(ServiceControl::Pause)
    }

    /// Resume the paused service.
    pub fn resume(&self) -> crate::Result<ServiceStatus> {
        self.send_control_command(ServiceControl::Continue)
    }

    /// Get the service status from the system.
    pub fn query_status(&self) -> crate::Result<ServiceStatus> {
        let mut raw_status = unsafe { mem::zeroed::<Services::SERVICE_STATUS_PROCESS>() };
        let mut bytes_needed: u32 = 0;
        let success = unsafe {
            Services::QueryServiceStatusEx(
                self.service_handle.raw_handle(),
                Services::SC_STATUS_PROCESS_INFO,
                &mut raw_status as *mut _ as _,
                std::mem::size_of::<Services::SERVICE_STATUS_PROCESS>() as u32,
                &mut bytes_needed,
            )
        };
        if success == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            ServiceStatus::from_raw_ex(raw_status)
                .map_err(|e| Error::ParseValue("service status", e))
        }
    }

    /// Mark the service for deletion from the service control manager database.
    ///
    /// The database entry is not removed until all open handles to the service have been closed
    /// and the service is stopped. If the service is not or cannot be stopped, the database entry
    /// is removed when the system is restarted. This function will return an error if the service
    /// has already been marked for deletion.
    pub fn delete(&self) -> crate::Result<()> {
        let success = unsafe { Services::DeleteService(self.service_handle.raw_handle()) };
        if success == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    /// Get the service config from the system.
    pub fn query_config(&self) -> crate::Result<ServiceConfig> {
        // As per docs, the maximum size of data buffer used by QueryServiceConfigW is 8K
        let mut data = vec![0u8; MAX_QUERY_BUFFER_SIZE];
        let mut bytes_written: u32 = 0;

        let success = unsafe {
            Services::QueryServiceConfigW(
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
                let raw_config = data.as_ptr() as *const Services::QUERY_SERVICE_CONFIGW;
                ServiceConfig::from_raw(*raw_config)
            }
        }
    }

    /// Update the service config.
    /// Caveat: You cannot reset the account name/password by passing NULL.
    ///
    /// This implementation does not currently expose the full flexibility of the
    /// `ChangeServiceConfigW` API. When calling the API it's possible to pass NULL in place of
    /// any of the string arguments to indicate that they should not be updated.
    ///
    /// If we wanted to support this we wouldn't be able to reuse the `ServiceInfo` struct.
    pub fn change_config(&self, service_info: &ServiceInfo) -> crate::Result<()> {
        let raw_info = RawServiceInfo::new(service_info)?;
        let success = unsafe {
            Services::ChangeServiceConfigW(
                self.service_handle.raw_handle(),
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
                raw_info.display_name.as_ptr(),
            )
        };

        if success == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            Ok(())
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
            unsafe { mem::zeroed::<Services::SERVICE_FAILURE_ACTIONS_FLAG>() };

        raw_failure_actions_flag.fFailureActionsOnNonCrashFailures = if enabled { 1 } else { 0 };

        unsafe {
            self.change_config2(
                Services::SERVICE_CONFIG_FAILURE_ACTIONS_FLAG,
                &mut raw_failure_actions_flag,
            )
            .map_err(Error::Winapi)
        }
    }

    /// Query the system for the boolean indication that the service is configured to run failure
    /// actions on non-crash failures.
    pub fn get_failure_actions_on_non_crash_failures(&self) -> crate::Result<bool> {
        let mut data = vec![0u8; MAX_QUERY_BUFFER_SIZE];

        let raw_failure_actions_flag: Services::SERVICE_FAILURE_ACTIONS_FLAG = unsafe {
            self.query_config2(Services::SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, &mut data)
                .map_err(Error::Winapi)?
        };
        Ok(raw_failure_actions_flag.fFailureActionsOnNonCrashFailures != 0)
    }

    pub fn set_config_service_sid_info(
        &self,
        mut service_sid_type: ServiceSidType,
    ) -> crate::Result<()> {
        // The structure we need to pass in is `SERVICE_SID_INFO`.
        // It has a single member that specifies the new SID type, and as such,
        // we can get away with not explicitly creating a structure in Rust.
        unsafe {
            self.change_config2(
                Services::SERVICE_CONFIG_SERVICE_SID_INFO,
                &mut service_sid_type,
            )
            .map_err(Error::Winapi)
        }
    }

    /// Query the configured failure actions for the service.
    pub fn get_failure_actions(&self) -> crate::Result<ServiceFailureActions> {
        unsafe {
            let mut data = vec![0u8; MAX_QUERY_BUFFER_SIZE];

            let raw_failure_actions: Services::SERVICE_FAILURE_ACTIONSW = self
                .query_config2(Services::SERVICE_CONFIG_FAILURE_ACTIONS, &mut data)
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
        let mut raw_failure_actions =
            unsafe { mem::zeroed::<Services::SERVICE_FAILURE_ACTIONSW>() };

        let mut reboot_msg = to_wide_slice(update.reboot_msg)
            .map_err(|_| Error::ArgumentHasNulByte("service action failures reboot message"))?;
        let mut command = to_wide_slice(update.command)
            .map_err(|_| Error::ArgumentHasNulByte("service action failures command"))?;
        let mut sc_actions: Option<Vec<Services::SC_ACTION>> = update
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
                Services::SERVICE_CONFIG_FAILURE_ACTIONS,
                &mut raw_failure_actions,
            )
            .map_err(Error::Winapi)
        }
    }

    /// Set service description.
    ///
    /// Required permission: [`ServiceAccess::CHANGE_CONFIG`].
    pub fn set_description(&self, description: impl AsRef<OsStr>) -> crate::Result<()> {
        let wide_str = WideCString::from_os_str(description)
            .map_err(|_| Error::ArgumentHasNulByte("service description"))?;
        let mut service_description = Services::SERVICE_DESCRIPTIONW {
            lpDescription: wide_str.as_ptr() as *mut _,
        };

        unsafe {
            self.change_config2(
                Services::SERVICE_CONFIG_DESCRIPTION,
                &mut service_description,
            )
            .map_err(Error::Winapi)
        }
    }

    /// Set if an auto-start service should be delayed.
    ///
    /// If true, the service is started after other auto-start services are started plus a short delay.
    /// Otherwise, the service is started during system boot. The default is false. This setting is
    /// ignored unless the service is an auto-start service.
    ///
    /// Required permission: [`ServiceAccess::CHANGE_CONFIG`].
    pub fn set_delayed_auto_start(&self, delayed: bool) -> crate::Result<()> {
        let mut delayed = Services::SERVICE_DELAYED_AUTO_START_INFO {
            fDelayedAutostart: delayed as i32,
        };
        unsafe {
            self.change_config2(
                Services::SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                &mut delayed,
            )
            .map_err(Error::Winapi)
        }
    }

    /// Set the preshutdown timeout value of the service.
    ///
    /// When the system prepares to shutdown, the service control manager will send [`ServiceControl::Preshutdown`]
    /// to any service that accepts [`ServiceControlAccept::PRESHUTDOWN`] and block shutdown until either the
    /// service is stopped or the preshutdown timeout has elapsed. The default value is 180 seconds on releases
    /// prior to Windows 10 build 15063, and 10 seconds afterwards. This value is irrelevant unless the service
    /// handles [`ServiceControl::Preshutdown`].
    ///
    /// Panics if the specified timeout is too large to fit as milliseconds in a `u32`.
    ///
    /// Required permission: [`ServiceAccess::CHANGE_CONFIG`].
    pub fn set_preshutdown_timeout(&self, timeout: Duration) -> crate::Result<()> {
        let mut timeout = Services::SERVICE_PRESHUTDOWN_INFO {
            dwPreshutdownTimeout: u32::try_from(timeout.as_millis()).expect("Too long timeout"),
        };
        unsafe {
            self.change_config2(Services::SERVICE_CONFIG_PRESHUTDOWN_INFO, &mut timeout)
                .map_err(Error::Winapi)
        }
    }

    /// Private helper to send the control commands to the system.
    fn send_control_command(&self, command: ServiceControl) -> crate::Result<ServiceStatus> {
        let mut raw_status = unsafe { mem::zeroed::<Services::SERVICE_STATUS>() };
        let success = unsafe {
            Services::ControlService(
                self.service_handle.raw_handle(),
                command.raw_service_control_type(),
                &mut raw_status,
            )
        };

        if success == 0 {
            Err(Error::Winapi(io::Error::last_os_error()))
        } else {
            ServiceStatus::from_raw(raw_status).map_err(|e| Error::ParseValue("service status", e))
        }
    }

    /// Private helper to query the optional configuration parameters of windows services.
    unsafe fn query_config2<T: Copy>(&self, kind: u32, data: &mut [u8]) -> io::Result<T> {
        let mut bytes_written: u32 = 0;

        let success = Services::QueryServiceConfig2W(
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
    unsafe fn change_config2<T>(&self, kind: u32, data: &mut T) -> io::Result<()> {
        let success = Services::ChangeServiceConfig2W(
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

/// The maximum size of data buffer used by QueryServiceConfigW and QueryServiceConfig2W is 8K
const MAX_QUERY_BUFFER_SIZE: usize = 8 * 1024;

fn to_wide_slice(
    s: Option<impl AsRef<OsStr>>,
) -> ::std::result::Result<Option<Vec<u16>>, ContainsNul<u16>> {
    if let Some(s) = s {
        Ok(Some(
            WideCString::from_os_str(s).map(|s| s.into_vec_with_nul())?,
        ))
    } else {
        Ok(None)
    }
}

pub(crate) fn to_wide(
    s: Option<impl AsRef<OsStr>>,
) -> ::std::result::Result<Option<WideCString>, ContainsNul<u16>> {
    if let Some(s) = s {
        Ok(Some(WideCString::from_os_str(s)?))
    } else {
        Ok(None)
    }
}

/// Escapes a given string, but also checks it does not contain any null bytes
fn escape_wide(s: impl AsRef<OsStr>) -> ::std::result::Result<WideString, ContainsNul<u16>> {
    let escaped = shell_escape::escape(Cow::Borrowed(s.as_ref()));
    let wide = WideCString::from_os_str(escaped)?;
    Ok(wide.to_ustring())
}

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
