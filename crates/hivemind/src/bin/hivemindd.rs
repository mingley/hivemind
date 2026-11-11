//! `hivemindd`: single-owner daemon for a hivemind store.
//!
//! Thin entry point; the real logic lives in `hivemind::daemon`.

fn main() {
    if let Err(err) = hivemind::daemon::main_from_env() {
        eprintln!("hivemindd: {err:#}");
        std::process::exit(1);
    }
}
