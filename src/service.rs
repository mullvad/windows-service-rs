use std::borrow::Cow;
use std::convert::TryFrom;
use std::ffi::{OsStr, OsString};
use std::os::raw::c_void;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr;
use std::time::Duration;
use std::{io, mem};

use widestring::{NulError, WideCStr, WideCString, WideString};
use winapi::shared::guiddef::{IsEqualGUID, GUID};
use winapi::shared::minwindef::DWORD;
use winapi::shared::winerror::{ERROR_SERVICE_SPECIFIC_ERROR, NO_ERROR};
use winapi::um::winbase::INFINITE;
use winapi::um::{dbt, winnt, winsvc, winuser};

use crate::sc_handle::ScHandle;
use crate::shell_escape;
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
            _ => Err(ParseRawError::InvalidInteger(raw)),
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
    /// in a `DWORD`.
    pub delay: Duration,
}

impl ServiceAction {
    pub fn from_raw(raw: winsvc::SC_ACTION) -> crate::Result<ServiceAction> {
        Ok(ServiceAction {
            action_type: ServiceActionType::from_raw(raw.Type)
                .map_err(Error::InvalidServiceActionType)?,
            delay: Duration::from_millis(raw.Delay as u64),
        })
    }

    pub fn to_raw(&self) -> winsvc::SC_ACTION {
        winsvc::SC_ACTION {
            Type: self.action_type.to_raw(),
            Delay: DWORD::try_from(self.delay.as_millis()).expect("Too long delay"),
        }
    }
}

/// A enum that representats the reset period for the failure counter.
///
/// # Panics
///
/// Converting this to the FFI form will panic if the period is too large to fit as seconds in a
/// `DWORD`.
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
            ServiceFailureResetPeriod::After(duration) => {
                DWORD::try_from(duration.as_secs()).expect("Too long reset period")
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

/// Same as `ServiceInfo` but with fields that are compatible with the Windows API.
pub(crate) struct RawServiceInfo {
    /// Service name
    pub name: WideCString,

    /// User-friendly service name
    pub display_name: WideCString,

    /// The service type
    pub service_type: DWORD,

    /// The service startup options
    pub start_type: DWORD,

    /// The severity of the error, and action taken, if this service fails to start.
    pub error_control: DWORD,

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
        let service_name =
            WideCString::from_os_str(&service_info.name).map_err(Error::InvalidServiceName)?;
        let display_name = WideCString::from_os_str(&service_info.display_name)
            .map_err(Error::InvalidDisplayName)?;
        let account_name =
            to_wide(service_info.account_name.as_ref()).map_err(Error::InvalidAccountName)?;
        let account_password = to_wide(service_info.account_password.as_ref())
            .map_err(Error::InvalidAccountPassword)?;

        // escape executable path and arguments and combine them into single command
        let executable_path =
            escape_wide(&service_info.executable_path).map_err(Error::InvalidExecutablePath)?;

        let mut launch_command_buffer = WideString::new();
        launch_command_buffer.push(executable_path);

        for (i, launch_argument) in service_info.launch_arguments.iter().enumerate() {
            let wide =
                escape_wide(launch_argument).map_err(|e| Error::InvalidLaunchArgument(i, e))?;

            launch_command_buffer.push_str(" ");
            launch_command_buffer.push(wide);
        }

        // Safety: We are sure launch_command_buffer does not contain nulls
        let launch_command = unsafe { WideCString::from_ustr_unchecked(launch_command_buffer) };

        let dependency_identifiers: Vec<OsString> = service_info
            .dependencies
            .iter()
            .map(|dependency| dependency.to_system_identifier())
            .collect();
        let joined_dependencies = double_nul_terminated::from_vec(&dependency_identifiers)
            .map_err(Error::InvalidDependency)?;

        Ok(Self {
            name: service_name,
            display_name: display_name,
            service_type: service_info.service_type.bits(),
            start_type: service_info.start_type.to_raw(),
            error_control: service_info.error_control.to_raw(),
            launch_command: launch_command,
            dependencies: joined_dependencies,
            account_name: account_name,
            account_password: account_password,
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

/// Enum describing the event type of HardwareProfileChange
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum HardwareProfileChangeParam {
    ConfigChanged = dbt::DBT_CONFIGCHANGED as u32,
    QueryChangeConfig = dbt::DBT_QUERYCHANGECONFIG as u32,
    ConfigChangeCanceled = dbt::DBT_CONFIGCHANGECANCELED as u32,
}

impl HardwareProfileChangeParam {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<Self, ParseRawError> {
        match raw {
            x if x == HardwareProfileChangeParam::ConfigChanged.to_raw() => {
                Ok(HardwareProfileChangeParam::ConfigChanged)
            }
            x if x == HardwareProfileChangeParam::QueryChangeConfig.to_raw() => {
                Ok(HardwareProfileChangeParam::QueryChangeConfig)
            }
            x if x == HardwareProfileChangeParam::ConfigChangeCanceled.to_raw() => {
                Ok(HardwareProfileChangeParam::ConfigChangeCanceled)
            }
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }
}

/// Enum indicates the current power source as
/// the Data member of GUID_ACDC_POWER_SOURCE notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PowerSource {
    Ac = winnt::PoAc,
    Dc = winnt::PoDc,
    Hot = winnt::PoHot,
}

impl PowerSource {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<PowerSource, ParseRawError> {
        match raw {
            x if x == PowerSource::Ac.to_raw() => Ok(PowerSource::Ac),
            x if x == PowerSource::Dc.to_raw() => Ok(PowerSource::Dc),
            x if x == PowerSource::Hot.to_raw() => Ok(PowerSource::Hot),
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }
}

/// Enum indicates the current monitor's display state as
/// the Data member of GUID_CONSOLE_DISPLAY_STATE notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum DisplayState {
    Off = winnt::PowerMonitorOff,
    On = winnt::PowerMonitorOn,
    Dimmed = winnt::PowerMonitorDim,
}

impl DisplayState {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<DisplayState, ParseRawError> {
        match raw {
            x if x == DisplayState::Off.to_raw() => Ok(DisplayState::Off),
            x if x == DisplayState::On.to_raw() => Ok(DisplayState::On),
            x if x == DisplayState::Dimmed.to_raw() => Ok(DisplayState::Dimmed),
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }
}

/// Enum indicates the combined status of user presence
/// across all local and remote sessions on the system as
/// the Data member of GUID_GLOBAL_USER_PRESENCE notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum UserStatus {
    Present = winnt::PowerUserPresent,
    Inactive = winnt::PowerUserInactive,
}

impl UserStatus {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<UserStatus, ParseRawError> {
        match raw {
            x if x == UserStatus::Present.to_raw() => Ok(UserStatus::Present),
            x if x == UserStatus::Inactive.to_raw() => Ok(UserStatus::Inactive),
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }
}

/// Enum indicates the current monitor state as
/// the Data member of GUID_MONITOR_POWER_ON notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum MonitorState {
    Off = 0,
    On = 1,
}

impl MonitorState {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<MonitorState, ParseRawError> {
        match raw {
            x if x == MonitorState::Off.to_raw() => Ok(MonitorState::Off),
            x if x == MonitorState::On.to_raw() => Ok(MonitorState::On),
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }
}

/// Enum indicates the battery saver state as
/// the Data member of GUID_POWER_SAVING_STATUS notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum BatterySaverState {
    Off = 0,
    On = 1,
}

impl BatterySaverState {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<BatterySaverState, ParseRawError> {
        match raw {
            x if x == BatterySaverState::Off.to_raw() => Ok(BatterySaverState::Off),
            x if x == BatterySaverState::On.to_raw() => Ok(BatterySaverState::On),
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }
}

/// Enum indicates the power scheme personality as
/// the Data member of GUID_POWERSCHEME_PERSONALITY notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerSchemePersonality {
    HighPerformance,
    PowerSaver,
    Automatic,
}

impl PowerSchemePersonality {
    pub fn from_guid(guid: &GUID) -> Result<PowerSchemePersonality, ParseRawError> {
        match guid {
            x if IsEqualGUID(x, &winnt::GUID_MIN_POWER_SAVINGS) => {
                Ok(PowerSchemePersonality::HighPerformance)
            }
            x if IsEqualGUID(x, &winnt::GUID_MAX_POWER_SAVINGS) => {
                Ok(PowerSchemePersonality::PowerSaver)
            }
            x if IsEqualGUID(x, &winnt::GUID_TYPICAL_POWER_SAVINGS) => {
                Ok(PowerSchemePersonality::Automatic)
            }
            x => Err(ParseRawError::InvalidGuid(string_from_guid(x))),
        }
    }
}

/// Enum indicates the current away mode state as
/// the Data member of GUID_SYSTEM_AWAYMODE notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum AwayModeState {
    Exiting = 0,
    Entering = 1,
}

impl AwayModeState {
    pub fn to_raw(&self) -> u32 {
        *self as u32
    }

    pub fn from_raw(raw: u32) -> Result<AwayModeState, ParseRawError> {
        match raw {
            x if x == AwayModeState::Exiting.to_raw() => Ok(AwayModeState::Exiting),
            x if x == AwayModeState::Entering.to_raw() => Ok(AwayModeState::Entering),
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }
}

/// Struct converted from winuser::POWERBROADCAST_SETTING
///
/// Please refer to MSDN for more info about the data members:
/// <https://docs.microsoft.com/en-us/windows/win32/power/power-setting-guid>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerBroadcastSetting {
    AcdcPowerSource(PowerSource),
    BatteryPercentageRemaining(u32),
    ConsoleDisplayState(DisplayState),
    GlobalUserPresence(UserStatus),
    IdleBackgroundTask,
    MonitorPowerOn(MonitorState),
    PowerSavingStatus(BatterySaverState),
    PowerSchemePersonality(PowerSchemePersonality),
    SystemAwayMode(AwayModeState),
}

impl PowerBroadcastSetting {
    /// Extract PowerBroadcastSetting from `raw`
    ///
    /// # Safety
    ///
    /// The `raw` must be a valid winuser::POWERBROADCAST_SETTING pointer.
    /// Otherwise, it is undefined behavior.
    pub unsafe fn from_raw(raw: *mut c_void) -> Result<PowerBroadcastSetting, ParseRawError> {
        let setting = &*(raw as *const winuser::POWERBROADCAST_SETTING);
        let data = &setting.Data as *const u8;

        match &setting.PowerSetting {
            x if IsEqualGUID(x, &winnt::GUID_ACDC_POWER_SOURCE) => {
                let power_source = *(data as *const u32);
                Ok(PowerBroadcastSetting::AcdcPowerSource(
                    PowerSource::from_raw(power_source)?,
                ))
            }
            x if IsEqualGUID(x, &winnt::GUID_BATTERY_PERCENTAGE_REMAINING) => {
                let percentage = *(data as *const u32);
                Ok(PowerBroadcastSetting::BatteryPercentageRemaining(
                    percentage,
                ))
            }
            x if IsEqualGUID(x, &winnt::GUID_CONSOLE_DISPLAY_STATE) => {
                let display_state = *(data as *const u32);
                Ok(PowerBroadcastSetting::ConsoleDisplayState(
                    DisplayState::from_raw(display_state)?,
                ))
            }
            x if IsEqualGUID(x, &winnt::GUID_GLOBAL_USER_PRESENCE) => {
                let user_status = *(data as *const u32);
                Ok(PowerBroadcastSetting::GlobalUserPresence(
                    UserStatus::from_raw(user_status)?,
                ))
            }
            x if IsEqualGUID(x, &winnt::GUID_IDLE_BACKGROUND_TASK) => {
                Ok(PowerBroadcastSetting::IdleBackgroundTask)
            }
            x if IsEqualGUID(x, &winnt::GUID_MONITOR_POWER_ON) => {
                let monitor_state = *(data as *const u32);
                Ok(PowerBroadcastSetting::MonitorPowerOn(
                    MonitorState::from_raw(monitor_state)?,
                ))
            }
            x if IsEqualGUID(x, &winnt::GUID_POWER_SAVING_STATUS) => {
                let battery_saver_state = *(data as *const u32);
                Ok(PowerBroadcastSetting::PowerSavingStatus(
                    BatterySaverState::from_raw(battery_saver_state)?,
                ))
            }
            x if IsEqualGUID(x, &winnt::GUID_POWERSCHEME_PERSONALITY) => {
                let guid = *(data as *const GUID);
                Ok(PowerBroadcastSetting::PowerSchemePersonality(
                    PowerSchemePersonality::from_guid(&guid)?,
                ))
            }
            x if IsEqualGUID(x, &winnt::GUID_SYSTEM_AWAYMODE) => {
                let away_mode_state = *(data as *const u32);
                Ok(PowerBroadcastSetting::SystemAwayMode(
                    AwayModeState::from_raw(away_mode_state)?,
                ))
            }
            x => Err(ParseRawError::InvalidGuid(string_from_guid(x))),
        }
    }
}

/// Enum describing the PowerEvent event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerEventParam {
    PowerStatusChange,
    ResumeAutomatic,
    ResumeSuspend,
    Suspend,
    PowerSettingChange(PowerBroadcastSetting),
    BatteryLow,
    OemEvent,
    QuerySuspend,
    QuerySuspendFailed,
    ResumeCritical,
}

impl PowerEventParam {
    /// Extract PowerEventParam from `event_type` and `event_data`
    ///
    /// # Safety
    ///
    /// Invalid `event_data` pointer may cause undefined behavior in some circumstances.
    /// Please refer to MSDN for more info about the requirements:
    /// <https://docs.microsoft.com/en-us/windows/win32/api/winsvc/nc-winsvc-lphandler_function_ex>
    pub unsafe fn from_event(
        event_type: u32,
        event_data: *mut c_void,
    ) -> Result<Self, ParseRawError> {
        match event_type as usize {
            winuser::PBT_APMPOWERSTATUSCHANGE => Ok(PowerEventParam::PowerStatusChange),
            winuser::PBT_APMRESUMEAUTOMATIC => Ok(PowerEventParam::ResumeAutomatic),
            winuser::PBT_APMRESUMESUSPEND => Ok(PowerEventParam::ResumeSuspend),
            winuser::PBT_APMSUSPEND => Ok(PowerEventParam::Suspend),
            winuser::PBT_POWERSETTINGCHANGE => Ok(PowerEventParam::PowerSettingChange(
                PowerBroadcastSetting::from_raw(event_data)?,
            )),
            winuser::PBT_APMBATTERYLOW => Ok(PowerEventParam::BatteryLow),
            winuser::PBT_APMOEMEVENT => Ok(PowerEventParam::OemEvent),
            winuser::PBT_APMQUERYSUSPEND => Ok(PowerEventParam::QuerySuspend),
            winuser::PBT_APMQUERYSUSPENDFAILED => Ok(PowerEventParam::QuerySuspendFailed),
            winuser::PBT_APMRESUMECRITICAL => Ok(PowerEventParam::ResumeCritical),
            _ => Err(ParseRawError::InvalidInteger(event_type)),
        }
    }
}

/// Enum describing the reason of a SessionChange event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SessionChangeReason {
    ConsoleConnect = winuser::WTS_CONSOLE_CONNECT as u32,
    ConsoleDisconnect = winuser::WTS_CONSOLE_DISCONNECT as u32,
    RemoteConnect = winuser::WTS_REMOTE_CONNECT as u32,
    RemoteDisconnect = winuser::WTS_REMOTE_DISCONNECT as u32,
    SessionLogon = winuser::WTS_SESSION_LOGON as u32,
    SessionLogoff = winuser::WTS_SESSION_LOGOFF as u32,
    SessionLock = winuser::WTS_SESSION_LOCK as u32,
    SessionUnlock = winuser::WTS_SESSION_UNLOCK as u32,
    SessionRemoteControl = winuser::WTS_SESSION_REMOTE_CONTROL as u32,
    SessionCreate = winuser::WTS_SESSION_CREATE as u32,
    SessionTerminate = winuser::WTS_SESSION_TERMINATE as u32,
}

impl SessionChangeReason {
    pub fn from_raw(raw: u32) -> Result<SessionChangeReason, ParseRawError> {
        match raw {
            x if x == SessionChangeReason::ConsoleConnect.to_raw() => {
                Ok(SessionChangeReason::ConsoleConnect)
            }
            x if x == SessionChangeReason::ConsoleDisconnect.to_raw() => {
                Ok(SessionChangeReason::ConsoleDisconnect)
            }
            x if x == SessionChangeReason::RemoteConnect.to_raw() => {
                Ok(SessionChangeReason::RemoteConnect)
            }
            x if x == SessionChangeReason::RemoteDisconnect.to_raw() => {
                Ok(SessionChangeReason::RemoteDisconnect)
            }
            x if x == SessionChangeReason::SessionLogon.to_raw() => {
                Ok(SessionChangeReason::SessionLogon)
            }
            x if x == SessionChangeReason::SessionLogoff.to_raw() => {
                Ok(SessionChangeReason::SessionLogoff)
            }
            x if x == SessionChangeReason::SessionLock.to_raw() => {
                Ok(SessionChangeReason::SessionLock)
            }
            x if x == SessionChangeReason::SessionUnlock.to_raw() => {
                Ok(SessionChangeReason::SessionUnlock)
            }
            x if x == SessionChangeReason::SessionRemoteControl.to_raw() => {
                Ok(SessionChangeReason::SessionRemoteControl)
            }
            x if x == SessionChangeReason::SessionCreate.to_raw() => {
                Ok(SessionChangeReason::SessionCreate)
            }
            x if x == SessionChangeReason::SessionTerminate.to_raw() => {
                Ok(SessionChangeReason::SessionTerminate)
            }
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }

    pub fn to_raw(&self) -> u32 {
        *self as u32
    }
}

/// Struct converted from winuser::WTSSESSION_NOTIFICATION
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionNotification {
    pub size: u32,
    pub session_id: u32,
}

impl SessionNotification {
    pub fn from_raw(raw: winuser::WTSSESSION_NOTIFICATION) -> Self {
        SessionNotification {
            size: raw.cbSize,
            session_id: raw.dwSessionId,
        }
    }
}

/// Struct describing the SessionChange event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionChangeParam {
    pub reason: SessionChangeReason,
    pub notification: SessionNotification,
}

impl SessionChangeParam {
    /// Extract SessionChangeParam from `event_data`
    ///
    /// # Safety
    ///
    /// The `event_data` must be a valid winuser::WTSSESSION_NOTIFICATION pointer.
    /// Otherwise, it is undefined behavior.
    pub unsafe fn from_event(
        event_type: u32,
        event_data: *mut c_void,
    ) -> Result<Self, ParseRawError> {
        let notification = *(event_data as *const winuser::WTSSESSION_NOTIFICATION);

        Ok(SessionChangeParam {
            reason: SessionChangeReason::from_raw(event_type)?,
            notification: SessionNotification::from_raw(notification),
        })
    }
}

/// Enum describing the service control operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceControl {
    Continue,
    Interrogate,
    NetBindAdd,
    NetBindDisable,
    NetBindEnable,
    NetBindRemove,
    ParamChange,
    Pause,
    Preshutdown,
    Shutdown,
    Stop,
    HardwareProfileChange(HardwareProfileChangeParam),
    PowerEvent(PowerEventParam),
    SessionChange(SessionChangeParam),
    TimeChange,
    TriggerEvent,
}

impl ServiceControl {
    /// Convert to ServiceControl from parameters received by `service_control_handler`
    ///
    /// # Safety
    ///
    /// Invalid `event_data` pointer may cause undefined behavior in some circumstances.
    /// Please refer to MSDN for more info about the requirements:
    /// <https://docs.microsoft.com/en-us/windows/win32/api/winsvc/nc-winsvc-lphandler_function_ex>
    pub unsafe fn from_raw(
        raw: u32,
        event_type: u32,
        event_data: *mut c_void,
    ) -> Result<Self, ParseRawError> {
        match raw {
            winsvc::SERVICE_CONTROL_CONTINUE => Ok(ServiceControl::Continue),
            winsvc::SERVICE_CONTROL_INTERROGATE => Ok(ServiceControl::Interrogate),
            winsvc::SERVICE_CONTROL_NETBINDADD => Ok(ServiceControl::NetBindAdd),
            winsvc::SERVICE_CONTROL_NETBINDDISABLE => Ok(ServiceControl::NetBindDisable),
            winsvc::SERVICE_CONTROL_NETBINDENABLE => Ok(ServiceControl::NetBindEnable),
            winsvc::SERVICE_CONTROL_NETBINDREMOVE => Ok(ServiceControl::NetBindRemove),
            winsvc::SERVICE_CONTROL_PARAMCHANGE => Ok(ServiceControl::ParamChange),
            winsvc::SERVICE_CONTROL_PAUSE => Ok(ServiceControl::Pause),
            winsvc::SERVICE_CONTROL_PRESHUTDOWN => Ok(ServiceControl::Preshutdown),
            winsvc::SERVICE_CONTROL_SHUTDOWN => Ok(ServiceControl::Shutdown),
            winsvc::SERVICE_CONTROL_STOP => Ok(ServiceControl::Stop),
            winsvc::SERVICE_CONTROL_HARDWAREPROFILECHANGE => {
                HardwareProfileChangeParam::from_raw(event_type)
                    .map(ServiceControl::HardwareProfileChange)
            }
            winsvc::SERVICE_CONTROL_POWEREVENT => {
                PowerEventParam::from_event(event_type, event_data).map(ServiceControl::PowerEvent)
            }
            winsvc::SERVICE_CONTROL_SESSIONCHANGE => {
                SessionChangeParam::from_event(event_type, event_data)
                    .map(ServiceControl::SessionChange)
            }
            winsvc::SERVICE_CONTROL_TIMECHANGE => Ok(ServiceControl::TimeChange),
            winsvc::SERVICE_CONTROL_TRIGGEREVENT => Ok(ServiceControl::TriggerEvent),
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }

    pub fn raw_service_control_type(&self) -> u32 {
        match self {
            ServiceControl::Continue => winsvc::SERVICE_CONTROL_CONTINUE,
            ServiceControl::Interrogate => winsvc::SERVICE_CONTROL_INTERROGATE,
            ServiceControl::NetBindAdd => winsvc::SERVICE_CONTROL_NETBINDADD,
            ServiceControl::NetBindDisable => winsvc::SERVICE_CONTROL_NETBINDDISABLE,
            ServiceControl::NetBindEnable => winsvc::SERVICE_CONTROL_NETBINDENABLE,
            ServiceControl::NetBindRemove => winsvc::SERVICE_CONTROL_NETBINDREMOVE,
            ServiceControl::ParamChange => winsvc::SERVICE_CONTROL_PARAMCHANGE,
            ServiceControl::Pause => winsvc::SERVICE_CONTROL_PAUSE,
            ServiceControl::Preshutdown => winsvc::SERVICE_CONTROL_PRESHUTDOWN,
            ServiceControl::Shutdown => winsvc::SERVICE_CONTROL_SHUTDOWN,
            ServiceControl::Stop => winsvc::SERVICE_CONTROL_STOP,
            ServiceControl::HardwareProfileChange(_) => {
                winsvc::SERVICE_CONTROL_HARDWAREPROFILECHANGE
            }
            ServiceControl::PowerEvent(_) => winsvc::SERVICE_CONTROL_POWEREVENT,
            ServiceControl::SessionChange(_) => winsvc::SERVICE_CONTROL_SESSIONCHANGE,
            ServiceControl::TimeChange => winsvc::SERVICE_CONTROL_TIMECHANGE,
            ServiceControl::TriggerEvent => winsvc::SERVICE_CONTROL_TRIGGEREVENT,
        }
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
            _ => Err(ParseRawError::InvalidInteger(raw)),
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

        /// The service is notified when the computer's hardware profile has changed.
        /// This enables the system to send SERVICE_CONTROL_HARDWAREPROFILECHANGE
        /// notifications to the service.
        const HARDWARE_PROFILE_CHANGE = winsvc::SERVICE_ACCEPT_HARDWAREPROFILECHANGE;

        /// The service is notified when the computer's power status has changed.
        /// This enables the system to send SERVICE_CONTROL_POWEREVENT notifications to the service.
        const POWER_EVENT = winsvc::SERVICE_ACCEPT_POWEREVENT;

        /// The service is notified when the computer's session status has changed.
        /// This enables the system to send SERVICE_CONTROL_SESSIONCHANGE notifications to the service.
        const SESSION_CHANGE = winsvc::SERVICE_ACCEPT_SESSIONCHANGE;

        /// The service is notified when the system time has changed.
        /// This enables the system to send SERVICE_CONTROL_TIMECHANGE notifications to the service.
        const TIME_CHANGE = winsvc::SERVICE_ACCEPT_TIMECHANGE;

        /// The service is notified when an event for which the service has registered occurs.
        /// This enables the system to send SERVICE_CONTROL_TRIGGEREVENT notifications to the service.
        const TRIGGER_EVENT = winsvc::SERVICE_ACCEPT_TRIGGEREVENT;
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
    ///
    /// # Panics
    ///
    /// Converting this to the FFI form will panic if the duration is too large to fit as
    /// milliseconds in a `DWORD`.
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

        raw_status.dwWaitHint =
            DWORD::try_from(self.wait_hint.as_millis()).expect("Too long wait_hint");

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
    pub fn start(&self, service_arguments: &[impl AsRef<OsStr>]) -> crate::Result<()> {
        let wide_service_arguments = service_arguments
            .iter()
            .map(|s| WideCString::from_os_str(s).map_err(Error::InvalidStartArgument))
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
        let mut data = [0u8; MAX_QUERY_BUFFER_SIZE];
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
            winsvc::ChangeServiceConfigW(
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
        let mut data = [0u8; MAX_QUERY_BUFFER_SIZE];

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
            let mut data = [0u8; MAX_QUERY_BUFFER_SIZE];

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
                command.raw_service_control_type(),
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
    unsafe fn query_config2<T: Copy>(
        &self,
        kind: DWORD,
        data: &mut [u8; MAX_QUERY_BUFFER_SIZE],
    ) -> io::Result<T> {
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

/// The maximum size of data buffer used by QueryServiceConfigW and QueryServiceConfig2W is 8K
const MAX_QUERY_BUFFER_SIZE: usize = 8 * 1024;

fn to_wide_slice(
    s: Option<impl AsRef<OsStr>>,
) -> ::std::result::Result<Option<Vec<u16>>, NulError<u16>> {
    if let Some(s) = s {
        Ok(Some(
            WideCString::from_os_str(s).map(|s| s.as_slice_with_nul().to_vec())?,
        ))
    } else {
        Ok(None)
    }
}

#[derive(err_derive::Error, Debug)]
pub enum ParseRawError {
    #[error(display = "Invalid integer value for the target type: {}", _0)]
    InvalidInteger(u32),

    #[error(display = "Invalid GUID value for the target type: {}", _0)]
    InvalidGuid(String),
}

fn string_from_guid(guid: &GUID) -> String {
    format!(
        "{:8X}-{:4X}-{:4X}-{:2X}{:2X}-{:2X}{:2X}{:2X}{:2X}{:2X}{:2X}",
        guid.Data1,
        guid.Data2,
        guid.Data3,
        guid.Data4[0],
        guid.Data4[1],
        guid.Data4[2],
        guid.Data4[3],
        guid.Data4[4],
        guid.Data4[5],
        guid.Data4[6],
        guid.Data4[7]
    )
}

pub(crate) fn to_wide(
    s: Option<impl AsRef<OsStr>>,
) -> ::std::result::Result<Option<WideCString>, NulError<u16>> {
    if let Some(s) = s {
        Ok(Some(WideCString::from_os_str(s)?))
    } else {
        Ok(None)
    }
}

/// Escapes a given string, but also checks it does not contain any null bytes
fn escape_wide(s: impl AsRef<OsStr>) -> ::std::result::Result<WideString, NulError<u16>> {
    let escaped = shell_escape::escape(Cow::Borrowed(s.as_ref()));
    let wide = WideCString::from_os_str(&escaped)?;
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
