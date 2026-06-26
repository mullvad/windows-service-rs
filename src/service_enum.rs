use std::{
    alloc::{alloc_zeroed, dealloc, handle_alloc_error, Layout},
    fmt::Debug,
    ops::Deref,
    ptr::NonNull,
};

use widestring::U16CStr;
use windows_sys::Win32::System::Services::{
    ENUM_SERVICE_STATE, ENUM_SERVICE_STATUS_PROCESSW, ENUM_SERVICE_TYPE, SERVICE_ACTIVE,
    SERVICE_DRIVER, SERVICE_FILE_SYSTEM_DRIVER, SERVICE_INACTIVE, SERVICE_KERNEL_DRIVER,
    SERVICE_STATE_ALL, SERVICE_WIN32, SERVICE_WIN32_OWN_PROCESS, SERVICE_WIN32_SHARE_PROCESS,
};

use crate::{service::ServiceStatus, Error};

/* -------------------------------------------------------------------------- */

/// A buffer that contains a list of [`RawEnumServiceStatus`].
pub struct RawEnumServices {
    // INVARIANT(safety): allocated with the global allocated using the layout returned by `EnumServices::layout`
    buffer: NonNull<u8>,
    // INVARIANT(safety): the size in bytes passed to `EnumServices::layout` to allocate `buffer`
    buffer_size: usize,
    service_count: usize,
}

// SAFETY: `RawEnumServices` is like a `Box<[ENUM_SERVICE_STATUS_PROCESSW]>`
unsafe impl Send for RawEnumServices {}
// SAFETY: `RawEnumServices` is like a `Box<[ENUM_SERVICE_STATUS_PROCESSW]>`
unsafe impl Sync for RawEnumServices {}

impl Drop for RawEnumServices {
    fn drop(&mut self) {
        // SAFETY: `buffer` has been allocated via the global allocator with this layout
        unsafe {
            dealloc(self.buffer.as_ptr(), Self::layout(self.buffer_size));
        }
    }
}

impl Clone for RawEnumServices {
    fn clone(&self) -> Self {
        let layout = Self::layout(self.buffer_size);
        // SAFETY: `self.buffer_size` is non-zero
        let Some(new_buffer) = NonNull::new(unsafe { alloc_zeroed(layout) }) else {
            handle_alloc_error(layout);
        };

        // SAFETY: both pointers are valid and cannot overlap as `new_buffer` is a new allocation.
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.buffer.as_ptr(),
                new_buffer.as_ptr(),
                self.buffer_size,
            )
        };

        // SAFETY: `new_buffer` has been initialized with the exact same data as `self.buffer` which itself is valid.
        unsafe { Self::from_parts(new_buffer, self.buffer_size, self.service_count) }
    }
}

impl RawEnumServices {
    pub(crate) fn layout(required_size: usize) -> Layout {
        Layout::from_size_align(required_size, align_of::<ENUM_SERVICE_STATUS_PROCESSW>()).unwrap()
    }

    /// Creates [`EnumServices`] from its components.
    ///
    /// # Safety
    ///
    /// - `buffer` must have been allocated with the global allocator using the [`Layout`] returned by [`Self::layout`]
    /// - `buffer_size` must be the value passed to [`Self::layout`]
    /// - the buffer must successfully have been initialized with `EnumServicesStatusExW`
    /// - `service_count` must be the value returned by `EnumServicesStatusExW` via `lpServicesReturned`
    pub(crate) unsafe fn from_parts(
        buffer: NonNull<u8>,
        buffer_size: usize,
        service_count: usize,
    ) -> Self {
        Self {
            buffer,
            buffer_size,
            service_count,
        }
    }

    /// Returns a slice over the [`RawEnumServiceStatus`]s.
    #[inline]
    pub fn as_slice(&self) -> &[RawEnumServiceStatus] {
        // SAFETY:
        // - `EnumServicesStatusExW` stores a slice of `ENUM_SERVICE_STATUS_PROCESSW` at the beginning of the buffer
        // - `service_count` is the number of service expected in this slice (returned by `ENUM_SERVICE_STATUS_PROCESSW`)
        // - `buffer` is aligned on `ENUM_SERVICE_STATUS_PROCESSW`
        // - `RawEnumServiceStatus` is transparent over `ENUM_SERVICE_STATUS_PROCESSW`
        unsafe {
            core::slice::from_raw_parts(
                self.buffer.as_ptr().cast::<RawEnumServiceStatus>(),
                self.service_count,
            )
        }
    }

    /// Parses the [`RawEnumServiceStatus`]s into [`EnumServiceStatus`]s.
    pub fn to_parsed(&self) -> crate::Result<Vec<EnumServiceStatus>> {
        self.iter().map(RawEnumServiceStatus::to_parsed).collect()
    }
}

impl Deref for RawEnumServices {
    type Target = [RawEnumServiceStatus];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[RawEnumServiceStatus]> for RawEnumServices {
    #[inline]
    fn as_ref(&self) -> &[RawEnumServiceStatus] {
        self.as_slice()
    }
}

impl<'a> IntoIterator for &'a RawEnumServices {
    type Item = &'a RawEnumServiceStatus;
    type IntoIter = core::slice::Iter<'a, RawEnumServiceStatus>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

/* -------------------------------------------------------------------------- */

/// A "raw" representation of a service status returned by [`RawEnumServices`].
#[repr(transparent)]
pub struct RawEnumServiceStatus(ENUM_SERVICE_STATUS_PROCESSW);

impl RawEnumServiceStatus {
    #[inline]
    pub fn service_name(&self) -> &U16CStr {
        unsafe { U16CStr::from_ptr_str(self.0.lpServiceName) }
    }

    #[inline]
    pub fn display_name(&self) -> &U16CStr {
        unsafe { U16CStr::from_ptr_str(self.0.lpDisplayName) }
    }

    pub fn status(&self) -> crate::Result<ServiceStatus> {
        ServiceStatus::from_raw_ex(self.0.ServiceStatusProcess)
            .map_err(|e| Error::ParseValue("service status", e))
    }

    pub fn to_parsed(&self) -> crate::Result<EnumServiceStatus> {
        Ok(EnumServiceStatus {
            name: self.service_name().to_string_lossy(),
            display_name: self.display_name().to_string_lossy(),
            status: self.status()?,
        })
    }
}

impl Debug for RawEnumServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // FIXME(MSRV >= 1.93): could use `fmt::from_fn` instead.
        struct FmtStatus(crate::Result<ServiceStatus>);
        impl Debug for FmtStatus {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match &self.0 {
                    Ok(status) => Debug::fmt(status, f),
                    Err(_) => Debug::fmt(&self.0, f),
                }
            }
        }

        f.debug_struct("ServiceStatus")
            .field("service_name", &self.service_name())
            .field("display_name", &self.display_name())
            .field("status", &FmtStatus(self.status()))
            .finish()
    }
}

/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone)]
pub struct EnumServiceStatus {
    pub name: String,
    pub display_name: String,
    pub status: ServiceStatus,
}

/* -------------------------------------------------------------------------- */

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct EnumServiceType: ENUM_SERVICE_TYPE  {
        const DRIVER = SERVICE_DRIVER;
        const FILE_SYSTEM_DRIVER = SERVICE_FILE_SYSTEM_DRIVER;
        const KERNEL_DRIVER = SERVICE_KERNEL_DRIVER;
        const WIN32 = SERVICE_WIN32;
        const WIN32_OWN_PROCESS = SERVICE_WIN32_OWN_PROCESS;
        const WIN32_SHARE_PROCESS = SERVICE_WIN32_SHARE_PROCESS;
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct EnumServiceState: ENUM_SERVICE_STATE  {
        const ACTIVE = SERVICE_ACTIVE;
        const INACTIVE = SERVICE_INACTIVE;
        const STATE_ALL = SERVICE_STATE_ALL;
    }
}

/* -------------------------------------------------------------------------- */
