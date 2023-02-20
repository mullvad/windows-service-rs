use std::borrow::Cow;
use std::convert::TryFrom;
use std::ffi::{OsStr, OsString};
use std::os::raw::c_void;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr;
use std::time::Duration;
use std::{io, mem};

use widestring::{error::ContainsNul, WideCStr, WideCString, WideString};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{ERROR_SERVICE_SPECIFIC_ERROR, NO_ERROR},
        Storage::FileSystem,
        System::{Power, RemoteDesktop, Services, SystemServices, WindowsProgramming::INFINITE},
        UI::WindowsAndMessaging,
    },
};

use crate::sc_handle::ScHandle;
use crate::shell_escape;
use crate::{double_nul_terminated, Error};

fn to_wide_slice(
    s: Option<impl AsRef<OsStr>>,
) -> ::std::result::Result<Option<Vec<u16>>, ContainsNul<u16>> {
    if let Some(s) = s {
        Ok(Some(
            WideCString::from_os_str(s).map(|s| s.into_vec_with_nul())?,
        ))
    } else {
        Ok(None)
    }
}

pub(crate) fn to_wide(
    s: Option<impl AsRef<OsStr>>,
) -> ::std::result::Result<Option<WideCString>, ContainsNul<u16>> {
    if let Some(s) = s {
        Ok(Some(WideCString::from_os_str(s)?))
    } else {
        Ok(None)
    }
}

/// Escapes a given string, but also checks it does not contain any null bytes
fn escape_wide(s: impl AsRef<OsStr>) -> ::std::result::Result<WideString, ContainsNul<u16>> {
    let escaped = shell_escape::escape(Cow::Borrowed(s.as_ref()));
    let wide = WideCString::from_os_str(escaped)?;
    Ok(wide.to_ustring())
}
