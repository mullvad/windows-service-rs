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
