//! Initial configuration for a PhaseQueen node

use std::collections::HashMap;
use std::time::Duration;

use sawtooth_sdk::consensus::{
    engine::{BlockId, PeerId},
    service::Service,
};

use crate::timing::retry_until_ok;

/// Contains the initial configuration loaded from on-chain settings and local configuration. The
/// `members` list is required; all other settings are optional (defaults used in their absence)
#[derive(Debug)]
pub struct PhaseQueenConfig {
    // Members of the PhaseQueen network
    pub members: Vec<PeerId>,

    /// How long to wait in between trying to publish blocks
    pub block_publishing_delay: Duration,

    /// How long to wait for an update to arrive from the validator
    pub update_recv_timeout: Duration,

    /// The base time to use for retrying with exponential backoff
    pub exponential_retry_base: Duration,

    /// The maximum time for retrying with exponential backoff
    pub exponential_retry_max: Duration,

    /// Where to store PbftState ("memory" or "disk+/path/to/file")
    pub storage_location: String,
}

impl PhaseQueenConfig {
    pub fn default() -> Self {
        PhaseQueenConfig {
            members: Vec::new(),
            block_publishing_delay: Duration::from_millis(5000),
            update_recv_timeout: Duration::from_millis(10),
            exponential_retry_base: Duration::from_millis(100),
            exponential_retry_max: Duration::from_millis(60000),
            storage_location: "memory".into(),
        }
    }

    /// Load configuration from on-chain Sawtooth settings.
    ///
    /// Configuration loads the following settings:
    /// + `sawtooth.consensus.algorithm.members` (required)
    /// + `sawtooth.consensus.algorithm.block_publishing_delay` (optional, default 1000 ms)
    ///
    /// # Panics
    /// + If the `sawtooth.consensus.algorithm.members` setting is not provided or is invalid
    pub fn load_settings(&mut self, block_id: BlockId, service: &mut dyn Service) {
        debug!("Getting on-chain settings for config");
        let settings: HashMap<String, String> = retry_until_ok(
            self.exponential_retry_base,
            self.exponential_retry_max,
            || {
                service.get_settings(
                    block_id.clone(),
                    vec![
                        String::from("sawtooth.consensus.algorithm.members"),
                        String::from("sawtooth.consensus.algorithm.block_publishing_delay"),
                        String::from("sawtooth.consensus.algorithm.idle_timeout"),
                    ],
                )
            },
        );

        // Get the on-chain list of PhaseQueen members or panic if it is not provided; the network cannot
        // function without this setting, since there is no way of knowing which nodes are members.
        self.members = get_members_from_settings(&settings);

        // Get durations
        merge_millis_setting_if_set(
            &settings,
            &mut self.block_publishing_delay,
            "sawtooth.consensus.algorithm.block_publishing_delay",
        );
    }
}

fn merge_setting_if_set_and_map<U, F, T>(
    settings_map: &HashMap<String, String>,
    setting_field: &mut U,
    setting_key: &str,
    map: F,
) where
    F: Fn(T) -> U,
    T: ::std::str::FromStr,
{
    if let Some(setting) = settings_map.get(setting_key) {
        if let Ok(setting_value) = setting.parse() {
            *setting_field = map(setting_value);
        }
    }
}

fn merge_millis_setting_if_set(
    settings_map: &HashMap<String, String>,
    setting_field: &mut Duration,
    setting_key: &str,
) {
    merge_setting_if_set_and_map(
        settings_map,
        setting_field,
        setting_key,
        Duration::from_millis,
    )
}

/// Get the list of PhaseQueen members as a Vec<PeerId> from settings
///
/// # Panics
/// + If the `sawtooth.consensus.algorithm.members` setting is unset or invalid
pub fn get_members_from_settings<S: std::hash::BuildHasher>(
    settings: &HashMap<String, String, S>,
) -> Vec<PeerId> {
    let members_setting_value = settings
        .get("sawtooth.consensus.algorithm.members")
        .expect("'sawtooth.consensus.algorithm.members' is empty; this setting must exist to use PhaseQueen");

    let members: Vec<String> = serde_json::from_str(members_setting_value).unwrap_or_else(|err| {
        panic!(
            "Unable to parse value at 'sawtooth.consensus.algorithm.members' due to error: {:?}",
            err
        )
    });

    members
        .into_iter()
        .map(|s| {
            hex::decode(s).unwrap_or_else(|err| {
                panic!("Unable to parse PeerId from string due to error: {:?}", err)
            })
        })
        .collect()
}
