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
