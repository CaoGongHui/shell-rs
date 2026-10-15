use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable, Size, ThemeStyled as _, h_flex, tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::host::{ConnectionState, HostOs, ProtocolKind};
use crate::i18n::t;

/// The colours a host without one of its own is drawn on, picked by its
/// address with [`mark_color`]. Saturated and dark enough for a white glyph
/// (4.5:1 at least), and never grey: grey is what a tab draws a host that is
/// not connected in. No red, which reads as an error, nor yellow, which a
/// white glyph cannot be read on. Data, like the brand colours, so the same
/// in either appearance.
const MARK_COLORS: [u32; 13] = [
    0x25_63EB, // blue
    0x4F_46E5, // indigo
    0x7C_3AED, // violet
    0x93_33EA, // purple
    0xA2_1CAF, // fuchsia
    0xBE_185D, // pink
    0xC2_410C, // orange
    0xB4_5309, // amber
    0x15_803D, // green
    0x04_7857, // emerald
    0x0F_766E, // teal
    0x0E_7490, // cyan
    0x03_69A1, // sky
];

/// What a glyph is drawn in on a [`MARK_COLORS`] colour.
const MARK_INK: u32 = 0xFF_FFFF;

/// The colour of a host's mark when its system has none (or is not known
/// yet): the same for an address every time, on every machine and in every
/// version, so a host keeps its colour. Blanks around it and its case do
/// not count: `COM3` is `com3`, and a host name is a host name in any case.
pub fn mark_color(address: &str) -> Rgba {
    rgb(MARK_COLORS[mark_index(address)])
}

/// Which of [`MARK_COLORS`] `address` gets.
fn mark_index(address: &str) -> usize {
    // FNV-1a: std's hashers may change between Rust versions, and with them
    // every host's colour.
    let hash = address
        .trim()
        .to_lowercase()
        .bytes()
        .fold(0x811C_9DC5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        });
    hash as usize % MARK_COLORS.len()
}

/// The mark that stands for a host: the operating system found on its host,
/// drawn on a badge in that project's own colour, or the first character of
/// the host name until a probe succeeds, as for a Telnet or serial host,
/// which nothing probes. Whatever has no colour of its own (the name's character, Apple's
/// mark) gets its address's.
///
/// Shared so the host tree and the start page show one identity for the
/// same host. The badge borrows `Avatar`'s treatment, a circle at the theme's
/// radius, so identity marks look alike wherever the product shows one.
#[derive(IntoElement)]
pub struct HostMark {
    id: ElementId,
    name: SharedString,
    address: SharedString,
    os: Option<HostOs>,
    protocol: ProtocolKind,
    /// The connection the mark stands for on a tab, when it has one.
    state: Option<ConnectionState>,
    size: Size,
    tooltip: bool,
}

impl HostMark {
    /// `address` picks the colour of a mark that has none of its own: the
    /// host's address, or a serial host's device.
    pub fn new(
        id: impl Into<ElementId>,
        name: impl Into<SharedString>,
        address: impl Into<SharedString>,
        os: Option<HostOs>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            address: address.into(),
            os,
            protocol: ProtocolKind::Ssh,
            state: None,
            size: Size::Medium,
            tooltip: true,
        }
    }

    /// The mark of a Telnet or serial host, whatever `os` says: the name's
    /// character, which the tooltip calls Telnet or a serial port.
    pub fn protocol(mut self, protocol: ProtocolKind) -> Self {
        self.protocol = protocol;
        self
    }

    /// The state of the connection a tab's mark stands for. Anything but
    /// connected draws the mark grey, glyph and all, and says so.
    pub fn connection_state(mut self, state: ConnectionState) -> Self {
        self.state = Some(state);
        self
    }

    /// No tooltip of its own, for a row that shows one for the whole row, as
    /// the host tree does, so two tooltips never stack.
    pub fn without_tooltip(mut self) -> Self {
        self.tooltip = false;
        self
    }
}

impl Sizable for HostMark {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl RenderOnce for HostMark {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let compact = matches!(self.size, Size::XSmall | Size::Small);
        let icon = |icon: Icon| {
            if compact {
                icon.size_3()
            } else {
                icon.size_5()
            }
            .into_any_element()
        };
        let own: Hsla = mark_color(&self.address).into();
        let white: Hsla = rgb(MARK_INK).into();
        let os = self.os.filter(|_| self.protocol.is_ssh());
        let (background, ink, glyph, description) = match os {
            Some(os) => {
                let (background, ink) = match (os.brand_color(), os.brand_foreground()) {
                    (Some(background), Some(ink)) => (background.into(), ink.into()),
                    // A monochrome mark (Apple's) on the address's colour,
                    // so no mark is ever grey.
                    _ => (own, white),
                };
                (
                    background,
                    ink,
                    icon(Icon::default().path(os.icon_path())),
                    SharedString::from(os.label()),
                )
            }
            // The name's first character, a Telnet or serial host's too: it
            // has no system to find, which is all the tooltip says otherwise.
            None => (
                own,
                white,
                div()
                    .map(|text| {
                        if compact {
                            text.text_xs()
                        } else {
                            text.text_base()
                        }
                    })
                    .child(
                        self.name
                            .chars()
                            .next()
                            .map(|character| character.to_uppercase().to_string())
                            .unwrap_or_default(),
                    )
                    .into_any_element(),
                match self.protocol {
                    ProtocolKind::Ssh => t!("shared.host_mark.unknown_os"),
                    ProtocolKind::Telnet => t!("shared.host_mark.telnet"),
                    ProtocolKind::Serial => t!("shared.host_mark.serial"),
                },
            ),
        };
        let offline = self.state.filter(|state| !state.is_connected());
        let (background, ink, description) = match offline {
            Some(state) => (
                cx.theme().muted,
                cx.theme().muted_foreground,
                t!(
                    "shared.host_mark.with_state",
                    mark = description,
                    state = state.label()
                ),
            ),
            None => (background, ink, description),
        };
        let tooltip = description.clone();
        h_flex()
            .id(self.id)
            .test_support()
            .aria_label(description)
            .when(self.tooltip, |badge| {
                badge.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            })
            .flex_shrink_0()
            .map(|badge| {
                if compact {
                    badge.size_5()
                } else {
                    badge.size_8()
                }
            })
            .justify_center()
            .rounded_full_style(cx)
            .bg(background)
            .text_color(ink)
            .child(glyph)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Hsla, Rgba, rgb};

    use super::{MARK_COLORS, mark_color, mark_index};

    /// WCAG relative luminance.
    fn luminance(color: Rgba) -> f32 {
        let linear = |channel: f32| {
            if channel <= 0.03928 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }

    #[test]
    fn an_address_keeps_its_colour_whatever_its_case_and_blanks() {
        assert_eq!(mark_color("10.0.1.12"), mark_color(" 10.0.1.12 "));
        assert_eq!(
            mark_color("Web-01.Example.com"),
            mark_color("web-01.example.com")
        );
        assert_eq!(mark_color("COM3"), mark_color("com3"));
        // FNV-1a of the address, fixed for good: a host keeps its colour
        // across versions.
        assert_eq!(mark_index(""), 0x811C_9DC5 % MARK_COLORS.len());
        assert_eq!(mark_index("a"), 0xE40C_292C % MARK_COLORS.len());
    }

    #[test]
    fn every_colour_is_vivid_and_carries_a_white_glyph() {
        for hex in MARK_COLORS {
            let color = rgb(hex);
            let hsla: Hsla = color.into();
            assert!(hsla.s >= 0.5, "{hex:06x} is too grey");
            let contrast = 1.05 / (luminance(color) + 0.05);
            assert!(contrast >= 4.5, "{hex:06x}: {contrast}");
        }
        let mut unique = MARK_COLORS.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), MARK_COLORS.len());
    }

    #[test]
    fn neighbouring_addresses_spread_over_the_colours() {
        let mut used: Vec<usize> = (1..=50)
            .map(|host| mark_index(&format!("10.0.0.{host}")))
            .collect();
        used.sort_unstable();
        used.dedup();
        assert!(used.len() > MARK_COLORS.len() / 2, "{}", used.len());
    }
}
