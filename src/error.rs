use crate::service;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(err_derive::Error, Debug)]
#[error(no_from)]
pub enum Error {
    /// Invalid account name.
    #[error(display = "Invalid account name")]
    InvalidAccountName(#[error(source)] NulError),

    /// Invalid account password.
    #[error(display = "Invalid account password")]
    InvalidAccountPassword(#[error(source)] NulError),

    /// Invalid display name.
    #[error(display = "Invalid display name")]
    InvalidDisplayName(#[error(source)] NulError),

    /// Invalid database name.
    #[error(display = "Invalid database name")]
    InvalidDatabaseName(#[error(source)] NulError),

    /// Invalid executable path.
    #[error(display = "Invalid executable path")]
    InvalidExecutablePath(#[error(source)] NulError),

    /// Invalid launch arguments.
    #[error(display = "Invalid launch argument at index {}", _0)]
    InvalidLaunchArgument(usize, #[error(source)] NulError),

    /// Launch arguments are not supported for kernel drivers.
    #[error(display = "Kernel drivers do not support launch arguments")]
    LaunchArgumentsNotSupported,

    /// Invalid dependency name.
    #[error(display = "Invalid dependency name")]
    InvalidDependency(#[error(source)] NulError),

    /// Invalid machine name.
    #[error(display = "Invalid machine name")]
    InvalidMachineName(#[error(source)] NulError),

    /// Invalid service name.
    #[error(display = "Invalid service name")]
    InvalidServiceName(#[error(source)] NulError),

    /// Invalid start argument.
    #[error(display = "Invalid start argument")]
    InvalidStartArgument(#[error(source)] NulError),

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

    /// Invalid reboot message
    #[error(display = "Invalid service action failures reboot message")]
    InvalidServiceActionFailuresRebootMessage(#[error(source)] NulError),

    /// Invalid command
    #[error(display = "Invalid service action failures command")]
    InvalidServiceActionFailuresCommand(#[error(source)] NulError),

    /// Invalid service description
    #[error(display = "Invalid service description")]
    InvalidServiceDescription(#[error(source)] NulError),

    /// IO error when calling winapi
    #[error(display = "IO error in winapi call")]
    Winapi(#[error(source)] std::io::Error),
}

/// Indicates a invalid nul value was found when converting a string to a wide string.
/// This error contains the position of the nul value, as well as the faulty string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NulError(usize, Option<Vec<u16>>);

impl NulError {
    /// Returns the position of the nul value in the slice that was provided to `U16CString`.
    pub fn nul_position(&self) -> usize {
        self.0
    }

    /// Consumes this error, returning the underlying vector of u16 values which generated the error
    /// in the first place.
    pub fn into_vec(self) -> Option<Vec<u16>> {
        self.1
    }
}

impl core::fmt::Display for NulError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "nul value found at position {}", self.0)
    }
}

impl std::error::Error for NulError {
    fn description(&self) -> &str {
        "nul value found"
    }
}

impl From<widestring::error::ContainsNul<u16>> for NulError {
    fn from(s: widestring::error::ContainsNul<u16>) -> NulError {
        NulError(s.nul_position(), s.into_vec())
    }
}
