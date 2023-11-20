/// This is an example program that demonstrates how to send a user-defined control code to a given service.
///
/// Run in command prompt as admin:
///
/// `notify_service.exe SERVICE_NAME`
///
/// Replace the `SERVICE_NAME` placeholder above with the name of the service that the program
/// should notify.

#[cfg(windows)]
fn main() -> windows_service::Result<()> {
    use std::env;
    use windows_service::{
        service::{ServiceAccess, UserEventCode},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let service_name = env::args().nth(1).unwrap_or("ping_service".to_owned());

    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let service = service_manager.open_service(
        &service_name,
        ServiceAccess::PAUSE_CONTINUE | ServiceAccess::USER_DEFINED_CONTROL,
    )?;

    const NO_OP: UserEventCode = unsafe { UserEventCode::from_unchecked(128) };
    const CUSTOM_STOP: UserEventCode = unsafe { UserEventCode::from_unchecked(130) };

    println!("Send `NO_OP` notification to {}", service_name);
    let state = service.notify(NO_OP)?;
    println!("{:?}", state.current_state);

    println!("Send `CUSTOM_STOP` notification to {}", service_name);
    let state = service.notify(CUSTOM_STOP)?;
    println!("{:?}", state.current_state);

    Ok(())
}

#[cfg(not(windows))]
fn main() {
    panic!("This program is only intended to run on Windows.");
}
