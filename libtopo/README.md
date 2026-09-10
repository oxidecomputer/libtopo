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

Inside the walker callback a `Node` also exposes `label()`, `parent()`,
`property(group, name)`, and `property_groups()`.

See [`examples/`](examples/) for runnable programs:

- `list_topology` — dump a scheme's nodes, FMRIs, labels, and (with `-v`)
  every property.
- `fmri_resolve` — parse an FMRI string and report present/replaced/unusable.
- `nvme_locations` — for every `nvme` node, print its devinfo instance,
  label, parent node, and the parent's label and `binding/slot`.
- `disk_locations` — join devinfo's `blkdev` → `nvme` → `pcieb` chain with
  the topology above, so each disk's `physical-slot#` sits next to its
  libtopo location label.

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
