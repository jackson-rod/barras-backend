//! `sqlx::migrate!` embeds the files of the repository's `migrations/` folder when the crate
//! compiles, and on a stable toolchain it cannot ask cargo to watch them: without this, a new or
//! edited migration would leave the crate, and every binary built on it, with the old list.
//!
//! The query macros read `SQLX_OFFLINE` while the crate compiles. `.cargo/config.toml` sets it,
//! but cargo reads that file from the folder it runs in, so a build started outside the checkout
//! (`cargo build --manifest-path …`) would miss it, and the macros would connect to whatever
//! database `DATABASE_URL` or a `.env` names. So this sets it for the crate's own compilation
//! when it is unset; a value already set wins, such as the `false` with which `cargo sqlx
//! prepare` builds against its database (plan, refinement 12).
#![allow(clippy::print_stdout)] // cargo reads a build script's instructions on its standard output

fn main() {
    println!("cargo::rerun-if-changed=../../../migrations");
    println!("cargo::rerun-if-env-changed=SQLX_OFFLINE");
    #[allow(clippy::disallowed_methods)] // a build script, whose input is what cargo hands it
    let offline = std::env::var_os("SQLX_OFFLINE");
    if offline.is_none() {
        println!("cargo::rustc-env=SQLX_OFFLINE=true");
    }
}
