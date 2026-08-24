//! USB serial port discovery for the board - migrated from this repo's original ad hoc
//! `ch340-dev` scratch tool (a one-off port-lister) into a real subcommand/helper here.

use serialport::{SerialPortInfo, SerialPortType};

/// Espressif's official USB vendor ID - this board's native USB-JTAG/serial port enumerates as
/// VID:PID `303a:1001` ("Espressif USB JTAG/serial debug unit"), confirmed via `espflash
/// board-info` against the physical hardware this project targets.
const ESPRESSIF_VID: u16 = 0x303A;

pub fn list_usb_ports() -> anyhow::Result<Vec<SerialPortInfo>> {
    let ports = serialport::available_ports()?;
    Ok(ports
        .into_iter()
        .filter(|p| matches!(p.port_type, SerialPortType::UsbPort(_)))
        .collect())
}

/// Resolves the board's serial port: `explicit` (a user-supplied `--port`) always wins; failing
/// that, exactly one Espressif-VID device is used automatically; failing *that*, exactly one USB
/// serial device of any kind is used as a fallback (e.g. running through a generic USB-serial
/// bridge rather than the board's native port). Anything more ambiguous is an error asking for
/// `--port` explicitly rather than guessing.
pub fn find_board_port(explicit: Option<&str>) -> anyhow::Result<String> {
    if let Some(p) = explicit {
        return Ok(p.to_string());
    }
    select_port(&list_usb_ports()?)
}

/// The actual selection algorithm, separated from `list_usb_ports`'s real hardware enumeration
/// so it's unit-testable against synthetic port lists (see the tests below) without needing a
/// physical device connected.
fn select_port(ports: &[SerialPortInfo]) -> anyhow::Result<String> {
    let espressif: Vec<&SerialPortInfo> = ports
        .iter()
        .filter(|p| matches!(&p.port_type, SerialPortType::UsbPort(info) if info.vid == ESPRESSIF_VID))
        .collect();

    match espressif.len() {
        1 => Ok(espressif[0].port_name.clone()),
        0 if ports.len() == 1 => Ok(ports[0].port_name.clone()),
        0 => anyhow::bail!(
            "Couldn't auto-detect the board's serial port ({} USB serial device(s) found, none matching \
             Espressif's VID 0x303a). Run `flasher ports` to see what's connected, then pass --port explicitly.",
            ports.len()
        ),
        _ => anyhow::bail!(
            "Multiple Espressif USB devices found ({}) - pass --port explicitly.",
            espressif.iter().map(|p| p.port_name.clone()).collect::<Vec<_>>().join(", ")
        ),
    }
}

pub fn print_ports() -> anyhow::Result<()> {
    let ports = list_usb_ports()?;
    if ports.is_empty() {
        println!("No USB serial devices found.");
        return Ok(());
    }
    for p in ports {
        if let SerialPortType::UsbPort(info) = &p.port_type {
            println!(
                "{}: {:04x}:{:04x} manufacturer={:?} product={:?} serial={:?}",
                p.port_name, info.vid, info.pid, info.manufacturer, info.product, info.serial_number
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb_port(name: &str, vid: u16) -> SerialPortInfo {
        SerialPortInfo {
            port_name: name.to_string(),
            port_type: SerialPortType::UsbPort(serialport::UsbPortInfo {
                vid,
                pid: 0x1001,
                serial_number: None,
                manufacturer: None,
                product: None,
            }),
        }
    }

    #[test]
    fn no_ports_is_an_error() {
        assert!(select_port(&[]).is_err());
    }

    #[test]
    fn single_espressif_port_is_picked_automatically() {
        let ports = [usb_port("/dev/ttyACM0", ESPRESSIF_VID)];
        assert_eq!(select_port(&ports).unwrap(), "/dev/ttyACM0");
    }

    #[test]
    fn espressif_port_is_picked_even_among_other_usb_devices() {
        let ports = [usb_port("/dev/ttyUSB0", 0x0403), usb_port("/dev/ttyACM0", ESPRESSIF_VID)];
        assert_eq!(select_port(&ports).unwrap(), "/dev/ttyACM0");
    }

    #[test]
    fn multiple_espressif_ports_is_an_error() {
        let ports = [usb_port("/dev/ttyACM0", ESPRESSIF_VID), usb_port("/dev/ttyACM1", ESPRESSIF_VID)];
        let err = select_port(&ports).unwrap_err().to_string();
        assert!(err.contains("/dev/ttyACM0"));
        assert!(err.contains("/dev/ttyACM1"));
    }

    #[test]
    fn single_non_espressif_port_falls_back_automatically() {
        let ports = [usb_port("/dev/ttyUSB0", 0x0403)];
        assert_eq!(select_port(&ports).unwrap(), "/dev/ttyUSB0");
    }

    #[test]
    fn multiple_non_espressif_ports_is_an_error() {
        let ports = [usb_port("/dev/ttyUSB0", 0x0403), usb_port("/dev/ttyUSB1", 0x0403)];
        assert!(select_port(&ports).is_err());
    }
}
