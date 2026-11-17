//! `sqlx::migrate!` embeds the files of the repository's `migrations/` folder when the crate
//! compiles, and on a stable toolchain it cannot ask cargo to watch them: without this, a new or
//! edited migration would leave the crate, and every binary built on it, with the old list.
#![allow(clippy::print_stdout)] // cargo reads a build script's instructions on its standard output

fn main() {
    println!("cargo::rerun-if-changed=../../../migrations");
}
