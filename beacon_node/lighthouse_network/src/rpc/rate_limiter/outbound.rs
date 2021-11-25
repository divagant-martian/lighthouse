use std::hash::Hash;
use std::time::{Duration, Instant};

use fnv::FnvHashMap;

use super::Nanosecs;

pub struct Limiter<Key: Hash + Eq + Clone> {
    /// Rate given to new keys.
    initial_rate: u64,
    /// Max permited rate for each key.
    max_rate: u64,
    /// After how long is the bucket considered full via replenishing 1T every `t`.
    token_increase: u64,
    /// Min time to wait since the last time the rate was increased .
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

        let total_tokens = key_info.tokens_in_use + tokens;
        if total_tokens <= key_info.rate {
            // Tokens available
            key_info.tokens_in_use = total_tokens;
            Ok(())
        } else {
            // Not enought tokens. Check if increasing the rate is allowed and enought to permit
            // the batch.
            let new_rate = (key_info.rate + self.token_increase).min(self.max_rate);
            if total_tokens <= new_rate
                && key_info.time_of_last_increase + self.min_increase_interval <= time_since_start
            {
                key_info.time_of_last_increase = time_since_start;
                key_info.tokens_in_use = total_tokens;
                key_info.rate = new_rate;
                Ok(())
            } else {
                Err(String::from("not enought tokens."))
            }
        }
    }

    pub fn replenish(&mut self, key: &Key, tokens: u64) -> Result<(), ()> {
        let key_info = self.key_info.get_mut(key).ok_or(())?;
        key_info.tokens_in_use = key_info.tokens_in_use.checked_sub(tokens).ok_or(())?;
        Ok(())
    }

    pub fn use_rejected(&mut self, key: &Key, tokens: u64) -> Result<(), ()> {
        let key_info = self.key_info.get_mut(key).ok_or(())?;
        // key_info.tokens_in_use
        // TODO: backoff
        Ok(())

    }

    #[cfg(test)]
    fn allows_f32s(
        &mut self,
        seconds_since_start: f32,
        key: &Key,
        tokens: u64,
    ) -> Result<(), String> {
        self.allows(Duration::from_secs_f32(seconds_since_start), key, tokens)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn rate_limit_without_replenish() {
        let mut limiter = Limiter {
            initial_rate: 4,
            key_info: FnvHashMap::default(),
            max_rate: 7,
            token_increase: 2,
            min_increase_interval: Duration::from_secs(1).as_nanos() as u64,
        };
        let key = 10;
        //             7-|                x  <= max rate
        //             6-|          x  x  x
        //             5-|       x  x  x  x
        //             4-| x  x  x  x  x  x
        // concurrent  3-| x  x  x  x  x  x
        //   tokens    2-| x  x  x  x  x  x
        //   in use    1-| x  x  x  x  x  x
        //                 +--+--+--+--+--+--> seconds
        //                 |  |  |  |  |  |
        //                 0     1     2

        // 4 concurrent tokens in use
        assert!(limiter.allows_f32s(0.0, &key, 4).is_ok());
        // no space left and it's too soon to request more.
        assert!(limiter.allows_f32s(0.5, &key, 1).is_err());
        // ready to request more: 5 concurrent tokens in use.
        assert!(limiter.allows_f32s(1.0, &key, 1).is_ok());
        // current rate allows an additional token: 6 concurrent tokens in use.
        assert!(limiter.allows_f32s(1.5, &key, 1).is_ok());
        // no space left and it's too soon to request more.
        assert!(limiter.allows_f32s(1.5, &key, 1).is_err());
        // batch is too large for the current rate and token increase, even if the rate could
        // increase now.
        assert!(limiter.allows_f32s(2.0, &key, 3).is_err());
        // rate could be increased to 8 since the time has passed and the token_increase allows to
        // fit this batch. Hoever it exceeds the max rate.
        assert!(limiter.allows_f32s(2.1, &key, 2).is_err());
        // However 1 token would allow to increase the rate to the max
        assert!(limiter.allows_f32s(2.5, &key, 1).is_ok());
        // Long time has passed without replenish. The batch is rejected.
        assert!(limiter.allows_f32s(4.0, &key, 1).is_err());
    }

    #[test]
    fn rate_limit_with_replenish() {
        let mut limiter = Limiter {
            initial_rate: 4,
            key_info: FnvHashMap::default(),
            max_rate: 7,
            token_increase: 2,
            min_increase_interval: Duration::from_secs(1).as_nanos() as u64,
        };
        let key = 10;
        //             7-|             x  <= max rate
        //             6-|       x  x  x
        //             5-|       x  x  x
        //             4-| x     x  x  x
        // concurrent  3-| x  x  x  x  x
        //   tokens    2-| x  x  x  x  x
        //   in use    1-| x  x  x  x  x
        //                 +--+--+--+--+--> seconds
        //                 |  |  |  |  |
        //                 0     1     2

        // 4 concurrent tokens in use, then 2 are given back.
        assert!(limiter.allows_f32s(0.0, &key, 4).is_ok());
        assert!(limiter.replenish(&key, 2).is_ok());
        // 1 more token fits the rate
        assert!(limiter.allows_f32s(0.5, &key, 1).is_ok());
        // 3 can be allowed if the rate is increased, which it can.
        assert!(limiter.allows_f32s(1.0, &key, 3).is_ok());
        // too soon for a rate increase
        assert!(limiter.allows_f32s(1.5, &key, 1).is_err());
        // rate now can be increased
        assert!(limiter.allows_f32s(2.0, &key, 1).is_ok());
        // max was reached so even after more time, this is rejected
        assert!(limiter.allows_f32s(9.0, &key, 1).is_err());
    }
}
