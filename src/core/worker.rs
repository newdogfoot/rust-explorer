use crossbeam_channel::{Sender, bounded};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default, Debug)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> std::io::Result<()> {
        if self.cancelled() {
            Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "작업을 취소했습니다",
            ))
        } else {
            Ok(())
        }
    }
}
type Task = Box<dyn FnOnce() + Send + 'static>;
#[derive(Clone)]
pub struct WorkerPool {
    tx: Sender<Task>,
}
impl WorkerPool {
    pub fn new(name: &str, threads: usize, capacity: usize) -> Self {
        let (tx, rx) = bounded::<Task>(capacity);
        for i in 0..threads {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("{name}-{i}"))
                .spawn(move || {
                    while let Ok(task) = rx.recv() {
                        task();
                    }
                })
                .expect("worker thread");
        }
        Self { tx }
    }
    pub fn submit(&self, f: impl FnOnce() + Send + 'static) -> Result<(), &'static str> {
        self.tx
            .try_send(Box::new(f))
            .map_err(|_| "작업 대기열이 가득 찼습니다. 잠시 후 다시 시도하세요.")
    }
    pub fn flush(&self, timeout: std::time::Duration) -> bool {
        let (tx, rx) = bounded(1);
        self.submit(move || {
            let _ = tx.send(());
        })
        .is_ok()
            && rx.recv_timeout(timeout).is_ok()
    }
}
