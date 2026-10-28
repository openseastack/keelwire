//! Network health analysis: instance conflicts ("bouncing needles"),
//! duplicate device instances and missing information.

use std::collections::BTreeMap;

use crate::registry::Device;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "lowercase"))]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Finding {
    pub severity: Severity,
    pub kind: String,
    pub message: String,
    pub devices: Vec<String>,
    pub pgn: Option<u32>,
    pub suggestion: Option<String>,
    /// One-click fix: the device to change ...
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub target: Option<String>,
    /// ... the primary-key signature of the message occurrence to change (instance conflicts) ...
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub key: Option<String>,
    /// ... and the instance to move it to.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub new_instance: Option<i64>,
}

/// Numeric instance in a primary-key signature such as "Instance=0, Type=Fuel".
fn instance_of(key: &str) -> Option<i64> {
    key.split(", ").find_map(|p| {
        let (n, v) = p.split_once('=')?;
        if n.to_ascii_lowercase().contains("instance") {
            v.trim().parse().ok()
        } else {
            None
        }
    })
}

/// `pgn_name` resolves PGN descriptions for messages.
pub fn analyze(devices: &[&Device], pgn_name: &dyn Fn(u32) -> String) -> Vec<Finding> {
    let mut out = Vec::new();
    let label = |d: &Device| -> String {
        let model = d.model.clone().or_else(|| d.product.as_ref().map(|p| p.model_id.clone())).unwrap_or_else(|| "device".into());
        match d.address {
            Some(a) => format!("{} @{}", model, a),
            None => model,
        }
    };

    // Rule 1: same PGN + same primary key from different devices.
    let mut by_key: BTreeMap<(u32, String), Vec<&Device>> = BTreeMap::new();
    for d in devices.iter().filter(|d| d.online) {
        for (pgn, o) in &d.observed {
            for k in &o.keys {
                by_key.entry((*pgn, k.clone())).or_default().push(d);
            }
        }
    }
    // Instances already in use per PGN, to recommend a free one.
    let mut used: BTreeMap<u32, Vec<i64>> = BTreeMap::new();
    for (pgn, key) in by_key.keys() {
        if let Some(i) = instance_of(key) {
            used.entry(*pgn).or_default().push(i);
        }
    }
    for ((pgn, key), ds) in &by_key {
        if ds.len() > 1 {
            let keep = ds[0];
            let change: Vec<String> = ds[1..].iter().map(|d| label(d)).collect();
            let free = used.get(pgn).map(|u| (0..=252).find(|i| !u.contains(i)).unwrap_or(0)).unwrap_or(1);
            let redundant = matches!(pgn, 129025 | 129026 | 129029 | 127250 | 127257 | 128267 | 130306);
            let mut advice = match instance_of(key) {
                Some(_) => format!(
                    "Keep {} and change the data instance of {} to {free} (the lowest instance not yet used for this message), then run the analysis again.",
                    label(keep),
                    change.join(" and ")
                ),
                None => format!("Change the source type or instance of {} so each device sends a distinct combination.", change.join(" and ")),
            };
            if redundant {
                advice.push_str(" If these are deliberate backups (for example two GPS antennas), the shared instance is intended and can be left as is.");
            }
            out.push(Finding {
                severity: if redundant { Severity::Warning } else { Severity::Error },
                kind: "instance_conflict".into(),
                message: format!("{} ({}) with {} is transmitted by {} devices: {}", pgn_name(*pgn), pgn, key, ds.len(), ds.iter().map(|d| label(d)).collect::<Vec<_>>().join(", ")),
                devices: ds.iter().map(|d| d.key.clone()).collect(),
                pgn: Some(*pgn),
                suggestion: Some(advice),
                target: ds.get(1).map(|d| d.key.clone()),
                key: Some(key.clone()),
                new_instance: instance_of(key).map(|_| free),
            });
        }
    }

    // Rule 2: identical (manufacturer, function, class, device instance).
    let mut by_ident: BTreeMap<(u16, u8, u8, u8), Vec<&Device>> = BTreeMap::new();
    for d in devices.iter().filter(|d| d.online) {
        if let Some(n) = d.name {
            by_ident.entry((n.manufacturer_code, n.device_function, n.device_class, n.device_instance())).or_default().push(d);
        }
    }
    let mut used_dev: BTreeMap<(u16, u8, u8), Vec<u8>> = BTreeMap::new();
    for (m, f, c, i) in by_ident.keys() {
        used_dev.entry((*m, *f, *c)).or_default().push(*i);
    }
    for ((m, f, c, inst), ds) in &by_ident {
        if ds.len() > 1 {
            let free = used_dev.get(&(*m, *f, *c)).map(|u| (0..=252u8).find(|i| !u.contains(i)).unwrap_or(0)).unwrap_or(1);
            out.push(Finding {
                severity: Severity::Warning,
                kind: "duplicate_device_instance".into(),
                message: format!("{} devices of the same kind share device instance {}: {}", ds.len(), inst, ds.iter().map(|d| label(d)).collect::<Vec<_>>().join(", ")),
                devices: ds.iter().map(|d| d.key.clone()).collect(),
                pgn: Some(60928),
                suggestion: Some(format!(
                    "Displays that select by device may mix these up. Set a different device instance on {} (Devices list, Dev. inst. column, or Configure → General).",
                    ds[1..].iter().map(|d| label(d)).collect::<Vec<_>>().join(" and ")
                )),
                target: ds.get(1).map(|d| d.key.clone()),
                key: None,
                new_instance: Some(free as i64),
            });
        }
    }

    // Rule 4: no product information.
    for d in devices.iter().filter(|d| d.online && d.name.is_some() && d.queried && d.product.is_none()) {
        out.push(Finding {
            severity: Severity::Info,
            kind: "no_product_info".into(),
            message: format!("{} did not answer a request for Product Information (126996)", label(d)),
            devices: vec![d.key.clone()],
            pgn: Some(126996),
            suggestion: None,
            target: Some(d.key.clone()),
            key: None,
            new_instance: None,
        });
    }

    // Rule 6: sources that never claimed an address.
    for d in devices.iter().filter(|d| d.online && d.name.is_none()) {
        out.push(Finding {
            severity: Severity::Warning,
            kind: "unclaimed_source".into(),
            message: format!("Source address {} transmits without a known address claim", d.address.map(|a| a.to_string()).unwrap_or_default()),
            devices: vec![d.key.clone()],
            pgn: None,
            suggestion: Some("Use Refresh to request address claims; persistent cases indicate a non-compliant device or gateway.".into()),
            target: Some(d.key.clone()),
            key: None,
            new_instance: None,
        });
    }

    out.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.kind.cmp(&b.kind)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iso::IsoName;
    use crate::registry::ObservedPgn;

    fn dev(key: &str, addr: u8, inst: u8, keys: &[&str]) -> Device {
        let mut n = IsoName { manufacturer_code: 137, device_class: 30, device_function: 140, ..Default::default() };
        n.set_device_instance(inst);
        let mut d = Device { key: key.into(), name: Some(n), address: Some(addr), online: true, ..Default::default() };
        d.observed.insert(127505, ObservedPgn { keys: keys.iter().map(|s| s.to_string()).collect(), ..Default::default() });
        d
    }

    #[test]
    fn detects_conflicts() {
        let a = dev("A", 1, 0, &["Instance=0, Type=Fuel"]);
        let b = dev("B", 2, 0, &["Instance=0, Type=Fuel", "Instance=1, Type=Fuel"]);
        let c = dev("C", 3, 1, &["Instance=2, Type=Fuel"]);
        let f = analyze(&[&a, &b, &c], &|_| "Fluid Level".into());
        assert_eq!(f.iter().filter(|x| x.kind == "instance_conflict").count(), 1);
        assert_eq!(f.iter().filter(|x| x.kind == "duplicate_device_instance").count(), 1);
        assert_eq!(f[0].severity, Severity::Error);
    }
}
