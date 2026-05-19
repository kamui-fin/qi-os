use alloc::vec;
use alloc::vec::Vec;
use x86_64::instructions::port::Port;

pub fn pci_read_32(bus: u16, slot: u16, func: u16, offset: u16) -> u32 {
    let address = (bus as u64) << 16
        | (slot as u64) << 11
        | (func as u64) << 8
        | ((offset as u64) & 0xFC)
        | 1 << 31;

    let mut addr_port = Port::new(0xCF8);
    unsafe { addr_port.write(address as u32) };

    let mut data_port = Port::<u32>::new(0xCFC);
    unsafe { data_port.read() }
}

pub fn pci_read_word(bus: u16, slot: u16, func: u16, offset: u16) -> u16 {
    ((pci_read_32(bus, slot, func, offset) >> (offset & 2) * 8) & 0xFFFF) as u16
}

pub fn pci_write(bus: u16, slot: u16, func: u16, offset: u16, value: u32) {
    let address = (bus as u64) << 16
        | (slot as u64) << 11
        | (func as u64) << 8
        | ((offset as u64) & 0xFC)
        | 1 << 31;

    let mut addr_port = Port::new(0xCF8);
    unsafe { addr_port.write(address as u32) };

    let mut data_port = Port::<u32>::new(0xCFC);
    unsafe { data_port.write(value) }
}

#[derive(Debug, Clone)]
pub enum HeaderType {
    GeneralDevice = 0x0,
    PciPciBridge = 0x1,
    PciCardBusBridge = 0x2,
}

impl TryFrom<u8> for HeaderType {
    type Error = ();

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::GeneralDevice),
            1 => Ok(Self::PciPciBridge),
            2 => Ok(Self::PciCardBusBridge),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PciDevice {
    pub bus: u16,
    pub device: u16,
    pub function_id: u16,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub revision_id: u8,
    pub prog_if: u8,
    pub header_type: HeaderType,
}

pub fn pci_enumerate() -> Vec<PciDevice> {
    let mut pci_devices = vec![];
    for bus in 0..256 {
        for device in 0..32 {
            for function in 0..8 {
                let vendor_id = pci_read_word(bus, device, function, 0x00);
                if vendor_id == 0xFFFF {
                    continue;
                }
                let device_id = pci_read_word(bus, device, function, 0x00 | 0x2);

                let prog_revision = pci_read_word(bus, device, function, 0x08);
                let revision_id = (prog_revision & 0xFF) as u8;
                let prog_if = ((prog_revision >> 8) & 0xFF) as u8;

                let class_subclass = pci_read_word(bus, device, function, 0x08 | 0x2);
                let subclass = class_subclass & 0xFF;
                let class = (class_subclass >> 8) & 0xFF;

                let header_type_raw = pci_read_word(bus, device, function, 0x0C | 0x2);
                let header_type = (header_type_raw & 0xFF) as u8;
                let is_multi_functional = (header_type & 0x80) != 0; // check bit 7
                let header_type = HeaderType::try_from((header_type & 0x7F) as u8).unwrap();

                let pci_data = PciDevice {
                    bus,
                    device,
                    vendor_id,
                    device_id,
                    function_id: function,
                    class: class as u8,
                    subclass: subclass as u8,
                    header_type,
                    revision_id,
                    prog_if,
                };

                pci_devices.push(pci_data);

                if function == 0 && !is_multi_functional {
                    break;
                }
            }
        }
    }
    pci_devices
}
