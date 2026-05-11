//! Idiomatic Rust bindings for illumos `libtopo`.
//!
//! See [`TopoHdl`] for the entry point. Open a handle, then take a
//! [`Snapshot`].

use std::ffi::{CStr, CString, NulError};
use std::os::raw::c_int;

pub use illumos_nvpair::{NvError, NvList, NvValue, OwnedNvList};

use libtopo_sys::{
    FM_FMRI_SCHEME_CPU, FM_FMRI_SCHEME_DEV, FM_FMRI_SCHEME_FMD, FM_FMRI_SCHEME_HC,
    FM_FMRI_SCHEME_LEGACY, FM_FMRI_SCHEME_MEM, FM_FMRI_SCHEME_MOD, FM_FMRI_SCHEME_PATH,
    FM_FMRI_SCHEME_PCIE, FM_FMRI_SCHEME_PKG, FM_FMRI_SCHEME_SVC, FM_FMRI_SCHEME_SW,
    FM_FMRI_SCHEME_ZFS, TOPO_VERSION, topo_close, topo_hdl_strfree, topo_hdl_t, topo_open,
    topo_snap_hold, topo_snap_release, topo_strerror,
};

/// Translate a libtopo error code into an owned message string.
pub(crate) fn topo_errmsg(err: c_int) -> String {
    // SAFETY: topo_strerror takes a c_int and returns either NULL or a
    // pointer to a static NUL-terminated string; both possibilities are
    // handled below.
    let p = unsafe { topo_strerror(err) };
    if p.is_null() {
        format!("unknown libtopo error ({err})")
    } else {
        // SAFETY: p is non-null per the check above and points to a static
        // NUL-terminated string owned by libtopo; we copy it out immediately.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

/// Errors returned by `libtopo` operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `topo_open` returned NULL.
    #[error("failed to open libtopo handle: {0}")]
    Open(String),

    /// A libtopo call returned a non-zero error code.
    #[error("libtopo: {0}")]
    Topo(String),

    /// A Rust string passed to a C API contained an interior NUL byte.
    #[error("interior nul byte in string argument")]
    Nul(#[from] NulError),
}

/// An FMA FMRI scheme — one of the `FM_FMRI_SCHEME_*` constants from
/// `<sys/fm/protocol.h>`. Useful when working with FMRIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    Hc,
    Mem,
    Cpu,
    Dev,
    Mod,
    Svc,
    Sw,
    Zfs,
    Pcie,
    Path,
    Fmd,
    Pkg,
    Legacy,
}

impl Scheme {
    /// The scheme name as a borrowed C string (e.g. `c"hc"` for [`Scheme::Hc`]).
    pub fn as_cstr(self) -> &'static CStr {
        let bytes: &'static [u8] = match self {
            Scheme::Hc => FM_FMRI_SCHEME_HC,
            Scheme::Mem => FM_FMRI_SCHEME_MEM,
            Scheme::Cpu => FM_FMRI_SCHEME_CPU,
            Scheme::Dev => FM_FMRI_SCHEME_DEV,
            Scheme::Mod => FM_FMRI_SCHEME_MOD,
            Scheme::Svc => FM_FMRI_SCHEME_SVC,
            Scheme::Sw => FM_FMRI_SCHEME_SW,
            Scheme::Zfs => FM_FMRI_SCHEME_ZFS,
            Scheme::Pcie => FM_FMRI_SCHEME_PCIE,
            Scheme::Path => FM_FMRI_SCHEME_PATH,
            Scheme::Fmd => FM_FMRI_SCHEME_FMD,
            Scheme::Pkg => FM_FMRI_SCHEME_PKG,
            Scheme::Legacy => FM_FMRI_SCHEME_LEGACY,
        };
        // The bindgen-generated FM_FMRI_SCHEME_* constants are nul-terminated
        // byte arrays sourced from <sys/fm/protocol.h>.
        CStr::from_bytes_with_nul(bytes).expect("FM_FMRI_SCHEME_* lacks NUL terminator")
    }
}

/// A libtopo handle.
///
/// Wraps `topo_hdl_t *`. Create via [`TopoHdl::open`]. Drop closes the
/// handle. `topo_hdl_t` is not thread-safe, so `TopoHdl` is `!Send` and
/// `!Sync` (falls out naturally from the raw pointer).
pub struct TopoHdl {
    hdl: *mut topo_hdl_t,
}

impl TopoHdl {
    /// Open a libtopo handle rooted at `/` (the usual case).
    pub fn open() -> Result<Self, Error> {
        Self::open_with_root(None)
    }

    /// Open a libtopo handle rooted at the given path, or `/` if `None`.
    pub fn open_with_root(root: Option<&str>) -> Result<Self, Error> {
        let root_c = root.map(CString::new).transpose()?;
        let root_ptr = root_c
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(std::ptr::null());

        let mut err: c_int = 0;
        // SAFETY: root_ptr is NULL or borrows from root_c (kept alive across
        // the call); err is a c_int out-param we own. topo_open returns NULL
        // on failure, which we check before constructing TopoHdl.
        let hdl = unsafe { topo_open(TOPO_VERSION as c_int, root_ptr, &mut err) };
        if hdl.is_null() {
            return Err(Error::Open(topo_errmsg(err)));
        }
        Ok(Self { hdl })
    }

    /// Hold a libtopo topology snapshot.
    ///
    /// The snapshot is built in memory by libtopo enumerator plugins and
    /// lives until the returned [`Snapshot`] is dropped. Only one
    /// snapshot is held per handle at a time.
    pub fn snapshot(&self) -> Result<Snapshot<'_>, Error> {
        Snapshot::new(self)
    }
}

impl Drop for TopoHdl {
    fn drop(&mut self) {
        // SAFETY: self.hdl is the valid topo_hdl_t we stored in
        // TopoHdl::open_with_root and have not exposed for double-close.
        unsafe { topo_close(self.hdl) };
    }
}

/// A held libtopo topology snapshot.
///
/// Created by [`TopoHdl::snapshot`]; released on drop.
pub struct Snapshot<'h> {
    hdl: &'h TopoHdl,
    uuid: String,
}

impl<'h> Snapshot<'h> {
    fn new(hdl: &'h TopoHdl) -> Result<Self, Error> {
        let mut err: c_int = 0;
        // SAFETY: hdl.hdl is a valid topo_hdl_t (held by &TopoHdl); NULL
        // as the uuid arg asks libtopo to generate one; err is an owned
        // out-param. Returns NULL on failure, checked below.
        let uuid_c = unsafe { topo_snap_hold(hdl.hdl, std::ptr::null(), &mut err) };
        if uuid_c.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        // SAFETY: uuid_c is non-null per the check above; libtopo owns the
        // memory until topo_hdl_strfree below. We copy out before freeing.
        let uuid = unsafe { CStr::from_ptr(uuid_c) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: uuid_c was allocated by topo_snap_hold against this same
        // handle; topo_hdl_strfree is the documented free path; we have not
        // stored the pointer elsewhere.
        unsafe { topo_hdl_strfree(hdl.hdl, uuid_c) };
        Ok(Self { hdl, uuid })
    }

    /// The snapshot's UUID, as returned by `topo_snap_hold`.
    pub fn uuid(&self) -> &str {
        &self.uuid
    }
}

impl Drop for Snapshot<'_> {
    fn drop(&mut self) {
        // SAFETY: self.hdl.hdl is the same valid topo_hdl_t we held the
        // snapshot against in Snapshot::new; release is paired 1:1 with hold.
        unsafe { topo_snap_release(self.hdl.hdl) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Unit tests (no libtopo runtime needed) ──

    #[test]
    fn scheme_as_cstr() {
        assert_eq!(Scheme::Hc.as_cstr().to_str().unwrap(), "hc");
        assert_eq!(Scheme::Mem.as_cstr().to_str().unwrap(), "mem");
        assert_eq!(Scheme::Cpu.as_cstr().to_str().unwrap(), "cpu");
        assert_eq!(Scheme::Sw.as_cstr().to_str().unwrap(), "sw");
        assert_eq!(Scheme::Legacy.as_cstr().to_str().unwrap(), "legacy-hc");
    }

    #[test]
    fn topo_errmsg_returns_nonempty_string() {
        // Whatever the error code, we should produce a non-empty descriptor.
        for err in [0, 100, -1, 9999] {
            let s = topo_errmsg(err);
            assert!(!s.is_empty(), "topo_errmsg({err}) was empty");
        }
    }

    #[test]
    fn error_display_open() {
        let e = Error::Open("nope".into());
        assert_eq!(e.to_string(), "failed to open libtopo handle: nope");
    }

    #[test]
    fn error_display_topo() {
        let e = Error::Topo("kaboom".into());
        assert_eq!(e.to_string(), "libtopo: kaboom");
    }

    #[test]
    fn error_from_nulerror() {
        // Exercise the #[from] NulError variant.
        let nul_err = CString::new("with\0nul").unwrap_err();
        let e: Error = nul_err.into();
        assert!(matches!(e, Error::Nul(_)));
    }

    // ── Integration tests (require libtopo on the system; via pfexec in CI) ──

    #[test]
    fn open_and_drop() {
        let _hdl = TopoHdl::open().expect("failed to open libtopo handle");
    }

    #[test]
    fn snapshot_and_drop() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        assert!(!snap.uuid().is_empty(), "snapshot UUID should be non-empty");
    }

    #[test]
    fn snapshot_uuid_is_36_chars() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let uuid = snap.uuid();
        assert_eq!(uuid.len(), 36, "expected a 36-char UUID, got: {uuid:?}");
    }

    #[test]
    fn resnapshot_same_handle() {
        let hdl = TopoHdl::open().expect("failed to open");
        let first = hdl.snapshot().expect("first snapshot");
        let first_uuid = first.uuid().to_owned();
        drop(first);
        let second = hdl.snapshot().expect("second snapshot");
        assert_eq!(
            second.uuid().len(),
            36,
            "re-snapshot UUID should also be 36 chars"
        );
        // The two snapshot UUIDs may or may not match — libtopo can return the
        // same UUID for unchanged hardware. We just verify the second succeeds.
        let _ = first_uuid;
    }
}
