//! Read-through / write-through task store.
//!
//! The PDK shared cache acts as a per-replica latency layer in front of
//! Anypoint Object Store v2. OS v2 is the persistent source of truth; the
//! hot cache only avoids repeat round-trips for follow-on requests landing
//! on the same replica within `taskHotCacheTtlSeconds`.
//!
//! Two values are stored per task:
//!
//!   * `task:<taskId>`     - full A2A `Task` JSON.
//!   * `meta:<taskId>`     - small `{ sequenceId, lastUpdatedAt }` JSON.
//!
//! The hot cache and OS v2 share the same key string. Hot-cache TTL is not
//! enforced by the PDK cache (which is FIFO with a fixed capacity), so we
//! piggy-back on a small `expires_at_unix` envelope.

use std::rc::Rc;

use pdk::cache::Cache;
use pdk::hl::HttpClient;
use pdk::logger;
use serde::{Deserialize, Serialize};

use crate::store::object_store_v2::{GetOutcome, ObjectStoreV2};

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
    os2: Rc<ObjectStoreV2>,
    hot_ttl_seconds: u32,
}

impl TaskStore {
    pub fn new(hot: Rc<dyn Cache>, os2: Rc<ObjectStoreV2>, hot_ttl_seconds: u32) -> Self {
        Self {
            hot,
            os2,
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

    /// Get the full Task JSON. Returns `None` if not found anywhere; OS v2
    /// outages are treated as "not found".
    pub async fn get_task(
        &self,
        client: &HttpClient,
        task_id: &str,
        now_unix: u64,
    ) -> Option<Vec<u8>> {
        let key = Self::task_key(task_id);
        if let Some(b) = self.read_hot(&key, now_unix) {
            logger::debug!("task-store: hot hit for {task_id}");
            return Some(b);
        }
        match self.os2.get(client, &key, now_unix).await {
            GetOutcome::Found(bytes) => {
                self.write_hot(&key, &bytes, now_unix);
                Some(bytes)
            }
            GetOutcome::NotFound => None,
            GetOutcome::Degraded => None,
        }
    }

    /// Persist the full Task JSON. Hot cache is updated synchronously; OS
    /// v2 PUT is best-effort.
    pub async fn put_task(
        &self,
        client: &HttpClient,
        task_id: &str,
        task_json: &[u8],
        now_unix: u64,
    ) {
        let key = Self::task_key(task_id);
        self.write_hot(&key, task_json, now_unix);
        self.os2.put(client, &key, task_json, now_unix).await;
    }

    /// Drop the task from both layers. Reserved for a future tasks/expire
    /// flow; v1 keeps canceled tasks (with `state = canceled`) so clients
    /// can retrieve the last known status.
    #[allow(dead_code)]
    pub async fn delete_task(
        &self,
        client: &HttpClient,
        task_id: &str,
        now_unix: u64,
    ) {
        let key = Self::task_key(task_id);
        self.hot.delete(&key);
        self.os2.delete(client, &key, now_unix).await;
        let mkey = Self::meta_key(task_id);
        self.hot.delete(&mkey);
        self.os2.delete(client, &mkey, now_unix).await;
    }

    /// Read the meta sidecar (sequence counter). Missing => return `None`
    /// so the caller starts a fresh sequence.
    pub async fn get_meta(
        &self,
        client: &HttpClient,
        task_id: &str,
        now_unix: u64,
    ) -> Option<TaskMeta> {
        let key = Self::meta_key(task_id);
        if let Some(bytes) = self.read_hot(&key, now_unix) {
            if let Ok(m) = serde_json::from_slice::<TaskMeta>(&bytes) {
                return Some(m);
            }
        }
        match self.os2.get(client, &key, now_unix).await {
            GetOutcome::Found(bytes) => {
                let parsed: Option<TaskMeta> = serde_json::from_slice(&bytes).ok();
                if let Some(ref m) = parsed {
                    if let Ok(b) = serde_json::to_vec(m) {
                        self.write_hot(&key, &b, now_unix);
                    }
                }
                parsed
            }
            GetOutcome::NotFound | GetOutcome::Degraded => None,
        }
    }

    /// Persist the meta sidecar. Hot cache + best-effort OS v2 PUT.
    pub async fn put_meta(
        &self,
        client: &HttpClient,
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
        self.os2.put(client, &key, &bytes, now_unix).await;
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
