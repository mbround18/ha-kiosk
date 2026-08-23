mod configure;
mod flash;
mod port;
mod repo;

use clap::{Parser, Subcommand};

/// The device's setup-mode AP IP - see firmware/src/net/ap_portal.rs's `AP_IP` (ESP-IDF's
/// default softAP subnet on this hardware; the contract doc's "conventional 192.168.4.1" isn't
/// what this board actually uses).
const DEFAULT_DEVICE_IP: &str = "192.168.71.1";

#[derive(Parser)]
#[command(name = "flasher", version, about = "Build, flash, and auto-provision the ha-kiosk firmware")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build the firmware (release) and flash it over USB.
    Flash {
        /// Serial port to flash (auto-detected if omitted - see `ports`).
        #[arg(long)]
        port: Option<String>,
        /// Keep a serial monitor open after flashing.
        #[arg(long)]
        monitor: bool,
        /// Also run `configure` immediately afterward (waits for the device to boot into setup
        /// mode first).
        #[arg(long)]
        configure: bool,
        #[arg(long, default_value = DEFAULT_DEVICE_IP)]
        ip: String,
    },
    /// Auto-provision a freshly flashed device (WiFi + Home Assistant) from this repo's `.env`,
    /// skipping the on-device setup screens entirely.
    ///
    /// Your computer must already be joined to the device's own setup WiFi network first (its
    /// name is shown on the kiosk's screen on first boot) - this only automates what happens
    /// once you're already talking to the device.
    Configure {
        /// The device's setup-mode IP (its AP address).
        #[arg(long, default_value = DEFAULT_DEVICE_IP)]
        ip: String,
        /// How long to wait for a physical "Trust" tap on the kiosk's screen, if this is the
        /// first time it's seen your Home Assistant instance's TLS certificate.
        #[arg(long, default_value_t = 120)]
        trust_timeout_secs: u64,
    },
    /// List connected USB serial devices (useful when `flash`'s auto-detection picks the wrong
    /// port, or finds none).
    Ports,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Flash { port, monitor, configure, ip } => {
            flash::run(port, monitor)?;
            if configure {
                println!();
                self::configure::run(ip, 120)?;
            }
            Ok(())
        }
        Command::Configure { ip, trust_timeout_secs } => self::configure::run(ip, trust_timeout_secs),
        Command::Ports => port::print_ports(),
    }
}
