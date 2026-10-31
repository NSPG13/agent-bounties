//! Optional process-local spacing of read-only HTTP attempt starts.
//! Not a provider quota, compute-unit budget or cross-process reservation.
use crate::ChainBaseError;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    sync::{Mutex as AsyncMutex, Semaphore},
    time::Instant,
};

const MAX_ENDPOINTS: usize = 128;
const MAX_WAITERS: usize = 64;
const MAX_WAIT: Duration = Duration::from_secs(2);
const SETTING: &str = "BASE_RPC_READ_WIRE_INTERVAL_MS";
type Key = [u8; 32];

#[derive(Debug, Clone)]
pub(super) enum Pacer {
    Disabled,
    Invalid,
    Enabled {
        interval: Duration,
        registry: Arc<Mutex<Registry>>,
    },
}
#[derive(Debug, Default)]
pub(super) struct Registry {
    entries: HashMap<Key, Arc<Entry>>,
}
#[derive(Debug)]
struct Entry {
    next: Arc<AsyncMutex<Instant>>,
    callers: Arc<Semaphore>,
}
static PROCESS: OnceLock<Pacer> = OnceLock::new();

fn unavailable(reason: &'static str) -> ChainBaseError {
    // Local capacity/configuration errors must not invent a provider HTTP429.
    ChainBaseError::RpcTransport(reason.into())
}
impl Pacer {
    pub fn process() -> Self {
        PROCESS
            .get_or_init(|| match std::env::var(SETTING) {
                Ok(value) => Self::parse(Some(&value)),
                Err(std::env::VarError::NotPresent) => Self::Disabled,
                Err(_) => Self::Invalid,
            })
            .clone()
    }
    fn parse(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::Disabled;
        };
        if raw == "0" {
            return Self::Disabled;
        }
        // No raw configuration is retained or included in diagnostics.
        if raw.is_empty() || !raw.bytes().all(|c| c.is_ascii_digit()) {
            return Self::Invalid;
        }
        match raw.parse::<u64>() {
            Ok(ms @ 10..=1000) => Self::Enabled {
                interval: Duration::from_millis(ms),
                registry: Arc::new(Mutex::new(Registry::default())),
            },
            _ => Self::Invalid,
        }
    }
    pub async fn wait(&self, endpoint: &str) -> Result<(), ChainBaseError> {
        let (interval, registry) = match self {
            Self::Disabled => return Ok(()),
            Self::Invalid => return Err(unavailable("RPC read pacing configuration unavailable")),
            Self::Enabled { interval, registry } => (*interval, registry),
        };
        let key: Key = Sha256::digest(endpoint.as_bytes()).into();
        let entry = {
            let mut state = registry
                .lock()
                .map_err(|_| unavailable("RPC read pacing registry unavailable"))?;
            if !state.entries.contains_key(&key) && state.entries.len() >= MAX_ENDPOINTS {
                // Only idle, elapsed entries can be discarded. Never discard a
                // waiting caller or a recent admission to create another burst.
                state.entries.retain(|_, value| {
                    Arc::strong_count(value) > 1
                        || value
                            .next
                            .try_lock()
                            .map_or(true, |next| *next > Instant::now())
                });
                if state.entries.len() >= MAX_ENDPOINTS {
                    return Err(unavailable("RPC read pacing endpoint capacity unavailable"));
                }
            }
            state
                .entries
                .entry(key)
                .or_insert_with(|| {
                    Arc::new(Entry {
                        next: Arc::new(AsyncMutex::new(Instant::now())),
                        callers: Arc::new(Semaphore::new(MAX_WAITERS)),
                    })
                })
                .clone()
        };
        let _caller = entry
            .callers
            .clone()
            .try_acquire_owned()
            .map_err(|_| unavailable("RPC read pacing caller capacity unavailable"))?;
        tokio::time::timeout(MAX_WAIT, async {
            let mut next = entry.next.lock().await;
            tokio::time::sleep_until(*next).await;
            // Reserve only at admission, never enqueue future timestamps. A
            // canceled waiting future drops both locks/permits without a gap.
            *next = Instant::now() + interval;
        })
        .await
        .map_err(|_| unavailable("RPC read pacing wait deadline elapsed"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
