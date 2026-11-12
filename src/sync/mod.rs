//! Selecting repositories, running workers and collecting results.
//!
//! M2 adds sequential sync; M4 wraps `worker::sync_one` in the worker pool.

pub mod hooks;
pub mod worker;
