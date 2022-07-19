use std::ffi::{OsStr, OsString};
use widestring::{error::ContainsNul, WideCStr, WideCString, WideString};
use windows_sys::core::PWSTR;

/// A helper to join a collection of `OsStr` into a nul-separated `WideString` ending with two nul
/// wide characters.
///
/// Input:
/// vec!["item one", "item two"]
///
/// Output:
/// "item one\0item two\0\0"
///
/// Returns None if the source collection is empty.
pub fn from_vec(source: &[impl AsRef<OsStr>]) -> Result<Option<WideString>, ContainsNul<u16>> {
    if source.is_empty() {
        Ok(None)
    } else {
        let mut wide = WideString::new();
        for s in source {
            let checked_str = WideCString::from_os_str(s)?;
            wide.push_slice(checked_str);
            wide.push_slice(&[0]);
        }
        wide.push_slice(&[0]);
        Ok(Some(wide))
    }
}

/// A helper to split a wide string pointer containing multiple nul-separated substrings, ending
/// with two nul characters into a collection of `OsString`.
///
/// Input:
/// "hello\0world\0\0"
///
/// Output:
/// ["hello", "world"]
pub unsafe fn parse_str_ptr(double_nul_terminated_string: PWSTR) -> Vec<OsString> {
    let mut results: Vec<OsString> = Vec::new();

    if !double_nul_terminated_string.is_null() {
        let mut next = double_nul_terminated_string;
        loop {
            let element = WideCStr::from_ptr_str(next);
            if element.is_empty() {
                break;
            } else {
                results.push(element.to_os_string());
                next = next.add(element.len() + 1);
            }
        }
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
