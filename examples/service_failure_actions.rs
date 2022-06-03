#[cfg(windows)]
fn main() -> windows_service::Result<()> {
    use std::ffi::OsString;
    use std::time::Duration;
    use windows_service::{
        service::{
            ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl,
            ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType,
            ServiceType,
        },
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    const SERVICE_NAME: &str = "service_failure_actions_example";

    let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let service_binary_path = ::std::env::current_exe()
        .unwrap()
        .with_file_name("service_failure_actions.exe");

    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("Service Failure Actions Example"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::OnDemand,
        error_control: ServiceErrorControl::Normal,
        executable_path: service_binary_path,
        launch_arguments: vec![],
        dependencies: vec![],
        account_name: None, // run as System
        account_password: None,
    };

    let service_access = ServiceAccess::QUERY_CONFIG
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::START
        | ServiceAccess::DELETE;

    println!("Create or open the service {}", SERVICE_NAME);
    let service = service_manager
        .create_service(&service_info, service_access)
        .or(service_manager.open_service(SERVICE_NAME, service_access))?;

    let actions = vec![
        ServiceAction {
            action_type: ServiceActionType::Restart,
            delay: Duration::from_secs(5),
        },
        ServiceAction {
            action_type: ServiceActionType::RunCommand,
            delay: Duration::from_secs(10),
        },
        ServiceAction {
            action_type: ServiceActionType::None,
            delay: Duration::default(),
        },
    ];

    println!("Update failure actions");
    let failure_actions = ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400 * 2)),
        reboot_msg: None,
        command: Some(OsString::from("ping 127.0.0.1")),
        actions: Some(actions),
    };
    service.update_failure_actions(failure_actions)?;

    println!("Query failure actions");
    let updated_failure_actions = service.get_failure_actions()?;
    println!("{:#?}", updated_failure_actions);

    println!("Enable failure actions on non-crash failures");
    service.set_failure_actions_on_non_crash_failures(true)?;

    println!("Query failure actions on non-crash failures enabled");
    let failure_actions_flag = service.get_failure_actions_on_non_crash_failures()?;
    println!(
        "Failure actions on non-crash failures enabled: {}",
        failure_actions_flag
    );

    println!("Delete the service {}", SERVICE_NAME);
    service.delete()?;

    Ok(())
}

#[cfg(not(windows))]
fn main() {
    panic!("This program is only intended to run on Windows.");
}
