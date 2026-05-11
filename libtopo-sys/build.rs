fn main() {
    println!("cargo:rustc-link-lib=topo");
    println!("cargo:rustc-link-search=native=/usr/lib/fm/amd64");
    println!("cargo:rustc-link-arg=-R/usr/lib/fm/amd64");
    println!("cargo:LIBDIRS=/usr/lib/fm/amd64");
}
