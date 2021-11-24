use std::hash::Hash;
use std::time::{Duration, Instant};

use fnv::FnvHashMap;

use super::Nanosecs;

pub struct Limiter<Key: Hash + Eq + Clone> {
    initial_rate: u64,
    /// After how long is the bucket considered full via replenishing 1T every `t`.
    increase_count: u64,
    /// How often is 1T replenished.
    min_increase_interval: Nanosecs,
    /// Rate for each key and current token usage.
    key_info: FnvHashMap<Key, KeyInfo>,
}

struct KeyInfo {
    /// How many tokens are in use for this key.
    tokens_in_use: u64,
    /// Max number of tokens we allow ourselves to use simultaneously for this key.
    rate: u64,
    /// Time at which the last increase ocurred.
    time_of_last_increase: Nanosecs,
}

impl KeyInfo {
    pub fn new(initial_rate: u64, time_since_start: Nanosecs) -> Self {
        KeyInfo {
            tokens_in_use: 0,
            rate: initial_rate,
            time_of_last_increase: time_since_start,
        }
    }
}

impl<Key: Hash + Eq + Clone> Limiter<Key> {
    pub fn allows(
        &mut self,
        time_since_start: Duration,
        key: &Key,
        tokens: u64,
    ) -> Result<(), String> {
        let time_since_start = time_since_start.as_nanos() as u64;
        let initial_rate = self.initial_rate;
        let key_info = self
            .key_info
            .entry(key.clone())
            .or_insert_with(|| KeyInfo::new(initial_rate, time_since_start));

        if key_info.tokens_in_use + tokens <= key_info.rate {
            key_info.tokens_in_use += tokens;
        } else if key_info.time_of_last_increase + self.min_increase_interval < time_since_start {
            if key_info.tokens_in_use + tokens <= key_info.rate + self.increase_count {
                // maybe have a max flat rate
            }
        }

        Ok(())
    }
}
