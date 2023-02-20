use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr,
    time::Duration,
};

use widestring::{WideCStr, WideCString, WideString};
use windows_sys::Win32::System::{Services, WindowsProgramming::INFINITE};

use crate::{
    service::{ParseRawError, ServiceType},
    Error,
};

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
pub(super) struct RawServiceInfo {
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
    pub(super) fn new(service_info: &ServiceInfo) -> crate::Result<Self> {
        let service_name = WideCString::from_os_str(&service_info.name)
            .map_err(|_| Error::ArgumentHasNulByte("service name"))?;
        let display_name = WideCString::from_os_str(&service_info.display_name)
            .map_err(|_| Error::ArgumentHasNulByte("display name"))?;
        let account_name = super::utils::to_wide(service_info.account_name.as_ref())
            .map_err(|_| Error::ArgumentHasNulByte("account name"))?;
        let account_password = super::utils::to_wide(service_info.account_password.as_ref())
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
            let executable_path = super::utils::escape_wide(&service_info.executable_path)
                .map_err(|_| Error::ArgumentHasNulByte("executable path"))?;
            launch_command_buffer.push(executable_path);

            for (i, launch_argument) in service_info.launch_arguments.iter().enumerate() {
                let wide = super::utils::escape_wide(launch_argument)
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
        let joined_dependencies = super::double_nul_terminated::from_slice(&dependency_identifiers)
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
    /// <https://docs.microsoft.com/en-us/windows/desktop/api/winsvc/ns-winsvc-query_service_configw>
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
        let dependencies = super::double_nul_terminated::parse_str_ptr(raw.lpDependencies)
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
