use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Wake, Waker};

struct Parker {
    ready: Mutex<bool>,
    cvar: Condvar,
}

impl Parker {
    fn wait(&self) {
        let mut ready = self.ready.lock().expect("pollster parker poisoned");
        while !*ready {
            ready = self.cvar.wait(ready).expect("pollster parker poisoned");
        }
        *ready = false;
    }
}

impl Wake for Parker {
    fn wake(self: Arc<Self>) {
        let mut ready = self.ready.lock().expect("pollster parker poisoned");
        *ready = true;
        self.cvar.notify_one();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let mut ready = self.ready.lock().expect("pollster parker poisoned");
        *ready = true;
        self.cvar.notify_one();
    }
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    let parker = Arc::new(Parker {
        ready: Mutex::new(false),
        cvar: Condvar::new(),
    });
    let waker: Waker = Waker::from(Arc::clone(&parker));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);

    loop {
        match Future::poll(Pin::as_mut(&mut future), &mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => parker.wait(),
        }
    }
}
