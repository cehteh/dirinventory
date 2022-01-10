#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]
#![feature(hash_set_entry)]

mod gatherer;
pub use gatherer::{Gatherer, GathererBuilder, GathererHandle, ProcessEntry, ProcessFn};

mod messages;
pub use messages::{DirectoryGatherMessage, InventoryEntryMessage};

mod objectpath;
pub use objectpath::ObjectPath;

mod internednames;
use internednames::{InternedName, InternedNames};

mod dirhandle;
pub use dirhandle::{used_handles, Dir};
pub use openat_ct as openat;

/// An user defined processing function can return any kind of error, this needs to be boxed
/// and dyn. Since error handling is expected to be the slow path, having the allocation and
/// vtable here shouldn't be an performance issue.
pub type DynError = Box<dyn std::error::Error + Send>;
/// Typedef for the DynError result.
pub type DynResult<T> = std::result::Result<T, DynError>;

#[cfg(test)]
mod test {
    use std::{thread, time};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::io::Write;
    use std::sync::Once;

    use env_logger;

    pub fn init_env_logging() {
        static LOGGER: Once = Once::new();
        LOGGER.call_once(|| {
            let counter: AtomicU64 = AtomicU64::new(0);
            let seq_num = move || counter.fetch_add(1, Ordering::Relaxed);

            let start = time::Instant::now();

            env_logger::Builder::from_default_env()
                .format(move |buf, record| {
                    let micros = start.elapsed().as_micros() as u64;
                    writeln!(
                        buf,
                        "{:0>12}: {:0>8}.{:0>6}: {:>5}: {}:{}: {}: {}",
                        seq_num(),
                        micros / 1000000,
                        micros % 1000000,
                        record.level().as_str(),
                        record.file().unwrap_or(""),
                        record.line().unwrap_or(0),
                        thread::current().name().unwrap_or("UNKNOWN"),
                        record.args()
                    )
                })
                .try_init()
                .unwrap();
        });
    }
}
