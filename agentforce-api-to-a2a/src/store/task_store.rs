//! In-memory (per-replica) task store.
//!
//! OS v2 persistence has been removed for now. The PDK shared cache
//! is the only backing layer; tasks live for `taskHotCacheTtlSeconds`
//! and disappear on policy reload or replica replacement.
//!
//! Two values are stored per task:
//!
//!   * `task:<taskId>`     - full A2A `Task` JSON.
//!   * `meta:<taskId>`     - small `{ sequenceId, lastUpdatedAt }` JSON.
//!
//! Hot-cache TTL is not enforced by the PDK cache (which is FIFO with a
//! fixed capacity), so we piggy-back on a small `expires_at_unix`
//! envelope.

use std::rc::Rc;

use pdk::cache::Cache;
use pdk::hl::HttpClient;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HotEnvelope {
    payload: Vec<u8>,
    expires_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMeta {
    pub sequence_id: u32,
    pub last_updated_at_unix: u64,
}

impl TaskMeta {
    pub fn next_sequence(&mut self, now_unix: u64) -> u32 {
        self.sequence_id = self.sequence_id.saturating_add(1);
        self.last_updated_at_unix = now_unix;
        self.sequence_id
    }

    pub fn initial(now_unix: u64) -> Self {
        Self {
            sequence_id: 1,
            last_updated_at_unix: now_unix,
        }
    }
}

pub struct TaskStore {
    hot: Rc<dyn Cache>,
    hot_ttl_seconds: u32,
}

impl TaskStore {
    pub fn new(hot: Rc<dyn Cache>, hot_ttl_seconds: u32) -> Self {
        Self {
            hot,
            hot_ttl_seconds,
        }
    }

    fn task_key(task_id: &str) -> String {
        format!("task:{task_id}")
    }
    fn meta_key(task_id: &str) -> String {
        format!("meta:{task_id}")
    }

    fn read_hot(&self, key: &str, now_unix: u64) -> Option<Vec<u8>> {
        let bytes = self.hot.get(key)?;
        let env: HotEnvelope = serde_json::from_slice(&bytes).ok()?;
        if env.expires_at_unix <= now_unix {
            // Stale entry; drop it lazily so we don't hold on to it.
            self.hot.delete(key);
            return None;
        }
        Some(env.payload)
    }

    fn write_hot(&self, key: &str, payload: &[u8], now_unix: u64) {
        let env = HotEnvelope {
            payload: payload.to_vec(),
            expires_at_unix: now_unix.saturating_add(self.hot_ttl_seconds as u64),
        };
        if let Ok(bytes) = serde_json::to_vec(&env) {
            let _ = self.hot.save(key, bytes);
        }
    }

    /// Get the full Task JSON. Returns `None` if not in the hot cache or
    /// the entry has expired. The `_client` parameter is unused now that
    /// OS v2 has been removed; kept on the signature so call-sites don't
    /// have to change while we iterate.
    pub async fn get_task(
        &self,
        _client: &HttpClient,
        task_id: &str,
        now_unix: u64,
    ) -> Option<Vec<u8>> {
        let key = Self::task_key(task_id);
        self.read_hot(&key, now_unix)
    }

    /// Persist the full Task JSON to the hot cache.
    pub async fn put_task(
        &self,
        _client: &HttpClient,
        task_id: &str,
        task_json: &[u8],
        now_unix: u64,
    ) {
        let key = Self::task_key(task_id);
        self.write_hot(&key, task_json, now_unix);
    }

    /// Drop the task from the hot cache. Reserved for a future
    /// tasks/expire flow; v1 keeps canceled tasks (with `state = canceled`)
    /// so clients can retrieve the last known status.
    #[allow(dead_code)]
    pub async fn delete_task(
        &self,
        _client: &HttpClient,
        task_id: &str,
        _now_unix: u64,
    ) {
        let key = Self::task_key(task_id);
        self.hot.delete(&key);
        let mkey = Self::meta_key(task_id);
        self.hot.delete(&mkey);
    }

    /// Read the meta sidecar (sequence counter). Missing => return `None`
    /// so the caller starts a fresh sequence.
    pub async fn get_meta(
        &self,
        _client: &HttpClient,
        task_id: &str,
        now_unix: u64,
    ) -> Option<TaskMeta> {
        let key = Self::meta_key(task_id);
        let bytes = self.read_hot(&key, now_unix)?;
        serde_json::from_slice::<TaskMeta>(&bytes).ok()
    }

    /// Persist the meta sidecar to the hot cache.
    pub async fn put_meta(
        &self,
        _client: &HttpClient,
        task_id: &str,
        meta: &TaskMeta,
        now_unix: u64,
    ) {
        let key = Self::meta_key(task_id);
        let bytes = match serde_json::to_vec(meta) {
            Ok(b) => b,
            Err(_) => return,
        };
        self.write_hot(&key, &bytes, now_unix);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_meta_initial_starts_at_one() {
        let m = TaskMeta::initial(100);
        assert_eq!(m.sequence_id, 1);
        assert_eq!(m.last_updated_at_unix, 100);
    }

    #[test]
    fn task_meta_next_sequence_increments_and_stamps() {
        let mut m = TaskMeta::initial(100);
        let n1 = m.next_sequence(110);
        let n2 = m.next_sequence(120);
        assert_eq!(n1, 2);
        assert_eq!(n2, 3);
        assert_eq!(m.last_updated_at_unix, 120);
    }

    #[test]
    fn hot_envelope_serializes_round_trip() {
        let env = HotEnvelope {
            payload: vec![1, 2, 3],
            expires_at_unix: 999,
        };
        let bytes = serde_json::to_vec(&env).unwrap();
        let back: HotEnvelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.payload, vec![1, 2, 3]);
        assert_eq!(back.expires_at_unix, 999);
    }
}
