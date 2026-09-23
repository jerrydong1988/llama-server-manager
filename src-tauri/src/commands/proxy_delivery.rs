//! Bound client-controlled receive/delivery lifetimes independently of inference.
use super::*;
use std::sync::atomic::AtomicUsize;

pub(super) const UPLOAD_IDLE: Duration = Duration::from_secs(15);
pub(super) const UPLOAD_TOTAL: Duration = Duration::from_secs(120);
pub(super) const DELIVERY_IDLE: Duration = Duration::from_secs(30);
const DELIVERY_CHUNK: usize = 64 * 1024;
const RESPONSE_BUDGET: usize = 256 * 1024 * 1024;
static RESPONSE_BYTES: std::sync::LazyLock<Arc<AtomicUsize>> =
    std::sync::LazyLock::new(|| Arc::new(AtomicUsize::new(0)));

pub(super) async fn receive(
    body: Body,
    limit: usize,
    idle: Duration,
    total: Duration,
) -> Result<Bytes, StatusCode> {
    let mut stream = body.into_data_stream();
    let receive = async {
        let mut bytes = bytes::BytesMut::new();
        loop {
            match tokio::time::timeout(idle, stream.next()).await {
                Ok(Some(Ok(chunk))) => {
                    if chunk.len() > limit.saturating_sub(bytes.len()) {
                        return Err(StatusCode::PAYLOAD_TOO_LARGE);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(Some(Err(_))) => return Err(StatusCode::BAD_REQUEST),
                Ok(None) => return Ok(bytes.freeze()),
                Err(_) => return Err(StatusCode::REQUEST_TIMEOUT),
            }
        }
    };
    tokio::time::timeout(total, receive)
        .await
        .unwrap_or(Err(StatusCode::REQUEST_TIMEOUT))
}

pub(super) struct ResponseReservation(usize, Arc<AtomicUsize>);
impl ResponseReservation {
    pub(super) fn acquire(bytes: usize) -> Option<Self> {
        Self::acquire_from(bytes, &RESPONSE_BYTES)
    }
    fn acquire_from(bytes: usize, counter: &Arc<AtomicUsize>) -> Option<Self> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(bytes)
                    .filter(|next| *next <= RESPONSE_BUDGET)
            })
            .ok()
            .map(|_| Self(bytes, counter.clone()))
    }
    pub(super) fn resize(&mut self, bytes: usize) -> bool {
        if bytes <= self.0 {
            self.1.fetch_sub(self.0 - bytes, Ordering::AcqRel);
            self.0 = bytes;
            return true;
        }
        let delta = bytes - self.0;
        if self
            .1
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(delta).filter(|n| *n <= RESPONSE_BUDGET)
            })
            .is_err()
        {
            return false;
        }
        self.0 = bytes;
        true
    }
    pub(super) fn attach(mut self, data: Bytes) -> Bytes {
        assert!(data.len() <= self.0);
        // A compact owned slice prevents a tiny response retaining a large Vec capacity.
        let compact = Bytes::from(data.to_vec().into_boxed_slice());
        drop(data);
        self.resize(compact.len());
        Bytes::from_owner(ReservedBytes {
            data: compact,
            _reservation: self,
        })
    }
}
impl Drop for ResponseReservation {
    fn drop(&mut self) {
        self.1.fetch_sub(self.0, Ordering::AcqRel);
    }
}
struct ReservedBytes {
    data: Bytes,
    _reservation: ResponseReservation,
}
impl AsRef<[u8]> for ReservedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Default)]
struct DeliveryState {
    chunk: Option<Result<Bytes, std::io::Error>>,
    done: bool,
}
#[derive(Default)]
struct DeliveryQueue {
    state: Mutex<DeliveryState>,
    ready: tokio::sync::Notify,
    consumed: tokio::sync::Notify,
}
impl DeliveryQueue {
    async fn drained(&self) {
        loop {
            let notified = self.consumed.notified();
            if self.state.lock().unwrap().chunk.is_none() {
                return;
            }
            notified.await;
        }
    }
    fn finish(&self, error: Option<std::io::Error>) {
        let mut state = self.state.lock().unwrap();
        if error.is_some() {
            state.chunk = error.map(Err);
        }
        state.done = true;
        drop(state);
        self.ready.notify_one();
    }
}
struct Delivery {
    queue: Arc<DeliveryQueue>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Delivery {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub(super) fn bounded_delivery(body: Body, idle: Duration, usage: Option<UsageHandle>) -> Body {
    let queue = Arc::new(DeliveryQueue::default());
    let producer = queue.clone();
    let task = tokio::spawn(async move {
        let mut stream = body.into_data_stream();
        while let Some(next) = stream.next().await {
            let bytes = match next {
                Ok(bytes) => bytes,
                Err(error) => {
                    producer.finish(Some(std::io::Error::other(error.to_string())));
                    return;
                }
            };
            for offset in (0..bytes.len()).step_by(DELIVERY_CHUNK) {
                let slice = &bytes[offset..(offset + DELIVERY_CHUNK).min(bytes.len())];
                let Some(reservation) = ResponseReservation::acquire(slice.len() * 2) else {
                    producer.finish(Some(std::io::Error::other(
                        "response memory budget exhausted",
                    )));
                    return;
                };
                // A downstream-held chunk must not pin the entire buffered response.
                let chunk = reservation.attach(Bytes::copy_from_slice(slice));
                producer.state.lock().unwrap().chunk = Some(Ok(chunk));
                producer.ready.notify_one();
                if tokio::time::timeout(idle, producer.drained())
                    .await
                    .is_err()
                {
                    if let Some(usage) = &usage {
                        usage.failure("delivery", "response_timeout");
                    }
                    // Clears queued bytes even when the consumer never polls again.
                    producer.finish(Some(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "downstream response stalled",
                    )));
                    return;
                }
            }
        }
        producer.finish(None);
    });
    Body::from_stream(futures_util::stream::unfold(
        Delivery { queue, task },
        |delivery| async move {
            loop {
                let notified = delivery.queue.ready.notified();
                let (chunk, done) = {
                    let mut state = delivery.queue.state.lock().unwrap();
                    (state.chunk.take(), state.done)
                };
                if let Some(chunk) = chunk {
                    delivery.queue.consumed.notify_one();
                    drop(notified);
                    return Some((chunk, delivery));
                }
                if done {
                    return None;
                }
                notified.await;
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unpolled_json_releases_its_reservation_after_delivery_timeout() {
        for size in [128, DELIVERY_CHUNK * 2] {
            let counter = Arc::new(AtomicUsize::new(0));
            let bytes = ResponseReservation::acquire_from(size * 2, &counter)
                .unwrap()
                .attach(Bytes::from(vec![b'x'; size]));
            let body = bounded_delivery(Body::from(bytes), Duration::from_millis(20), None);
            tokio::time::timeout(Duration::from_secs(2), async {
                while counter.load(Ordering::Acquire) != 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            assert!(
                receive(body, size, Duration::from_secs(1), Duration::from_secs(2))
                    .await
                    .is_err()
            );
        }
    }
    #[tokio::test]
    async fn incomplete_upload_expires_and_regular_body_is_preserved() {
        let delay = Duration::from_millis(20);
        let pending =
            Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
        assert_eq!(
            receive(pending, 1024, delay, delay * 3).await,
            Err(StatusCode::REQUEST_TIMEOUT)
        );
        assert_eq!(
            receive(Body::from("normal"), 1024, delay, delay * 3)
                .await
                .unwrap(),
            "normal"
        );
        assert_eq!(
            receive(Body::from("oversize"), 3, delay, delay * 3).await,
            Err(StatusCode::PAYLOAD_TOO_LARGE)
        );
        let slow = futures_util::stream::unfold((), |_| async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            Some((Ok::<_, std::io::Error>(Bytes::from_static(b"x")), ()))
        });
        assert_eq!(
            receive(Body::from_stream(slow), 1024, delay, delay * 2).await,
            Err(StatusCode::REQUEST_TIMEOUT)
        );
    }
    #[tokio::test]
    async fn stalled_delivery_drops_source_without_downstream_polling() {
        struct Dropped(Arc<AtomicUsize>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        let source = futures_util::stream::unfold(Dropped(dropped.clone()), |guard| async {
            Some((
                Ok::<_, std::io::Error>(Bytes::from(vec![b'x'; DELIVERY_CHUNK])),
                guard,
            ))
        });
        let body = bounded_delivery(Body::from_stream(source), Duration::from_millis(20), None);
        tokio::time::timeout(Duration::from_secs(1), async {
            while dropped.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(receive(
            body,
            DELIVERY_CHUNK * 4,
            Duration::from_secs(1),
            Duration::from_secs(2)
        )
        .await
        .is_err());
        let body = bounded_delivery(Body::from("ordinary"), Duration::from_secs(1), None);
        assert_eq!(
            receive(body, 100, Duration::from_secs(1), Duration::from_secs(2))
                .await
                .unwrap(),
            "ordinary"
        );
    }
    #[test]
    fn response_budget_survives_byte_slices_and_clones() {
        let counter = Arc::new(AtomicUsize::new(0));
        let reserve = |n| ResponseReservation::acquire_from(n, &counter);
        let reservation = reserve(RESPONSE_BUDGET).unwrap();
        assert!(reserve(1).is_none());
        let bytes = reservation.attach(Bytes::from_static(b"retained"));
        let remaining = reserve(RESPONSE_BUDGET - bytes.len()).unwrap();
        let clone = bytes.slice(1..);
        drop(bytes);
        assert!(reserve(1).is_none());
        drop(clone);
        drop(remaining);
        assert!(reserve(RESPONSE_BUDGET).is_some());
    }
}
