// Ping service example.
//
// You can install and uninstall this service using other example programs.
// All commands mentioned below shall be executed in Command Prompt with Administrator privileges.
//
// Service installation: `install_service.exe`
// Service uninstallation: `uninstall_service.exe`
//
// Start the service: `net start ping_service`
// Stop the service: `net stop ping_service`
//
// Ping server sends a text message to local UDP port 1234 once a second.
// You can verify that service works by running netcat, i.e: `ncat -ul 1234`.
//
#[cfg(windows)]
#[macro_use]
extern crate windows_service;

#[cfg(windows)]
fn main() {
    ping_service::run();
}

#[cfg(not(windows))]
fn main() {
    panic!("This program is only intended to run on Windows.");
}

#[cfg(windows)]
mod ping_service {
    use std::ffi::OsString;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_dispatcher;

    static SERVICE_NAME: &'static str = "ping_service";
    static SERVICE_TYPE: ServiceType = ServiceType::OwnProcess;

    pub fn run() {
        // Register generated `service_main` with the system and start the service blocking main
        // thread until the service is stopped.
        service_dispatcher::start_dispatcher(SERVICE_NAME, service_main).unwrap();
    }

    // Generate the windows service boilerplate.
    // The boilerplate contains the low-level service entry function (service_main) that parses
    // incoming service arguments into Vec<OsString> and passes them to user defined service
    // entry (handle_service_main).
    define_windows_service!(service_main, handle_service_main);

    // Service entry function which is called on background thread by the system with service
    // parameters. There is no stdout or stderr at this point so make sure to configure the log
    // output to file if needed.
    pub fn handle_service_main(_arguments: Vec<OsString>) {
        // Create an event channel to funnel events to worker.
        let (shutdown_tx, shutdown_rx) = mpsc::channel();

        // Define system service event handler that will be receiving service events.
        let event_handler = move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                // Notifies a service to report its current status information to the service
                // control manager. Always return NoError even if not implemented.
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,

                // Handle stop
                ServiceControl::Stop => {
                    shutdown_tx.send(()).unwrap();
                    ServiceControlHandlerResult::NoError
                }

                _ => ServiceControlHandlerResult::NotImplemented,
            }
        };

        // Register system service event handler.
        // The returned status handle should be used to report service status changes to the system.
        let status_handle =
            service_control_handler::register_control_handler(SERVICE_NAME, event_handler).unwrap();

        // Tell the system that service is running
        status_handle
            .set_service_status(ServiceStatus {
                service_type: SERVICE_TYPE,
                current_state: ServiceState::Running,
                controls_accepted: ServiceControlAccept::STOP,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
            })
            .unwrap();

        let worker_thread_handle = thread::spawn(move || {
            let ipv4 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
            let sender_addr = SocketAddr::new(ipv4, 0);
            let receiver_addr = SocketAddr::new(ipv4, 1234);
            let msg = "ping\n".as_bytes();
            let socket = UdpSocket::bind(sender_addr).unwrap();

            loop {
                // For demo purposes this worker sends a UDP packet once a second.
                let _ = socket.send_to(msg, receiver_addr);

                // Poll shutdown event.
                match shutdown_rx.recv_timeout(Duration::from_secs(1)) {
                    // Break the loop either upon stop or channel disconnect
                    Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,

                    // Continue work if no events were received within the timeout
                    Err(mpsc::RecvTimeoutError::Timeout) => (),
                };
            }
        });

        // Block current thread while worker thread is running.
        worker_thread_handle.join().unwrap();

        // Tell the system that service has stopped.
        status_handle
            .set_service_status(ServiceStatus {
                service_type: SERVICE_TYPE,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
            })
            .unwrap();
    }

}
