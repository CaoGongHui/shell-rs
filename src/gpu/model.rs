//! What GPU/NPU shows of a host: its accelerators, the processes computing
//! on them, and the figures summed over all of them. Plain data and
//! arithmetic, so the tests need no window.

use std::time::Duration;

/// Who made a device, which says which tool read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Ascend,
}

/// Whether a device says it is well.
#[derive(Clone, Debug, PartialEq)]
pub enum Health {
    Ok,
    /// An NVIDIA GPU's uncorrectable ECC errors since its driver loaded,
    /// some.
    EccErrors(u64),
    /// What `npu-smi` says of an Ascend NPU besides OK: 「Warning」,
    /// 「Alarm」, 「Critical」…, as it says it.
    Reported(String),
    /// Not said: an NVIDIA GPU without ECC.
    Unknown,
}

/// How much a device's state wants looking at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Concern {
    None,
    Warning,
    Critical,
}

impl Health {
    pub fn concern(&self) -> Concern {
        match self {
            Health::Ok | Health::Unknown => Concern::None,
            Health::EccErrors(_) => Concern::Critical,
            // npu-smi's grades: Warning is minor, Alarm major, Critical
            // urgent; anything else (UNKNOWN) is at least worth a look.
            Health::Reported(word) => match word.to_ascii_lowercase().as_str() {
                "alarm" | "critical" => Concern::Critical,
                _ => Concern::Warning,
            },
        }
    }
}

/// From this temperature up a device runs warm, and from `HOT` hot enough
/// to slow itself down.
pub const WARM: f32 = 80.;
pub const HOT: f32 = 90.;

/// How worrying a temperature is. Busy is not: a GPU at 100% and memory
/// taken up front to 90% are how training and serving run, so neither
/// share is ever colored.
pub fn heat(celsius: f32) -> Concern {
    if celsius >= HOT {
        Concern::Critical
    } else if celsius >= WARM {
        Concern::Warning
    } else {
        Concern::None
    }
}

/// One accelerator: a GPU, or one chip of an NPU card.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub vendor: Vendor,
    /// 「GPU 0」, 「NPU 3」, or 「NPU 0.1」 for a chip of a card with more
    /// than one.
    pub label: String,
    /// What it goes by from one reading to the next, for whether it is
    /// unfolded: its UUID, else its bus id.
    pub key: String,
    pub name: String,
    pub bus_id: Option<String>,
    pub uuid: Option<String>,
    /// NVIDIA's performance state: 「P0」 is the fastest.
    pub pstate: Option<String>,
    /// How busy it is, in percent: an NVIDIA GPU's kernels, an Ascend's AI
    /// Cores.
    pub utilization: Option<f32>,
    /// Bytes of its own memory in use and in all: an Ascend's HBM, where
    /// it has some.
    pub memory_used: Option<u64>,
    pub memory_total: Option<u64>,
    /// Celsius.
    pub temperature: Option<f32>,
    /// Watts drawn, and the most it may draw.
    pub power: Option<f32>,
    pub power_limit: Option<f32>,
    /// Fan speed in percent; data-center cards have no fan of their own.
    pub fan: Option<f32>,
    pub health: Health,
}

impl Device {
    pub fn memory_percent(&self) -> Option<f32> {
        match (self.memory_used, self.memory_total) {
            (Some(used), Some(total)) if total > 0 => Some(used as f32 / total as f32 * 100.),
            _ => None,
        }
    }
}

/// A process computing on a device.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuProcess {
    /// Where its device is in the reading's list; `None` when the tool
    /// named a device it did not list.
    pub device: Option<usize>,
    pub pid: u32,
    /// The program as the tool names it: a path, for `nvidia-smi`.
    pub name: String,
    /// Its owner and command line, from `ps`: missing for a process `ps`
    /// cannot see (another PID namespace).
    pub user: Option<String>,
    pub command: Option<String>,
    /// Bytes of the device's memory it holds.
    pub memory: Option<u64>,
}

impl GpuProcess {
    /// What to call it: its command line, else its program.
    pub fn title(&self) -> &str {
        self.command.as_deref().unwrap_or(&self.name)
    }
}

/// One reading of a host's accelerators.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    /// How long the host has been up: a GPU gone after a reboot, or a
    /// reboot nobody asked for, shows here.
    pub uptime: Option<Duration>,
    pub devices: Vec<Device>,
    /// By device, then the most memory first.
    pub processes: Vec<GpuProcess>,
    /// The drivers' versions, one per tool that said.
    pub drivers: Vec<String>,
    /// What a tool printed instead of answering, line by line: why there
    /// are fewer devices than the host has. Shown as it came, it is what
    /// someone looking after the host needs to see (「GPU is lost」).
    pub errors: Vec<String>,
    /// A tool did not answer in time and was stopped.
    pub timed_out: bool,
}

/// The figures over every device: what reads at a glance on a host with
/// eight. Each leaves out the devices that did not say.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    /// The mean.
    pub utilization: Option<f32>,
    /// Summed.
    pub memory_used: Option<u64>,
    pub memory_total: Option<u64>,
    /// The highest.
    pub temperature: Option<f32>,
    /// Summed.
    pub power: Option<f32>,
}

impl Summary {
    /// The memory in use over all of it, in percent.
    pub fn memory_percent(&self) -> Option<f32> {
        match (self.memory_used, self.memory_total) {
            (Some(used), Some(total)) if total > 0 => Some(used as f32 / total as f32 * 100.),
            _ => None,
        }
    }
}

impl Reading {
    pub fn summary(&self) -> Summary {
        let utilizations: Vec<f32> = self
            .devices
            .iter()
            .filter_map(|device| device.utilization)
            .collect();
        let memory: Vec<(u64, u64)> = self
            .devices
            .iter()
            .filter_map(|device| Some((device.memory_used?, device.memory_total?)))
            .collect();
        let powers: Vec<f32> = self.devices.iter().filter_map(|d| d.power).collect();
        Summary {
            utilization: (!utilizations.is_empty())
                .then(|| utilizations.iter().sum::<f32>() / utilizations.len() as f32),
            memory_used: (!memory.is_empty()).then(|| memory.iter().map(|m| m.0).sum()),
            memory_total: (!memory.is_empty()).then(|| memory.iter().map(|m| m.1).sum()),
            temperature: self
                .devices
                .iter()
                .filter_map(|device| device.temperature)
                .reduce(f32::max),
            power: (!powers.is_empty()).then(|| powers.iter().sum()),
        }
    }

    /// The model every device is, when they are all the same one.
    pub fn model(&self) -> Option<&str> {
        let first = self.devices.first()?;
        self.devices
            .iter()
            .all(|device| device.name == first.name)
            .then_some(first.name.as_str())
    }
}

const GIB: f64 = (1u64 << 30) as f64;
const MIB: f64 = (1u64 << 20) as f64;

/// 「61.2 / 80 GB」, 「512 MB / 24 GB」: memory in use and in all, as tight
/// as a device's row needs it. A total of tens of GB reads whole, as cards
/// are sold.
pub fn format_memory(used: Option<u64>, total: Option<u64>) -> String {
    let total = total.map(|total| {
        let gib = total as f64 / GIB;
        if gib >= 10. {
            format!("{gib:.0} GB")
        } else if gib >= 1. {
            format!("{gib:.1} GB")
        } else {
            format!("{:.0} MB", total as f64 / MIB)
        }
    });
    // In GB it goes without its unit, which the total gives.
    let used = used.map(|used| {
        if used as f64 >= GIB {
            format!("{:.1}", used as f64 / GIB)
        } else {
            format!("{:.0} MB", used as f64 / MIB)
        }
    });
    match (used, total) {
        (Some(used), Some(total)) => format!("{used} / {total}"),
        (None, Some(total)) => format!("— / {total}"),
        (Some(used), None) if used.ends_with("MB") => used,
        (Some(used), None) => format!("{used} GB"),
        (None, None) => "—".into(),
    }
}

/// 「12.4 GB」: a process's share of a device's memory.
pub fn format_process_memory(bytes: Option<u64>) -> String {
    match bytes {
        None => "—".into(),
        Some(bytes) if bytes as f64 >= GIB => format!("{:.1} GB", bytes as f64 / GIB),
        Some(bytes) => format!("{:.0} MB", bytes as f64 / MIB),
    }
}

/// 「65 °C」.
pub fn format_temperature(celsius: Option<f32>) -> String {
    celsius.map_or("—".into(), |celsius| format!("{celsius:.0} °C"))
}

/// 「512 W」.
pub fn format_power(watts: Option<f32>) -> String {
    watts.map_or("—".into(), |watts| format!("{watts:.0} W"))
}

/// 「512 / 700 W」: drawn, and the most it may draw.
pub fn format_power_limit(watts: Option<f32>, limit: Option<f32>) -> String {
    match (watts, limit) {
        (Some(watts), Some(limit)) => format!("{watts:.0} / {limit:.0} W"),
        (watts, _) => format_power(watts),
    }
}

/// 「92%」: a whole share, which is all a bar's label needs.
pub fn format_share(percent: Option<f32>) -> String {
    percent.map_or("—".into(), |percent| format!("{percent:.0}%"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(utilization: Option<f32>, memory: Option<(u64, u64)>) -> Device {
        Device {
            vendor: Vendor::Nvidia,
            label: "GPU 0".into(),
            key: "GPU-0".into(),
            name: "NVIDIA H100 80GB HBM3".into(),
            bus_id: None,
            uuid: None,
            pstate: None,
            utilization,
            memory_used: memory.map(|m| m.0),
            memory_total: memory.map(|m| m.1),
            temperature: None,
            power: None,
            power_limit: None,
            fan: None,
            health: Health::Ok,
        }
    }

    #[test]
    fn the_summary_leaves_out_what_a_device_did_not_say() {
        let gib = 1u64 << 30;
        let mut hot = device(Some(90.), Some((60 * gib, 80 * gib)));
        hot.temperature = Some(71.);
        hot.power = Some(500.);
        let mut quiet = device(Some(10.), None);
        quiet.temperature = Some(40.);
        let mut silent = device(None, Some((gib, 80 * gib)));
        silent.power = Some(100.);
        let reading = Reading {
            devices: vec![hot, quiet, silent],
            ..Reading::default()
        };
        assert_eq!(
            reading.summary(),
            Summary {
                utilization: Some(50.),
                memory_used: Some(61 * gib),
                memory_total: Some(160 * gib),
                temperature: Some(71.),
                power: Some(600.),
            }
        );
        assert_eq!(reading.model(), Some("NVIDIA H100 80GB HBM3"));
        assert_eq!(Reading::default().summary(), Summary::default());
    }

    #[test]
    fn a_mixed_host_has_no_one_model() {
        let mut other = device(None, None);
        other.name = "NVIDIA A100-SXM4-40GB".into();
        let reading = Reading {
            devices: vec![device(None, None), other],
            ..Reading::default()
        };
        assert_eq!(reading.model(), None);
    }

    #[test]
    fn only_heat_and_ill_health_are_a_concern() {
        assert_eq!(heat(79.), Concern::None);
        assert_eq!(heat(80.), Concern::Warning);
        assert_eq!(heat(91.), Concern::Critical);
        assert_eq!(Health::Ok.concern(), Concern::None);
        assert_eq!(Health::Unknown.concern(), Concern::None);
        assert_eq!(Health::EccErrors(2).concern(), Concern::Critical);
        assert_eq!(
            Health::Reported("Warning".into()).concern(),
            Concern::Warning
        );
        assert_eq!(
            Health::Reported("Alarm".into()).concern(),
            Concern::Critical
        );
        assert_eq!(
            Health::Reported("UNKNOWN".into()).concern(),
            Concern::Warning
        );
    }

    #[test]
    fn amounts_read_as_cards_are_sold() {
        let mib = 1u64 << 20;
        assert_eq!(
            format_memory(Some(62_668 * mib), Some(81_559 * mib)),
            "61.2 / 80 GB"
        );
        assert_eq!(format_memory(Some(mib), Some(24_564 * mib)), "1 MB / 24 GB");
        assert_eq!(
            format_memory(Some(3379 * mib), Some(65_536 * mib)),
            "3.3 / 64 GB"
        );
        assert_eq!(
            format_memory(Some(512 * mib), Some(4096 * mib)),
            "512 MB / 4.0 GB"
        );
        assert_eq!(format_memory(None, Some(81_559 * mib)), "— / 80 GB");
        assert_eq!(format_memory(None, None), "—");
        assert_eq!(format_process_memory(Some(17_400 * mib)), "17.0 GB");
        assert_eq!(format_process_memory(Some(300 * mib)), "300 MB");
        assert_eq!(format_temperature(Some(64.6)), "65 °C");
        assert_eq!(format_power_limit(Some(512.3), Some(700.)), "512 / 700 W");
        assert_eq!(format_power_limit(Some(92.9), None), "93 W");
        assert_eq!(format_share(Some(91.6)), "92%");
        assert_eq!(format_share(None), "—");
    }
}
