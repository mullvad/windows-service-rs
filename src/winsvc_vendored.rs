#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use winapi::shared::minwindef::{BOOL, DWORD};
use winapi::shared::ntdef::LPWSTR;
use winapi::{ENUM, STRUCT};

pub use winapi::um::winsvc::*;

ENUM! {enum SC_ACTION_TYPE {
    SC_ACTION_NONE = 0,
    SC_ACTION_RESTART = 1,
    SC_ACTION_REBOOT = 2,
    SC_ACTION_RUN_COMMAND = 3,
}}
STRUCT! {struct SC_ACTION {
    Type: SC_ACTION_TYPE,
    Delay: DWORD,
}}
pub type LPSC_ACTION = *mut SC_ACTION;
STRUCT! {struct SERVICE_FAILURE_ACTIONSW {
    dwResetPeriod: DWORD,
    lpRebootMsg: LPWSTR,
    lpCommand: LPWSTR,
    cActions: DWORD,
    lpsaActions: LPSC_ACTION,
}}

STRUCT! {struct SERVICE_FAILURE_ACTIONS_FLAG {
    fFailureActionsOnNonCrashFailures: BOOL,
}}
