//! Proof-of-concept iced UI for spawningpool.
//!
//! The point of this crate is not the UI itself but to prove that an iced
//! interface can be rendered to a deterministic PNG in a headless cloud
//! environment — no GPU, window, or display server. The rendering path is
//! `iced_test`'s software (tiny-skia) renderer, exercised by the snapshot
//! test in `tests/`.

use iced::widget::{button, column, container, text};
use iced::{Center, Element, Fill, Theme};

/// A trivial counter, the canonical iced example.
#[derive(Debug, Clone, Copy, Default)]
pub struct Counter {
    value: i64,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    Increment,
    Decrement,
}

impl Counter {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Increment => self.value += 1,
            Message::Decrement => self.value -= 1,
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let content = column![
            text("spawningpool").size(48),
            text("Create hyper-specific, 0-waste agents").size(16),
            text(self.value).size(80),
            button("Increment").on_press(Message::Increment),
            button("Decrement").on_press(Message::Decrement),
        ]
        .spacing(20)
        .align_x(Center);

        container(content).center(Fill).into()
    }
}

/// The theme used for snapshots. Pinned to a single variant so renders are
/// deterministic regardless of the host's light/dark preference.
pub fn theme() -> Theme {
    Theme::Light
}
