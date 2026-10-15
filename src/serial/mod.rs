//! 串口: the serial ports of this computer, for serial hosts. Which ports
//! there are, for the host form, and the terminal transport that opens one.

mod ports;
mod transport;

pub use ports::available_ports;
pub use transport::{SerialTransportProvider, check};
