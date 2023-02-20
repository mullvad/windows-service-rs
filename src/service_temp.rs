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
    /// Enum describing the types of Windows services.
    pub struct ServiceType: u32 {
        /// File system driver service.
        const FILE_SYSTEM_DRIVER = Services::SERVICE_FILE_SYSTEM_DRIVER;

        /// Driver service.
        const KERNEL_DRIVER = Services::SERVICE_KERNEL_DRIVER;

        /// Service that runs in its own process.
        const OWN_PROCESS = Services::SERVICE_WIN32_OWN_PROCESS;

        /// Service that shares a process with one or more other services.
        const SHARE_PROCESS = Services::SERVICE_WIN32_SHARE_PROCESS;

        /// The service runs in its own process under the logged-on user account.
        const USER_OWN_PROCESS = Services::SERVICE_USER_OWN_PROCESS;

        /// The service shares a process with one or more other services that run under the logged-on user account.
        const USER_SHARE_PROCESS = Services::SERVICE_USER_SHARE_PROCESS;

        /// The service can be interactive.
        const INTERACTIVE_PROCESS = SystemServices::SERVICE_INTERACTIVE_PROCESS;
    }
}

/// Enum describing the event type of HardwareProfileChange
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum HardwareProfileChangeParam {
    ConfigChanged = SystemServices::DBT_CONFIGCHANGED,
    QueryChangeConfig = SystemServices::DBT_QUERYCHANGECONFIG,
    ConfigChangeCanceled = SystemServices::DBT_CONFIGCHANGECANCELED,
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
#[repr(i32)]
pub enum PowerSource {
    Ac = Power::PoAc,
    Dc = Power::PoDc,
    Hot = Power::PoHot,
}

impl PowerSource {
    pub fn to_raw(&self) -> i32 {
        *self as i32
    }

    pub fn from_raw(raw: i32) -> Result<PowerSource, ParseRawError> {
        match raw {
            x if x == PowerSource::Ac.to_raw() => Ok(PowerSource::Ac),
            x if x == PowerSource::Dc.to_raw() => Ok(PowerSource::Dc),
            x if x == PowerSource::Hot.to_raw() => Ok(PowerSource::Hot),
            _ => Err(ParseRawError::InvalidIntegerSigned(raw)),
        }
    }
}

/// Enum indicates the current monitor's display state as
/// the Data member of GUID_CONSOLE_DISPLAY_STATE notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum DisplayState {
    Off = SystemServices::PowerMonitorOff,
    On = SystemServices::PowerMonitorOn,
    Dimmed = SystemServices::PowerMonitorDim,
}

impl DisplayState {
    pub fn to_raw(&self) -> i32 {
        *self as i32
    }

    pub fn from_raw(raw: i32) -> Result<DisplayState, ParseRawError> {
        match raw {
            x if x == DisplayState::Off.to_raw() => Ok(DisplayState::Off),
            x if x == DisplayState::On.to_raw() => Ok(DisplayState::On),
            x if x == DisplayState::Dimmed.to_raw() => Ok(DisplayState::Dimmed),
            _ => Err(ParseRawError::InvalidIntegerSigned(raw)),
        }
    }
}

/// Enum indicates the combined status of user presence
/// across all local and remote sessions on the system as
/// the Data member of GUID_GLOBAL_USER_PRESENCE notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum UserStatus {
    Present = SystemServices::PowerUserPresent,
    Inactive = SystemServices::PowerUserInactive,
}

impl UserStatus {
    pub fn to_raw(&self) -> i32 {
        *self as i32
    }

    pub fn from_raw(raw: i32) -> Result<UserStatus, ParseRawError> {
        match raw {
            x if x == UserStatus::Present.to_raw() => Ok(UserStatus::Present),
            x if x == UserStatus::Inactive.to_raw() => Ok(UserStatus::Inactive),
            _ => Err(ParseRawError::InvalidIntegerSigned(raw)),
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

// FIXME: Remove this function if microsoft/windows-rs#1798 gets merged and published.
fn is_equal_guid(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
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
            x if is_equal_guid(x, &SystemServices::GUID_MIN_POWER_SAVINGS) => {
                Ok(PowerSchemePersonality::HighPerformance)
            }
            x if is_equal_guid(x, &SystemServices::GUID_MAX_POWER_SAVINGS) => {
                Ok(PowerSchemePersonality::PowerSaver)
            }
            x if is_equal_guid(x, &SystemServices::GUID_TYPICAL_POWER_SAVINGS) => {
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

/// Struct converted from Power::POWERBROADCAST_SETTING
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
    /// The `raw` must be a valid Power::POWERBROADCAST_SETTING pointer.
    /// Otherwise, it is undefined behavior.
    pub unsafe fn from_raw(raw: *mut c_void) -> Result<PowerBroadcastSetting, ParseRawError> {
        let setting = &*(raw as *const Power::POWERBROADCAST_SETTING);
        let data = &setting.Data as *const u8;

        match &setting.PowerSetting {
            x if is_equal_guid(x, &SystemServices::GUID_ACDC_POWER_SOURCE) => {
                let power_source = *(data as *const i32);
                Ok(PowerBroadcastSetting::AcdcPowerSource(
                    PowerSource::from_raw(power_source)?,
                ))
            }
            x if is_equal_guid(x, &SystemServices::GUID_BATTERY_PERCENTAGE_REMAINING) => {
                let percentage = *(data as *const u32);
                Ok(PowerBroadcastSetting::BatteryPercentageRemaining(
                    percentage,
                ))
            }
            x if is_equal_guid(x, &SystemServices::GUID_CONSOLE_DISPLAY_STATE) => {
                let display_state = *(data as *const i32);
                Ok(PowerBroadcastSetting::ConsoleDisplayState(
                    DisplayState::from_raw(display_state)?,
                ))
            }
            x if is_equal_guid(x, &SystemServices::GUID_GLOBAL_USER_PRESENCE) => {
                let user_status = *(data as *const i32);
                Ok(PowerBroadcastSetting::GlobalUserPresence(
                    UserStatus::from_raw(user_status)?,
                ))
            }
            x if is_equal_guid(x, &SystemServices::GUID_IDLE_BACKGROUND_TASK) => {
                Ok(PowerBroadcastSetting::IdleBackgroundTask)
            }
            x if is_equal_guid(x, &SystemServices::GUID_MONITOR_POWER_ON) => {
                let monitor_state = *(data as *const u32);
                Ok(PowerBroadcastSetting::MonitorPowerOn(
                    MonitorState::from_raw(monitor_state)?,
                ))
            }
            x if is_equal_guid(x, &SystemServices::GUID_POWER_SAVING_STATUS) => {
                let battery_saver_state = *(data as *const u32);
                Ok(PowerBroadcastSetting::PowerSavingStatus(
                    BatterySaverState::from_raw(battery_saver_state)?,
                ))
            }
            x if is_equal_guid(x, &SystemServices::GUID_POWERSCHEME_PERSONALITY) => {
                let guid = *(data as *const GUID);
                Ok(PowerBroadcastSetting::PowerSchemePersonality(
                    PowerSchemePersonality::from_guid(&guid)?,
                ))
            }
            x if is_equal_guid(x, &SystemServices::GUID_SYSTEM_AWAYMODE) => {
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
        match event_type {
            WindowsAndMessaging::PBT_APMPOWERSTATUSCHANGE => Ok(PowerEventParam::PowerStatusChange),
            WindowsAndMessaging::PBT_APMRESUMEAUTOMATIC => Ok(PowerEventParam::ResumeAutomatic),
            WindowsAndMessaging::PBT_APMRESUMESUSPEND => Ok(PowerEventParam::ResumeSuspend),
            WindowsAndMessaging::PBT_APMSUSPEND => Ok(PowerEventParam::Suspend),
            WindowsAndMessaging::PBT_POWERSETTINGCHANGE => Ok(PowerEventParam::PowerSettingChange(
                PowerBroadcastSetting::from_raw(event_data)?,
            )),
            WindowsAndMessaging::PBT_APMBATTERYLOW => Ok(PowerEventParam::BatteryLow),
            WindowsAndMessaging::PBT_APMOEMEVENT => Ok(PowerEventParam::OemEvent),
            WindowsAndMessaging::PBT_APMQUERYSUSPEND => Ok(PowerEventParam::QuerySuspend),
            WindowsAndMessaging::PBT_APMQUERYSUSPENDFAILED => {
                Ok(PowerEventParam::QuerySuspendFailed)
            }
            WindowsAndMessaging::PBT_APMRESUMECRITICAL => Ok(PowerEventParam::ResumeCritical),
            _ => Err(ParseRawError::InvalidInteger(event_type)),
        }
    }
}

/// Enum describing the reason of a SessionChange event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SessionChangeReason {
    ConsoleConnect = WindowsAndMessaging::WTS_CONSOLE_CONNECT,
    ConsoleDisconnect = WindowsAndMessaging::WTS_CONSOLE_DISCONNECT,
    RemoteConnect = WindowsAndMessaging::WTS_REMOTE_CONNECT,
    RemoteDisconnect = WindowsAndMessaging::WTS_REMOTE_DISCONNECT,
    SessionLogon = WindowsAndMessaging::WTS_SESSION_LOGON,
    SessionLogoff = WindowsAndMessaging::WTS_SESSION_LOGOFF,
    SessionLock = WindowsAndMessaging::WTS_SESSION_LOCK,
    SessionUnlock = WindowsAndMessaging::WTS_SESSION_UNLOCK,
    SessionRemoteControl = WindowsAndMessaging::WTS_SESSION_REMOTE_CONTROL,
    SessionCreate = WindowsAndMessaging::WTS_SESSION_CREATE,
    SessionTerminate = WindowsAndMessaging::WTS_SESSION_TERMINATE,
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

/// Struct converted from RemoteDesktop::WTSSESSION_NOTIFICATION
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionNotification {
    pub size: u32,
    pub session_id: u32,
}

impl SessionNotification {
    pub fn from_raw(raw: RemoteDesktop::WTSSESSION_NOTIFICATION) -> Self {
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
    /// The `event_data` must be a valid RemoteDesktop::WTSSESSION_NOTIFICATION pointer.
    /// Otherwise, it is undefined behavior.
    pub unsafe fn from_event(
        event_type: u32,
        event_data: *mut c_void,
    ) -> Result<Self, ParseRawError> {
        let notification = *(event_data as *const RemoteDesktop::WTSSESSION_NOTIFICATION);

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
            Services::SERVICE_CONTROL_CONTINUE => Ok(ServiceControl::Continue),
            Services::SERVICE_CONTROL_INTERROGATE => Ok(ServiceControl::Interrogate),
            Services::SERVICE_CONTROL_NETBINDADD => Ok(ServiceControl::NetBindAdd),
            Services::SERVICE_CONTROL_NETBINDDISABLE => Ok(ServiceControl::NetBindDisable),
            Services::SERVICE_CONTROL_NETBINDENABLE => Ok(ServiceControl::NetBindEnable),
            Services::SERVICE_CONTROL_NETBINDREMOVE => Ok(ServiceControl::NetBindRemove),
            Services::SERVICE_CONTROL_PARAMCHANGE => Ok(ServiceControl::ParamChange),
            Services::SERVICE_CONTROL_PAUSE => Ok(ServiceControl::Pause),
            Services::SERVICE_CONTROL_PRESHUTDOWN => Ok(ServiceControl::Preshutdown),
            Services::SERVICE_CONTROL_SHUTDOWN => Ok(ServiceControl::Shutdown),
            Services::SERVICE_CONTROL_STOP => Ok(ServiceControl::Stop),
            Services::SERVICE_CONTROL_HARDWAREPROFILECHANGE => {
                HardwareProfileChangeParam::from_raw(event_type)
                    .map(ServiceControl::HardwareProfileChange)
            }
            Services::SERVICE_CONTROL_POWEREVENT => {
                PowerEventParam::from_event(event_type, event_data).map(ServiceControl::PowerEvent)
            }
            Services::SERVICE_CONTROL_SESSIONCHANGE => {
                SessionChangeParam::from_event(event_type, event_data)
                    .map(ServiceControl::SessionChange)
            }
            Services::SERVICE_CONTROL_TIMECHANGE => Ok(ServiceControl::TimeChange),
            Services::SERVICE_CONTROL_TRIGGEREVENT => Ok(ServiceControl::TriggerEvent),
            _ => Err(ParseRawError::InvalidInteger(raw)),
        }
    }

    pub fn raw_service_control_type(&self) -> u32 {
        match self {
            ServiceControl::Continue => Services::SERVICE_CONTROL_CONTINUE,
            ServiceControl::Interrogate => Services::SERVICE_CONTROL_INTERROGATE,
            ServiceControl::NetBindAdd => Services::SERVICE_CONTROL_NETBINDADD,
            ServiceControl::NetBindDisable => Services::SERVICE_CONTROL_NETBINDDISABLE,
            ServiceControl::NetBindEnable => Services::SERVICE_CONTROL_NETBINDENABLE,
            ServiceControl::NetBindRemove => Services::SERVICE_CONTROL_NETBINDREMOVE,
            ServiceControl::ParamChange => Services::SERVICE_CONTROL_PARAMCHANGE,
            ServiceControl::Pause => Services::SERVICE_CONTROL_PAUSE,
            ServiceControl::Preshutdown => Services::SERVICE_CONTROL_PRESHUTDOWN,
            ServiceControl::Shutdown => Services::SERVICE_CONTROL_SHUTDOWN,
            ServiceControl::Stop => Services::SERVICE_CONTROL_STOP,
            ServiceControl::HardwareProfileChange(_) => {
                Services::SERVICE_CONTROL_HARDWAREPROFILECHANGE
            }
            ServiceControl::PowerEvent(_) => Services::SERVICE_CONTROL_POWEREVENT,
            ServiceControl::SessionChange(_) => Services::SERVICE_CONTROL_SESSIONCHANGE,
            ServiceControl::TimeChange => Services::SERVICE_CONTROL_TIMECHANGE,
            ServiceControl::TriggerEvent => Services::SERVICE_CONTROL_TRIGGEREVENT,
        }
    }
}

/// Service state returned as a part of [`ServiceStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ServiceState {
    Stopped = Services::SERVICE_STOPPED,
    StartPending = Services::SERVICE_START_PENDING,
    StopPending = Services::SERVICE_STOP_PENDING,
    Running = Services::SERVICE_RUNNING,
    ContinuePending = Services::SERVICE_CONTINUE_PENDING,
    PausePending = Services::SERVICE_PAUSE_PENDING,
    Paused = Services::SERVICE_PAUSED,
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

    fn to_raw(self) -> u32 {
        self as u32
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
/// [`dwWin32ExitCode`]: Services::SERVICE_STATUS::dwWin32ExitCode
/// [`dwServiceSpecificExitCode`]: Services::SERVICE_STATUS::dwServiceSpecificExitCode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceExitCode {
    Win32(u32),
    ServiceSpecific(u32),
}

impl ServiceExitCode {
    /// A `ServiceExitCode` indicating success, no errors.
    pub const NO_ERROR: Self = ServiceExitCode::Win32(NO_ERROR);

    fn copy_to(&self, raw_service_status: &mut Services::SERVICE_STATUS) {
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

impl<'a> From<&'a Services::SERVICE_STATUS> for ServiceExitCode {
    fn from(service_status: &'a Services::SERVICE_STATUS) -> Self {
        if service_status.dwWin32ExitCode == ERROR_SERVICE_SPECIFIC_ERROR {
            ServiceExitCode::ServiceSpecific(service_status.dwServiceSpecificExitCode)
        } else {
            ServiceExitCode::Win32(service_status.dwWin32ExitCode)
        }
    }
}

impl<'a> From<&'a Services::SERVICE_STATUS_PROCESS> for ServiceExitCode {
    fn from(service_status: &'a Services::SERVICE_STATUS_PROCESS) -> Self {
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
        const NETBIND_CHANGE = Services::SERVICE_ACCEPT_NETBINDCHANGE;

        /// The service can reread its startup parameters without being stopped and restarted.
        const PARAM_CHANGE = Services::SERVICE_ACCEPT_PARAMCHANGE;

        /// The service can be paused and continued.
        const PAUSE_CONTINUE = Services::SERVICE_ACCEPT_PAUSE_CONTINUE;

        /// The service can perform preshutdown tasks.
        /// Mutually exclusive with shutdown.
        const PRESHUTDOWN = Services::SERVICE_ACCEPT_PRESHUTDOWN;

        /// The service is notified when system shutdown occurs.
        /// Mutually exclusive with preshutdown.
        const SHUTDOWN = Services::SERVICE_ACCEPT_SHUTDOWN;

        /// The service can be stopped.
        const STOP = Services::SERVICE_ACCEPT_STOP;

        /// The service is notified when the computer's hardware profile has changed.
        /// This enables the system to send SERVICE_CONTROL_HARDWAREPROFILECHANGE
        /// notifications to the service.
        const HARDWARE_PROFILE_CHANGE = Services::SERVICE_ACCEPT_HARDWAREPROFILECHANGE;

        /// The service is notified when the computer's power status has changed.
        /// This enables the system to send SERVICE_CONTROL_POWEREVENT notifications to the service.
        const POWER_EVENT = Services::SERVICE_ACCEPT_POWEREVENT;

        /// The service is notified when the computer's session status has changed.
        /// This enables the system to send SERVICE_CONTROL_SESSIONCHANGE notifications to the service.
        const SESSION_CHANGE = Services::SERVICE_ACCEPT_SESSIONCHANGE;

        /// The service is notified when the system time has changed.
        /// This enables the system to send SERVICE_CONTROL_TIMECHANGE notifications to the service.
        const TIME_CHANGE = Services::SERVICE_ACCEPT_TIMECHANGE;

        /// The service is notified when an event for which the service has registered occurs.
        /// This enables the system to send SERVICE_CONTROL_TRIGGEREVENT notifications to the service.
        const TRIGGER_EVENT = Services::SERVICE_ACCEPT_TRIGGEREVENT;
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
/// [`SERVICE_STATUS`]: Services::SERVICE_STATUS
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
    /// milliseconds in a `u32`.
    pub wait_hint: Duration,

    /// Process ID of the service
    /// This is only retrieved when querying the service status.
    pub process_id: Option<u32>,
}

impl ServiceStatus {
    pub(crate) fn to_raw(&self) -> Services::SERVICE_STATUS {
        let mut raw_status = unsafe { mem::zeroed::<Services::SERVICE_STATUS>() };
        raw_status.dwServiceType = self.service_type.bits();
        raw_status.dwCurrentState = self.current_state.to_raw();
        raw_status.dwControlsAccepted = self.controls_accepted.bits();

        self.exit_code.copy_to(&mut raw_status);

        raw_status.dwCheckPoint = self.checkpoint;

        raw_status.dwWaitHint =
            u32::try_from(self.wait_hint.as_millis()).expect("Too long wait_hint");

        raw_status
    }

    /// Tries to parse a `SERVICE_STATUS` into a Rust [`ServiceStatus`].
    ///
    /// # Errors
    ///
    /// Returns an error if the `dwCurrentState` field does not represent a valid [`ServiceState`].
    fn from_raw(raw: Services::SERVICE_STATUS) -> Result<Self, ParseRawError> {
        Ok(ServiceStatus {
            service_type: ServiceType::from_bits_truncate(raw.dwServiceType),
            current_state: ServiceState::from_raw(raw.dwCurrentState)?,
            controls_accepted: ServiceControlAccept::from_bits_truncate(raw.dwControlsAccepted),
            exit_code: ServiceExitCode::from(&raw),
            checkpoint: raw.dwCheckPoint,
            wait_hint: Duration::from_millis(raw.dwWaitHint as u64),
            process_id: None,
        })
    }

    /// Tries to parse a `SERVICE_STATUS_PROCESS` into a Rust [`ServiceStatus`].
    ///
    /// # Errors
    ///
    /// Returns an error if the `dwCurrentState` field does not represent a valid [`ServiceState`].
    fn from_raw_ex(raw: Services::SERVICE_STATUS_PROCESS) -> Result<Self, ParseRawError> {
        let current_state = ServiceState::from_raw(raw.dwCurrentState)?;
        let process_id = match current_state {
            ServiceState::Running => Some(raw.dwProcessId),
            _ => None,
        };
        Ok(ServiceStatus {
            service_type: ServiceType::from_bits_truncate(raw.dwServiceType),
            current_state,
            controls_accepted: ServiceControlAccept::from_bits_truncate(raw.dwControlsAccepted),
            exit_code: ServiceExitCode::from(&raw),
            checkpoint: raw.dwCheckPoint,
            wait_hint: Duration::from_millis(raw.dwWaitHint as u64),
            process_id,
        })
    }
}

#[derive(Debug)]
pub enum ParseRawError {
    InvalidInteger(u32),
    InvalidIntegerSigned(i32),
    InvalidGuid(String),
}

impl std::error::Error for ParseRawError {}

impl std::fmt::Display for ParseRawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInteger(u) => {
                write!(f, "invalid unsigned integer for the target type: {}", u)
            }
            Self::InvalidIntegerSigned(i) => {
                write!(f, "invalid signed integer for the target type: {}", i)
            }
            Self::InvalidGuid(guid) => {
                write!(f, "invalid GUID value for the target type: {}", guid)
            }
        }
    }
}

fn string_from_guid(guid: &GUID) -> String {
    format!(
        "{:8X}-{:4X}-{:4X}-{:2X}{:2X}-{:2X}{:2X}{:2X}{:2X}{:2X}{:2X}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}
