//! The core library is tokio-based; gpui has its own executor. One shared runtime runs the core's futures,
//! and gpui tasks await the returned `JoinHandle`s (which work on any executor).
use std::sync::OnceLock;
use tokio::runtime::Runtime;

pub fn get() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime"))
}
