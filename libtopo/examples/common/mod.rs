//! Shared helpers for the `nvme_locations` and `disk_locations` examples:
//! walking the `hc` topology and collecting, for every `nvme` node, the
//! facts needed to place the controller in the chassis.

use libtopo::hc::{NVME, TOPO_BINDING_SLOT, TOPO_IO_INSTANCE, TOPO_PGROUP_BINDING, TOPO_PGROUP_IO};
use libtopo::{Error, Node, PropValue, Scheme, Snapshot, WalkAction};

/// Where libtopo places one `nvme` node.
#[derive(Debug)]
pub struct NvmeLocation {
    /// Instance of the `nvme` topo node within its parent (usually 0).
    pub node_instance: u64,
    /// `io/instance`: the devinfo driver instance, i.e. the `N` in `nvme<N>`.
    /// This is the join key to the devinfo tree.
    pub io_instance: Option<u32>,
    /// `protocol/label` on the `nvme` node itself, if any.
    pub label: Option<String>,
    /// The parent node (a `bay` or `slot` on Oxide platforms).
    pub parent: Option<ParentLocation>,
}

/// The node an `nvme` node hangs off of.
#[derive(Debug)]
pub struct ParentLocation {
    pub name: String,
    pub instance: u64,
    /// `protocol/label`, e.g. `N3` for a U.2 bay or `M.2 East`.
    pub label: Option<String>,
    /// `binding/slot`: the PCIe physical slot number the platform topology
    /// map binds this bay or slot to. Comparable to devinfo's
    /// `physical-slot#` on the upstream `pcieb` bridge.
    pub binding_slot: Option<u32>,
}

impl NvmeLocation {
    /// The label an operator would use for this controller: the node's own
    /// label if present, else its parent's.
    pub fn location(&self) -> Option<&str> {
        self.label
            .as_deref()
            .or_else(|| self.parent.as_ref().and_then(|p| p.label.as_deref()))
    }
}

/// Walk the `hc` scheme and collect every `nvme` node, sorted by
/// `io/instance` (nodes without one sort last).
pub fn collect_nvme_locations(snap: &Snapshot<'_>) -> Result<Vec<NvmeLocation>, Error> {
    let mut out = Vec::new();
    snap.walk(Scheme::Hc, |node| {
        if node.name() != NVME {
            return Ok(WalkAction::Continue);
        }
        out.push(NvmeLocation {
            node_instance: node.instance(),
            io_instance: u32_prop(&node, TOPO_PGROUP_IO, TOPO_IO_INSTANCE),
            label: label_of(&node),
            parent: node.parent().map(|p| ParentLocation {
                name: p.name().into_owned(),
                instance: p.instance(),
                label: label_of(&p),
                binding_slot: u32_prop(&p, TOPO_PGROUP_BINDING, TOPO_BINDING_SLOT),
            }),
        });
        Ok(WalkAction::Continue)
    })?;
    out.sort_by_key(|l| (l.io_instance.is_none(), l.io_instance));
    Ok(out)
}

/// A node's label, treating "no label" and an empty label alike.
fn label_of(node: &Node<'_>) -> Option<String> {
    node.label().ok().filter(|l| !l.is_empty())
}

/// Read a `uint32` property, or `None` if it is missing or not a uint32.
fn u32_prop(node: &Node<'_>, group: &str, name: &str) -> Option<u32> {
    match node.property(group, name) {
        Ok(PropValue::UInt32(v)) => Some(v),
        Ok(other) => {
            eprintln!(
                "warn: {}[{}] {group}/{name} is not a uint32: {other:?}",
                node.name(),
                node.instance(),
            );
            None
        }
        Err(_) => None,
    }
}

/// Render an optional value for a table cell.
pub fn cell<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or_else(|| "-".to_string(), |v| v.to_string())
}
