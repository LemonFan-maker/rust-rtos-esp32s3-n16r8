use core::fmt;

use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_storage::FlashStorage as EspFlash;

use super::partition::Partition;

pub const FLASH_WORD_SIZE: u32 = 4;
pub const FLASH_SECTOR_SIZE: u32 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    ReadError,
    WriteError,
    EraseError,
    OutOfBounds,
    AlignmentError,
    Busy,
    WriteProtected,
    NotInitialized,
    Unsupported,
    Other(i32),
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
            Self::Unsupported => write!(f, "Operation not supported by backend"),
            Self::Other(code) => write!(f, "Flash error code {}", code),
        }
    }
}

fn map_esp_error(
    e: esp_storage::FlashStorageError,
    op_error: StorageError,
) -> StorageError {
    match e {
        esp_storage::FlashStorageError::IoError => op_error,
        esp_storage::FlashStorageError::IoTimeout => StorageError::Busy,
        esp_storage::FlashStorageError::CantUnlock => StorageError::WriteProtected,
        esp_storage::FlashStorageError::NotAligned => StorageError::AlignmentError,
        esp_storage::FlashStorageError::OutOfBounds => StorageError::OutOfBounds,

        esp_storage::FlashStorageError::OtherCoreRunning => StorageError::Busy,
        esp_storage::FlashStorageError::Other(code) => StorageError::Other(code),
        _ => op_error,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FlashConfig {
    pub total_size: u32,
    pub sector_size: u32,
    pub block_size: u32,
    pub page_size: u32,
    pub partition_offset: u32,
    pub partition_size: u32,
}

impl Default for FlashConfig {
    fn default() -> Self {
        Self {
            total_size: 16 * 1024 * 1024,
            sector_size: FLASH_SECTOR_SIZE,
            block_size: FLASH_SECTOR_SIZE,
            page_size: FLASH_WORD_SIZE,
            partition_offset: 0x410000,
            partition_size: 0xBF0000,
        }
    }
}

impl FlashConfig {
    pub fn absolute(&self, offset: u32, len: usize) -> Result<u32, StorageError> {
        let end = offset as u64 + len as u64;
        if end > self.partition_size as u64 {
            return Err(StorageError::OutOfBounds);
        }
        Ok(self.partition_offset + offset)
    }
}

#[derive(Debug)]
pub struct FlashStorage<'d> {
    esp: EspFlash<'d>,
    config: FlashConfig,
    initialized: bool,
}

impl<'d> FlashStorage<'d> {
    pub fn new(flash: esp_hal::peripherals::FLASH<'d>, config: FlashConfig) -> Self {
        Self {
            esp: EspFlash::new(flash),
            config,
            initialized: false,
        }
    }

    pub fn with_defaults(flash: esp_hal::peripherals::FLASH<'d>) -> Self {
        Self::new(flash, FlashConfig::default())
    }

    pub fn from_partition(flash: esp_hal::peripherals::FLASH<'d>, partition: &Partition) -> Self {
        let mut config = FlashConfig::default();
        config.partition_offset = partition.offset;
        config.partition_size = partition.size;
        Self::new(flash, config)
    }

    pub fn multicore_auto_park(mut self) -> Self {
        self.esp = self.esp.multicore_auto_park();
        self
    }

    pub fn init(&mut self) -> Result<(), StorageError> {
        let capacity = self.esp.capacity() as u32;
        self.config.total_size = capacity;

        if self.config.partition_offset as u64 + self.config.partition_size as u64
            > capacity as u64
        {
            return Err(StorageError::OutOfBounds);
        }

        if self.config.sector_size != FLASH_SECTOR_SIZE
            || self.config.page_size != FLASH_WORD_SIZE
        {
            return Err(StorageError::AlignmentError);
        }

        if self.config.block_size % self.config.sector_size != 0 {
            return Err(StorageError::AlignmentError);
        }

        self.initialized = true;
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn config(&self) -> &FlashConfig {
        &self.config
    }

    pub fn capacity(&self) -> u32 {
        self.esp.capacity() as u32
    }

    pub fn block_count(&self) -> u32 {
        self.config.partition_size / self.config.block_size
    }

    pub fn block_size(&self) -> u32 {
        self.config.block_size
    }

    fn block_to_address(&self, block: u32) -> Result<u32, StorageError> {
        let offset = block as u64 * self.config.block_size as u64;
        if offset >= self.config.partition_size as u64 {
            return Err(StorageError::OutOfBounds);
        }
        Ok(self.config.partition_offset + offset as u32)
    }

    pub fn read(&mut self, offset: u32, buffer: &mut [u8]) -> Result<(), StorageError> {
        self.ensure_ready()?;
        if offset % FLASH_WORD_SIZE != 0 || (buffer.len() as u32) % FLASH_WORD_SIZE != 0 {
            return Err(StorageError::AlignmentError);
        }
        let addr = self.config.absolute(offset, buffer.len())?;
        self.esp
            .read(addr, buffer)
            .map_err(|e| map_esp_error(e, StorageError::ReadError))
    }

    pub fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), StorageError> {
        self.ensure_ready()?;
        if offset % FLASH_WORD_SIZE != 0 || (data.len() as u32) % FLASH_WORD_SIZE != 0 {
            return Err(StorageError::AlignmentError);
        }
        let addr = self.config.absolute(offset, data.len())?;
        self.esp
            .write(addr, data)
            .map_err(|e| map_esp_error(e, StorageError::WriteError))
    }

    pub fn erase(&mut self, offset: u32, len: u32) -> Result<(), StorageError> {
        self.ensure_ready()?;
        if offset % FLASH_SECTOR_SIZE != 0 || len % FLASH_SECTOR_SIZE != 0 {
            return Err(StorageError::AlignmentError);
        }
        let addr = self.config.absolute(offset, len as usize)?;
        self.esp
            .erase(addr, addr + len)
            .map_err(|e| map_esp_error(e, StorageError::EraseError))
    }

    fn ensure_ready(&self) -> Result<(), StorageError> {
        if !self.initialized {
            return Err(StorageError::NotInitialized);
        }
        Ok(())
    }

    pub fn read_block(&mut self, block: u32, buffer: &mut [u8]) -> Result<(), StorageError> {
        if buffer.len() as u64 > self.config.block_size as u64 {
            return Err(StorageError::OutOfBounds);
        }
        let base = self.block_to_address(block)? - self.config.partition_offset;
        self.read(base, buffer)
    }

    pub fn write_block(&mut self, block: u32, data: &[u8]) -> Result<(), StorageError> {
        if data.len() as u64 > self.config.block_size as u64 {
            return Err(StorageError::OutOfBounds);
        }
        let base = self.block_to_address(block)? - self.config.partition_offset;
        self.write(base, data)
    }

    pub fn erase_block(&mut self, block: u32) -> Result<(), StorageError> {
        let base = self.block_to_address(block)? - self.config.partition_offset;
        self.erase(base, self.config.block_size)
    }

    pub fn sync(&mut self) -> Result<(), StorageError> {
        self.ensure_ready()
    }

    pub fn read_jedec_id(&mut self) -> Result<[u8; 3], StorageError> {
        Err(StorageError::Unsupported)
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
    fn test_config_absolute() {
        let config = FlashConfig {
            total_size: 16 * 1024 * 1024,
            sector_size: 4096,
            block_size: 4096,
            page_size: 4,
            partition_offset: 0x100000,
            partition_size: 0x200000,
        };

        assert_eq!(config.absolute(0, 4).unwrap(), 0x100000);
        assert_eq!(config.absolute(4096, 4).unwrap(), 0x101000);
        assert_eq!(
            config.absolute(0x200000 - 4, 8),
            Err(StorageError::OutOfBounds)
        );
    }
}
