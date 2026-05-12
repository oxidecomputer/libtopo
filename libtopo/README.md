# libtopo

Idiomatic Rust bindings for illumos `libtopo`.

This crate provides a safe wrapper around the raw FFI bindings in
[libtopo-sys](../libtopo-sys/), the hardware topology library used
by FMA (Fault Management Architecture) and other illumos consumers.

## Usage

```rust
use libtopo::{Scheme, TopoHdl, WalkAction};

let hdl = TopoHdl::open()?;
let snap = hdl.snapshot()?;
println!("snapshot uuid: {}", snap.uuid());

snap.walk(Scheme::Hc, |node| {
    let fmri = node.resource()?;
    let fmri_str = hdl.fmri_to_string(&fmri)?;
    println!("{}[{}]\t{}", node.name(), node.instance(), fmri_str);
    Ok(WalkAction::Continue)
})?;
```

See [`examples/`](examples/) for runnable programs.

## Privileges

Most libtopo operations require elevated privileges to enumerate
hardware. Run with `pfexec` or appropriate RBAC profiles.

## Testing

Tests must be run on an illumos host since they link against `libtopo`.
Walker-dependent integration tests gracefully skip when the host has no
hardware topology (e.g. a CI VM).

```sh
pfexec cargo test
```
