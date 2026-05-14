//! Task state storage.
//!
//! `task_store` is currently backed by the per-replica PDK shared cache
//! only. Anypoint Object Store v2 persistence has been removed for now;
//! tasks live for `taskHotCacheTtlSeconds` and disappear on policy
//! reload or replica replacement.

pub mod task_store;
