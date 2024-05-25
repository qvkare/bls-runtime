//! # wasi-common
//!
//! This is Wasmtime's legacy implementation of WASI 0.1 (Preview 1). The
//! Wasmtime maintainers suggest all users upgrade to the implementation
//! of WASI 0.1 and 0.2 provided by the `wasmtime-wasi` crate. This
//! implementation remains in the wasmtime tree because it is required to use
//! the `wasmtime-wasi-threads` crate, an implementation of the `wasi-threads`
//! proposal which is not compatible with WASI 0.2.
//!
//! In addition to integration with Wasmtime, this implementation may be used
//! by other runtimes by disabling the `wasmtime` feature on this crate.
//!
//! ## The `WasiFile` and `WasiDir` traits
//!
//! The WASI specification only defines one `handle` type, `fd`, on which all
//! operations on both files and directories (aka dirfds) are defined. We
//! believe this is a design mistake, and are architecting wasi-common to make
//! this straightforward to correct in future snapshots of WASI.  Wasi-common
//! internally treats files and directories as two distinct resource types in
//! the table - `Box<dyn WasiFile>` and `Box<dyn WasiDir>`. The snapshot 0 and
//! 1 interfaces via `fd` will attempt to downcast a table element to one or
//! both of these interfaces depending on what is appropriate - e.g.
//! `fd_close` operates on both files and directories, `fd_read` only operates
//! on files, and `fd_readdir` only operates on directories.

//! The `WasiFile` and `WasiDir` traits are defined by `wasi-common` in terms
//! of types defined directly in the crate's source code (I decided it should
//! NOT those generated by the `wiggle` proc macros, see snapshot architecture
//! below), as well as the `cap_std::time` family of types.  And, importantly,
//! `wasi-common` itself provides no implementation of `WasiDir`, and only two
//! trivial implementations of `WasiFile` on the `crate::pipe::{ReadPipe,
//! WritePipe}` types, which in turn just delegate to `std::io::{Read,
//! Write}`. In order for `wasi-common` to access the local filesystem at all,
//! you need to provide `WasiFile` and `WasiDir` impls through either the new
//! `wasi-cap-std-sync` crate found at `crates/wasi-common/cap-std-sync` - see
//! the section on that crate below - or by providing your own implementation
//! from elsewhere.
//!
//! This design makes it possible for `wasi-common` embedders to statically
//! reason about access to the local filesystem by examining what impls are
//! linked into an application. We found that this separation of concerns also
//! makes it pretty enjoyable to write alternative implementations, e.g. a
//! virtual filesystem.
//!
//! Implementations of the `WasiFile` and `WasiDir` traits are provided
//! for synchronous embeddings (i.e. Config::async_support(false)) in
//! `wasi_common::sync` and for Tokio embeddings in `wasi_common::tokio`.
//!
//! ## Traits for the rest of WASI's features
//!
//! Other aspects of a WASI implementation are not yet considered resources
//! and accessed by `handle`. We plan to correct this design deficiency in
//! WASI in the future, but for now we have designed the following traits to
//! provide embedders with the same sort of implementation flexibility they
//! get with WasiFile/WasiDir:
//!
//! * Timekeeping: `WasiSystemClock` and `WasiMonotonicClock` provide the two
//! interfaces for a clock. `WasiSystemClock` represents time as a
//! `cap_std::time::SystemTime`, and `WasiMonotonicClock` represents time as
//! `cap_std::time::Instant`.  * Randomness: we re-use the `cap_rand::RngCore`
//! trait to represent a randomness source. A trivial `Deterministic` impl is
//! provided.  * Scheduling: The `WasiSched` trait abstracts over the
//! `sched_yield` and `poll_oneoff` functions.
//!
//! Users can provide implementations of each of these interfaces to the
//! `WasiCtx::builder(...)` function. The
//! `wasi_cap_std_sync::WasiCtxBuilder::new()` function uses this public
//! interface to plug in its own implementations of each of these resources.

#![warn(clippy::cast_sign_loss)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod clocks;
mod ctx;
pub mod dir;
mod error;
pub mod file;
pub mod pipe;
pub mod random;
pub mod sched;
pub mod snapshots;
mod string_array;
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
#[cfg(feature = "sync")]
pub mod sync;
pub mod table;
#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
#[cfg(feature = "tokio")]
pub mod tokio;

pub use cap_rand::RngCore;
pub use clocks::{SystemTimeSpec, WasiClocks, WasiMonotonicClock, WasiSystemClock};
pub use ctx::WasiCtx;
pub use dir::WasiDir;
pub use error::{Error, ErrorExt, I32Exit};
pub use file::WasiFile;
pub use sched::{Poll, WasiSched};
pub use string_array::{StringArray, StringArrayError};
pub use table::Table;

mod blockless;
pub use blockless::{
    BlocklessConfig, BlocklessConfigVersion, BlocklessModule, DriverConfig, LoggerLevel,
    ModuleType, Permission, Stderr, Stdout,
};

// The only difference between these definitions for sync vs async is whether
// the wasmtime::Funcs generated are async (& therefore need an async Store and an executor to run)
// or whether they have an internal "dummy executor" that expects the implementation of all
// the async funcs to poll to Ready immediately.
#[cfg(feature = "wasmtime")]
#[doc(hidden)]
#[macro_export]
macro_rules! define_wasi {
    ($async_mode:tt $($bounds:tt)*) => {

    use wasmtime::Linker;

    pub fn add_to_linker<T, U>(
        linker: &mut Linker<T>,
        get_cx: impl Fn(&mut T) -> &mut U + Send + Sync + Copy + 'static,
    ) -> anyhow::Result<()>
        where U: Send
                + crate::snapshots::preview_0::wasi_unstable::WasiUnstable
                + crate::snapshots::preview_1::wasi_snapshot_preview1::WasiSnapshotPreview1,
            $($bounds)*
    {
        snapshots::preview_1::add_wasi_snapshot_preview1_to_linker(linker, get_cx)?;
        snapshots::preview_0::add_wasi_unstable_to_linker(linker, get_cx)?;
        Ok(())
    }

    pub mod snapshots {
        pub mod preview_1 {
            wiggle::wasmtime_integration!({
                // The wiggle code to integrate with lives here:
                target: crate::snapshots::preview_1,
                witx: ["$CARGO_MANIFEST_DIR/witx/preview1/wasi_snapshot_preview1.witx"],
                errors: { errno => trappable Error },
                $async_mode: *
            });
        }
        pub mod preview_0 {
            wiggle::wasmtime_integration!({
                // The wiggle code to integrate with lives here:
                target: crate::snapshots::preview_0,
                witx: ["$CARGO_MANIFEST_DIR/witx/preview0/wasi_unstable.witx"],
                errors: { errno => trappable Error },
                $async_mode: *
            });
        }
    }
}}

/// Exit the process with a conventional OS error code as long as Wasmtime
/// understands the error. If the error is not an `I32Exit` or `Trap`, return
/// the error back to the caller for it to decide what to do.
///
/// Note: this function is designed for usage where it is acceptable for
/// Wasmtime failures to terminate the parent process, such as in the Wasmtime
/// CLI; this would not be suitable for use in multi-tenant embeddings.
#[cfg_attr(docsrs, doc(cfg(feature = "exit")))]
#[cfg(feature = "exit")]
pub fn maybe_exit_on_error(e: anyhow::Error) -> anyhow::Error {
    use std::process;
    use wasmtime::Trap;

    // If a specific WASI error code was requested then that's
    // forwarded through to the process here without printing any
    // extra error information.
    let code = e.downcast_ref::<crate::I32Exit>().map(|e| e.0);
    if let Some(exit) = code {
        // Print the error message in the usual way.
        // On Windows, exit status 3 indicates an abort (see below),
        // so return 1 indicating a non-zero status to avoid ambiguity.
        if cfg!(windows) && exit >= 3 {
            process::exit(1);
        }
        process::exit(exit);
    }

    // If the program exited because of a trap, return an error code
    // to the outside environment indicating a more severe problem
    // than a simple failure.
    if e.is::<Trap>() {
        eprintln!("Error: {:?}", e);

        if cfg!(unix) {
            // On Unix, return the error code of an abort.
            process::exit(128 + libc::SIGABRT);
        } else if cfg!(windows) {
            // On Windows, return 3.
            // https://docs.microsoft.com/en-us/cpp/c-runtime-library/reference/abort?view=vs-2019
            process::exit(3);
        }
    }

    e
}
