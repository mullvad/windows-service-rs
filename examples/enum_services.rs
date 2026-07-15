#[cfg(windows)]
fn main() -> windows_service::Result<()> {
    use windows_service::{
        service_enum::{EnumServiceState, EnumServiceType},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager_access = ServiceManagerAccess::ENUMERATE_SERVICE;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let services = service_manager.enum_services_status_raw(
        EnumServiceType::WIN32_OWN_PROCESS,
        EnumServiceState::STATE_ALL,
    )?;

    let longest_name = services
        .iter()
        .map(|s| s.service_name().chars().count())
        .max()
        .unwrap_or(0);

    {
        const TITLE_NAME: &str = "Name";
        const TITLE_DISPLAY_NAME: &str = "Display Name";

        print!("{TITLE_NAME}");
        for _ in TITLE_NAME.chars().count()..=longest_name {
            print!(" ");
        }
        println!("{TITLE_DISPLAY_NAME}");
        println!(
            "{:-<width$}",
            "",
            width = longest_name + TITLE_DISPLAY_NAME.chars().count() + 1
        );
    }

    for service in &services {
        let name_len = service.service_name().chars().count();
        print!("{}", service.service_name().display());

        for _ in name_len..=longest_name {
            print!(" ");
        }

        println!("{}", service.display_name().display());
    }

    Ok(())
}

#[cfg(not(windows))]
fn main() {
    panic!("This program is only intended to run on Windows.");
}
