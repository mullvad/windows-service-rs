// Copyright 2017 Amagicom AB.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A crate that provides facilities for management and implementation of windows services.
//!
//! # Implementing windows service
//!
//! This section describes the steps of implementing a program that runs as a windows service, for
//! complete source code of such program take a look at examples folder.
//!
//! ## Basics
//!
//! Each windows service has to implement a service entry function `fn(argc: u32, argv: *mut *mut
//! u16)` and register it with the system from the application's `main`.
//!
//! This crate provides a handy [`define_windows_service!`] macro to generate a low level
//! boilerplate for the service entry function that parses input from the system and delegates
//! handling to user defined higher level function `fn(arguments: Vec<OsString>)`.
//!
//! This guide references the low level entry function as `ffi_service_main` and higher
//! level function as `my_service_main` but it's up to developer how to call them.
//!
//! ```rust,no_run
//! #[macro_use]
//! extern crate windows_service;
//!
//! use std::ffi::OsString;
//! use windows_service::{Result, service_dispatcher};
//!
//! define_windows_service!(ffi_service_main, my_service_main);
//!
//! fn my_service_main(arguments: Vec<OsString>) {
//!     // The entry point where execution will start on a background thread after a call to
//!     // `service_dispatcher::start` from `main`.
//! }
//!
//! fn main() -> Result<()> {
//!     // Register generated `ffi_service_main` with the system and start the service, blocking
//!     // this thread until the service is stopped.
//!     service_dispatcher::start("myservice", ffi_service_main)?;
//!     Ok(())
//! }
//! ```
//!
//! ## Handling service events
//!
//! The first thing that a windows service should do early in its lifecycle is to subscribe for
//! service events such as stop or pause and many other.
//!
//! ```rust,no_run
//! extern crate windows_service;
//!
//! use std::ffi::OsString;
//! use windows_service::service::ServiceControl;
//! use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
//! use windows_service::Result;
//!
//! fn my_service_main(arguments: Vec<OsString>) {
//!     if let Err(_e) = run_service(arguments) {
//!         // Handle errors in some way.
//!     }
//! }
//!
//! fn run_service(arguments: Vec<OsString>) -> Result<()> {
//!     let event_handler = move |control_event| -> ServiceControlHandlerResult {
//!         match control_event {
//!             ServiceControl::Stop => {
//!                 // Handle stop event and return control back to the system.
//!                 ServiceControlHandlerResult::NoError
//!             }
//!             // All services must accept Interrogate even if it's a no-op.
//!             ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
//!             _ => ServiceControlHandlerResult::NotImplemented,
//!         }
//!     };
//!
//!     // Register system service event handler
//!     let status_handle = service_control_handler::register("myservice", event_handler)?;
//!     Ok(())
//! }
//! ```
//!
//! Please see the corresponding MSDN article that describes how event handler works internally:\
//! <https://msdn.microsoft.com/en-us/library/windows/desktop/ms685149(v=vs.85).aspx>
//!
//! ## Updating service status
//!
//! When application that implements a windows service is launched by the system, it's
//! automatically put in the [`StartPending`] state.
//!
//! The application needs to complete the initialization, obtain [`ServiceStatusHandle`] (see
//! [`service_control_handler::register`]) and transition to [`Running`] state.
//!
//! If service has a lengthy initialization, it should immediately tell the system how
//! much time it needs to complete it, by sending the [`StartPending`] state, time
//! estimate using [`ServiceStatus::wait_hint`] and increment [`ServiceStatus::checkpoint`] each
//! time the service completes a step in initialization.
//!
//! The system will attempt to kill a service that is not able to transition to [`Running`]
//! state before the proposed [`ServiceStatus::wait_hint`] expired.
//!
//! The same concept applies when transitioning between other pending states and their
//! corresponding target states.
//!
//! Note that it's safe to clone [`ServiceStatusHandle`] and use it from any thread.
//!
//! ```rust,no_run
//! extern crate windows_service;
//!
//! use std::ffi::OsString;
//! use std::time::Duration;
//! use windows_service::service::{
//!     ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
//!     ServiceType,
//! };
//! use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
//!
//! fn my_service_main(arguments: Vec<OsString>) {
//!     if let Err(_e) = run_service(arguments) {
//!         // Handle error in some way.
//!     }
//! }
//!
//! fn run_service(arguments: Vec<OsString>) -> windows_service::Result<()> {
//!     let event_handler = move |control_event| -> ServiceControlHandlerResult {
//!         match control_event {
//!             ServiceControl::Stop | ServiceControl::Interrogate => {
//!                 ServiceControlHandlerResult::NoError
//!             }
//!             _ => ServiceControlHandlerResult::NotImplemented,
//!         }
//!     };
//!
//!     // Register system service event handler
//!     let status_handle = service_control_handler::register("my_service_name", event_handler)?;
//!
//!     let next_status = ServiceStatus {
//!         // Should match the one from system service registry
//!         service_type: ServiceType::OWN_PROCESS,
//!         // The new state
//!         current_state: ServiceState::Running,
//!         // Accept stop events when running
//!         controls_accepted: ServiceControlAccept::STOP,
//!         // Used to report an error when starting or stopping only, otherwise must be zero
//!         exit_code: ServiceExitCode::Win32(0),
//!         // Only used for pending states, otherwise must be zero
//!         checkpoint: 0,
//!         // Only used for pending states, otherwise must be zero
//!         wait_hint: Duration::default(),
//!         // Unused for setting status
//!         process_id: None,
//!     };
//!
//!     // Tell the system that the service is running now
//!     status_handle.set_service_status(next_status)?;
//!
//!     // Do some work
//!
//!     Ok(())
//! }
//! ```
//!
//! Please refer to the "Service State Transitions" article on MSDN for more info:\
//! <https://msdn.microsoft.com/en-us/library/windows/desktop/ee126211(v=vs.85).aspx>
//!
//! [`ServiceStatusHandle`]: service_control_handler::ServiceStatusHandle
//! [`ServiceStatus::wait_hint`]: service::ServiceStatus::wait_hint
//! [`ServiceStatus::checkpoint`]: service::ServiceStatus::checkpoint
//! [`StartPending`]: service::ServiceState::StartPending
//! [`Running`]: service::ServiceState::Running

#![cfg(windows)]

pub type Result<T> = std::result::Result<T, Error>;

#[derive(err_derive::Error, Debug)]
#[error(no_from)]
pub enum Error {
    /// Account name contains a nul byte.
    #[error(display = "Account name contains a nul byte")]
    AccountNameHasNulByte,

    /// Account password contains a nul byte.
    #[error(display = "Account password contains a nul byte")]
    AccountPasswordHasNulByte,

    /// Display name contains a nul byte.
    #[error(display = "Display name contains a nul byte")]
    DisplayNameHasNulByte,

    /// Database name contains a nul byte.
    #[error(display = "Database name contains a nul byte")]
    DatabaseNameHasNulByte,

    /// Executable path contains a nul byte.
    #[error(display = "Executable path contains a nul byte")]
    ExecutablePathHasNulByte,

    /// Launch arguments contains a nul byte.
    #[error(display = "Launch argument at index {} contains a nul byte", _0)]
    LaunchArgumentHasNulByte(usize),

    /// Launch arguments are not supported for kernel drivers.
    #[error(display = "Kernel drivers do not support launch arguments")]
    LaunchArgumentsNotSupported,

    /// Dependency name contains a nul byte.
    #[error(display = "Dependency name contains a nul byte")]
    DependencyHasNulByte,

    /// Machine name contains a nul byte.
    #[error(display = "Machine name contains a nul byte")]
    MachineNameHasNulByte,

    /// Service name contains a nul byte.
    #[error(display = "Service name contains a nul byte")]
    ServiceNameHasNulByte,

    /// Start argument contains a nul byte.
    #[error(display = "Start argument contains a nul byte")]
    StartArgumentHasNulByte,

    /// Invalid raw representation of [`ServiceState`](service::ServiceState).
    #[error(display = "Invalid service state value")]
    InvalidServiceState(#[error(source)] service::ParseRawError),

    /// Invalid raw representation of [`ServiceStartType`](service::ServiceStartType).
    #[error(display = "Invalid service start value")]
    InvalidServiceStartType(#[error(source)] service::ParseRawError),

    /// Invalid raw representation of [`ServiceErrorControl`](service::ServiceErrorControl).
    #[error(display = "Invalid service error control value")]
    InvalidServiceErrorControl(#[error(source)] service::ParseRawError),

    /// Invalid raw representation of [`ServiceActionType`](service::ServiceActionType).
    #[error(display = "Invalid service action type")]
    InvalidServiceActionType(#[error(source)] service::ParseRawError),

    /// Reboot message contains a nul byte.
    #[error(display = "Service action failures reboot message contains a nul byte")]
    ServiceActionFailuresRebootMessageHasNulByte,

    /// Command contains a nul byte.
    #[error(display = "Service action failures command contains a nul byte")]
    ServiceActionFailuresCommandHasNulByte,

    /// Service description contains a nul byte.
    #[error(display = "Service description contains a nul byte")]
    ServiceDescriptionHasNulByte,

    /// IO error when calling winapi
    #[error(display = "IO error in winapi call")]
    Winapi(#[error(source)] std::io::Error),
}

mod sc_handle;
pub mod service;
pub mod service_control_handler;
pub mod service_manager;
#[macro_use]
pub mod service_dispatcher;

mod double_nul_terminated;
mod shell_escape;
