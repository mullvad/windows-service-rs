# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- Add support for service failure actions. (See: `ServiceFailureActions`, 
  `Service::update_failure_actions`, `Service::get_failure_actions`, 
  `Service::set_failure_actions_on_non_crash_failures`, 
  `Service::get_failure_actions_on_non_crash_failures`)

### Changed
- Bumped the MSRV to 1.33, because of err-derive upgrade which depend on quote, and to use
  `Duration::as_millis()`.

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
