use windows_sys::Win32::{Storage::FileSystem, System::Services};

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
