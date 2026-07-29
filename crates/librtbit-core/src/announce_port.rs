use std::{
    num::NonZeroU16,
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
};

/// Runtime-mutable port advertised to BitTorrent peer-discovery services.
///
/// A zero atomic value represents a disabled announcement. Clones share the
/// same value, allowing tracker, DHT, and LSD workers to observe lease changes
/// without being restarted.
#[derive(Clone, Debug, Default)]
pub struct AnnouncePort(Arc<AtomicU16>);

impl AnnouncePort {
    pub fn new(port: Option<NonZeroU16>) -> Self {
        Self(Arc::new(AtomicU16::new(
            port.map(NonZeroU16::get).unwrap_or_default(),
        )))
    }

    pub fn get(&self) -> Option<NonZeroU16> {
        NonZeroU16::new(self.0.load(Ordering::Acquire))
    }

    pub fn set(&self, port: NonZeroU16) {
        self.0.store(port.get(), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::AnnouncePort;
    use std::num::NonZeroU16;
    use std::thread;

    #[test]
    fn clones_observe_runtime_updates() {
        let port = AnnouncePort::new(NonZeroU16::new(4241));
        let worker_port = port.clone();

        port.set(NonZeroU16::new(51_234).unwrap());

        assert_eq!(worker_port.get().map(NonZeroU16::get), Some(51_234));
    }

    #[test]
    fn concurrent_readers_observe_only_complete_updates() {
        let port = AnnouncePort::new(NonZeroU16::new(4241));
        let readers = (0..4)
            .map(|_| {
                let port = port.clone();
                thread::spawn(move || {
                    for _ in 0..10_000 {
                        let observed = port.get().map(NonZeroU16::get).unwrap();
                        assert!(matches!(observed, 4241 | 51_234));
                    }
                })
            })
            .collect::<Vec<_>>();

        for _ in 0..10_000 {
            port.set(NonZeroU16::new(51_234).unwrap());
            port.set(NonZeroU16::new(4241).unwrap());
        }
        port.set(NonZeroU16::new(51_234).unwrap());

        for reader in readers {
            reader.join().unwrap();
        }
        assert_eq!(port.get().map(NonZeroU16::get), Some(51_234));
    }
}
