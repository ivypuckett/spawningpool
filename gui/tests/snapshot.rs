//! Headless snapshot test.
//!
//! `iced_test`'s `Simulator` draws the UI with the software (tiny-skia)
//! renderer and writes a PNG — no GPU or display needed. On the first run the
//! reference image is created under `snapshots/`; on later runs the freshly
//! rendered frame is compared against it byte-for-byte.
//!
//! Note: `matches_image` appends the renderer name, so the file on disk is
//! `snapshots/counter-tiny-skia.png`.

use iced_test::simulator;
use spawningpool_gui::Counter;

#[test]
fn counter_snapshot() {
    let mut counter = Counter::default();

    // Drive a real interaction so the snapshot reflects post-update state.
    let mut ui = simulator(counter.view());
    let _ = ui.click("Increment");
    for message in ui.into_messages() {
        counter.update(message);
    }

    let mut ui = simulator(counter.view());
    let snapshot = ui.snapshot(&spawningpool_gui::theme()).unwrap();

    assert!(
        snapshot.matches_image("snapshots/counter").unwrap(),
        "rendered UI does not match the committed snapshot",
    );
}
