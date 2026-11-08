//! What the media and preview loops need from a phone connection. `Outbox` is the real one;
//! tests use a channel, because an `Outbox` can only be made by the agent.
use secpar_host_core::{
    agent::{BoxFut, Outbox},
    proto::ServerMsg,
};

pub trait Push: Clone + Send + Sync + 'static {
    /// False once the connection is gone.
    fn msg(&self, m: ServerMsg) -> bool;
    fn bin(&self, b: Vec<u8>) -> BoxFut<bool>;
    fn closed(&self) -> bool;
}

impl Push for Outbox {
    fn msg(&self, m: ServerMsg) -> bool {
        self.send(m)
    }
    fn bin(&self, b: Vec<u8>) -> BoxFut<bool> {
        let out = self.clone();
        Box::pin(async move { out.send_bin(b).await })
    }
    fn closed(&self) -> bool {
        self.is_closed()
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    /// Records everything sent; `close()` makes it behave like a dropped connection.
    #[derive(Clone, Default)]
    pub struct Fake {
        pub msgs: Arc<Mutex<Vec<ServerMsg>>>,
        pub bins: Arc<Mutex<Vec<Vec<u8>>>>,
        pub gone: Arc<AtomicBool>,
    }

    impl Fake {
        pub fn close(&self) {
            self.gone.store(true, Ordering::SeqCst);
        }
    }

    impl Push for Fake {
        fn msg(&self, m: ServerMsg) -> bool {
            if self.gone.load(Ordering::SeqCst) {
                return false;
            }
            self.msgs.lock().unwrap().push(m);
            true
        }
        fn bin(&self, b: Vec<u8>) -> BoxFut<bool> {
            let alive = !self.gone.load(Ordering::SeqCst);
            if alive {
                self.bins.lock().unwrap().push(b);
            }
            Box::pin(async move { alive })
        }
        fn closed(&self) -> bool {
            self.gone.load(Ordering::SeqCst)
        }
    }
}
