pub mod inbound;
pub mod outbound;


/// Nanoseconds since a given time.
// Maintained as u64 to reduce footprint
// NOTE: this also implies that the rate limiter will manage checking if a batch is allowed for at
//       most <init time> + u64::MAX nanosecs, ~500 years. So it is realistic to assume this is fine.
type Nanosecs = u64;
