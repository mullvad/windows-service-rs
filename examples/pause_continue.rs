/// This is an example program that demonstrates how to pause and resume a given service.
///
/// Run in command prompt as admin:
///
/// `pause_continue.exe SERVICE_NAME`
///
/// Replace the `SERVICE_NAME` placeholder above with the name of the service that the program
/// should manipulate. By default the program manipulates a WMI system service (Winmgmt) when the
/// first argument is omitted.

#[cfg(windows)]
fn main() -> windows_service::Result<()> {
    use std::env;
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let service_name = env::args().nth(1).unwrap_or("Winmgmt".to_owned());

    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let service = service_manager.open_service(&service_name, ServiceAccess::PAUSE_CONTINUE)?;

    println!("Pause {}", service_name);
    let paused_state = service.pause()?;
    println!("{:?}", paused_state.current_state);

    println!("Resume {}", service_name);
    let resumed_state = service.resume()?;
    println!("{:?}", resumed_state.current_state);

    Ok(())
}

#[cfg(not(windows))]
fn main() {
    panic!("This program is only intended to run on Windows.");
}
