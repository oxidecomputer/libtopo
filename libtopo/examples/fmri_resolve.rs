//! Parse an FMRI string and report its status (present/replaced/unusable).
//!
//! Run with `pfexec`:
//!
//! ```sh
//! pfexec cargo run --example fmri_resolve -- 'hc:///chassis=0'
//! ```

use libtopo::TopoHdl;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: {} <fmri-string>",
            args.first().map_or("fmri_resolve", |s| s.as_str())
        );
        std::process::exit(2);
    }
    let fmri_str = &args[1];

    let hdl = TopoHdl::open().expect("failed to open libtopo handle");
    // Hold a snapshot — fmri_present/replaced/unusable consult the
    // topology to resolve the resource.
    let _snap = hdl.snapshot().expect("failed to take snapshot");

    let fmri = match hdl.fmri_parse(fmri_str) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("fmri_parse failed: {e}");
            std::process::exit(1);
        }
    };

    let formatted = hdl
        .fmri_to_string(&fmri)
        .unwrap_or_else(|e| format!("<fmri_to_string failed: {e}>"));
    println!("fmri: {formatted}");

    print_predicate("present", hdl.fmri_present(&fmri));
    print_predicate("replaced", hdl.fmri_replaced(&fmri));
    print_predicate("unusable", hdl.fmri_unusable(&fmri));
}

fn print_predicate(name: &str, result: Result<bool, libtopo::Error>) {
    match result {
        Ok(b) => println!("{name}: {b}"),
        Err(e) => println!("{name}: error: {e}"),
    }
}
