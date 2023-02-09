use std::ffi::{OsStr, OsString};
use widestring::{error::ContainsNul, WideCStr, WideCString, WideString};
use windows_sys::core::PWSTR;

/// A helper to join a slice of `OsStr`s into a nul-separated `WideString` ending with two nul
/// wide characters.
///
/// Input:
/// &["Hello", "World"]
///
/// Output:
/// "Hello\0World\0\0"
///
/// Returns None if the source collection is empty.
pub fn from_slice(source: &[impl AsRef<OsStr>]) -> Result<Option<WideString>, ContainsNul<u16>> {
    if source.is_empty() {
        Ok(None)
    } else {
        let capacity = source.iter().map(|s| s.as_ref().len() + 1).sum::<usize>() + 1;
        let mut wide = WideString::with_capacity(capacity);
        for s in source {
            let checked_str = WideCString::from_os_str(s)?;
            wide.push_slice(checked_str);
            wide.push_slice([0]);
        }
        wide.push_slice([0]);
        Ok(Some(wide))
    }
}

/// A helper to split a wide string pointer containing multiple nul-separated substrings, ending
/// with two nul characters into a collection of `OsString`.
///
/// Input:
/// "Hello\0World\0\0"
///
/// Output:
/// ["Hello", "World"]
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
    fn test_from_slice() {
        assert_eq!(
            Some(WideString::from_str("Hello\0World\0\0")),
            from_slice(&["Hello", "World"]).unwrap(),
        );
    }

    #[test]
    fn test_from_slice_empty() {
        assert_eq!(None, from_slice(&[] as &[&str]).unwrap());
    }

    #[test]
    fn test_from_slice_with_nul() {
        assert!(from_slice(&["Hello", "\0World"]).is_err());
    }

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
