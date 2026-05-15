//! Idiomatic Rust bindings for illumos `libtopo`.
//!
//! See [`TopoHdl`] for the entry point. Open a handle, take a [`Snapshot`],
//! then [`Snapshot::walk`] to visit the topology.

use std::borrow::Cow;
use std::cell::Cell;
use std::ffi::{CStr, CString, NulError};
use std::marker::PhantomData;
use std::os::raw::{c_char, c_int, c_void};

pub use illumos_nvpair::{NvError, NvList, NvValue, OwnedNvList};

use illumos_nvpair_sys::{
    boolean_t, data_type_t, data_type_t_DATA_TYPE_BOOLEAN_VALUE, data_type_t_DATA_TYPE_DOUBLE,
    data_type_t_DATA_TYPE_INT32, data_type_t_DATA_TYPE_INT32_ARRAY, data_type_t_DATA_TYPE_INT64,
    data_type_t_DATA_TYPE_INT64_ARRAY, data_type_t_DATA_TYPE_NVLIST,
    data_type_t_DATA_TYPE_NVLIST_ARRAY, data_type_t_DATA_TYPE_STRING,
    data_type_t_DATA_TYPE_STRING_ARRAY, data_type_t_DATA_TYPE_UINT32,
    data_type_t_DATA_TYPE_UINT32_ARRAY, data_type_t_DATA_TYPE_UINT64,
    data_type_t_DATA_TYPE_UINT64_ARRAY, nvlist_dup, nvlist_next_nvpair, nvpair_name, nvpair_t,
    nvpair_value_boolean_value, nvpair_value_double, nvpair_value_int32, nvpair_value_int32_array,
    nvpair_value_int64, nvpair_value_int64_array, nvpair_value_nvlist, nvpair_value_nvlist_array,
    nvpair_value_string, nvpair_value_string_array, nvpair_value_uint32, nvpair_value_uint32_array,
    nvpair_value_uint64, nvpair_value_uint64_array, uint_t,
};
use libtopo_sys::{
    FM_FMRI_SCHEME_CPU, FM_FMRI_SCHEME_DEV, FM_FMRI_SCHEME_FMD, FM_FMRI_SCHEME_HC,
    FM_FMRI_SCHEME_LEGACY, FM_FMRI_SCHEME_MEM, FM_FMRI_SCHEME_MOD, FM_FMRI_SCHEME_PATH,
    FM_FMRI_SCHEME_PCIE, FM_FMRI_SCHEME_PKG, FM_FMRI_SCHEME_SVC, FM_FMRI_SCHEME_SW,
    FM_FMRI_SCHEME_ZFS, TOPO_PROP_GROUP, TOPO_PROP_GROUP_DSTAB, TOPO_PROP_GROUP_NAME,
    TOPO_PROP_GROUP_NSTAB, TOPO_PROP_GROUP_VERSION, TOPO_PROP_VAL, TOPO_PROP_VAL_NAME,
    TOPO_PROP_VAL_TYPE, TOPO_PROP_VAL_VAL, TOPO_VERSION, TOPO_WALK_CHILD, TOPO_WALK_ERR,
    TOPO_WALK_NEXT, TOPO_WALK_TERMINATE, tnode_t, topo_close, topo_fmri_expand, topo_fmri_nvl2str,
    topo_fmri_present, topo_fmri_replaced, topo_fmri_str2nvl, topo_fmri_unusable, topo_hdl_strfree,
    topo_hdl_t, topo_instance_t, topo_node_asru, topo_node_fru, topo_node_instance,
    topo_node_label, topo_node_name, topo_node_resource, topo_open, topo_prop_getprop,
    topo_prop_getprops, topo_snap_hold, topo_snap_release, topo_strerror,
    topo_type_t_TOPO_TYPE_BOOLEAN, topo_type_t_TOPO_TYPE_DOUBLE, topo_type_t_TOPO_TYPE_FMRI,
    topo_type_t_TOPO_TYPE_FMRI_ARRAY, topo_type_t_TOPO_TYPE_INT32,
    topo_type_t_TOPO_TYPE_INT32_ARRAY, topo_type_t_TOPO_TYPE_INT64,
    topo_type_t_TOPO_TYPE_INT64_ARRAY, topo_type_t_TOPO_TYPE_SIZE, topo_type_t_TOPO_TYPE_STRING,
    topo_type_t_TOPO_TYPE_STRING_ARRAY, topo_type_t_TOPO_TYPE_TIME, topo_type_t_TOPO_TYPE_UINT32,
    topo_type_t_TOPO_TYPE_UINT32_ARRAY, topo_type_t_TOPO_TYPE_UINT64,
    topo_type_t_TOPO_TYPE_UINT64_ARRAY, topo_walk_fini, topo_walk_init, topo_walk_step,
    topo_walk_t,
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

/// Cast a `*mut illumos_nvpair_sys::nvlist_t` to `*mut libtopo_sys::nvlist_t`.
/// See [`Fmri::as_raw_topo`] for why the cross-sys-crate cast is layout-safe.
#[inline]
fn np_to_topo_nvl(p: *mut illumos_nvpair_sys::nvlist_t) -> *mut libtopo_sys::nvlist_t {
    p.cast()
}

/// Cast a `*mut libtopo_sys::nvlist_t` to `*mut illumos_nvpair_sys::nvlist_t`.
/// See [`Fmri::as_raw_topo`] for why the cross-sys-crate cast is layout-safe.
#[inline]
fn topo_to_np_nvl(p: *mut libtopo_sys::nvlist_t) -> *mut illumos_nvpair_sys::nvlist_t {
    p.cast()
}

/// Best-effort copy of an nvpair's name, used when constructing
/// [`NvError::ValueReadFailed`] from a failed `nvpair_value_*` call.
///
/// # Safety
///
/// `nvp` must be a valid `nvpair_t` pointer from a live nvlist (or null,
/// which produces an empty string).
unsafe fn pair_name_lossy(nvp: *mut nvpair_t) -> String {
    if nvp.is_null() {
        return String::new();
    }
    // SAFETY: nvp is non-null per the check above.
    let p = unsafe { nvpair_name(nvp) };
    if p.is_null() {
        return String::new();
    }
    // SAFETY: p is non-null and points to a NUL-terminated string owned by
    // the nvpair; we copy it out.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Build an `Error::NvPair(NvError::ValueReadFailed{...})` for a failed
/// `nvpair_value_*` call, capturing the pair name, the type we tried to
/// read, and the errno-style return code.
///
/// # Safety
///
/// `nvp` must be valid for [`pair_name_lossy`].
unsafe fn nvp_read_err(nvp: *mut nvpair_t, type_code: data_type_t, errno: i32) -> Error {
    Error::NvPair(NvError::ValueReadFailed {
        // SAFETY: caller upholds nvp validity.
        pair_name: unsafe { pair_name_lossy(nvp) },
        type_code,
        errno,
    })
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

    /// An error reading values from an nvlist (e.g. via [`Fmri::inspect`]).
    #[error(transparent)]
    NvPair(#[from] NvError),

    /// [`TopoHdl::snapshot`] was called a second time on the same handle.
    /// See the doc comment on `snapshot` for the upstream libtopo bug
    /// this guards against (illumos issue 18110).
    #[error(
        "a snapshot has already been taken on this TopoHdl; \
         re-snapshotting the same handle dereferences freed memory \
         in libtopo (illumos issue 18110: \
         https://www.illumos.org/issues/18110). \
         Open a fresh TopoHdl for each snapshot."
    )]
    SnapshotAlreadyTaken,
}

/// An FMA FMRI scheme — one of the `FM_FMRI_SCHEME_*` constants from
/// `<sys/fm/protocol.h>`.
///
/// Each scheme names a different kind of resource: [`Scheme::Hc`] for
/// hardware components, [`Scheme::Cpu`] for CPUs, [`Scheme::Svc`] for
/// SMF services, and so on. Passed to [`Snapshot::walk`] to scope a
/// walk to one scheme.
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
    /// Tracks whether [`Self::snapshot`] has already been called on this
    /// handle. A second call returns [`Error::SnapshotAlreadyTaken`] —
    /// see the doc comment on `snapshot` for the upstream libtopo bug
    /// this guards against.
    snapshot_taken: Cell<bool>,
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
        Ok(Self {
            hdl,
            snapshot_taken: Cell::new(false),
        })
    }

    /// Hold a libtopo topology snapshot.
    ///
    /// The snapshot is built in memory by libtopo enumerator plugins and
    /// lives until the returned [`Snapshot`] is dropped. Only one
    /// snapshot is held per handle at a time.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SnapshotAlreadyTaken`] if called more than once
    /// on the same `TopoHdl`. The supported pattern is to open a fresh
    /// `TopoHdl` for each snapshot you need, mirroring how
    /// `fmd_topo.c::fmd_topo_update()` uses libtopo upstream.
    ///
    /// Calling `snapshot` a second time after the previous `Snapshot`
    /// has been dropped triggers a use-after-free inside libtopo's
    /// `pciebus.so` enumerator: `topo_snap_release` tears down the
    /// snapshot's tnodes via `topo_snap_destroy`'s bottom-up walk, but
    /// `pciebus.so`'s per-handle state (kept alive by `topo_modhash`
    /// until `topo_close`) retains pointers into the freed tnodes. The
    /// next `topo_snap_hold` re-enters PCIe enumeration and dereferences
    /// them in `pgroup_get`. The default umem allocator hands the freed
    /// bytes back intact so the bug is normally silent; under libumem
    /// audit mode it's a deterministic SIGSEGV. See illumos issue
    /// <https://www.illumos.org/issues/18110>.
    pub fn snapshot(&self) -> Result<Snapshot<'_>, Error> {
        if self.snapshot_taken.replace(true) {
            return Err(Error::SnapshotAlreadyTaken);
        }
        Snapshot::new(self)
    }

    /// Render an FMRI as its canonical string form via `topo_fmri_nvl2str`.
    pub fn fmri_to_string(&self, fmri: &Fmri) -> Result<String, Error> {
        let mut out: *mut c_char = std::ptr::null_mut();
        let mut err: c_int = 0;
        // SAFETY: self.hdl is valid; fmri.as_raw_topo() borrows from the live
        // Fmri (see Fmri::as_raw_topo); out and err are owned out-params.
        let rc = unsafe { topo_fmri_nvl2str(self.hdl, fmri.as_raw_topo(), &mut out, &mut err) };
        if rc != 0 || out.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        // SAFETY: out is non-null per the check above and points to a libtopo-
        // allocated NUL-terminated string we copy out before freeing.
        let s = unsafe { CStr::from_ptr(out) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: out was allocated by topo_fmri_nvl2str against this same
        // handle; topo_hdl_strfree is the documented free path.
        unsafe { topo_hdl_strfree(self.hdl, out) };
        Ok(s)
    }

    /// Parse a string FMRI into an [`Fmri`] via `topo_fmri_str2nvl`.
    pub fn fmri_parse(&self, s: &str) -> Result<Fmri, Error> {
        let cstr = CString::new(s)?;
        let mut out: *mut libtopo_sys::nvlist_t = std::ptr::null_mut();
        let mut err: c_int = 0;
        // SAFETY: self.hdl is valid; cstr is alive for the call; out and err
        // are owned out-params.
        let rc = unsafe { topo_fmri_str2nvl(self.hdl, cstr.as_ptr(), &mut out, &mut err) };
        if rc != 0 || out.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        // SAFETY: rc == 0 and out is non-null; libtopo transferred ownership
        // of the nvlist (caller frees via nvlist_free, which OwnedNvList's
        // Drop does). topo_to_np_nvl reinterprets between sys-crate nvlist_t
        // types — see Fmri::as_raw_topo for why this is layout-safe.
        Ok(Fmri(unsafe { OwnedNvList::from_raw(topo_to_np_nvl(out)) }))
    }

    /// Is the resource named by `fmri` present? Via `topo_fmri_present`.
    pub fn fmri_present(&self, fmri: &Fmri) -> Result<bool, Error> {
        self.fmri_bool_query(fmri, topo_fmri_present, "topo_fmri_present")
    }

    /// Has the resource named by `fmri` been replaced? Via `topo_fmri_replaced`.
    pub fn fmri_replaced(&self, fmri: &Fmri) -> Result<bool, Error> {
        self.fmri_bool_query(fmri, topo_fmri_replaced, "topo_fmri_replaced")
    }

    /// Is the resource named by `fmri` unusable? Via `topo_fmri_unusable`.
    pub fn fmri_unusable(&self, fmri: &Fmri) -> Result<bool, Error> {
        self.fmri_bool_query(fmri, topo_fmri_unusable, "topo_fmri_unusable")
    }

    fn fmri_bool_query(
        &self,
        fmri: &Fmri,
        f: unsafe extern "C" fn(*mut topo_hdl_t, *mut libtopo_sys::nvlist_t, *mut c_int) -> c_int,
        name: &'static str,
    ) -> Result<bool, Error> {
        let mut err: c_int = 0;
        // SAFETY: `f` is one of topo_fmri_{present,replaced,unusable}, whose
        // libtopo-sys signature exactly matches this fn pointer type;
        // self.hdl is valid; fmri.as_raw_topo() borrows from the live Fmri.
        let ret = unsafe { f(self.hdl, fmri.as_raw_topo(), &mut err) };
        if err != 0 {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        if ret < 0 {
            return Err(Error::Topo(format!("{name} returned {ret}")));
        }
        Ok(ret != 0)
    }

    /// Expand `fmri` in place via `topo_fmri_expand`, filling in any
    /// authority/scheme details libtopo can resolve.
    pub fn fmri_expand(&self, fmri: &mut Fmri) -> Result<(), Error> {
        let mut err: c_int = 0;
        // SAFETY: self.hdl is valid; fmri.as_raw_topo() yields a live nvlist
        // pointer; topo_fmri_expand mutates the nvlist in place, which is
        // why fmri is &mut.
        let rc = unsafe { topo_fmri_expand(self.hdl, fmri.as_raw_topo(), &mut err) };
        if rc != 0 {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        Ok(())
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
/// Created by [`TopoHdl::snapshot`]; released on drop. Iterate the
/// topology via [`Snapshot::walk`].
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

    /// Walk every node in the given FMRI scheme, invoking `visit` for each.
    ///
    /// The `Node` passed to the closure is valid only for the duration
    /// of that single invocation — this matches libtopo's contract that
    /// node utilities are "callable from topo_walk_step() callback or
    /// module enumeration." Copy out what you need (name, FMRI, etc.)
    /// inside the closure.
    ///
    /// This always uses `TOPO_WALK_CHILD` (depth-first descent through
    /// children); sibling-only walks are not yet exposed.
    pub fn walk<F>(&self, scheme: Scheme, mut visit: F) -> Result<(), Error>
    where
        F: for<'cb> FnMut(Node<'cb>) -> Result<WalkAction, Error>,
    {
        struct Thunk<'a, F: ?Sized> {
            visit: &'a mut F,
            error: Option<Error>,
        }

        unsafe extern "C" fn trampoline<F>(
            hdl: *mut topo_hdl_t,
            tnode: *mut tnode_t,
            arg: *mut c_void,
        ) -> c_int
        where
            F: for<'cb> FnMut(Node<'cb>) -> Result<WalkAction, Error>,
        {
            // SAFETY: `arg` is the `&mut Thunk<F>` we passed to topo_walk_init.
            let thunk = unsafe { &mut *(arg as *mut Thunk<'_, F>) };
            let node = Node {
                hdl,
                tnode,
                _marker: PhantomData,
            };
            match (thunk.visit)(node) {
                Ok(WalkAction::Continue) => TOPO_WALK_NEXT as c_int,
                Ok(WalkAction::Stop) => TOPO_WALK_TERMINATE as c_int,
                Err(e) => {
                    thunk.error = Some(e);
                    TOPO_WALK_ERR as c_int
                }
            }
        }

        let mut thunk: Thunk<'_, F> = Thunk {
            visit: &mut visit,
            error: None,
        };
        let scheme_ptr = scheme.as_cstr().as_ptr();

        let mut err: c_int = 0;
        // SAFETY: self.hdl.hdl is valid; scheme_ptr borrows from the static
        // FM_FMRI_SCHEME_* byte arrays so it lives for 'static; trampoline's
        // signature matches topo_walk_cb_t; &mut thunk lives on this stack
        // frame until after topo_walk_step completes (it's read back below).
        let walker = unsafe {
            topo_walk_init(
                self.hdl.hdl,
                scheme_ptr,
                Some(trampoline::<F>),
                &mut thunk as *mut _ as *mut c_void,
                &mut err,
            )
        };
        if walker.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }

        struct WalkGuard(*mut topo_walk_t);
        impl Drop for WalkGuard {
            fn drop(&mut self) {
                // SAFETY: self.0 is the non-null walker returned by
                // topo_walk_init above, freed exactly once via this Drop.
                unsafe { topo_walk_fini(self.0) };
            }
        }
        let _guard = WalkGuard(walker);

        // SAFETY: walker is non-null (checked above); TOPO_WALK_CHILD is a
        // valid flag constant. The callback (trampoline) is the one we passed
        // to topo_walk_init and stays valid for the duration of this call.
        let status = unsafe { topo_walk_step(walker, TOPO_WALK_CHILD as c_int) };

        if let Some(e) = thunk.error.take() {
            return Err(e);
        }
        // topo_walk_step's documented success values are TOPO_WALK_NEXT
        // (more nodes to visit) and TOPO_WALK_TERMINATE (callback said
        // stop). Anything else — TOPO_WALK_ERR (-1) or an unexpected
        // value — is a failure.
        let next = TOPO_WALK_NEXT as c_int;
        let term = TOPO_WALK_TERMINATE as c_int;
        if status != next && status != term {
            return Err(Error::Topo(format!(
                "topo_walk_step failed: status={status}"
            )));
        }
        Ok(())
    }
}

impl Drop for Snapshot<'_> {
    fn drop(&mut self) {
        // SAFETY: self.hdl.hdl is the same valid topo_hdl_t we held the
        // snapshot against in Snapshot::new; release is paired 1:1 with hold.
        unsafe { topo_snap_release(self.hdl.hdl) };
    }
}

/// Decision returned from a [`Snapshot::walk`] visitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkAction {
    /// Continue to the next node.
    Continue,
    /// Stop the walk; remaining nodes are not visited.
    Stop,
}

/// An FMRI — owned wrapper over a libtopo-produced `nvlist_t`.
///
/// Most operations on FMRIs live on [`TopoHdl`] because the libtopo C
/// functions require a handle. Use [`Fmri::inspect`] to obtain a
/// pure-Rust read-only view via [`NvList`].
#[derive(Debug)]
pub struct Fmri(OwnedNvList);

impl Fmri {
    /// Deep-copy the FMRI contents into an inspectable [`NvList`].
    pub fn inspect(&self) -> Result<NvList, NvError> {
        self.0.inspect()
    }

    /// The raw nvlist pointer cast to libtopo's `nvlist_t` type.
    ///
    /// `libtopo_sys::nvlist_t` and `illumos_nvpair_sys::nvlist_t` are both
    /// bindgen-generated opaque wrappers over the same C `struct nvlist`
    /// from `<sys/nvpair.h>`. They have identical layout; Rust treats them
    /// as nominally distinct only because each `-sys` crate generates its
    /// own opaque type. The pointer cast is a no-op at runtime and
    /// ABI-safe. Helpers [`np_to_topo_nvl`] and [`topo_to_np_nvl`] funnel
    /// every site through a single named conversion; this doc is the
    /// canonical layout rationale.
    fn as_raw_topo(&self) -> *mut libtopo_sys::nvlist_t {
        np_to_topo_nvl(self.0.as_raw())
    }
}

/// A node in the topology, valid only for the duration of one walker callback.
///
/// `Node` is not `Send`/`Sync`. Its lifetime `'cb` is bound to a single
/// invocation of the visitor closure passed to [`Snapshot::walk`]; you
/// cannot store a `Node` past the closure return. Copy out anything you
/// need (name, FMRI, properties) inside the closure.
pub struct Node<'cb> {
    hdl: *mut topo_hdl_t,
    tnode: *mut tnode_t,
    _marker: PhantomData<&'cb ()>,
}

impl<'cb> Node<'cb> {
    /// The node's name (e.g., `"motherboard"`, `"chip"`, `"cpu"`).
    ///
    /// Borrows from libtopo's internal storage for the duration of the
    /// callback invocation. Non-UTF-8 bytes are replaced (matching the
    /// convention of [`Node::label`]); the returned `Cow` is `Borrowed`
    /// for valid UTF-8 and `Owned` otherwise. An empty string indicates
    /// the tnode has no name.
    pub fn name(&self) -> Cow<'cb, str> {
        // SAFETY: self.tnode is valid for 'cb (bound by PhantomData); per
        // the libtopo header, topo_node_name is callable from inside a
        // walker callback. Returns NULL only if the tnode has no name.
        let p = unsafe { topo_node_name(self.tnode) };
        if p.is_null() {
            return Cow::Borrowed("");
        }
        // SAFETY: p is non-null per the check above and points into the
        // tnode's internal storage, valid for as long as the tnode is — 'cb.
        unsafe { CStr::from_ptr(p) }.to_string_lossy()
    }

    /// The node's instance number within its parent group.
    pub fn instance(&self) -> topo_instance_t {
        // SAFETY: self.tnode is valid for 'cb; topo_node_instance reads an
        // integer field from the tnode struct and cannot fail.
        unsafe { topo_node_instance(self.tnode) }
    }

    /// The node's resource FMRI (its identity) via `topo_node_resource`.
    pub fn resource(&self) -> Result<Fmri, Error> {
        let mut out: *mut libtopo_sys::nvlist_t = std::ptr::null_mut();
        let mut err: c_int = 0;
        // SAFETY: self.tnode is valid for 'cb; out and err are owned out-params.
        let rc = unsafe { topo_node_resource(self.tnode, &mut out, &mut err) };
        if rc != 0 || out.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        // SAFETY: rc == 0 and out is non-null; libtopo transferred ownership.
        // topo_to_np_nvl re-types the same pointer — see Fmri::as_raw_topo.
        Ok(Fmri(unsafe { OwnedNvList::from_raw(topo_to_np_nvl(out)) }))
    }

    /// The node's ASRU (Automatic Service Reduction Unit) FMRI via
    /// `topo_node_asru`.
    ///
    /// The `priv_in` argument to libtopo is hardcoded NULL — callers cannot
    /// currently supply a private ASRU-computation context. A future
    /// `asru_with(priv_in)` may add this.
    pub fn asru(&self) -> Result<Fmri, Error> {
        let mut out: *mut libtopo_sys::nvlist_t = std::ptr::null_mut();
        let mut err: c_int = 0;
        // SAFETY: self.tnode is valid for 'cb; out, the NULL priv_in arg,
        // and err are well-formed; rc/out are checked below.
        let rc = unsafe { topo_node_asru(self.tnode, &mut out, std::ptr::null_mut(), &mut err) };
        if rc != 0 || out.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        // SAFETY: rc == 0 and out is non-null; libtopo transferred ownership.
        // topo_to_np_nvl re-types the same pointer — see Fmri::as_raw_topo.
        Ok(Fmri(unsafe { OwnedNvList::from_raw(topo_to_np_nvl(out)) }))
    }

    /// The node's FRU (Field Replaceable Unit) FMRI via `topo_node_fru`.
    ///
    /// The `priv_in` argument to libtopo is hardcoded NULL — callers cannot
    /// currently supply a private FRU-computation context. A future
    /// `fru_with(priv_in)` may add this.
    pub fn fru(&self) -> Result<Fmri, Error> {
        let mut out: *mut libtopo_sys::nvlist_t = std::ptr::null_mut();
        let mut err: c_int = 0;
        // SAFETY: self.tnode is valid for 'cb; out, the NULL priv_in arg,
        // and err are well-formed; rc/out are checked below.
        let rc = unsafe { topo_node_fru(self.tnode, &mut out, std::ptr::null_mut(), &mut err) };
        if rc != 0 || out.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        // SAFETY: rc == 0 and out is non-null; libtopo transferred ownership.
        // topo_to_np_nvl re-types the same pointer — see Fmri::as_raw_topo.
        Ok(Fmri(unsafe { OwnedNvList::from_raw(topo_to_np_nvl(out)) }))
    }

    /// The node's human-readable label (if any) via `topo_node_label`.
    pub fn label(&self) -> Result<String, Error> {
        let mut out: *mut c_char = std::ptr::null_mut();
        let mut err: c_int = 0;
        // SAFETY: self.tnode is valid for 'cb; out and err are owned
        // out-params; rc/out are checked below.
        let rc = unsafe { topo_node_label(self.tnode, &mut out, &mut err) };
        if rc != 0 || out.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        // SAFETY: out is non-null per the check above and points to a
        // libtopo-allocated NUL-terminated string we copy out before freeing.
        let s = unsafe { CStr::from_ptr(out) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: out was allocated by topo_node_label against this handle;
        // topo_hdl_strfree is the documented free path.
        unsafe { topo_hdl_strfree(self.hdl, out) };
        Ok(s)
    }

    /// Read one property by group and name.
    ///
    /// The returned [`PropValue`] reflects libtopo's `TOPO_TYPE_*` for the
    /// property; callers pattern-match on the variant rather than asking
    /// for a specific type up front. Properties of types this crate does
    /// not yet model (e.g. `TOPO_TYPE_FMRI_ARRAY`) appear as
    /// [`PropValue::Unknown`].
    pub fn property(&self, group: &str, name: &str) -> Result<PropValue, Error> {
        let g = CString::new(group)?;
        let n = CString::new(name)?;
        let mut out: *mut libtopo_sys::nvlist_t = std::ptr::null_mut();
        let mut err: c_int = 0;
        // SAFETY: self.tnode is valid for 'cb; g and n are alive for the
        // call; out and err are owned out-params; args=NULL.
        let rc = unsafe {
            topo_prop_getprop(
                self.tnode,
                g.as_ptr(),
                n.as_ptr(),
                std::ptr::null_mut(),
                &mut out,
                &mut err,
            )
        };
        if rc != 0 || out.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        // SAFETY: rc == 0 and out is non-null; libtopo transferred ownership
        // of the nvlist. topo_to_np_nvl re-types the same pointer — see
        // Fmri::as_raw_topo.
        let owned = unsafe { OwnedNvList::from_raw(topo_to_np_nvl(out)) };
        // SAFETY: owned holds the only reference to the property nvlist for
        // the duration of parse_property; parse_property reads but does not
        // free.
        let prop = unsafe { parse_property(owned.as_raw()) }?;
        Ok(prop.value)
    }

    /// Enumerate every property group and property on this node.
    pub fn property_groups(&self) -> Result<Vec<PropertyGroup>, Error> {
        let mut err: c_int = 0;
        // SAFETY: self.tnode is valid for 'cb; err is an owned out-param;
        // topo_prop_getprops returns NULL on error, which is checked below.
        let raw = unsafe { topo_prop_getprops(self.tnode, &mut err) };
        if raw.is_null() {
            return Err(Error::Topo(topo_errmsg(err)));
        }
        // SAFETY: raw is non-null; libtopo transferred ownership. topo_to_np_nvl
        // re-types the same pointer — see Fmri::as_raw_topo.
        let owned = unsafe { OwnedNvList::from_raw(topo_to_np_nvl(raw)) };
        let mut groups = Vec::new();
        let mut nvp: *mut nvpair_t = std::ptr::null_mut();
        loop {
            // SAFETY: owned holds the outer nvlist; nvp is either null (first
            // iteration) or a pointer returned by the previous call.
            nvp = unsafe { nvlist_next_nvpair(owned.as_raw(), nvp) };
            if nvp.is_null() {
                break;
            }
            // SAFETY: nvp is non-null per the check above.
            let name_ptr = unsafe { nvpair_name(nvp) };
            if name_ptr.is_null() {
                continue;
            }
            // SAFETY: name_ptr is non-null per the check above and points to
            // a NUL-terminated string owned by the nvpair.
            let key = unsafe { CStr::from_ptr(name_ptr) }.to_bytes_with_nul();
            if key != TOPO_PROP_GROUP {
                continue;
            }
            let mut group_nvl: *mut illumos_nvpair_sys::nvlist_t = std::ptr::null_mut();
            // SAFETY: nvp is valid and is the property-group nvlist entry;
            // group_nvl receives the nested nvlist pointer (aliasing outer).
            if unsafe { nvpair_value_nvlist(nvp, &mut group_nvl) } != 0 {
                continue;
            }
            // SAFETY: group_nvl is a valid nested nvlist within owned.
            groups.push(unsafe { parse_property_group(group_nvl) }?);
        }
        Ok(groups)
    }
}

/// A property value, decoded from libtopo's `TOPO_TYPE_*` codes.
///
/// Returned by [`Node::property`] and as part of [`Property`] entries in
/// [`Node::property_groups`]. Callers pattern-match on the variant to
/// access the typed value.
#[derive(Debug)]
pub enum PropValue {
    Boolean(bool),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Double(f64),
    String(String),
    Fmri(Fmri),
    /// A `TOPO_TYPE_TIME` value — an int64 timestamp at the nvpair layer.
    Time(i64),
    /// A `TOPO_TYPE_SIZE` value — a uint64 size in bytes at the nvpair layer.
    Size(u64),
    Int32Array(Vec<i32>),
    UInt32Array(Vec<u32>),
    Int64Array(Vec<i64>),
    UInt64Array(Vec<u64>),
    StringArray(Vec<String>),
    /// A `TOPO_TYPE_FMRI_ARRAY` value — each entry is deep-copied into an
    /// independently-owned [`Fmri`].
    FmriArray(Vec<Fmri>),
    /// An unknown `TOPO_TYPE_*` code.
    Unknown {
        type_code: u32,
    },
}

/// A single property within a [`PropertyGroup`].
#[derive(Debug)]
pub struct Property {
    pub name: String,
    pub value: PropValue,
}

/// A property group on a node, returned by [`Node::property_groups`].
#[derive(Debug)]
pub struct PropertyGroup {
    pub name: String,
    /// libtopo's per-group name-stability label (e.g. `"Private"`, `"Stable"`).
    /// `None` if the underlying nvlist entry was absent or unreadable.
    pub name_stability: Option<String>,
    /// libtopo's per-group data-stability label. `None` if the underlying
    /// nvlist entry was absent or unreadable.
    pub data_stability: Option<String>,
    /// libtopo's per-group version. `None` if the underlying nvlist entry
    /// was absent or unreadable.
    pub version: Option<i32>,
    pub properties: Vec<Property>,
}

/// Property nvlists from libtopo are allocated without `NV_UNIQUE_NAME`,
/// so the typed `nvlist_lookup_*` accessors don't work on them — they
/// return `ENOTSUP`. We iterate via `nvlist_next_nvpair` and extract values
/// directly with `nvpair_value_*` instead.
///
/// On a `nvpair_value_*` failure (e.g. requested type does not match the
/// pair's actual type, yielding `ENOTSUP`), the helpers return
/// `Error::NvPair(NvError::ValueReadFailed { pair_name, type_code, errno })`
/// with the requested `data_type_t` and the raw return code preserved.
///
/// # Safety
///
/// `nvp` must be a valid `nvpair_t` pointer from a live nvlist.
unsafe fn nvp_string(nvp: *mut nvpair_t) -> Result<String, Error> {
    let mut out: *mut c_char = std::ptr::null_mut();
    // SAFETY: nvp is a valid nvpair_t; out is an owned local.
    let rc = unsafe { nvpair_value_string(nvp, &mut out) };
    if rc != 0 || out.is_null() {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_STRING, rc) });
    }
    // SAFETY: out is non-null and points to a NUL-terminated string owned by
    // the nvlist; we copy it out before any potential free.
    Ok(unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned())
}

unsafe fn nvp_int32(nvp: *mut nvpair_t) -> Result<i32, Error> {
    let mut v: i32 = 0;
    // SAFETY: nvp is a valid nvpair_t; v is an owned local.
    let rc = unsafe { nvpair_value_int32(nvp, &mut v) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_INT32, rc) });
    }
    Ok(v)
}

unsafe fn nvp_uint32(nvp: *mut nvpair_t) -> Result<u32, Error> {
    let mut v: u32 = 0;
    // SAFETY: nvp is a valid nvpair_t; v is an owned local.
    let rc = unsafe { nvpair_value_uint32(nvp, &mut v) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_UINT32, rc) });
    }
    Ok(v)
}

unsafe fn nvp_int64(nvp: *mut nvpair_t) -> Result<i64, Error> {
    let mut v: i64 = 0;
    // SAFETY: nvp is a valid nvpair_t; v is an owned local.
    let rc = unsafe { nvpair_value_int64(nvp, &mut v) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_INT64, rc) });
    }
    Ok(v)
}

unsafe fn nvp_uint64(nvp: *mut nvpair_t) -> Result<u64, Error> {
    let mut v: u64 = 0;
    // SAFETY: nvp is a valid nvpair_t; v is an owned local.
    let rc = unsafe { nvpair_value_uint64(nvp, &mut v) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_UINT64, rc) });
    }
    Ok(v)
}

unsafe fn nvp_double(nvp: *mut nvpair_t) -> Result<f64, Error> {
    let mut v: f64 = 0.0;
    // SAFETY: nvp is a valid nvpair_t; v is an owned local.
    let rc = unsafe { nvpair_value_double(nvp, &mut v) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_DOUBLE, rc) });
    }
    Ok(v)
}

unsafe fn nvp_boolean(nvp: *mut nvpair_t) -> Result<bool, Error> {
    let mut v: boolean_t = 0;
    // SAFETY: nvp is a valid nvpair_t; v is an owned local.
    let rc = unsafe { nvpair_value_boolean_value(nvp, &mut v) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_BOOLEAN_VALUE, rc) });
    }
    Ok(v != 0)
}

unsafe fn nvp_int32_array(nvp: *mut nvpair_t) -> Result<Vec<i32>, Error> {
    let mut ptr: *mut i32 = std::ptr::null_mut();
    let mut n: uint_t = 0;
    // SAFETY: nvp is a valid nvpair_t; ptr/n are owned locals.
    let rc = unsafe { nvpair_value_int32_array(nvp, &mut ptr, &mut n) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_INT32_ARRAY, rc) });
    }
    if n == 0 || ptr.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: ptr points to n consecutive i32 values owned by the nvlist.
    Ok(unsafe { std::slice::from_raw_parts(ptr, n as usize) }.to_vec())
}

unsafe fn nvp_uint32_array(nvp: *mut nvpair_t) -> Result<Vec<u32>, Error> {
    let mut ptr: *mut u32 = std::ptr::null_mut();
    let mut n: uint_t = 0;
    // SAFETY: nvp is a valid nvpair_t; ptr/n are owned locals.
    let rc = unsafe { nvpair_value_uint32_array(nvp, &mut ptr, &mut n) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_UINT32_ARRAY, rc) });
    }
    if n == 0 || ptr.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: ptr points to n consecutive u32 values owned by the nvlist.
    Ok(unsafe { std::slice::from_raw_parts(ptr, n as usize) }.to_vec())
}

unsafe fn nvp_int64_array(nvp: *mut nvpair_t) -> Result<Vec<i64>, Error> {
    let mut ptr: *mut i64 = std::ptr::null_mut();
    let mut n: uint_t = 0;
    // SAFETY: nvp is a valid nvpair_t; ptr/n are owned locals.
    let rc = unsafe { nvpair_value_int64_array(nvp, &mut ptr, &mut n) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_INT64_ARRAY, rc) });
    }
    if n == 0 || ptr.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: ptr points to n consecutive i64 values owned by the nvlist.
    Ok(unsafe { std::slice::from_raw_parts(ptr, n as usize) }.to_vec())
}

unsafe fn nvp_uint64_array(nvp: *mut nvpair_t) -> Result<Vec<u64>, Error> {
    let mut ptr: *mut u64 = std::ptr::null_mut();
    let mut n: uint_t = 0;
    // SAFETY: nvp is a valid nvpair_t; ptr/n are owned locals.
    let rc = unsafe { nvpair_value_uint64_array(nvp, &mut ptr, &mut n) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_UINT64_ARRAY, rc) });
    }
    if n == 0 || ptr.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: ptr points to n consecutive u64 values owned by the nvlist.
    Ok(unsafe { std::slice::from_raw_parts(ptr, n as usize) }.to_vec())
}

unsafe fn nvp_string_array(nvp: *mut nvpair_t) -> Result<Vec<String>, Error> {
    let mut ptr: *mut *mut c_char = std::ptr::null_mut();
    let mut n: uint_t = 0;
    // SAFETY: nvp is a valid nvpair_t; ptr/n are owned locals.
    let rc = unsafe { nvpair_value_string_array(nvp, &mut ptr, &mut n) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_STRING_ARRAY, rc) });
    }
    if n == 0 || ptr.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: ptr points to n consecutive *mut c_char each NUL-terminated.
    let slice = unsafe { std::slice::from_raw_parts(ptr, n as usize) };
    let mut out = Vec::with_capacity(slice.len());
    for &p in slice {
        if p.is_null() {
            return Err(Error::Topo("null entry in string array".into()));
        }
        // SAFETY: p is non-null and NUL-terminated.
        out.push(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned());
    }
    Ok(out)
}

/// Take an FMRI-typed nvpair value (a nested nvlist) and detach it by
/// duplicating into an independent allocation that the returned [`Fmri`]
/// owns.
///
/// If `nvp` isn't actually nvlist-typed, `nvpair_value_nvlist` returns
/// `ENOTSUP` and surfaces as `Error::NvPair(NvError::ValueReadFailed)`.
/// An `nvlist_dup` failure surfaces as `Error::Topo` since it isn't an
/// nvpair-read error.
///
/// # Safety
///
/// `nvp` must be a valid `nvpair_t` pointer from a live nvlist.
unsafe fn nvp_fmri(nvp: *mut nvpair_t) -> Result<Fmri, Error> {
    let mut inner: *mut illumos_nvpair_sys::nvlist_t = std::ptr::null_mut();
    // SAFETY: nvp is valid; inner receives a pointer aliasing the nvlist.
    let rc = unsafe { nvpair_value_nvlist(nvp, &mut inner) };
    if rc != 0 || inner.is_null() {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_NVLIST, rc) });
    }
    let mut copy: *mut illumos_nvpair_sys::nvlist_t = std::ptr::null_mut();
    // SAFETY: inner is a valid nvlist; nvlist_dup makes an independent copy.
    let rc = unsafe { nvlist_dup(inner, &mut copy, 0) };
    if rc != 0 || copy.is_null() {
        return Err(Error::Topo(format!("nvlist_dup failed for FMRI: {rc}")));
    }
    // SAFETY: copy is independently allocated; ownership transfers to
    // OwnedNvList::Drop -> nvlist_free.
    Ok(Fmri(unsafe { OwnedNvList::from_raw(copy) }))
}

/// Take an FMRI-array-typed nvpair value (a nested array of nvlists) and
/// detach it by duplicating each element into an independent [`Fmri`].
///
/// If `nvp` isn't actually an nvlist-array, `nvpair_value_nvlist_array`
/// returns `ENOTSUP` and surfaces as `Error::NvPair(NvError::ValueReadFailed)`.
/// A per-element `nvlist_dup` failure surfaces as `Error::Topo`.
///
/// # Safety
///
/// `nvp` must be a valid `nvpair_t` pointer from a live nvlist.
unsafe fn nvp_fmri_array(nvp: *mut nvpair_t) -> Result<Vec<Fmri>, Error> {
    let mut ptr: *mut *mut illumos_nvpair_sys::nvlist_t = std::ptr::null_mut();
    let mut n: uint_t = 0;
    // SAFETY: nvp is a valid nvpair_t; ptr/n are owned locals.
    let rc = unsafe { nvpair_value_nvlist_array(nvp, &mut ptr, &mut n) };
    if rc != 0 {
        // SAFETY: nvp is valid (caller-upheld).
        return Err(unsafe { nvp_read_err(nvp, data_type_t_DATA_TYPE_NVLIST_ARRAY, rc) });
    }
    if n == 0 || ptr.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: ptr points to n consecutive *mut nvlist_t each aliasing nvp's
    // source nvlist; the slice's contents stay valid while nvp is alive.
    let slice = unsafe { std::slice::from_raw_parts(ptr, n as usize) };
    let mut out = Vec::with_capacity(slice.len());
    for &inner in slice {
        if inner.is_null() {
            return Err(Error::Topo("null nvlist in FMRI array".into()));
        }
        let mut copy: *mut illumos_nvpair_sys::nvlist_t = std::ptr::null_mut();
        // SAFETY: inner is a valid nvlist; nvlist_dup makes an independent copy.
        let dup_rc = unsafe { nvlist_dup(inner, &mut copy, 0) };
        if dup_rc != 0 || copy.is_null() {
            return Err(Error::Topo(format!(
                "nvlist_dup failed in FMRI array: {dup_rc}"
            )));
        }
        // SAFETY: copy is independently allocated; ownership transfers to
        // OwnedNvList::Drop -> nvlist_free.
        out.push(Fmri(unsafe { OwnedNvList::from_raw(copy) }));
    }
    Ok(out)
}

/// Decode the `TOPO_PROP_VAL_VAL` nvpair using the supplied type code into a
/// [`PropValue`].
///
/// If `type_code` and `value_nvp`'s actual nvpair type disagree, the
/// underlying `nvpair_value_*` call returns `ENOTSUP` and we propagate
/// that as `Err` — no UB.
///
/// # Safety
///
/// `value_nvp` must be a valid nvpair_t.
unsafe fn decode_prop_value(type_code: u32, value_nvp: *mut nvpair_t) -> Result<PropValue, Error> {
    // Local uppercase aliases for the bindgen-generated snake_case type-code
    // constants — Rust's `non_upper_case_globals` lint forbids snake_case
    // names in match-arm pattern position.
    const BOOLEAN: u32 = topo_type_t_TOPO_TYPE_BOOLEAN;
    const INT32: u32 = topo_type_t_TOPO_TYPE_INT32;
    const UINT32: u32 = topo_type_t_TOPO_TYPE_UINT32;
    const INT64: u32 = topo_type_t_TOPO_TYPE_INT64;
    const UINT64: u32 = topo_type_t_TOPO_TYPE_UINT64;
    const DOUBLE: u32 = topo_type_t_TOPO_TYPE_DOUBLE;
    const STRING: u32 = topo_type_t_TOPO_TYPE_STRING;
    const FMRI: u32 = topo_type_t_TOPO_TYPE_FMRI;
    const TIME: u32 = topo_type_t_TOPO_TYPE_TIME;
    const SIZE: u32 = topo_type_t_TOPO_TYPE_SIZE;
    const INT32_ARRAY: u32 = topo_type_t_TOPO_TYPE_INT32_ARRAY;
    const UINT32_ARRAY: u32 = topo_type_t_TOPO_TYPE_UINT32_ARRAY;
    const INT64_ARRAY: u32 = topo_type_t_TOPO_TYPE_INT64_ARRAY;
    const UINT64_ARRAY: u32 = topo_type_t_TOPO_TYPE_UINT64_ARRAY;
    const STRING_ARRAY: u32 = topo_type_t_TOPO_TYPE_STRING_ARRAY;
    const FMRI_ARRAY: u32 = topo_type_t_TOPO_TYPE_FMRI_ARRAY;

    // SAFETY for every branch: value_nvp is valid. A type mismatch returns
    // ENOTSUP from libnvpair, not UB.
    Ok(match type_code {
        BOOLEAN => PropValue::Boolean(unsafe { nvp_boolean(value_nvp) }?),
        INT32 => PropValue::Int32(unsafe { nvp_int32(value_nvp) }?),
        UINT32 => PropValue::UInt32(unsafe { nvp_uint32(value_nvp) }?),
        INT64 => PropValue::Int64(unsafe { nvp_int64(value_nvp) }?),
        UINT64 => PropValue::UInt64(unsafe { nvp_uint64(value_nvp) }?),
        DOUBLE => PropValue::Double(unsafe { nvp_double(value_nvp) }?),
        STRING => PropValue::String(unsafe { nvp_string(value_nvp) }?),
        FMRI => PropValue::Fmri(unsafe { nvp_fmri(value_nvp) }?),
        TIME => PropValue::Time(unsafe { nvp_int64(value_nvp) }?),
        SIZE => PropValue::Size(unsafe { nvp_uint64(value_nvp) }?),
        INT32_ARRAY => PropValue::Int32Array(unsafe { nvp_int32_array(value_nvp) }?),
        UINT32_ARRAY => PropValue::UInt32Array(unsafe { nvp_uint32_array(value_nvp) }?),
        INT64_ARRAY => PropValue::Int64Array(unsafe { nvp_int64_array(value_nvp) }?),
        UINT64_ARRAY => PropValue::UInt64Array(unsafe { nvp_uint64_array(value_nvp) }?),
        STRING_ARRAY => PropValue::StringArray(unsafe { nvp_string_array(value_nvp) }?),
        FMRI_ARRAY => PropValue::FmriArray(unsafe { nvp_fmri_array(value_nvp) }?),
        _ => PropValue::Unknown { type_code },
    })
}

/// Parse a single-property nvlist into a [`Property`].
///
/// libtopo property nvlists carry three entries: `TOPO_PROP_VAL_NAME`
/// (string), `TOPO_PROP_VAL_TYPE` (uint32), and `TOPO_PROP_VAL_VAL`
/// (typed value). Any missing entry results in an `Err` return; the
/// function is sound regardless.
///
/// # Safety
///
/// `prop_nvl` must point to a valid nvlist.
unsafe fn parse_property(prop_nvl: *mut illumos_nvpair_sys::nvlist_t) -> Result<Property, Error> {
    let mut name: Option<String> = None;
    let mut type_code: Option<u32> = None;
    let mut value_nvp: *mut nvpair_t = std::ptr::null_mut();

    let mut nvp: *mut nvpair_t = std::ptr::null_mut();
    loop {
        // SAFETY: prop_nvl is alive; nvp is null (first iter) or a pair
        // returned by the previous call.
        nvp = unsafe { nvlist_next_nvpair(prop_nvl, nvp) };
        if nvp.is_null() {
            break;
        }
        // SAFETY: nvp is non-null per the check above.
        let name_ptr = unsafe { nvpair_name(nvp) };
        if name_ptr.is_null() {
            continue;
        }
        // SAFETY: name_ptr is non-null and NUL-terminated.
        let key = unsafe { CStr::from_ptr(name_ptr) }.to_bytes_with_nul();
        if key == TOPO_PROP_VAL_NAME {
            name = Some(unsafe { nvp_string(nvp) }?);
        } else if key == TOPO_PROP_VAL_TYPE {
            type_code = Some(unsafe { nvp_uint32(nvp) }?);
        } else if key == TOPO_PROP_VAL_VAL {
            value_nvp = nvp;
        }
    }

    let name = name.ok_or_else(|| Error::Topo("property missing name".into()))?;
    let type_code = type_code.ok_or_else(|| Error::Topo("property missing type".into()))?;
    if value_nvp.is_null() {
        return Err(Error::Topo(format!("property {name:?} missing value")));
    }
    // SAFETY: value_nvp is a valid nvpair_t whose dynamic type is described by
    // type_code (sourced from the same property nvlist).
    let value = unsafe { decode_prop_value(type_code, value_nvp) }?;
    Ok(Property { name, value })
}

/// Parse a property-group nvlist into a [`PropertyGroup`].
///
/// Expected entries (string) `TOPO_PROP_GROUP_NAME`, (string)
/// `TOPO_PROP_GROUP_NSTAB`, (string) `TOPO_PROP_GROUP_DSTAB`, (int32)
/// `TOPO_PROP_GROUP_VERSION`, plus zero or more (nvlist) `TOPO_PROP_VAL`
/// entries. If the group name is missing, returns `Err`; the stability
/// strings and version come back as `None` if absent or unreadable.
/// The function is sound regardless of which entries are present.
///
/// # Safety
///
/// `group_nvl` must point to a valid nvlist.
unsafe fn parse_property_group(
    group_nvl: *mut illumos_nvpair_sys::nvlist_t,
) -> Result<PropertyGroup, Error> {
    let mut name: Option<String> = None;
    let mut name_stability: Option<String> = None;
    let mut data_stability: Option<String> = None;
    let mut version: Option<i32> = None;
    let mut properties = Vec::new();

    let mut nvp: *mut nvpair_t = std::ptr::null_mut();
    loop {
        // SAFETY: group_nvl is alive; nvp is null (first iter) or a pair
        // returned by the previous call.
        nvp = unsafe { nvlist_next_nvpair(group_nvl, nvp) };
        if nvp.is_null() {
            break;
        }
        // SAFETY: nvp is non-null per the check above.
        let name_ptr = unsafe { nvpair_name(nvp) };
        if name_ptr.is_null() {
            continue;
        }
        // SAFETY: name_ptr is non-null and NUL-terminated.
        let key = unsafe { CStr::from_ptr(name_ptr) }.to_bytes_with_nul();
        if key == TOPO_PROP_GROUP_NAME {
            name = Some(unsafe { nvp_string(nvp) }?);
        } else if key == TOPO_PROP_GROUP_NSTAB {
            name_stability = unsafe { nvp_string(nvp) }.ok();
        } else if key == TOPO_PROP_GROUP_DSTAB {
            data_stability = unsafe { nvp_string(nvp) }.ok();
        } else if key == TOPO_PROP_GROUP_VERSION {
            version = unsafe { nvp_int32(nvp) }.ok();
        } else if key == TOPO_PROP_VAL {
            let mut prop_nvl: *mut illumos_nvpair_sys::nvlist_t = std::ptr::null_mut();
            // SAFETY: nvp is valid and of nvlist type; prop_nvl receives an
            // aliasing pointer into group_nvl.
            if unsafe { nvpair_value_nvlist(nvp, &mut prop_nvl) } != 0 {
                continue;
            }
            // SAFETY: prop_nvl is a valid property nvlist nested within group_nvl.
            properties.push(unsafe { parse_property(prop_nvl) }?);
        }
    }

    let name = name.ok_or_else(|| Error::Topo("property group missing name".into()))?;
    Ok(PropertyGroup {
        name,
        name_stability,
        data_stability,
        version,
        properties,
    })
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
        let nul_err = CString::new("with\0nul").unwrap_err();
        let e: Error = nul_err.into();
        assert!(matches!(e, Error::Nul(_)));
    }

    #[test]
    fn error_from_nverror() {
        let nv_err = NvError::NullName;
        let e: Error = nv_err.into();
        assert!(matches!(e, Error::NvPair(_)));
    }

    #[test]
    fn walk_action_eq() {
        assert_eq!(WalkAction::Continue, WalkAction::Continue);
        assert_ne!(WalkAction::Continue, WalkAction::Stop);
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

    // When illumos issue 18110 is fixed upstream, remove the
    // `#[ignore]` from this test and delete `resnapshot_same_handle_panics`
    // below. The wrapper's panic guard in `TopoHdl::snapshot` should
    // also be removed at that point so this path actually works.
    //
    // Tracking: https://www.illumos.org/issues/18110
    #[test]
    #[ignore = "panics due to libtopo bug; re-enable when illumos issue 18110 is fixed"]
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
        // The two snapshot UUIDs may or may not match — libtopo can return
        // the same UUID for unchanged hardware. We just verify the second
        // succeeds against the same handle after the first was dropped.
        let _ = first_uuid;
    }

    // Companion to `resnapshot_same_handle`: pins down the wrapper's
    // current behavior of returning `Error::SnapshotAlreadyTaken` on
    // a second `snapshot()` call, until illumos issue 18110 is fixed
    // and the guard is removed.
    //
    // Tracking: https://www.illumos.org/issues/18110
    #[test]
    fn resnapshot_same_handle_returns_error() {
        let hdl = TopoHdl::open().expect("failed to open");
        let first = hdl.snapshot().expect("first snapshot");
        drop(first);
        let second = hdl.snapshot();
        assert!(
            matches!(second, Err(Error::SnapshotAlreadyTaken)),
            "expected Err(SnapshotAlreadyTaken) on second snapshot"
        );
    }

    /// Whether `err` reports an empty topology (e.g. a CI VM with no real
    /// hardware). Tests treat this as "skip" so they pass on hardware hosts
    /// and on hardware-less CI images alike.
    fn is_empty_topology(err: &Error) -> bool {
        matches!(err, Error::Topo(msg) if msg.contains("empty topology"))
    }

    /// Walk the hc tree and capture the first node's resource FMRI, or
    /// return `Ok(None)` if the topology is empty.
    fn first_hc_resource(snap: &Snapshot<'_>) -> Result<Option<Fmri>, Error> {
        let mut captured: Option<Fmri> = None;
        match snap.walk(Scheme::Hc, |node| {
            captured = Some(node.resource()?);
            Ok(WalkAction::Stop)
        }) {
            Ok(()) => Ok(captured),
            Err(e) if is_empty_topology(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    #[test]
    fn walk_hc_counts_at_least_one_node() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let mut count = 0;
        match snap.walk(Scheme::Hc, |_node| {
            count += 1;
            Ok(WalkAction::Continue)
        }) {
            Ok(()) => {
                assert!(
                    count >= 1,
                    "expected to walk at least one node, got {count}"
                );
            }
            Err(e) if is_empty_topology(&e) => {
                eprintln!("skipping: empty hc topology (no hardware?)");
            }
            Err(e) => panic!("walk failed: {e}"),
        }
    }

    #[test]
    fn walk_stop_terminates_early() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let mut total = 0;
        match snap.walk(Scheme::Hc, |_node| {
            total += 1;
            Ok(WalkAction::Continue)
        }) {
            Ok(()) => {}
            Err(e) if is_empty_topology(&e) => {
                eprintln!("skipping: empty hc topology");
                return;
            }
            Err(e) => panic!("walk failed: {e}"),
        }
        assert!(total >= 1, "expected at least one node, got {total}");
        if total < 2 {
            eprintln!("skipping: fewer than 2 nodes; can't compare Stop vs Continue");
            return;
        }
        let mut stopped = 0;
        snap.walk(Scheme::Hc, |_node| {
            stopped += 1;
            Ok(WalkAction::Stop)
        })
        .expect("walk failed");
        assert_eq!(stopped, 1, "Stop should terminate after the first node");
        assert!(
            total > stopped,
            "early-stopped walk should visit fewer nodes"
        );
    }

    #[test]
    fn walk_propagates_closure_error() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        // Use a Nul sentinel rather than Topo so we exercise that a non-Topo
        // error variant flows through unchanged.
        let nul_err = CString::new("with\0nul").unwrap_err();
        let mut closure_called = false;
        let result = snap.walk(Scheme::Hc, |_node| {
            closure_called = true;
            Err(Error::Nul(nul_err.clone()))
        });
        match result {
            Err(Error::Nul(_)) => assert!(closure_called),
            Err(ref e) if is_empty_topology(e) => {
                eprintln!("skipping: empty hc topology");
            }
            Ok(()) if !closure_called => {
                eprintln!("skipping: walk visited no nodes");
            }
            other => panic!("expected Nul sentinel error, got {other:?}"),
        }
    }

    #[test]
    fn walk_node_resource_and_format() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let Some(fmri) = first_hc_resource(&snap).expect("walk failed") else {
            eprintln!("skipping: empty hc topology");
            return;
        };
        let s = hdl.fmri_to_string(&fmri).expect("fmri_to_string failed");
        assert!(!s.is_empty(), "FMRI string should not be empty");
        assert!(s.contains(':'), "FMRI should contain ':'; got {s:?}");
    }

    #[test]
    fn fmri_roundtrip_through_strings() {
        // Hardware-independent: parse a known FMRI and format it back.
        // topo_fmri_str2nvl / nvl2str are pure parsers, no topology needed.
        let hdl = TopoHdl::open().expect("failed to open");
        let _snap = hdl.snapshot().expect("failed to take snapshot");
        let original = "hc:///chassis=0";
        let parsed = hdl.fmri_parse(original).expect("fmri_parse failed");
        let roundtripped = hdl.fmri_to_string(&parsed).expect("fmri_to_string failed");
        assert_eq!(original, roundtripped);
    }

    #[test]
    fn fmri_roundtrip_via_node_resource() {
        // Hardware path: walk to a real node, format its FMRI, parse it
        // back, format again, assert round-trip.
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let Some(fmri) = first_hc_resource(&snap).expect("walk failed") else {
            eprintln!("skipping: empty hc topology");
            return;
        };
        let original = hdl.fmri_to_string(&fmri).expect("fmri_to_string failed");
        let parsed = hdl.fmri_parse(&original).expect("fmri_parse failed");
        let roundtripped = hdl.fmri_to_string(&parsed).expect("fmri_to_string failed");
        assert_eq!(original, roundtripped);
    }

    #[test]
    fn fmri_present_on_node_resource() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let Some(fmri) = first_hc_resource(&snap).expect("walk failed") else {
            eprintln!("skipping: empty hc topology");
            return;
        };
        let present = hdl.fmri_present(&fmri).expect("fmri_present failed");
        assert!(present, "node's own resource should be present");
    }

    #[test]
    fn fmri_replaced_on_node_resource() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let Some(fmri) = first_hc_resource(&snap).expect("walk failed") else {
            eprintln!("skipping: empty hc topology");
            return;
        };
        let _ = hdl.fmri_replaced(&fmri).expect("fmri_replaced failed");
    }

    #[test]
    fn fmri_unusable_on_node_resource() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let Some(fmri) = first_hc_resource(&snap).expect("walk failed") else {
            eprintln!("skipping: empty hc topology");
            return;
        };
        let _ = hdl.fmri_unusable(&fmri).expect("fmri_unusable failed");
    }

    #[test]
    fn node_label_works() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let mut got_a_label = false;
        match snap.walk(Scheme::Hc, |node| {
            if let Ok(label) = node.label() {
                assert!(!label.is_empty(), "label should be non-empty");
                got_a_label = true;
                return Ok(WalkAction::Stop);
            }
            Ok(WalkAction::Continue)
        }) {
            Ok(()) => {
                if !got_a_label {
                    eprintln!("note: walked nodes but none had a label");
                }
            }
            Err(e) if is_empty_topology(&e) => {
                eprintln!("skipping: empty hc topology");
            }
            Err(e) => panic!("walk failed: {e}"),
        }
    }

    #[test]
    fn node_property_groups_returns_groups() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let mut checked = false;
        match snap.walk(Scheme::Hc, |node| {
            let groups = node.property_groups()?;
            assert!(!groups.is_empty(), "expected at least one property group");
            for pg in &groups {
                assert!(
                    !pg.name.is_empty(),
                    "property group name should be non-empty"
                );
            }
            checked = true;
            Ok(WalkAction::Stop)
        }) {
            Ok(()) => assert!(checked, "expected to inspect at least one node"),
            Err(e) if is_empty_topology(&e) => {
                eprintln!("skipping: empty hc topology");
            }
            Err(e) => panic!("walk failed: {e}"),
        }
    }

    #[test]
    fn node_property_resource_is_fmri() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let mut checked = false;
        match snap.walk(Scheme::Hc, |node| {
            let pv = node.property("protocol", "resource")?;
            match pv {
                PropValue::Fmri(f) => {
                    let s = hdl.fmri_to_string(&f)?;
                    assert!(s.starts_with("hc:"), "expected hc-scheme FMRI, got {s:?}");
                    checked = true;
                }
                other => panic!("expected PropValue::Fmri, got {other:?}"),
            }
            Ok(WalkAction::Stop)
        }) {
            Ok(()) => assert!(checked, "expected to inspect at least one node"),
            Err(e) if is_empty_topology(&e) => {
                eprintln!("skipping: empty hc topology");
            }
            Err(e) => panic!("walk failed: {e}"),
        }
    }

    #[test]
    fn node_property_unknown_returns_err() {
        let hdl = TopoHdl::open().expect("failed to open");
        let snap = hdl.snapshot().expect("failed to take snapshot");
        let mut checked = false;
        match snap.walk(Scheme::Hc, |node| {
            let r = node.property("nope-group", "nope-name");
            assert!(matches!(r, Err(Error::Topo(_))), "expected Err, got {r:?}");
            checked = true;
            Ok(WalkAction::Stop)
        }) {
            Ok(()) => assert!(checked, "expected to inspect at least one node"),
            Err(e) if is_empty_topology(&e) => {
                eprintln!("skipping: empty hc topology");
            }
            Err(e) => panic!("walk failed: {e}"),
        }
    }

    #[test]
    fn prop_value_debug_unknown() {
        let v = PropValue::Unknown { type_code: 9999 };
        let s = format!("{v:?}");
        assert!(
            s.contains("9999"),
            "expected Debug to mention the type code, got {s:?}"
        );
    }
}
