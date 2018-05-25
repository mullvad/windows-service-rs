# windows-service

A crate that provides facilities for management and implementation of windows services.

## Implementing windows service

This section describes the steps of implementing a program that runs as a windows service, for
complete source code of such program take a look at examples folder.

### Basics

Each windows service has to implement a service entry function `fn(argc: u32, argv: *mut *mut
u16)` and register it with the system from the application's `main`.

This crate provides a handy [`define_windows_service!`] macro to generate a low level
boilerplate for the service entry function that parses input from the system and delegates
handling to user defined higher level function `fn(arguments: Vec<OsString>)`.

This guide references the low level entry function as `ffi_service_main` and higher
level function as `my_service_main` but it's up to developer how to call them.

```rust
#[macro_use]
extern crate windows_service;
use std::ffi::OsString;
use windows_service::service_dispatcher;

define_windows_service!(ffi_service_main, my_service_main);

fn my_service_main(arguments: Vec<OsString>) {
    // The entry point where execution will start on a background thread after a call to
    // [`service_dispatcher::start`] from `main`.
}

fn main() {
    // Register generated `ffi_service_main` with the system and start the service blocking main
    // thread until the service is stopped.
    service_dispatcher::start("myservice", ffi_service_main).unwrap();
}
```

### Handling service events

The first thing that a windows service should do early in its lifecycle is to subscribe for
service events such as stop or pause and many other.

It's worth to mention that events are dispatched concurrently so it's important to make sure
that your code is thread safe, the simplest way is to use [`std::sync::mpsc::channel`].

```rust
#[macro_use]
extern crate windows_service;
use std::ffi::OsString;
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};

fn my_service_main(arguments: Vec<OsString>) {
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop => {
                // Handle stop event and return control back to the system.
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    // Register system service event handler.
    let status_handle = service_control_handler::register("myservice", event_handler).unwrap();
}
```

### Updating service status

The service status handle ([`service_control_handler::ServiceStatusHandle`]) is issued upon
successful event handler registration (see [`service_control_handler::register`])
and should be used to notify the system about any changes to the service's internal state
during its lifecycle.

Note that it's safe to clone the service status handle to pass it to other thread.

```rust
#[macro_use]
extern crate windows_service;
use std::ffi::OsString;
use std::thread;
use windows_service::service::{
    ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};

fn my_service_main(arguments: Vec<OsString>) {
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Interrogate => {
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler).unwrap();

    // Please refer to documentation for `ServiceStatus` regarding the rules of assigning each
    // of the fields.
    let service_status = ServiceStatus {
        service_type: ServiceType::OwnProcess,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
    };

    // Tell the system that the service is running now
    status_handle.set_service_status(service_status);

    // Do some work..
}
```

License: MIT/Apache-2.0
