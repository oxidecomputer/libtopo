//! Join devinfo's view of NVMe disks with libtopo's chassis locations.
//!
//! First walks the `hc` topology and indexes every `nvme` node by its
//! `io/instance`. Then walks the devinfo tree the way a hardware monitor
//! does: each `blkdev` node's parent should be an `nvme` controller, whose
//! parent should be a `pcieb` bridge carrying a `physical-slot#` property.
//! The nvme driver instance joins the two views. For every disk this
//! prints the devfs path, the bridge's physical slot, the nvme instance,
//! the topo node path (parent and `nvme` node), the location label, and the
//! `binding/slot` the topology map assigns that parent (which should agree
//! with `physical-slot#`).
//!
//! A disk that libtopo cannot place is still listed, with `-` in the
//! topology columns.
//!
//! ```sh
//! pfexec cargo run --example disk_locations
//! ```

mod common;

use std::collections::HashMap;

use common::{NvmeLocation, cell, collect_nvme_locations};
use illumos_devinfo::{DevInfo, Node};
use libtopo::TopoHdl;

/// One `blkdev` node and what devinfo says about its ancestry.
struct DevinfoDisk {
    devfs_path: String,
    nvme_instance: Option<i32>,
    physical_slot: Option<i64>,
}

fn main() {
    let hdl = TopoHdl::open().expect("failed to open libtopo handle");
    let snap = hdl.snapshot().expect("failed to take snapshot");
    let by_io_instance: HashMap<u32, NvmeLocation> = match collect_nvme_locations(&snap) {
        Ok(locations) => locations
            .into_iter()
            .filter_map(|l| l.io_instance.map(|i| (i, l)))
            .collect(),
        Err(e) => {
            eprintln!("hc walk failed: {e}");
            std::process::exit(1);
        }
    };

    let disks = match devinfo_disks() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("devinfo walk failed: {e}");
            std::process::exit(1);
        }
    };
    if disks.is_empty() {
        println!("no blkdev nodes in the devinfo tree");
        return;
    }

    println!(
        "{:<48} {:<10} {:<10} {:<20} {:<14} binding/slot",
        "devfs path", "phys slot", "nvme inst", "topo node", "location"
    );
    for d in &disks {
        let topo = d
            .nvme_instance
            .and_then(|i| u32::try_from(i).ok())
            .and_then(|i| by_io_instance.get(&i));
        let (topo_node, binding_slot) = match topo {
            Some(t) => {
                let nvme = format!("nvme[{}]", t.node_instance);
                match &t.parent {
                    Some(p) => (
                        format!("{}[{}]/{nvme}", p.name, p.instance),
                        cell(p.binding_slot),
                    ),
                    None => (nvme, "-".to_string()),
                }
            }
            None => ("-".to_string(), "-".to_string()),
        };
        println!(
            "{:<48} {:<10} {:<10} {:<20} {:<14} {}",
            d.devfs_path,
            cell(d.physical_slot),
            cell(d.nvme_instance),
            topo_node,
            cell(topo.and_then(|t| t.location())),
            binding_slot,
        );
    }
}

/// Every `blkdev` node in the devinfo tree, with its nvme driver instance
/// and the upstream bridge's `physical-slot#` where those exist.
fn devinfo_disks() -> anyhow::Result<Vec<DevinfoDisk>> {
    let mut devinfo = DevInfo::new_force_load()?;
    let mut disks = Vec::new();
    for node in devinfo.walk_node() {
        let node = node?;
        if node.driver_name().as_deref() != Some("blkdev") {
            continue;
        }
        // devinfo paths are relative to the /devices mount.
        let devfs_path = format!("/devices{}", node.devfs_path()?);

        let nvme = parent_with_driver(&node, "nvme")?;
        let nvme_instance = nvme.as_ref().and_then(Node::instance);
        let pcieb = match &nvme {
            Some(n) => parent_with_driver(n, "pcieb")?,
            None => None,
        };
        let physical_slot = pcieb.as_ref().and_then(|p| i64_prop(p, "physical-slot#"));

        if nvme.is_none() {
            eprintln!("warn: {devfs_path}: parent is not an nvme node");
        } else if pcieb.is_none() {
            eprintln!("warn: {devfs_path}: nvme parent is not a pcieb node");
        }
        disks.push(DevinfoDisk {
            devfs_path,
            nvme_instance,
            physical_slot,
        });
    }
    disks.sort_by_key(|d| (d.nvme_instance.is_none(), d.nvme_instance));
    Ok(disks)
}

/// The node's parent if it is bound to `driver`, else `None`.
fn parent_with_driver<'a>(node: &Node<'a>, driver: &str) -> anyhow::Result<Option<Node<'a>>> {
    Ok(node
        .parent()?
        .filter(|p| p.driver_name().as_deref() == Some(driver)))
}

/// An integer devinfo property by name, or `None` if missing or not an int.
fn i64_prop(node: &Node<'_>, name: &str) -> Option<i64> {
    node.props()
        .filter_map(Result::ok)
        .find(|p| p.name() == name)
        .and_then(|p| p.as_i64())
}
