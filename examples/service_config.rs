#[cfg(windows)]
fn main() -> windows_service::Result<()> {
    use std::env;
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let service_name = env::args().nth(1).unwrap_or("netlogon".to_owned());

    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let service = service_manager.open_service(service_name, ServiceAccess::QUERY_CONFIG)?;

    let config = service.query_config()?;
    println!("{:#?}", config);
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    panic!("This program is only intended to run on Windows.");
}
