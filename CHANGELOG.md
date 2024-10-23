# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]
- Upgrade `windows-sys` dependency to 0.59 and bump the MSRV to 1.60.0


## [0.7.0] - 2024-04-12
### Added
- Breaking: Add support for user-defined control codes in services.
  (See: `Service::notify` and `notify_service.rs` example). This is breaking since
  the `ServiceControl` enum was exhaustive in version 0.6.0.
- Breaking: Add support for `LidSwitchStateChange` in `PowerBroadcastSetting`. This is breaking
  since `PowerBroadcastSetting` was an exhaustive enum in version 0.6.0.
- Breaking: Add support for `SERVICE_SYSTEM_START` and `SERVICE_BOOT_START` in service
  start type. This is breaking since the `ServiceStartType` enum is exhaustive.
- Add function for obtaining service SID infos. (See: `Service::get_config_service_sid_info`).

### Changed
- Breaking: Make a bunch of enums `#[non_exhaustive]`: `Error`, `PowerBroadcastSetting`,
  `PowerEventParam` and `ServiceControl`.
- Breaking: Upgrade `windows-sys` dependency to 0.52


## [0.6.0] - 2023-03-07
### Added
- Add support for delayed autostart in services. (See: `Service::set_delayed_auto_start`)
- Add support for specifying a preshutdown timeout. (See: `Service::set_preshutdown_timeout`)
- Add function for obtaining service names from display names.
  (See: `ServiceManager::service_name_from_display_name`)

### Changed
- Breaking: Consolidate `Error` type. Remove dependency on `err-derive`.
- Breaking: `Service::delete` does not consume `self` any longer. Make sure to `drop` a reference
  to `Service` manually if you plan to poll SCM synchronously to determine when the service is
  removed from system. (See `uninstall_service.rs` example)


## [0.5.0] - 2022-07-20
### Added
- Implement `AsRawHandle` for `ServiceStatusHandle`. Allows using the service handles
  with other Windows APIs, not covered by this crate.

### Changed
- Upgrade the crate to Rust 2021 edition and bump the MSRV to 1.58.0
- Breaking: Change `winapi` dependency to `windows-sys`. This is a breaking change since
  some of the low level Windows types are exposed in the public API of this library.
- Breaking: Update `widestring` dependency to 1.0 and remove it from the public API.
- Breaking: Change `ServiceState::to_raw` to take `self` instead of `&self`.


## [0.4.0] - 2021-08-12
### Changed
- Breaking: `ServiceDependency::from_system_identifier()`, `ServiceManager::new()`,
  `ServiceManager::local_computer()`, `ServiceManager::remote_computer()` now take
  `impl AsRef<OsStr>` arguments.
- Upgrade err-derive dependency to 0.3.0
- `ServiceStatusHandle` is now Sync.

### Fixed
- Don't escape binary path for kernel drivers as they don't support that.


## [0.3.1] - 2020-10-27
### Added
- Add support for service description. (See: `Service::set_description`)

### Fixed
- Fix segmentation fault in `Service` functions, that query service config, by moving buffer
  allocation to heap.


## [0.3.0] - 2020-06-18
### Added
- Add support for configuring the service SID info.
- Add support for changing mandatory configuration settings on service.
- Add support for service failure actions. (See: `ServiceFailureActions`,
  `Service::update_failure_actions`, `Service::get_failure_actions`,
  `Service::set_failure_actions_on_non_crash_failures`,
  `Service::get_failure_actions_on_non_crash_failures`)
- Add support to pause and continue services. (See: `Service::pause` and `Service::resume`)
- Use `QueryServiceStatusEx` when querying service status. Allows getting the process ID of a
  running service

### Changed
- Bumped the MSRV to 1.34, because of err-derive upgrade which depend on quote, to use
  `Duration::as_millis()` and the `TryFrom` trait.
- Breaking: `ServiceManager::create_service()` now expects a borrowed `ServiceInfo` argument.


## [0.2.0] - 2019-04-01
### Added
- Add `ServiceExitCode::NO_ERROR` constant for easy access to the success value.
- Add `Service::start` for starting services programmatically.
- Add `Service::query_config` for getting the config of the service.
- Add `ServiceInfo::dependencies` for specifying service dependencies.

### Changed
- Changed `service_control_handler::register` to accept an `FnMut` rather than just an `Fn` for the
  `event_handler` closure.
- Upgrade to Rust 2018. This raises the minimum required Rust version to 1.31.0.
- Replace error-chain error library with err-derive. So all error types are changed.
- Change `ServiceType` implementation to use the `bitflags!` macro.

### Fixed
- Fix invalid pointer manipulations in service creation routine in ServiceManager.
- Fix memory leak in `service_control_handler::register` that did not release `event_handler` in
  the case of an error.
- Treat FFI return code 0 as error, instead of treating 1 as success.


## [0.1.0] - 2018-06-04
### Added
- Initial release with support for installing, uninstalling and implementing Windows services.
