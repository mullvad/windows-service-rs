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
