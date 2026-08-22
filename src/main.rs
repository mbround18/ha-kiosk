use serialport::SerialPortType;

fn main() {
    let ports = serialport::available_ports().expect("failed to enumerate serial ports");

    let usb_ports: Vec<_> = ports
        .into_iter()
        .filter(|p| matches!(p.port_type, SerialPortType::UsbPort(_)))
        .collect();

    if usb_ports.is_empty() {
        println!("No USB serial devices found.");
        return;
    }

    for p in usb_ports {
        if let SerialPortType::UsbPort(info) = &p.port_type {
            println!(
                "{}: {:04x}:{:04x} manufacturer={:?} product={:?} serial={:?}",
                p.port_name,
                info.vid,
                info.pid,
                info.manufacturer,
                info.product,
                info.serial_number
            );
        }
    }
}
