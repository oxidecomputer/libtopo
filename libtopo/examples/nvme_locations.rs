//! Report the chassis location of every NVMe controller libtopo knows about.
//!
//! Walks the `hc` scheme and, for each `nvme` node, prints its `io/instance`
//! (the devinfo driver instance, i.e. the `N` in `nvme<N>`), its label, and
//! its parent node's name, label, and `binding/slot`. On Oxide platforms the
//! parent is a `bay` (U.2, labelled `N0`-`N9`) or a `slot` (M.2, labelled
//! `M.2 East` / `M.2 West`), and `binding/slot` is the PCIe physical slot
//! number the topology map ties that location to. The final column is the
//! operator-facing location: the node's own label, else its parent's.
//!
//! Also reports how long opening the handle and taking the snapshot took,
//! since a hardware monitor may want to do this on a schedule.
//!
//! ```sh
//! pfexec cargo run --example nvme_locations
//! ```

mod common;

use std::time::Instant;

use common::{cell, collect_nvme_locations};
use libtopo::TopoHdl;

fn main() {
    let t0 = Instant::now();
    let hdl = TopoHdl::open().expect("failed to open libtopo handle");
    let opened = t0.elapsed();
    let t1 = Instant::now();
    let snap = hdl.snapshot().expect("failed to take snapshot");
    let snapped = t1.elapsed();

    println!("topo_open:      {opened:?}");
    println!("topo_snap_hold: {snapped:?}");
    println!("snapshot uuid:  {}", snap.uuid());
    println!();

    let locations = match collect_nvme_locations(&snap) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("hc walk failed: {e}");
            std::process::exit(1);
        }
    };
    if locations.is_empty() {
        println!("no nvme nodes in the hc topology");
        return;
    }

    println!(
        "{:<10} {:<12} {:<12} {:<14} {:<14} {:<13} location",
        "node", "io/instance", "label", "parent", "parent label", "binding/slot"
    );
    for l in &locations {
        let (parent, parent_label, binding_slot) = match &l.parent {
            Some(p) => (
                format!("{}[{}]", p.name, p.instance),
                cell(p.label.as_deref()),
                cell(p.binding_slot),
            ),
            None => ("-".to_string(), "-".to_string(), "-".to_string()),
        };
        println!(
            "{:<10} {:<12} {:<12} {:<14} {:<14} {:<13} {}",
            format!("nvme[{}]", l.node_instance),
            cell(l.io_instance),
            cell(l.label.as_deref()),
            parent,
            parent_label,
            binding_slot,
            cell(l.location()),
        );
    }
}
