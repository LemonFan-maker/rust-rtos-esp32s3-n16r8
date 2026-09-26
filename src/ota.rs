use core::fmt;

use esp_bootloader_esp_idf::ota::OtaImageState;
use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
use esp_bootloader_esp_idf::partitions::{
    AppPartitionSubType, Error as PartitionError, PARTITION_TABLE_MAX_LEN,
};

pub const OTA_WRITE_SIZE: usize = 4;
pub const OTA_ERASE_SIZE: usize = 4096;
pub const ESP_IMAGE_MAGIC: u8 = 0xE9;

#[derive(Debug)]
pub enum OtaError {
    Bootloader(PartitionError),
    InvalidImageHeader,
    InvalidImageLength,
    ImageTooLarge,
    NotComplete,
    TargetChanged,
    CapacityOverflow,
}

impl fmt::Display for OtaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bootloader(error) => write!(f, "bootloader OTA error: {:?}", error),
            Self::InvalidImageHeader => write!(f, "invalid ESP image header"),
            Self::InvalidImageLength => write!(f, "image length is not a non-zero 4-byte multiple"),
            Self::ImageTooLarge => write!(f, "image does not fit in the next OTA slot"),
            Self::NotComplete => write!(f, "OTA image is incomplete"),
            Self::CapacityOverflow => write!(f, "OTA erase range exceeds the partition capacity"),
            Self::TargetChanged => write!(f, "OTA target slot changed during update"),
        }
    }
}

impl From<PartitionError> for OtaError {
    fn from(error: PartitionError) -> Self {
        Self::Bootloader(error)
    }
}

pub struct OtaUpdate {
    pub slot: AppPartitionSubType,
    pub bytes_written: usize,
}

/// Streaming writer for one complete ESP-IDF application image.
///
/// The caller must erase the target first, feed the image bytes in order, and call
/// `finish` only after the complete image has arrived. The bootloader selection is
/// changed to `New` only by `finish`, so a failed transfer does not activate a partial image.
pub struct OtaSession<'a, F>
where
    F: embedded_storage::Storage + embedded_storage::nor_flash::NorFlash,
{
    updater: OtaUpdater<'a, F>,
    target_slot: AppPartitionSubType,
    image_len: usize,
    erase_len: usize,
    received: usize,
    written: usize,
    pending: [u8; OTA_WRITE_SIZE],
    pending_len: usize,
}

impl<'a, F> OtaSession<'a, F>
where
    F: embedded_storage::Storage + embedded_storage::nor_flash::NorFlash,
{
    pub fn begin(
        flash: &'a mut F,
        partition_table: &'a mut [u8; PARTITION_TABLE_MAX_LEN],
        image_len: usize,
    ) -> Result<Self, OtaError> {
        if image_len == 0 || image_len % OTA_WRITE_SIZE != 0 {
            return Err(OtaError::InvalidImageLength);
        }

        let mut updater = OtaUpdater::new(flash, partition_table)?;
        let (capacity, target_slot) = {
            let (region, slot) = updater.next_partition()?;
            (region.partition_size(), slot)
        };

        let erase_len = align_up(image_len, OTA_ERASE_SIZE);
        if image_len > capacity || erase_len > capacity {
            return Err(OtaError::ImageTooLarge);
        }

        Ok(Self {
            updater,
            target_slot,
            image_len,
            erase_len,
            received: 0,
            written: 0,
            pending: [0; OTA_WRITE_SIZE],
            pending_len: 0,
        })
    }

    pub fn target_slot(&self) -> AppPartitionSubType {
        self.target_slot
    }

    pub fn image_len(&self) -> usize {
        self.image_len
    }

    pub fn erase_target(&mut self) -> Result<(), OtaError> {
        let (mut region, slot) = self.updater.next_partition()?;
        if slot != self.target_slot {
            return Err(OtaError::TargetChanged);
        }
        if self.erase_len > u32::MAX as usize {
            return Err(OtaError::CapacityOverflow);
        }
        embedded_storage::nor_flash::NorFlash::erase(
            &mut region,
            0,
            self.erase_len as u32,
        )?;
        Ok(())
    }

    /// Feed the next contiguous bytes from the application image.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), OtaError> {
        let next_received = self
            .received
            .checked_add(bytes.len())
            .ok_or(OtaError::InvalidImageLength)?;
        if next_received > self.image_len {
            return Err(OtaError::InvalidImageLength);
        }

        for byte in bytes {
            self.pending[self.pending_len] = *byte;
            self.pending_len += 1;
            self.received += 1;

            if self.pending_len == OTA_WRITE_SIZE {
                let word = self.pending;
                self.write_aligned(&word)?;
                self.pending_len = 0;
                self.pending = [0; OTA_WRITE_SIZE];
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<OtaUpdate, OtaError> {
        if self.received != self.image_len || self.pending_len != 0 {
            return Err(OtaError::NotComplete);
        }

        self.updater.activate_next_partition()?;
        self.updater.set_current_ota_state(OtaImageState::New)?;

        Ok(OtaUpdate {
            slot: self.target_slot,
            bytes_written: self.written,
        })
    }

    fn write_aligned(&mut self, bytes: &[u8; OTA_WRITE_SIZE]) -> Result<(), OtaError> {
        let (mut region, slot) = self.updater.next_partition()?;
        if slot != self.target_slot {
            return Err(OtaError::TargetChanged);
        }

        embedded_storage::nor_flash::NorFlash::write(
            &mut region,
            self.written as u32,
            bytes,
        )?;
        self.written += bytes.len();
        Ok(())
    }
}

/// Mark the currently selected image valid after its application-level self-checks pass.
pub fn mark_current_valid(
    flash: &mut impl embedded_storage::Storage,
    partition_table: &mut [u8; PARTITION_TABLE_MAX_LEN],
) -> Result<bool, OtaError> {
    let mut updater = OtaUpdater::new(flash, partition_table)?;
    match updater.current_ota_state() {
        Ok(OtaImageState::PendingVerify) => {
            updater.set_current_ota_state(OtaImageState::Valid)?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(PartitionError::InvalidState) => Ok(false),
        Err(error) => Err(error.into()),
    }
}
pub fn validate_image_header(header: &[u8]) -> Result<(), OtaError> {
    if header.first().copied() != Some(ESP_IMAGE_MAGIC) {
        return Err(OtaError::InvalidImageHeader);
    }
    Ok(())
}

const fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}
