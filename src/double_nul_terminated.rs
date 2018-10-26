use std::ffi::{OsStr, OsString};
use widestring::{NulError, WideCStr, WideCString, WideString};
use winapi::shared::ntdef::LPWSTR;

/// A helper to join a collection of `OsStr` into a nul-separated `WideString` ending with two nul
/// bytes.
///
/// Sample output:
/// "item one\0item two\0\0"
///
/// Returns None if the source collection is empty.
pub(crate) fn from_vec<T: AsRef<OsStr>>(
    source: &[T],
) -> ::std::result::Result<Option<WideString>, NulError> {
    if source.len() > 0 {
        let mut wide = WideString::new();
        for s in source {
            let checked_str = WideCString::from_str(s)?;
            wide.push_slice(checked_str);
            wide.push_slice(&[0]);
        }
        wide.push_slice(&[0]);
        Ok(Some(wide))
    } else {
        Ok(None)
    }
}

/// A helper to split a C-string pointer ending with two nul bytes into a collection of `OsString`.
///
/// Input:
/// "hello\0world\0\0"
///
/// Output:
/// ["hello", "world"]
pub(crate) unsafe fn parse_str_ptr(double_nul_terminated_string: LPWSTR) -> Vec<OsString> {
    let mut results: Vec<OsString> = Vec::new();

    if !double_nul_terminated_string.is_null() {
        let mut next = double_nul_terminated_string;
        while {
            let element = WideCStr::from_ptr_str(next);
            if element.is_empty() {
                false
            } else {
                results.push(element.to_os_string());
                next = next.add(element.len() + 1);
                true
            }
        } {}
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nul_byte_string() {
        let mut raw_data: Vec<u16> = vec![0];
        assert!(unsafe { parse_str_ptr(raw_data.as_mut_ptr()) }.is_empty());
    }

    #[test]
    fn test_nul_ptr_string() {
        assert!(unsafe { parse_str_ptr(::std::ptr::null_mut()) }.is_empty());
    }

    #[test]
    fn test_with_values() {
        let mut raw_data = WideString::from_str("Hello\0World\0\0").into_vec();
        assert_eq!(
            unsafe { parse_str_ptr(raw_data.as_mut_ptr()) },
            vec![OsString::from("Hello"), OsString::from("World")]
        );
    }
}
