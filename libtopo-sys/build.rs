fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rustc-link-lib=topo");
    println!("cargo:rustc-link-search=native=/usr/lib/fm/amd64");
    println!("cargo:rustc-link-arg=-R/usr/lib/fm/amd64");
    // Expose the libtopo install dir to direct dependents as DEP_TOPO_LIBDIRS,
    // so they can set RPATH for the non-standard /usr/lib/fm/amd64 location.
    println!("cargo:LIBDIRS=/usr/lib/fm/amd64");
}
