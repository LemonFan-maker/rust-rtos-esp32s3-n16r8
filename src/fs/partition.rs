use core::fmt;

const PARTITION_TABLE_MAGIC: u16 = 0xAA50;

const MAX_PARTITION_ENTRIES: usize = 95;

pub const PARTITION_TABLE_OFFSET: u32 = 0x8000;

const PARTITION_ENTRY_SIZE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PartitionType {
    App = 0x00,
    Data = 0x01,
    Unknown(u8),
}

impl From<u8> for PartitionType {
    fn from(value: u8) -> Self {
        match value {
            0x00 => Self::App,
            0x01 => Self::Data,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DataSubType {
    Ota = 0x00,
    Phy = 0x01,
    Nvs = 0x02,
    CoreDump = 0x03,
    NvsKeys = 0x04,
    EFuse = 0x05,
    Undefined = 0x06,
    Spiffs = 0x82,
    Fat = 0x81,
    LittleFs = 0x83,
    Unknown(u8),
}

impl From<u8> for DataSubType {
    fn from(value: u8) -> Self {
        match value {
            0x00 => Self::Ota,
            0x01 => Self::Phy,
            0x02 => Self::Nvs,
            0x03 => Self::CoreDump,
            0x04 => Self::NvsKeys,
            0x05 => Self::EFuse,
            0x06 => Self::Undefined,
            0x81 => Self::Fat,
            0x82 => Self::Spiffs,
            0x83 => Self::LittleFs,
            other => Self::Unknown(other),
        }
    }
}

impl DataSubType {
    pub fn as_u8(&self) -> u8 {
        match self {
            Self::Ota => 0x00,
            Self::Phy => 0x01,
            Self::Nvs => 0x02,
            Self::CoreDump => 0x03,
            Self::NvsKeys => 0x04,
            Self::EFuse => 0x05,
            Self::Undefined => 0x06,
            Self::Fat => 0x81,
            Self::Spiffs => 0x82,
            Self::LittleFs => 0x83,
            Self::Unknown(v) => *v,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AppSubType {
    Factory = 0x00,
    Ota(u8),
    Test = 0x20,
    Unknown(u8),
}

impl From<u8> for AppSubType {
    fn from(value: u8) -> Self {
        match value {
            0x00 => Self::Factory,
            0x10..=0x1F => Self::Ota(value - 0x10),
            0x20 => Self::Test,
            other => Self::Unknown(other),
        }
    }
}

impl AppSubType {
    pub fn as_u8(&self) -> u8 {
        match self {
            Self::Factory => 0x00,
            Self::Ota(n) => 0x10 + n,
            Self::Test => 0x20,
            Self::Unknown(v) => *v,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PartitionFlags {
    pub encrypted: bool,
    pub readonly: bool,
}

impl From<u32> for PartitionFlags {
    fn from(value: u32) -> Self {
        Self {
            encrypted: (value & 0x01) != 0,
            readonly: (value & 0x02) != 0,
        }
    }
}

#[derive(Clone)]
pub struct Partition {
    pub label: heapless::String<16>,
    pub partition_type: PartitionType,
    pub subtype: u8,
    pub offset: u32,
    pub size: u32,
    pub flags: PartitionFlags,
}

impl Partition {
    pub fn from_bytes(data: &[u8; PARTITION_ENTRY_SIZE]) -> Option<Self> {
        let magic = u16::from_le_bytes([data[0], data[1]]);
        if magic != PARTITION_TABLE_MAGIC {
            return None;
        }

        let partition_type = PartitionType::from(data[2]);
        let subtype = data[3];
        let offset = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let size = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);

        let label_bytes = &data[12..28];
        let label_len = label_bytes.iter().position(|&b| b == 0).unwrap_or(16);
        let label_str = core::str::from_utf8(&label_bytes[..label_len]).ok()?;
        let mut label = heapless::String::new();
        label.push_str(label_str).ok()?;

        let flags = PartitionFlags::from(u32::from_le_bytes([data[28], data[29], data[30], data[31]]));

        Some(Self {
            label,
            partition_type,
            subtype,
            offset,
            size,
            flags,
        })
    }

    pub fn is_data(&self) -> bool {
        matches!(self.partition_type, PartitionType::Data)
    }

    pub fn is_app(&self) -> bool {
        matches!(self.partition_type, PartitionType::App)
    }

    pub fn data_subtype(&self) -> Option<DataSubType> {
        if self.is_data() {
            Some(DataSubType::from(self.subtype))
        } else {
            None
        }
    }

    pub fn app_subtype(&self) -> Option<AppSubType> {
        if self.is_app() {
            Some(AppSubType::from(self.subtype))
        } else {
            None
        }
    }

    pub fn is_littlefs(&self) -> bool {
        self.is_data() && self.subtype == DataSubType::LittleFs.as_u8()
    }

    pub fn is_spiffs(&self) -> bool {
        self.is_data() && self.subtype == DataSubType::Spiffs.as_u8()
    }

    pub fn is_nvs(&self) -> bool {
        self.is_data() && self.subtype == DataSubType::Nvs.as_u8()
    }

    pub fn end_offset(&self) -> u32 {
        self.offset + self.size
    }

    pub fn block_count(&self, block_size: u32) -> u32 {
        self.size / block_size
    }
}

impl fmt::Debug for Partition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Partition")
            .field("label", &self.label.as_str())
            .field("type", &self.partition_type)
            .field("subtype", &self.subtype)
            .field("offset", &format_args!("0x{:08X}", self.offset))
            .field("size", &format_args!("0x{:08X} ({}KB)", self.size, self.size / 1024))
            .field("flags", &self.flags)
            .finish()
    }
}

pub struct PartitionTable {
    partitions: heapless::Vec<Partition, MAX_PARTITION_ENTRIES>,
}

impl PartitionTable {
    pub const fn new() -> Self {
        Self {
            partitions: heapless::Vec::new(),
        }
    }

    pub fn from_flash_data(data: &[u8]) -> Option<Self> {
        let mut table = Self::new();

        if data.len() < PARTITION_ENTRY_SIZE {
            return None;
        }

        for chunk in data.chunks_exact(PARTITION_ENTRY_SIZE) {
            let entry_data: &[u8; PARTITION_ENTRY_SIZE] = chunk.try_into().ok()?;

            if entry_data[0] == 0xFF && entry_data[1] == 0xFF {
                break;
            }

            if let Some(partition) = Partition::from_bytes(entry_data) {
                table.partitions.push(partition).ok()?;
            } else {
                break;
            }
        }

        if table.partitions.is_empty() {
            None
        } else {
            Some(table)
        }
    }

    pub fn add_partition(
        &mut self,
        label: &str,
        partition_type: PartitionType,
        subtype: u8,
        offset: u32,
        size: u32,
    ) -> Result<(), ()> {
        let mut label_str = heapless::String::new();
        label_str.push_str(label).map_err(|_| ())?;

        self.partitions.push(Partition {
            label: label_str,
            partition_type,
            subtype,
            offset,
            size,
            flags: PartitionFlags::default(),
        }).map_err(|_| ())
    }

    pub fn find_by_label(&self, label: &str) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.label.as_str() == label)
    }

    pub fn find_littlefs(&self) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.is_littlefs())
    }

    pub fn find_spiffs(&self) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.is_spiffs())
    }

    pub fn find_nvs(&self) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.is_nvs())
    }

    pub fn find_by_type(&self, partition_type: PartitionType) -> impl Iterator<Item = &Partition> {
        self.partitions.iter().filter(move |p| p.partition_type == partition_type)
    }

    pub fn find_data_by_subtype(&self, subtype: DataSubType) -> Option<&Partition> {
        self.partitions.iter().find(|p| {
            p.is_data() && p.subtype == subtype.as_u8()
        })
    }

    pub fn partitions(&self) -> &[Partition] {
        &self.partitions
    }

    pub fn len(&self) -> usize {
        self.partitions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.partitions.is_empty()
    }
}

impl Default for PartitionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for PartitionTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PartitionTable")
            .field("count", &self.partitions.len())
            .field("partitions", &self.partitions.as_slice())
            .finish()
    }
}

pub mod presets {
    use super::*;

    pub fn default_4mb() -> PartitionTable {
        let mut table = PartitionTable::new();

        table.add_partition("nvs", PartitionType::Data, DataSubType::Nvs.as_u8(),
            0x9000, 0x6000).ok();

        table.add_partition("phy_init", PartitionType::Data, DataSubType::Phy.as_u8(),
            0xF000, 0x1000).ok();

        table.add_partition("factory", PartitionType::App, AppSubType::Factory.as_u8(),
            0x10000, 0x100000).ok();

        table.add_partition("storage", PartitionType::Data, DataSubType::LittleFs.as_u8(),
            0x110000, 0x2F0000).ok();

        table
    }

    pub fn default_16mb_ota() -> PartitionTable {
        let mut table = PartitionTable::new();

        table.add_partition("nvs", PartitionType::Data, DataSubType::Nvs.as_u8(),
            0x9000, 0x6000).ok();

        table.add_partition("phy_init", PartitionType::Data, DataSubType::Phy.as_u8(),
            0xF000, 0x1000).ok();

        table.add_partition("otadata", PartitionType::Data, DataSubType::Ota.as_u8(),
            0x10000, 0x2000).ok();

        table.add_partition("factory", PartitionType::App, AppSubType::Factory.as_u8(),
            0x12000, 0x400000).ok();

        table.add_partition("ota_0", PartitionType::App, 0x10,
            0x412000, 0x400000).ok();

        table.add_partition("ota_1", PartitionType::App, 0x11,
            0x812000, 0x400000).ok();

        table.add_partition("storage", PartitionType::Data, DataSubType::LittleFs.as_u8(),
            0xC12000, 0x3EE000).ok();

        table
    }

    pub fn simple_16mb() -> PartitionTable {
        let mut table = PartitionTable::new();

        table.add_partition("nvs", PartitionType::Data, DataSubType::Nvs.as_u8(),
            0x9000, 0x6000).ok();

        table.add_partition("factory", PartitionType::App, AppSubType::Factory.as_u8(),
            0x10000, 0x400000).ok();

        table.add_partition("storage", PartitionType::Data, DataSubType::LittleFs.as_u8(),
            0x410000, 0xBF0000).ok();

        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_from_bytes() {
        let mut data = [0u8; 32];
        data[0] = 0x50;
        data[1] = 0xAA;
        data[2] = 0x01;
        data[3] = 0x83;
        data[4] = 0x00;
        data[5] = 0x00;
        data[6] = 0x11;
        data[7] = 0x00;
        data[8] = 0x00;
        data[9] = 0x00;
        data[10] = 0x2F;
        data[11] = 0x00;
        data[12..19].copy_from_slice(b"storage");

        let partition = Partition::from_bytes(&data).unwrap();
        assert_eq!(partition.label.as_str(), "storage");
        assert!(partition.is_data());
        assert!(partition.is_littlefs());
        assert_eq!(partition.offset, 0x00110000);
        assert_eq!(partition.size, 0x002F0000);
    }

    #[test]
    fn test_preset_4mb() {
        let table = presets::default_4mb();
        assert_eq!(table.len(), 4);
        assert!(table.find_by_label("storage").is_some());
        assert!(table.find_littlefs().is_some());
    }
}
