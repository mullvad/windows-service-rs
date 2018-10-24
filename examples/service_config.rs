#[cfg(windows)]
extern crate windows_service;

#[cfg(windows)]
fn main() -> windows_service::Result<()> {
    use std::ffi::OsString;
    use std::thread;
    use std::time::Duration;
    use windows_service::service::{
        ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState,
        ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    // This example installs the service defined in `examples/ping_service.rs`.
    // In the real world code you would set the executable path to point to your own binary
    // that implements windows service.
    let service_binary_path = ::std::env::current_exe()
        .unwrap()
        .with_file_name("ping_service.exe");

    let service_info = ServiceInfo {
        name: OsString::from("ping_service"),
        display_name: OsString::from("Ping service"),
        service_type: ServiceType::OwnProcess,
        start_type: ServiceStartType::OnDemand,
        error_control: ServiceErrorControl::Normal,
        executable_path: service_binary_path,
        launch_arguments: vec![],
        dependencies: vec![],
        account_name: None, // run as System
        account_password: None,
    };
    let service = service_manager.create_service(
        service_info,
        ServiceAccess::QUERY_STATUS
            | ServiceAccess::STOP
            | ServiceAccess::DELETE
            | ServiceAccess::QUERY_CONFIG,
    )?;

    let service_status = service.query_status()?;

    let config = service.query_config()?;
    println!("{:?}", config);

    if service_status.current_state != ServiceState::Stopped {
        service.stop()?;
        // Wait for service to stop
        thread::sleep(Duration::from_secs(1));
    }

    service.delete()?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    panic!("This program is only intended to run on Windows.");
}
