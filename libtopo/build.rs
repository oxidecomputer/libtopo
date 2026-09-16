fn main() {
    println!("cargo:rerun-if-env-changed=DEP_TOPO_LIBDIRS");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("illumos") {
        return;
    }
    // libtopo-sys exports its install dir (via `links = "topo"`) so this
    // crate's tests, examples, and doctests can find libtopo.so.1 in its
    // non-standard location at load time.
    if let Ok(dirs) = std::env::var("DEP_TOPO_LIBDIRS") {
        for dir in dirs.split(':') {
            println!("cargo:rustc-link-arg=-R{dir}");
        }
    }
}
