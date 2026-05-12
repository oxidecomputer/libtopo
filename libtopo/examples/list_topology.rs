//! Open a libtopo handle, take a snapshot, and walk a topology scheme,
//! printing each node's name, instance, FMRI, and label (if any).
//!
//! The scheme defaults to `hc`; pass another scheme name as the first
//! positional argument to walk it instead. Pass `-v` / `--verbose`
//! (anywhere in argv) to also dump every property group and property:
//!
//! ```sh
//! pfexec cargo run --example list_topology               # hc (default)
//! pfexec cargo run --example list_topology -- cpu
//! pfexec cargo run --example list_topology -- pcie -v
//! pfexec cargo run --example list_topology -- -v
//! ```

use libtopo::{PropValue, Scheme, TopoHdl, WalkAction};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let verbose = args.iter().any(|a| a == "-v" || a == "--verbose");
    let scheme = match args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(String::as_str)
    {
        None => Scheme::Hc,
        Some(name) => match parse_scheme(name) {
            Some(s) => s,
            None => {
                eprintln!("unknown scheme: {name:?}");
                eprintln!(
                    "valid schemes: hc, cpu, mem, dev, mod, svc, sw, zfs, pcie, path, fmd, pkg, legacy-hc"
                );
                std::process::exit(2);
            }
        },
    };

    let hdl = TopoHdl::open().expect("failed to open libtopo handle");
    let snap = hdl.snapshot().expect("failed to take snapshot");

    println!("snapshot uuid: {}", snap.uuid());
    println!("scheme:        {:?}", scheme);
    println!();

    let walk = snap.walk(scheme, |node| {
        let fmri = match node.resource() {
            Ok(f) => f,
            Err(e) => {
                eprintln!(
                    "warn: resource lookup failed on node {}[{}]: {e}",
                    node.name(),
                    node.instance(),
                );
                return Ok(WalkAction::Continue);
            }
        };
        let fmri_str = hdl
            .fmri_to_string(&fmri)
            .unwrap_or_else(|e| format!("<fmri_to_string failed: {e}>"));
        println!("{}[{}]\t{}", node.name(), node.instance(), fmri_str);
        if let Ok(label) = node.label()
            && !label.is_empty()
        {
            println!("  label: {label}");
        }

        if verbose {
            match node.property_groups() {
                Ok(groups) => {
                    for pg in &groups {
                        println!("  [{}]", pg.name);
                        for p in &pg.properties {
                            print_prop_value(&hdl, &p.name, &p.value);
                        }
                    }
                }
                Err(e) => eprintln!(
                    "warn: property_groups failed on {}[{}]: {e}",
                    node.name(),
                    node.instance(),
                ),
            }
        }
        Ok(WalkAction::Continue)
    });

    if let Err(e) = walk {
        eprintln!("walk failed: {e}");
        std::process::exit(1);
    }
}

fn print_prop_value(hdl: &TopoHdl, name: &str, v: &PropValue) {
    match v {
        PropValue::String(s) => println!("    {name} = {s:?}"),
        PropValue::Boolean(v) => println!("    {name} = {v}"),
        PropValue::Int32(v) => println!("    {name} = {v}"),
        PropValue::UInt32(v) => println!("    {name} = {v}"),
        PropValue::Int64(v) => println!("    {name} = {v}"),
        PropValue::UInt64(v) => println!("    {name} = {v}"),
        PropValue::Double(v) => println!("    {name} = {v}"),
        PropValue::Fmri(f) => match hdl.fmri_to_string(f) {
            Ok(s) => println!("    {name} = {s}"),
            Err(_) => println!("    {name} = <fmri (unstringable)>"),
        },
        PropValue::Time(t) => println!("    {name} = {t} (time)"),
        PropValue::Size(b) => println!("    {name} = {b} bytes"),
        PropValue::StringArray(vs) => println!("    {name} = {vs:?}"),
        PropValue::Int32Array(vs) => println!("    {name} = {vs:?}"),
        PropValue::UInt32Array(vs) => println!("    {name} = {vs:?}"),
        PropValue::Int64Array(vs) => println!("    {name} = {vs:?}"),
        PropValue::UInt64Array(vs) => println!("    {name} = {vs:?}"),
        PropValue::FmriArray(fs) => {
            let rendered: Vec<String> = fs
                .iter()
                .map(|f| {
                    hdl.fmri_to_string(f)
                        .unwrap_or_else(|_| "<unstringable>".into())
                })
                .collect();
            println!("    {name} = {rendered:?}");
        }
        PropValue::Unknown { type_code } => {
            println!("    {name} = <unsupported type {type_code}>")
        }
    }
}

fn parse_scheme(s: &str) -> Option<Scheme> {
    match s {
        "hc" => Some(Scheme::Hc),
        "cpu" => Some(Scheme::Cpu),
        "mem" => Some(Scheme::Mem),
        "dev" => Some(Scheme::Dev),
        "mod" => Some(Scheme::Mod),
        "svc" => Some(Scheme::Svc),
        "sw" => Some(Scheme::Sw),
        "zfs" => Some(Scheme::Zfs),
        "pcie" => Some(Scheme::Pcie),
        "path" => Some(Scheme::Path),
        "fmd" => Some(Scheme::Fmd),
        "pkg" => Some(Scheme::Pkg),
        "legacy-hc" => Some(Scheme::Legacy),
        _ => None,
    }
}
