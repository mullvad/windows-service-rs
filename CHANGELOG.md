# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]


## [0.2.0] - 2019-04-01
### Added
- Add support for specifying service dependencies when creating a service.
- A `ServiceExitCode::NO_ERROR` constant for easy access to the success value.
- Add `Service::start` for starting services programmatically.
- Add `Service::query_config` for getting the config of the service.

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
