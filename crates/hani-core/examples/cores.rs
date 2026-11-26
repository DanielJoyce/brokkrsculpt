// SPDX-License-Identifier: AGPL-3.0-only

//! How many worker threads this machine's work wants, by
//! [`hani_core::calibrate`]:
//!
//! ```text
//! cargo run --release -p hani-core --example cores
//! ```

fn main() {
    let most = std::thread::available_parallelism().map_or(1, usize::from);
    let trials = hani_core::calibrate::measure(&hani_core::calibrate::counts(most));
    let one = trials.first().map_or(1.0, |trial| trial.millis);
    for trial in &trials {
        println!(
            "{:>3} threads: {:>7.1} ms, {:.2} times one",
            trial.threads,
            trial.millis,
            one / trial.millis
        );
    }
    if let Some(threads) = hani_core::calibrate::recommend(&trials) {
        println!("recommended: {threads} of {most}");
    }
}
