//! Persistent task storage.
//!
//! `object_store_v2` talks to Anypoint Object Store v2 (the persistent
//! source of truth) and `task_store` is the read-through / write-through
//! wrapper that fronts OS v2 with the PDK shared cache for low-latency
//! repeat reads.

pub mod object_store_v2;
pub mod task_store;
