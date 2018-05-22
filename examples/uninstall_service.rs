#[cfg(windows)]
extern crate windows_service;

#[cfg(windows)]
fn main() {
    use std::thread;
    use std::time::Duration;
    use windows_service::service::{ServiceAccess, ServiceState};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access).unwrap();

    let service_access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
    let service = service_manager
        .open_service("ping_service", service_access)
        .unwrap();

    let service_status = service.query_status().unwrap();
    if service_status.current_state != ServiceState::Stopped {
        service.stop().unwrap();
        // Wait for service to stop
        thread::sleep(Duration::from_secs(1));
    }

    service.delete().unwrap();
}

#[cfg(not(windows))]
fn main() {
    panic!("This program is only intended to run on Windows.");
}
