# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- Add support for specifying service dependencies when creating a service.
- A `ServiceExitCode::NO_ERROR` constant for easy access to the success value.

### Fixed
- Fix invalid pointer manipulations in service creation routine in ServiceManager.


## [0.1.0] - 2018-06-04
### Added
- Initial release with support for installing, uninstalling and implementing Windows services.
