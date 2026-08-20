//! The sale state machine: a buy, a sell, or a close applied to the sale's own data.
//!
//! Knows about virtual reserves, the two accounting buckets, fees, slippage, and the
//! open or closed flag. Knows nothing about `AccountWithMetadata`, PDAs, or
//! `ChainedCall`, which is what lets the solvency property test run against a state
//! machine instead of hand-built account fixtures.
//!
//! Filled in by GTM-508, GTM-510, GTM-511 and GTM-512. GTM-514 targets this crate.
