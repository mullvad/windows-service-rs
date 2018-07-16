# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- A `ServiceExitCode::NO_ERROR` constant for easy access to the success value.

### Changed
- Changed `service_control_handler::register` to accept an `FnMut` rather than just an `Fn` for the
  `event_handler` closure.


## [0.1.0] - 2018-06-04
### Added
- Initial release with support for installing, uninstalling and implementing Windows services.
