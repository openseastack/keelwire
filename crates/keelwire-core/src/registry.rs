//! Device registry keyed by ISO NAME.

use std::collections::{BTreeMap, HashMap};

use crate::iso::{ConfigInfo, IsoName, ProductInfo};

/// Key used for sources heard before (or without) an address claim.
pub fn pseudo_key(address: u8) -> u64 {
    0xFFFF_FFFF_FFFF_FF00 | address as u64
}

/// Same physical device: unique number, manufacturer, function and class match
/// (instances and other configurable NAME fields may differ).
pub fn same_device(a: &IsoName, b: &IsoName) -> bool {
    a.unique_number == b.unique_number && a.manufacturer_code == b.manufacturer_code && a.device_function == b.device_function && a.device_class == b.device_class && a.industry_group == b.industry_group
}

pub fn is_pseudo(key: u64) -> bool {
    key & 0xFFFF_FFFF_FFFF_FF00 == 0xFFFF_FFFF_FFFF_FF00
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObservedPgn {
    pub count: u64,
    pub last_us: u64,
    pub last_data: Vec<u8>,
    pub destination: u8,
    /// Distinct primary-key signatures (e.g. instance / source) seen recently for this PGN.
    pub keys: Vec<String>,
    /// Last time (µs) each entry of `keys` was seen.
    #[cfg_attr(feature = "serde", serde(default))]
    pub key_seen: Vec<u64>,
    /// Last payload for each entry of `keys` (one "occurrence" per instance,
    /// shown one line per instance in the device's Messages tab).
    #[cfg_attr(feature = "serde", serde(default))]
    pub key_data: Vec<Vec<u8>>,
}

impl ObservedPgn {
    /// Record a primary-key signature and drop ones not seen for `max_age_us`.
    pub fn note_key(&mut self, key: String, now_us: u64, max_age_us: u64) {
        self.note_key_data(key, now_us, max_age_us, &[]);
    }

    /// As `note_key`, also remembering the payload for that key.
    pub fn note_key_data(&mut self, key: String, now_us: u64, max_age_us: u64, data: &[u8]) {
        self.key_seen.resize(self.keys.len(), now_us);
        self.key_data.resize(self.keys.len(), Vec::new());
        match self.keys.iter().position(|k| *k == key) {
            Some(i) => {
                self.key_seen[i] = now_us;
                self.key_data[i] = data.to_vec();
            }
            None if self.keys.len() < 32 => {
                self.keys.push(key);
                self.key_seen.push(now_us);
                self.key_data.push(data.to_vec());
            }
            None => {}
        }
        let mut i = 0;
        while i < self.keys.len() {
            if now_us.saturating_sub(self.key_seen[i]) > max_age_us {
                self.keys.remove(i);
                self.key_seen.remove(i);
                self.key_data.remove(i);
            } else {
                i += 1;
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChannelLabel {
    pub instance: u8,
    pub data_source: u8,
    pub indicator: u8,
    pub hardware_channel: Option<u8>,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Device {
    /// Hex NAME, or "addr-XX" for an unclaimed source.
    pub key: String,
    pub name_raw: u64,
    pub name: Option<IsoName>,
    pub address: Option<u8>,
    pub address_history: Vec<(u64, u8)>,
    pub first_seen_us: u64,
    pub last_seen_us: u64,
    pub online: bool,
    pub product: Option<ProductInfo>,
    pub config: Option<ConfigInfo>,
    pub tx_pgns: Option<Vec<u32>>,
    pub rx_pgns: Option<Vec<u32>>,
    pub heartbeat_interval_ms: Option<u16>,
    pub heartbeat_last_us: Option<u64>,
    pub observed: BTreeMap<u32, ObservedPgn>,
    pub frames: u64,
    pub labels: Vec<ChannelLabel>,
    pub device_label: Option<String>,
    // Resolved texts (filled by the engine from the PGN database)
    pub manufacturer: Option<String>,
    pub device_class_text: Option<String>,
    pub device_function_text: Option<String>,
    pub model: Option<String>,
    /// Outstanding discovery requests done flags.
    pub queried: bool,
}

impl Device {
    pub fn device_instance(&self) -> Option<u8> {
        self.name.map(|n| n.device_instance())
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(tag = "kind", rename_all = "snake_case"))]
pub enum RegistryEvent {
    NewDevice { key: String, address: u8 },
    AddressChanged { key: String, old: u8, new: u8 },
    Contention { address: u8, winner: String, loser: String },
    DuplicateAddress { address: u8, keys: Vec<String> },
    CannotClaim { key: String },
    /// NAME changed (e.g. instance edited) for the same physical device.
    Renamed { old: String, new: String },
    Offline { key: String },
    Online { key: String },
}

pub struct Registry {
    pub devices: HashMap<u64, Device>,
    by_addr: [Option<u64>; 256],
    /// (name, address, ts) of last lost contention, for duplicate detection.
    lost: HashMap<u64, (u8, u64)>,
    pub offline_after_us: u64,
}

impl Default for Registry {
    fn default() -> Self {
        Registry { devices: HashMap::new(), by_addr: [None; 256], lost: HashMap::new(), offline_after_us: 60_000_000 }
    }
}

impl Registry {
    pub fn key_for_address(&self, address: u8) -> Option<u64> {
        self.by_addr[address as usize]
    }

    pub fn by_address(&self, address: u8) -> Option<&Device> {
        self.by_addr[address as usize].and_then(|k| self.devices.get(&k))
    }

    pub fn by_address_mut(&mut self, address: u8) -> Option<&mut Device> {
        let k = self.by_addr[address as usize]?;
        self.devices.get_mut(&k)
    }

    pub fn get(&self, key: &str) -> Option<&Device> {
        self.devices.values().find(|d| d.key == key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Device> {
        self.devices.values_mut().find(|d| d.key == key)
    }

    pub fn clear(&mut self) {
        self.devices.clear();
        self.by_addr = [None; 256];
        self.lost.clear();
    }

    /// Record any frame from `address`, creating a pseudo device if unknown.
    pub fn touch(&mut self, ts: u64, address: u8) -> (&mut Device, Option<RegistryEvent>) {
        let mut ev = None;
        let key = match self.by_addr[address as usize] {
            Some(k) => k,
            None => {
                let k = pseudo_key(address);
                self.devices.entry(k).or_insert_with(|| Device {
                    key: format!("addr-{:02X}", address),
                    name_raw: k,
                    address: Some(address),
                    first_seen_us: ts,
                    online: true,
                    ..Default::default()
                });
                self.by_addr[address as usize] = Some(k);
                ev = Some(RegistryEvent::NewDevice { key: format!("addr-{:02X}", address), address });
                k
            }
        };
        let d = self.devices.get_mut(&key).unwrap();
        d.last_seen_us = ts;
        d.frames += 1;
        if !d.online {
            d.online = true;
            if ev.is_none() {
                ev = Some(RegistryEvent::Online { key: d.key.clone() });
            }
        }
        (d, ev)
    }

    /// Process an address claim (PGN 60928) from `address`.
    pub fn claim(&mut self, ts: u64, address: u8, name_raw: u64) -> Vec<RegistryEvent> {
        let mut events = Vec::new();
        let name = IsoName::from_u64(name_raw);
        let key_str = name.hex();
        if address == crate::frame::NULL_ADDRESS {
            events.push(RegistryEvent::CannotClaim { key: key_str.clone() });
            if let Some(d) = self.devices.get_mut(&name_raw) {
                if let Some(a) = d.address.take() {
                    if self.by_addr[a as usize] == Some(name_raw) {
                        self.by_addr[a as usize] = None;
                    }
                }
            }
            return events;
        }
        // Same physical device whose NAME changed (device/system instance edited):
        // re-key it instead of treating the new NAME as another device.
        if !self.devices.contains_key(&name_raw) {
            let renamed = self
                .devices
                .iter()
                .find(|(k, d)| !is_pseudo(**k) && d.name.map(|o| same_device(&o, &name)).unwrap_or(false))
                .map(|(k, _)| *k);
            if let Some(old) = renamed {
                let mut d = self.devices.remove(&old).unwrap();
                d.name_raw = name_raw;
                d.name = Some(name);
                d.key = key_str.clone();
                for slot in self.by_addr.iter_mut() {
                    if *slot == Some(old) {
                        *slot = Some(name_raw);
                    }
                }
                self.devices.insert(name_raw, d);
                events.push(RegistryEvent::Renamed { old: IsoName::from_u64(old).hex(), new: key_str.clone() });
            }
        }
        // Contention: another real device currently holds this address.
        if let Some(holder) = self.by_addr[address as usize] {
            if holder != name_raw && !is_pseudo(holder) {
                let (winner, loser) = if name_raw < holder { (name_raw, holder) } else { (holder, name_raw) };
                let wk = IsoName::from_u64(winner).hex();
                let lk = IsoName::from_u64(loser).hex();
                events.push(RegistryEvent::Contention { address, winner: wk.clone(), loser: lk.clone() });
                if let Some(&(a, t)) = self.lost.get(&loser) {
                    if a == address && ts.saturating_sub(t) < 10_000_000 {
                        events.push(RegistryEvent::DuplicateAddress { address, keys: vec![wk, lk] });
                    }
                }
                self.lost.insert(loser, (address, ts));
                if winner == holder {
                    // Newcomer lost: register it without an address.
                    self.ensure(ts, name_raw, name, None, &mut events);
                    return events;
                }
                if let Some(d) = self.devices.get_mut(&holder) {
                    d.address = None;
                }
            }
        }
        // Migrate pseudo device data if present.
        let pseudo = match self.by_addr[address as usize] {
            Some(k) if is_pseudo(k) => self.devices.remove(&k),
            _ => None,
        };
        self.ensure(ts, name_raw, name, Some(address), &mut events);
        let d = self.devices.get_mut(&name_raw).unwrap();
        if let Some(p) = pseudo {
            d.first_seen_us = d.first_seen_us.min(p.first_seen_us);
            d.frames += p.frames;
            for (k, v) in p.observed {
                d.observed.entry(k).or_insert(v);
            }
            if d.product.is_none() {
                d.product = p.product;
            }
            if d.config.is_none() {
                d.config = p.config;
            }
            if d.tx_pgns.is_none() {
                d.tx_pgns = p.tx_pgns;
            }
            if d.rx_pgns.is_none() {
                d.rx_pgns = p.rx_pgns;
            }
        }
        self.by_addr[address as usize] = Some(name_raw);
        events
    }

    fn ensure(&mut self, ts: u64, name_raw: u64, name: IsoName, address: Option<u8>, events: &mut Vec<RegistryEvent>) {
        let key = name.hex();
        match self.devices.get_mut(&name_raw) {
            Some(d) => {
                d.name = Some(name);
                d.last_seen_us = ts;
                d.online = true;
                if let Some(new) = address {
                    if d.address != Some(new) {
                        if let Some(old) = d.address {
                            if self.by_addr[old as usize] == Some(name_raw) {
                                self.by_addr[old as usize] = None;
                            }
                            events.push(RegistryEvent::AddressChanged { key: key.clone(), old, new });
                        }
                        d.address = Some(new);
                        d.address_history.push((ts, new));
                    }
                }
            }
            None => {
                self.devices.insert(
                    name_raw,
                    Device {
                        key: key.clone(),
                        name_raw,
                        name: Some(name),
                        address,
                        address_history: address.map(|a| vec![(ts, a)]).unwrap_or_default(),
                        first_seen_us: ts,
                        last_seen_us: ts,
                        online: true,
                        ..Default::default()
                    },
                );
                if let Some(a) = address {
                    events.push(RegistryEvent::NewDevice { key, address: a });
                }
            }
        }
    }

    /// Mark devices offline when silent for longer than their heartbeat budget.
    pub fn sweep(&mut self, now: u64) -> Vec<RegistryEvent> {
        let mut ev = Vec::new();
        for d in self.devices.values_mut() {
            let budget = d.heartbeat_interval_ms.map(|i| (i as u64 * 3_000).max(self.offline_after_us)).unwrap_or(self.offline_after_us);
            if d.online && now.saturating_sub(d.last_seen_us) > budget {
                d.online = false;
                ev.push(RegistryEvent::Offline { key: d.key.clone() });
            }
        }
        ev
    }

    pub fn list(&self) -> Vec<&Device> {
        let mut v: Vec<&Device> = self.devices.values().collect();
        v.sort_by_key(|d| (d.address.unwrap_or(255), d.key.clone()));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nm(u: u32) -> u64 {
        IsoName { unique_number: u, manufacturer_code: 137, device_function: 130, device_class: 25, industry_group: 4, ..Default::default() }.to_u64()
    }

    #[test]
    fn pseudo_then_claim_migrates() {
        let mut r = Registry::default();
        r.touch(0, 5);
        r.by_address_mut(5).unwrap().observed.insert(127250, ObservedPgn { count: 3, ..Default::default() });
        let ev = r.claim(10, 5, nm(1));
        assert!(matches!(ev[0], RegistryEvent::NewDevice { address: 5, .. }));
        let d = r.by_address(5).unwrap();
        assert_eq!(d.name_raw, nm(1));
        assert_eq!(d.observed[&127250].count, 3);
        assert_eq!(r.devices.len(), 1);
    }

    #[test]
    fn address_change_and_contention() {
        let mut r = Registry::default();
        r.claim(0, 10, nm(5));
        let ev = r.claim(1, 11, nm(5));
        assert!(matches!(ev[0], RegistryEvent::AddressChanged { old: 10, new: 11, .. }));
        assert!(r.by_address(10).is_none());
        // lower NAME wins the address
        let ev = r.claim(2, 11, nm(1));
        assert!(matches!(&ev[0], RegistryEvent::Contention { address: 11, .. }));
        assert_eq!(r.by_address(11).unwrap().name_raw, nm(1));
        assert_eq!(r.devices[&nm(5)].address, None);
        // loser tries the same address again quickly -> duplicate
        let ev = r.claim(3, 11, nm(5));
        assert!(ev.iter().any(|e| matches!(e, RegistryEvent::DuplicateAddress { .. })));
    }

    #[test]
    fn instance_change_keeps_identity() {
        let mut r = Registry::default();
        r.claim(0, 17, nm(7));
        r.by_address_mut(17).unwrap().frames = 42;
        let mut n = IsoName::from_u64(nm(7));
        n.set_device_instance(3);
        let ev = r.claim(1, 17, n.to_u64());
        assert!(ev.iter().any(|e| matches!(e, RegistryEvent::Renamed { .. })));
        assert!(!ev.iter().any(|e| matches!(e, RegistryEvent::Contention { .. })));
        assert_eq!(r.devices.len(), 1);
        let d = r.by_address(17).unwrap();
        assert_eq!(d.frames, 42);
        assert_eq!(d.device_instance(), Some(3));
    }

    #[test]
    fn primary_keys_expire() {
        let mut o = ObservedPgn::default();
        o.note_key("Instance=0".into(), 0, 120_000_000);
        o.note_key("Instance=1".into(), 100_000_000, 120_000_000);
        o.note_key("Instance=1".into(), 130_000_000, 120_000_000);
        assert_eq!(o.keys, vec!["Instance=1".to_string()]);
    }

    #[test]
    fn offline_sweep() {
        let mut r = Registry::default();
        r.claim(0, 1, nm(1));
        assert!(r.sweep(30_000_000).is_empty());
        assert_eq!(r.sweep(61_000_001).len(), 1);
        let (_, ev) = r.touch(62_000_000, 1);
        assert!(matches!(ev, Some(RegistryEvent::Online { .. })));
    }
}
