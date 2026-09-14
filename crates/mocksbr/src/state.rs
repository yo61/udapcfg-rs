//! A virtual device's parameter state.
//!
//! Two tiers, mirroring a real Squeezebox: *working memory* is what the
//! device is running, *NVRAM* is what survives a reboot. `set_data`
//! writes working memory, `save` copies it to NVRAM, and `reset` reloads
//! working memory from NVRAM.
//!
//! go-udap's mock saves on every set, so a later reset observes the most
//! recent values — matching real-SBR behaviour on the test bench. The
//! two transitions stay separate here so a test can exercise a set
//! *without* a save and see the change discarded.

use std::collections::BTreeMap;
use udap::parameters;

type Params = BTreeMap<String, Vec<u8>>;

pub(crate) struct DeviceState {
    working: Params,
    nvram: Params,
}

impl DeviceState {
    /// A device in factory condition: both tiers hold the parameter
    /// table's factory defaults.
    pub(crate) fn factory() -> Self {
        let defaults: Params = parameters::PARAMETERS
            .iter()
            .map(|p| (p.name.to_owned(), p.factory_default.as_bytes().to_vec()))
            .collect();
        DeviceState {
            working: defaults.clone(),
            nvram: defaults,
        }
    }

    /// A device whose NVRAM carries `seed` over the factory defaults.
    ///
    /// Both tiers get it: the seed models a device that was configured
    /// and saved before the test began, so a reset must find it.
    pub(crate) fn factory_with(seed: &Params) -> Self {
        let mut state = Self::factory();
        for (name, value) in seed {
            state.working.insert(name.clone(), value.clone());
            state.nvram.insert(name.clone(), value.clone());
        }
        state
    }

    /// The value the device is currently running.
    pub(crate) fn get(&self, name: &str) -> Option<&[u8]> {
        self.working.get(name).map(Vec::as_slice)
    }

    /// Applies a write to working memory. Unrecognised names are stored
    /// as given — the mock does not second-guess the client.
    pub(crate) fn apply_set(&mut self, updates: Params) {
        self.working.extend(updates);
    }

    /// Commits working memory to NVRAM.
    pub(crate) fn apply_save(&mut self) {
        self.nvram = self.working.clone();
    }

    /// Reloads working memory from NVRAM, discarding uncommitted writes.
    pub(crate) fn apply_reset(&mut self) {
        self.working = self.nvram.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(name: &str, value: &[u8]) -> Params {
        let mut map = Params::new();
        map.insert(name.to_owned(), value.to_vec());
        map
    }

    #[test]
    fn a_factory_device_reports_the_table_defaults() {
        let state = DeviceState::factory();
        assert_eq!(state.get("wireless_channel"), Some(b"6".as_slice()));
        assert_eq!(
            state.get("lan_subnet_mask"),
            Some(b"255.255.255.0".as_slice())
        );
    }

    #[test]
    fn a_factory_device_knows_every_parameter() {
        let state = DeviceState::factory();
        for name in parameters::names() {
            assert!(
                state.get(name).is_some(),
                "{name} missing from factory state"
            );
        }
    }

    #[test]
    fn set_changes_what_get_reports() {
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_channel", b"11"));
        assert_eq!(state.get("wireless_channel"), Some(b"11".as_slice()));
    }

    #[test]
    fn set_leaves_other_parameters_alone() {
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_channel", b"11"));
        assert_eq!(state.get("wireless_region_id"), Some(b"4".as_slice()));
    }

    #[test]
    fn reset_without_a_save_discards_the_change() {
        // Working memory reloads from NVRAM, which never saw the write.
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_channel", b"11"));
        state.apply_reset();
        assert_eq!(state.get("wireless_channel"), Some(b"6".as_slice()));
    }

    #[test]
    fn reset_after_a_save_keeps_the_change() {
        // The pair that gives save a meaning: the same reset yields a
        // different answer depending on whether save ran.
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_channel", b"11"));
        state.apply_save();
        state.apply_reset();
        assert_eq!(state.get("wireless_channel"), Some(b"11".as_slice()));
    }

    #[test]
    fn save_does_not_disturb_working_memory() {
        let mut state = DeviceState::factory();
        state.apply_set(one("hostname", b"bedroom"));
        state.apply_save();
        assert_eq!(state.get("hostname"), Some(b"bedroom".as_slice()));
    }

    #[test]
    fn a_non_utf8_value_survives_the_full_cycle() {
        // ADR-6, end to end through the state model.
        let ssid = vec![0xff, 0xfe, 0x41];
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_SSID", &ssid));
        state.apply_save();
        state.apply_reset();
        assert_eq!(state.get("wireless_SSID"), Some(ssid.as_slice()));
    }

    #[test]
    fn an_unknown_name_is_stored_but_does_not_displace_a_known_one() {
        let mut state = DeviceState::factory();
        state.apply_set(one("not_a_real_parameter", b"x"));
        assert_eq!(state.get("not_a_real_parameter"), Some(b"x".as_slice()));
        assert_eq!(state.get("wireless_channel"), Some(b"6".as_slice()));
    }
}
