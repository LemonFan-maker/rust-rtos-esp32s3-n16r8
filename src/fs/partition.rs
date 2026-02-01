//! ESP32 分区表支持
//!
//! 解析和管理 ESP32 分区表，支持定位文件系统分区

use core::fmt;

/// 分区表魔数 (ESP-IDF 格式)
const PARTITION_TABLE_MAGIC: u16 = 0xAA50;

/// 分区表最大条目数
const MAX_PARTITION_ENTRIES: usize = 95;

/// 分区表在 Flash 中的偏移量 (默认 0x8000)
pub const PARTITION_TABLE_OFFSET: u32 = 0x8000;

/// 单个分区条目大小
const PARTITION_ENTRY_SIZE: usize = 32;

/// 分区类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PartitionType {
    /// 应用程序分区
    App = 0x00,
    /// 数据分区
    Data = 0x01,
    /// 未知类型
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

/// 数据分区子类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DataSubType {
    /// OTA 数据
    Ota = 0x00,
    /// PHY 初始化数据
    Phy = 0x01,
    /// NVS (Non-Volatile Storage)
    Nvs = 0x02,
    /// Core dump
    CoreDump = 0x03,
    /// NVS 密钥
    NvsKeys = 0x04,
    /// eFuse 模拟
    EFuse = 0x05,
    /// 未定义/用户自定义 (0x06-0x7F)
    Undefined = 0x06,
    /// SPIFFS 文件系统
    Spiffs = 0x82,
    /// FAT 文件系统
    Fat = 0x81,
    /// LittleFS 文件系统 (用户自定义，常用 0x83)
    LittleFs = 0x83,
    /// 未知子类型
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
    /// 转换为 u8 值
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

/// 应用分区子类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AppSubType {
    /// 工厂应用
    Factory = 0x00,
    /// OTA 应用 0-15
    Ota(u8),
    /// 测试应用
    Test = 0x20,
    /// 未知
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
    /// 转换为 u8 值
    pub fn as_u8(&self) -> u8 {
        match self {
            Self::Factory => 0x00,
            Self::Ota(n) => 0x10 + n,
            Self::Test => 0x20,
            Self::Unknown(v) => *v,
        }
    }
}

/// 分区标志
#[derive(Debug, Clone, Copy, Default)]
pub struct PartitionFlags {
    /// 分区已加密
    pub encrypted: bool,
    /// 分区只读
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

/// 单个分区描述
#[derive(Clone)]
pub struct Partition {
    /// 分区标签 (最长15字符 + null)
    pub label: heapless::String<16>,
    /// 分区类型
    pub partition_type: PartitionType,
    /// 子类型 (原始值)
    pub subtype: u8,
    /// 分区在 Flash 中的偏移量
    pub offset: u32,
    /// 分区大小 (字节)
    pub size: u32,
    /// 分区标志
    pub flags: PartitionFlags,
}

impl Partition {
    /// 从原始字节解析分区条目
    pub fn from_bytes(data: &[u8; PARTITION_ENTRY_SIZE]) -> Option<Self> {
        // 检查魔数
        let magic = u16::from_le_bytes([data[0], data[1]]);
        if magic != PARTITION_TABLE_MAGIC {
            return None;
        }

        let partition_type = PartitionType::from(data[2]);
        let subtype = data[3];
        let offset = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let size = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);

        // 解析标签 (12-27 字节，null 结尾)
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

    /// 检查是否为数据分区
    pub fn is_data(&self) -> bool {
        matches!(self.partition_type, PartitionType::Data)
    }

    /// 检查是否为应用分区
    pub fn is_app(&self) -> bool {
        matches!(self.partition_type, PartitionType::App)
    }

    /// 获取数据子类型
    pub fn data_subtype(&self) -> Option<DataSubType> {
        if self.is_data() {
            Some(DataSubType::from(self.subtype))
        } else {
            None
        }
    }

    /// 获取应用子类型
    pub fn app_subtype(&self) -> Option<AppSubType> {
        if self.is_app() {
            Some(AppSubType::from(self.subtype))
        } else {
            None
        }
    }

    /// 检查是否为 LittleFS 分区
    pub fn is_littlefs(&self) -> bool {
        self.is_data() && self.subtype == DataSubType::LittleFs.as_u8()
    }

    /// 检查是否为 SPIFFS 分区
    pub fn is_spiffs(&self) -> bool {
        self.is_data() && self.subtype == DataSubType::Spiffs.as_u8()
    }

    /// 检查是否为 NVS 分区
    pub fn is_nvs(&self) -> bool {
        self.is_data() && self.subtype == DataSubType::Nvs.as_u8()
    }

    /// 获取分区结束地址
    pub fn end_offset(&self) -> u32 {
        self.offset + self.size
    }

    /// 计算分区包含的块数 (给定块大小)
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

/// 分区表
pub struct PartitionTable {
    /// 分区列表
    partitions: heapless::Vec<Partition, MAX_PARTITION_ENTRIES>,
}

impl PartitionTable {
    /// 创建空分区表
    pub const fn new() -> Self {
        Self {
            partitions: heapless::Vec::new(),
        }
    }

    /// 从 Flash 数据解析分区表
    ///
    /// # 参数
    /// - `data`: 从 PARTITION_TABLE_OFFSET 读取的原始数据
    ///
    /// # 返回
    /// 解析后的分区表，如果解析失败返回 None
    pub fn from_flash_data(data: &[u8]) -> Option<Self> {
        let mut table = Self::new();

        // 分区表数据应该至少包含一个条目
        if data.len() < PARTITION_ENTRY_SIZE {
            return None;
        }

        // 解析每个分区条目
        for chunk in data.chunks_exact(PARTITION_ENTRY_SIZE) {
            let entry_data: &[u8; PARTITION_ENTRY_SIZE] = chunk.try_into().ok()?;

            // 检查是否为结束标记 (全 0xFF 或魔数不匹配)
            if entry_data[0] == 0xFF && entry_data[1] == 0xFF {
                break;
            }

            if let Some(partition) = Partition::from_bytes(entry_data) {
                table.partitions.push(partition).ok()?;
            } else {
                // 无效条目，停止解析
                break;
            }
        }

        if table.partitions.is_empty() {
            None
        } else {
            Some(table)
        }
    }

    /// 手动创建分区 (用于已知分区布局)
    ///
    /// # 参数
    /// - `label`: 分区标签
    /// - `partition_type`: 分区类型
    /// - `subtype`: 子类型
    /// - `offset`: Flash 偏移量
    /// - `size`: 分区大小
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

    /// 按标签查找分区
    pub fn find_by_label(&self, label: &str) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.label.as_str() == label)
    }

    /// 查找第一个 LittleFS 分区
    pub fn find_littlefs(&self) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.is_littlefs())
    }

    /// 查找第一个 SPIFFS 分区
    pub fn find_spiffs(&self) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.is_spiffs())
    }

    /// 查找第一个 NVS 分区
    pub fn find_nvs(&self) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.is_nvs())
    }

    /// 查找指定类型的所有分区
    pub fn find_by_type(&self, partition_type: PartitionType) -> impl Iterator<Item = &Partition> {
        self.partitions.iter().filter(move |p| p.partition_type == partition_type)
    }

    /// 查找指定数据子类型的分区
    pub fn find_data_by_subtype(&self, subtype: DataSubType) -> Option<&Partition> {
        self.partitions.iter().find(|p| {
            p.is_data() && p.subtype == subtype.as_u8()
        })
    }

    /// 获取所有分区
    pub fn partitions(&self) -> &[Partition] {
        &self.partitions
    }

    /// 获取分区数量
    pub fn len(&self) -> usize {
        self.partitions.len()
    }

    /// 检查分区表是否为空
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

/// 常用分区布局预设
pub mod presets {
    use super::*;

    /// 创建默认 4MB Flash 分区布局
    ///
    /// 布局:
    /// - nvs: 0x9000, 24KB
    /// - phy_init: 0xF000, 4KB
    /// - factory: 0x10000, 1MB
    /// - storage: 0x110000, ~2.9MB (LittleFS)
    pub fn default_4mb() -> PartitionTable {
        let mut table = PartitionTable::new();
        
        // NVS 分区
        table.add_partition("nvs", PartitionType::Data, DataSubType::Nvs.as_u8(), 
            0x9000, 0x6000).ok();
        
        // PHY 初始化数据
        table.add_partition("phy_init", PartitionType::Data, DataSubType::Phy.as_u8(),
            0xF000, 0x1000).ok();
        
        // 工厂应用
        table.add_partition("factory", PartitionType::App, AppSubType::Factory.as_u8(),
            0x10000, 0x100000).ok();
        
        // LittleFS 存储分区 (剩余空间)
        table.add_partition("storage", PartitionType::Data, DataSubType::LittleFs.as_u8(),
            0x110000, 0x2F0000).ok();
        
        table
    }

    /// 创建 16MB Flash 分区布局 (适用于 ESP32-S3-N16R8)
    ///
    /// 布局:
    /// - nvs: 0x9000, 24KB
    /// - phy_init: 0xF000, 4KB  
    /// - factory: 0x10000, 4MB
    /// - ota_0: 0x410000, 4MB
    /// - ota_1: 0x810000, 4MB
    /// - storage: 0xC10000, ~4MB (LittleFS)
    pub fn default_16mb_ota() -> PartitionTable {
        let mut table = PartitionTable::new();
        
        // NVS 分区
        table.add_partition("nvs", PartitionType::Data, DataSubType::Nvs.as_u8(),
            0x9000, 0x6000).ok();
        
        // PHY 初始化数据
        table.add_partition("phy_init", PartitionType::Data, DataSubType::Phy.as_u8(),
            0xF000, 0x1000).ok();
        
        // OTA 数据
        table.add_partition("otadata", PartitionType::Data, DataSubType::Ota.as_u8(),
            0x10000, 0x2000).ok();
        
        // 工厂应用
        table.add_partition("factory", PartitionType::App, AppSubType::Factory.as_u8(),
            0x12000, 0x400000).ok();
        
        // OTA 应用 0
        table.add_partition("ota_0", PartitionType::App, 0x10,  // OTA 0
            0x412000, 0x400000).ok();
        
        // OTA 应用 1
        table.add_partition("ota_1", PartitionType::App, 0x11,  // OTA 1
            0x812000, 0x400000).ok();
        
        // LittleFS 存储分区
        table.add_partition("storage", PartitionType::Data, DataSubType::LittleFs.as_u8(),
            0xC12000, 0x3EE000).ok();
        
        table
    }

    /// 创建简单的单应用 16MB 布局 (最大存储空间)
    ///
    /// 布局:
    /// - nvs: 0x9000, 24KB
    /// - factory: 0x10000, 4MB
    /// - storage: 0x410000, ~12MB (LittleFS)
    pub fn simple_16mb() -> PartitionTable {
        let mut table = PartitionTable::new();
        
        // NVS 分区
        table.add_partition("nvs", PartitionType::Data, DataSubType::Nvs.as_u8(),
            0x9000, 0x6000).ok();
        
        // 工厂应用
        table.add_partition("factory", PartitionType::App, AppSubType::Factory.as_u8(),
            0x10000, 0x400000).ok();
        
        // LittleFS 存储分区 (剩余约 12MB)
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
        // 模拟一个有效的分区条目
        let mut data = [0u8; 32];
        data[0] = 0x50; // 魔数低字节
        data[1] = 0xAA; // 魔数高字节
        data[2] = 0x01; // 类型: Data
        data[3] = 0x83; // 子类型: LittleFS
        // offset = 0x00110000
        data[4] = 0x00;
        data[5] = 0x00;
        data[6] = 0x11;
        data[7] = 0x00;
        // size = 0x002F0000
        data[8] = 0x00;
        data[9] = 0x00;
        data[10] = 0x2F;
        data[11] = 0x00;
        // label = "storage"
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
