//! Boat files (`*.keelwire`): full network state for offline analysis.

use keelwire_core::registry::Device;
use keelwire_core::sniffer::PacketRecord;
use serde::{Deserialize, Serialize};

use crate::engine::{Bank, State};

pub const FORMAT: &str = "keelwire-boat";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoatFile {
    pub format: String,
    pub version: u32,
    pub saved_at: String,
    pub app_version: String,
    #[serde(default)]
    pub vessel: Option<String>,
    pub devices: Vec<Device>,
    #[serde(default)]
    pub banks: Vec<Bank>,
    /// Last message per (pgn, source).
    #[serde(default)]
    pub messages: Vec<PacketRecord>,
}

impl BoatFile {
    pub fn from_state(st: &State, vessel: Option<String>) -> Self {
        let mut last: std::collections::BTreeMap<(u32, u8), PacketRecord> = Default::default();
        for r in st.ring.iter() {
            last.insert((r.pgn, r.source), r.clone());
        }
        BoatFile {
            format: FORMAT.into(),
            version: 1,
            saved_at: keelwire_core::formats::format_iso_time(keelwire_hal::clock::unix_now_us()),
            app_version: crate::VERSION.into(),
            vessel,
            devices: st.registry.list().into_iter().cloned().collect(),
            banks: st.banks.values().cloned().collect(),
            messages: last.into_values().collect(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("not an Keelwire boat file (format '{}')", self.format));
        }
        if self.version > 1 {
            return Err(format!("boat file version {} is newer than this application supports", self.version));
        }
        Ok(())
    }

    pub fn apply(self, st: &mut State) {
        for mut d in self.devices {
            d.online = true;
            let key = d.name_raw;
            if let Some(a) = d.address {
                if let Some(n) = d.name {
                    st.registry.claim(d.last_seen_us, a, n.to_u64());
                } else {
                    st.registry.touch(d.last_seen_us, a);
                }
            }
            if let Some(slot) = st.registry.devices.get_mut(&key) {
                *slot = d;
            } else if let Some(a) = d.address {
                if let Some(slot) = st.registry.by_address_mut(a) {
                    *slot = d;
                }
            }
        }
        for b in self.banks {
            st.banks.insert(b.instance, b);
        }
        for m in self.messages {
            st.ring.push(m);
        }
    }
}
