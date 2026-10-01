//! Measurement helpers using production storage and process adapters.
//! Python owns deployment, suite selection and reporting.
pub mod amortization;
pub mod io;
pub mod protocol;
use anyhow::{Result, ensure};

pub(crate) fn pin(cpus: &[usize]) -> Result<()> {
    #[cfg(target_os = "linux")]
    if !cpus.is_empty() {
        ensure!(
            cpus.iter().all(|&c| c < libc::CPU_SETSIZE as usize),
            "invalid CPU index"
        );
        // SAFETY: initialized set and checked indices; pid zero pins this thread.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            for &cpu in cpus {
                libc::CPU_SET(cpu, &mut set);
            }
            ensure!(
                libc::sched_setaffinity(0, std::mem::size_of_val(&set), &set) == 0,
                "CPU affinity: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    Ok(())
}

struct IoRuntime(Option<tokio::runtime::Runtime>);
impl IoRuntime {
    fn new(threads: usize, cpus: Vec<usize>) -> Result<Self> {
        let check = cpus.clone();
        std::thread::spawn(move || pin(&check))
            .join()
            .expect("affinity check")?;
        Ok(Self(Some(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(threads)
                .enable_all()
                .on_thread_start(move || pin(&cpus).expect("validated affinity"))
                .build()?,
        )))
    }
    fn handle(&self) -> &tokio::runtime::Handle {
        self.0.as_ref().unwrap().handle()
    }
}
impl Drop for IoRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}
