//! Smooth bounded recovery reads without retrying or extending their deadline.
use super::*;
use std::time::Duration;
use tokio::{sync::Mutex, time::Instant};

pub(super) struct PacedReads<'a, T: JsonRpcTransport + ?Sized> {
    inner: &'a T,
    endpoints: [&'a str; 2],
    next: [Mutex<Instant>; 2],
    interval: Duration,
}

impl<'a, T: JsonRpcTransport + ?Sized> PacedReads<'a, T> {
    pub(super) fn new(inner: &'a T, endpoints: [&'a str; 2]) -> Self {
        Self {
            inner,
            endpoints,
            next: std::array::from_fn(|_| Mutex::new(Instant::now())),
            interval: Duration::from_millis(100),
        }
    }
}

#[async_trait::async_trait]
impl<T: JsonRpcTransport + ?Sized> JsonRpcTransport for PacedReads<'_, T> {
    async fn post_json_value(&self, rpc: &str, request: &Value) -> Result<Value, ChainBaseError> {
        let index = self
            .endpoints
            .iter()
            .position(|endpoint| *endpoint == rpc)
            .ok_or_else(unavailable)?;
        // Only the private read allowlist above can reach this wrapper. Keep a
        // second boundary here so it can never delay or dispatch a mutation.
        if !matches!(
            request["method"].as_str(),
            Some(
                "eth_chainId"
                    | "eth_getBlockByNumber"
                    | "eth_call"
                    | "eth_getCode"
                    | "eth_getTransactionCount"
                    | "eth_getTransactionByHash"
                    | "eth_getTransactionReceipt"
                    | "eth_estimateGas"
                    | "eth_getBalance"
            )
        ) {
            return Err(unavailable());
        }
        {
            let mut next = self.next[index].lock().await;
            tokio::time::sleep_until(*next).await;
            // Reserve only when dispatch is imminent. Cancellation while waiting
            // leaves no task, request, or future slot behind.
            *next = Instant::now() + self.interval;
        }
        // The original transport still enforces Retry-After, admission and no
        // redirects. This wrapper adds no retries; existing bounded read retries
        // remain in the transport. The enclosing stage bounds all waits.
        self.inner.post_json_value(rpc, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    #[derive(Default)]
    struct Recorded(StdMutex<Vec<(String, Instant)>>);
    #[async_trait::async_trait]
    impl JsonRpcTransport for Recorded {
        async fn post_json_value(&self, rpc: &str, _: &Value) -> Result<Value, ChainBaseError> {
            self.0.lock().unwrap().push((rpc.into(), Instant::now()));
            Err(unavailable())
        }
    }
    fn request() -> Value {
        json!({"method":"eth_chainId"})
    }

    #[tokio::test]
    async fn concurrent_reads_are_spaced_per_provider_and_errors_are_not_retried() {
        let recorded = Recorded::default();
        let mut paced = PacedReads::new(&recorded, ["a", "b"]);
        paced.interval = Duration::from_millis(20);
        let req = request();
        let results = tokio::join!(
            paced.post_json_value("a", &req),
            paced.post_json_value("a", &req),
            paced.post_json_value("b", &req)
        );
        assert!(results.0.is_err() && results.1.is_err() && results.2.is_err());
        let events = recorded.0.lock().unwrap();
        assert_eq!(events.len(), 3);
        let a: Vec<_> = events
            .iter()
            .filter(|(endpoint, _)| endpoint == "a")
            .collect();
        assert!(a[1].1.duration_since(a[0].1) >= paced.interval);
        assert_eq!(
            events
                .iter()
                .filter(|(endpoint, _)| endpoint == "b")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn cancellation_during_pacing_dispatches_nothing_and_does_not_reserve_another_slot() {
        let recorded = Recorded::default();
        let mut paced = PacedReads::new(&recorded, ["a", "b"]);
        paced.interval = Duration::from_millis(100);
        let req = request();
        assert!(paced.post_json_value("a", &req).await.is_err());
        let reserved = *paced.next[0].lock().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(1), paced.post_json_value("a", &req))
                .await
                .is_err()
        );
        assert_eq!(*paced.next[0].lock().await, reserved);
        assert_eq!(recorded.0.lock().unwrap().len(), 1);
        assert!(paced.post_json_value("other", &req).await.is_err());
        assert!(paced
            .post_json_value("b", &json!({"method":"eth_sendRawTransaction"}))
            .await
            .is_err());
        assert_eq!(recorded.0.lock().unwrap().len(), 1);
    }
}
