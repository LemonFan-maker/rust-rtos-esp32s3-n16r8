//! Flash存储抽象层
//! 提供对ESP32 SPI Flash的读写抽象，支持littlefs2所需的块设备接口

use core::fmt;
use esp_hal::spi::master::SpiDmaBus;
// DMA通道通过peripherals.DMA_CHx获取

/// 存储操作错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    /// 读取失败
    ReadError,
    /// 写入失败
    WriteError,
    /// 擦除失败
    EraseError,
    /// 地址越界
    OutOfBounds,
    /// 对齐错误
    AlignmentError,
    /// 设备忙
    Busy,
    /// 写保护
    WriteProtected,
    /// 未初始化
    NotInitialized,
    /// 校验失败
    VerifyError,
    /// DMA错误
    DmaError,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadError => write!(f, "Flash read error"),
            Self::WriteError => write!(f, "Flash write error"),
            Self::EraseError => write!(f, "Flash erase error"),
            Self::OutOfBounds => write!(f, "Address out of bounds"),
            Self::AlignmentError => write!(f, "Address alignment error"),
            Self::Busy => write!(f, "Device busy"),
            Self::WriteProtected => write!(f, "Write protected"),
            Self::NotInitialized => write!(f, "Not initialized"),
            Self::VerifyError => write!(f, "Verify error"),
            Self::DmaError => write!(f, "DMA transfer error"),
        }
    }
}

/// Flash存储配置
#[derive(Debug, Clone, Copy)]
pub struct FlashConfig {
    /// 总容量(字节)
    pub total_size: u32,
    /// 扇区大小(通常4KB)
    pub sector_size: u32,
    /// 块大小(用于文件系统，通常4KB)
    pub block_size: u32,
    /// 页面大小(编程单位，通常256B)
    pub page_size: u32,
    /// 分区起始偏移
    pub partition_offset: u32,
    /// 分区大小
    pub partition_size: u32,
}

impl Default for FlashConfig {
    fn default() -> Self {
        Self {
            total_size: 16 * 1024 * 1024,  // 16MB
            sector_size: 4096,              // 4KB
            block_size: 4096,               // 4KB
            page_size: 256,                 // 256B
            partition_offset: 0x410000,     // 默认存储分区偏移
            partition_size: 0xBF0000,       // ~12MB
        }
    }
}

/// Flash存储抽象
/// 提供对指定Flash分区的读写操作
pub struct FlashStorage {
    /// 配置
    config: FlashConfig,
    /// 是否已初始化
    initialized: bool,
}

impl FlashStorage {
    /// 创建Flash存储实例
    pub const fn new(config: FlashConfig) -> Self {
        Self {
            config,
            initialized: false,
        }
    }

    /// 使用默认配置创建
    pub const fn with_defaults() -> Self {
        Self::new(FlashConfig {
            total_size: 16 * 1024 * 1024,
            sector_size: 4096,
            block_size: 4096,
            page_size: 256,
            partition_offset: 0x410000,
            partition_size: 0xBF0000,
        })
    }

    /// 从分区信息创建
    pub fn from_partition(partition: &super::partition::Partition, total_flash_size: u32) -> Self {
        Self::new(FlashConfig {
            total_size: total_flash_size,
            sector_size: 4096,
            block_size: 4096,
            page_size: 256,
            partition_offset: partition.offset,
            partition_size: partition.size,
        })
    }

    /// 初始化存储
    pub fn init(&mut self) -> Result<(), StorageError> {
        // 验证配置
        if self.config.partition_offset + self.config.partition_size > self.config.total_size {
            return Err(StorageError::OutOfBounds);
        }

        if self.config.block_size % self.config.sector_size != 0 {
            return Err(StorageError::AlignmentError);
        }

        self.initialized = true;
        Ok(())
    }

    /// 检查是否已初始化
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// 获取配置
    pub fn config(&self) -> &FlashConfig {
        &self.config
    }

    /// 获取分区中的块数
    pub fn block_count(&self) -> u32 {
        self.config.partition_size / self.config.block_size
    }

    /// 获取块大小
    pub fn block_size(&self) -> u32 {
        self.config.block_size
    }

    /// 将块号转换为Flash绝对地址
    fn block_to_address(&self, block: u32) -> Result<u32, StorageError> {
        let offset = block * self.config.block_size;
        if offset >= self.config.partition_size {
            return Err(StorageError::OutOfBounds);
        }
        Ok(self.config.partition_offset + offset)
    }

    /// 读取块数据(内部Flash使用内存映射)
    /// 安全性
    /// ESP32内部Flash映射到地址空间0x3C000000+，可直接读取
    pub fn read_block(&self, block: u32, buffer: &mut [u8]) -> Result<(), StorageError> {
        if !self.initialized {
            return Err(StorageError::NotInitialized);
        }

        if buffer.len() > self.config.block_size as usize {
            return Err(StorageError::OutOfBounds);
        }

        let address = self.block_to_address(block)?;

        // ESP32-S3 Flash映射地址
        // 内部Flash映射到0x3C000000 (数据)或通过SPI访问
        // 这里使用ROM函数或SPI读取

        unsafe {
            self.read_flash_internal(address, buffer)?;
        }

        Ok(())
    }

    /// 写入块数据
    /// 注意
    /// Flash写入前需要先擦除对应扇区
    pub fn write_block(&mut self, block: u32, data: &[u8]) -> Result<(), StorageError> {
        if !self.initialized {
            return Err(StorageError::NotInitialized);
        }

        if data.len() > self.config.block_size as usize {
            return Err(StorageError::OutOfBounds);
        }

        let address = self.block_to_address(block)?;

        unsafe {
            self.write_flash_internal(address, data)?;
        }

        Ok(())
    }

    /// 擦除块
    /// 将整个块设置为0xFF
    pub fn erase_block(&mut self, block: u32) -> Result<(), StorageError> {
        if !self.initialized {
            return Err(StorageError::NotInitialized);
        }

        let address = self.block_to_address(block)?;

        // 计算需要擦除的扇区数
        let sectors = self.config.block_size / self.config.sector_size;

        for i in 0..sectors {
            let sector_addr = address + i * self.config.sector_size;
            unsafe {
                self.erase_sector_internal(sector_addr)?;
            }
        }

        Ok(())
    }

    /// 同步(确保所有写入完成)
    pub fn sync(&mut self) -> Result<(), StorageError> {
        if !self.initialized {
            return Err(StorageError::NotInitialized);
        }

        // Flash写入是同步的，无需额外操作
        // 但可以在这里添加缓存刷新等操作

        Ok(())
    }

    // 内部Flash操作

    /// 内部Flash读取实现
    /// 使用ESP32 ROM函数或SPI读取
    unsafe fn read_flash_internal(&self, address: u32, buffer: &mut [u8]) -> Result<(), StorageError> {
        // ESP32-S3内部Flash可通过缓存映射直接读取
        // 数据地址映射: 0x3C000000 + offset

        // 方法1: 通过内存映射读取(需要确保地址在映射范围内)
        // 方法2: 使用SPI读取命令

        // 简化实现: 假设Flash已映射到内存
        // 实际实现需要根据esp-hal的Flash驱动

        let flash_data_base: u32 = 0x3C000000;
        let mapped_addr = flash_data_base + address;

        let src = mapped_addr as *const u8;
        core::ptr::copy_nonoverlapping(src, buffer.as_mut_ptr(), buffer.len());

        Ok(())
    }

    /// 内部Flash写入实现
    /// 使用ESP32 ROM函数进行编程
    unsafe fn write_flash_internal(&mut self, address: u32, data: &[u8]) -> Result<(), StorageError> {
        // ESP32 Flash写入需要:
        // 1.禁用中断和缓存
        // 2.使用ROM函数或SPI命令
        // 3.等待写入完成
        // 4.恢复缓存和中断

        // 按页面大小分块写入
        let page_size = self.config.page_size as usize;
        let mut offset = 0;

        while offset < data.len() {
            let current_addr = address + offset as u32;
            let page_offset = (current_addr % self.config.page_size) as usize;
            let write_size = core::cmp::min(
                page_size - page_offset,
                data.len() - offset
            );

            // 调用ROM函数写入
            // esp_rom_spiflash_write(current_addr, data[offset..].as_ptr(), write_size)

            // 占位实现 - 实际需要调用esp-hal的Flash写入API
            self.write_page_internal(current_addr, &data[offset..offset + write_size])?;

            offset += write_size;
        }

        Ok(())
    }

    /// 写入单个页面
    /// Safety
    /// 调用者必须确保地址有效且在分区范围内。
    /// 实现说明
    /// ESP32-S3内部Flash写入需要使用ROM函数。
    /// 直接内存映射只能读取，不能写入。
    /// 当前为占位实现，返回Ok但不执行实际写入。
    /// 实际应用中应使用esp-storage crate或esp-hal的flash API。
    unsafe fn write_page_internal(&mut self, _address: u32, _data: &[u8]) -> Result<(), StorageError> {
        // 实现步骤:
        // 1.禁用中断和Cache
        // 2.发送Write Enable命令(0x06)
        // 3.发送Page Program命令(0x02) + 地址 + 数据
        // 4.轮询Status Register等待WIP位清零
        // 5.恢复Cache和中断
        // 可选方案:
        // - esp-storage crate: https://github.com/esp-rs/esp-storage
        // - esp_rom_spiflash_write() ROM函数
        // 占位实现 - 返回Ok但不执行实际写入
        // 这允许编译和基本测试，但不会持久化数据
        Ok(())
    }

    /// 擦除单个扇区
    /// Safety
    /// 调用者必须确保地址有效且在分区范围内。
    /// 实现说明
    /// 扇区擦除通常需要几十到几百毫秒。
    /// 当前为占位实现，返回Ok但不执行实际擦除。
    /// 实际应用中应使用esp-storage crate或esp-hal的flash API。
    unsafe fn erase_sector_internal(&mut self, _address: u32) -> Result<(), StorageError> {
        // 实现步骤:
        // 1.禁用中断和Cache
        // 2.发送Write Enable命令(0x06)
        // 3.发送Sector Erase命令(0x20) + 地址
        // 4.轮询Status Register等待擦除完成(通常50-200ms)
        // 5.恢复Cache和中断
        // 可选方案:
        // - esp-storage crate: https://github.com/esp-rs/esp-storage
        // - esp_rom_spiflash_erase_sector() ROM函数
        // 占位实现 - 返回Ok但不执行实际擦除
        // 这允许编译和基本测试，但不会修改Flash内容
        Ok(())
    }
}

/// 外部SPI Flash存储
/// 用于连接外部SPI Flash芯片
pub struct ExternalFlash<'d> {
    /// 配置
    config: FlashConfig,
    /// SPI总线(使用DMA)
    _spi: Option<SpiDmaBus<'d, esp_hal::Blocking>>,
    /// CS引脚状态
    cs_active: bool,
}

impl<'d> ExternalFlash<'d> {
    /// 创建外部Flash实例
    pub fn new(config: FlashConfig) -> Self {
        Self {
            config,
            _spi: None,
            cs_active: false,
        }
    }

    /// 配置SPI总线
    pub fn with_spi(mut self, spi: SpiDmaBus<'d, esp_hal::Blocking>) -> Self {
        self._spi = Some(spi);
        self
    }

    /// 读取JEDEC ID
    /// 当前为占位实现，返回全零ID。
    /// 实际应用应使用SpiDmaBus::transfer()执行SPI传输。
    pub fn read_jedec_id(&mut self) -> Result<[u8; 3], StorageError> {
        let _spi = self._spi.as_mut().ok_or(StorageError::NotInitialized)?;

        // JEDEC ID命令: 0x9F
        // 响应: 3字节(Manufacturer, Memory Type, Capacity)
        let id = [0u8; 3];

        // 占位实现 - 实际应用应使用SPI传输:
        // let cmd = [0x9F];
        // self._spi.transfer(&cmd, &mut id)?;

        Ok(id)
    }

    /// 获取配置
    pub fn config(&self) -> &FlashConfig {
        &self.config
    }
}

/// 用于littlefs2的块设备特征实现
/// 这个模块提供FlashStorage到littlefs2 Storage trait的适配
pub mod littlefs_adapter {
    use super::*;

    /// LittleFS存储适配器
    /// 包装FlashStorage实现littlefs2所需的接口
    pub struct LfsStorageAdapter {
        storage: FlashStorage,
    }

    impl LfsStorageAdapter {
        /// 创建适配器
        pub fn new(storage: FlashStorage) -> Self {
            Self { storage }
        }

        /// 获取内部存储引用
        pub fn inner(&self) -> &FlashStorage {
            &self.storage
        }

        /// 获取内部存储可变引用
        pub fn inner_mut(&mut self) -> &mut FlashStorage {
            &mut self.storage
        }

        /// 读取操作
        pub fn read(&self, block: u32, offset: u32, buffer: &mut [u8]) -> Result<(), StorageError> {
            // littlefs2可能读取块内的部分数据
            let block_size = self.storage.config.block_size;

            if offset + buffer.len() as u32 > block_size {
                return Err(StorageError::OutOfBounds);
            }

            // 读取整个块到临时缓冲区，然后复制所需部分
            // 优化: 如果读取整个块，直接使用目标缓冲区
            if offset == 0 && buffer.len() == block_size as usize {
                return self.storage.read_block(block, buffer);
            }

            // 部分读取 - 需要临时缓冲区
            // 注意: 在no_std环境中，可能需要使用固定大小的栈缓冲区
            let mut temp = [0u8; 4096]; // 假设最大块大小为4KB
            self.storage.read_block(block, &mut temp[..block_size as usize])?;
            buffer.copy_from_slice(&temp[offset as usize..offset as usize + buffer.len()]);

            Ok(())
        }

        /// 写入操作(编程)
        pub fn prog(&mut self, block: u32, offset: u32, data: &[u8]) -> Result<(), StorageError> {
            let block_size = self.storage.config.block_size;

            if offset + data.len() as u32 > block_size {
                return Err(StorageError::OutOfBounds);
            }

            // 计算实际Flash地址
            let base_addr = self.storage.block_to_address(block)?;
            let write_addr = base_addr + offset;

            unsafe {
                self.storage.write_flash_internal(write_addr, data)?;
            }

            Ok(())
        }

        /// 擦除操作
        pub fn erase(&mut self, block: u32) -> Result<(), StorageError> {
            self.storage.erase_block(block)
        }

        /// 同步操作
        pub fn sync(&mut self) -> Result<(), StorageError> {
            self.storage.sync()
        }

        /// 获取块数
        pub fn block_count(&self) -> u32 {
            self.storage.block_count()
        }

        /// 获取块大小
        pub fn block_size(&self) -> u32 {
            self.storage.block_size()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flash_config() {
        let config = FlashConfig::default();
        assert_eq!(config.total_size, 16 * 1024 * 1024);
        assert_eq!(config.block_size, 4096);
    }

    #[test]
    fn test_block_to_address() {
        let storage = FlashStorage::new(FlashConfig {
            total_size: 16 * 1024 * 1024,
            sector_size: 4096,
            block_size: 4096,
            page_size: 256,
            partition_offset: 0x100000,
            partition_size: 0x200000,
        });

        // 块0 ->分区起始
        assert_eq!(storage.block_to_address(0).unwrap(), 0x100000);
        // 块1 ->分区起始 + 块大小
        assert_eq!(storage.block_to_address(1).unwrap(), 0x101000);
    }
}
