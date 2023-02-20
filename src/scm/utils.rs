use std::{borrow::Cow, ffi::OsStr};

use widestring::{error::ContainsNul, WideCString, WideString};

pub(super) fn to_wide(
    s: Option<impl AsRef<OsStr>>,
) -> Result<Option<WideCString>, ContainsNul<u16>> {
    if let Some(s) = s {
        Ok(Some(WideCString::from_os_str(s)?))
    } else {
        Ok(None)
    }
}

pub(super) fn to_wide_slice(
    s: Option<impl AsRef<OsStr>>,
) -> Result<Option<Vec<u16>>, ContainsNul<u16>> {
    if let Some(s) = s {
        Ok(Some(
            WideCString::from_os_str(s).map(|s| s.into_vec_with_nul())?,
        ))
    } else {
        Ok(None)
    }
}

/// Escapes a given string, but also checks it does not contain any null bytes
pub(super) fn escape_wide(s: impl AsRef<OsStr>) -> Result<WideString, ContainsNul<u16>> {
    let escaped = super::shell_escape::escape(Cow::Borrowed(s.as_ref()));
    let wide = WideCString::from_os_str(escaped)?;
    Ok(wide.to_ustring())
}
