//! GDMA内存到内存(mem2mem)拷贝
//!
//! ESP32-S3的GDMA引擎支持一个"内存到内存"伪外设: 由DMA硬件完成
//! 一段内存到另一段内存的搬运, 期间不占用CPU、不经过数据缓存。
//! 本模块封装esp-hal的[`Mem2Mem`]/[`SimpleMem2Mem`], 提供:
//! - [`GdmaCopy`]: 复用同一通道与描述符的多次拷贝;
//! - [`copy_blocking`]: 一次性阻塞拷贝。
//!
//! 约束: 源与目标缓冲区必须位于GDMA可达的内部DRAM(见`dma::is_dma_safe`),
//! 且地址按esp-hal要求对齐。描述符数组需为`'static`(通常来自`static`或
//! `esp_hal::dma_descriptors!`)。
//!
//! [`Mem2Mem`]: esp_hal::dma::Mem2Mem
//! [`SimpleMem2Mem`]: esp_hal::dma::SimpleMem2Mem

use esp_hal::dma::{
    AnyGdmaChannel, BurstConfig, DmaChannelConvert, DmaDescriptor, DmaEligible, DmaError,
    Mem2Mem, SimpleMem2Mem,
};
use esp_hal::Blocking;

/// 复用的GDMA内存拷贝器: 持有mem2mem通道与收发描述符。
pub struct GdmaCopy<'d> {
    inner: SimpleMem2Mem<'d, Blocking>,
}

impl<'d> GdmaCopy<'d> {
    /// 用一条GDMA通道与一个DMA兼容外设(如SPI2)构造拷贝器。
    ///
    /// `peripheral`仅用于向GDMA声明一个未被其他DMA使用者占用的外设ID;
    /// mem2mem不真正驱动该外设。
    pub fn new(
        channel: impl DmaChannelConvert<AnyGdmaChannel<'d>>,
        peripheral: impl DmaEligible,
        rx_descriptors: &'d mut [DmaDescriptor],
        tx_descriptors: &'d mut [DmaDescriptor],
    ) -> Result<Self, DmaError> {
        let mem2mem = Mem2Mem::new(channel, peripheral);
        let inner = SimpleMem2Mem::new(
            mem2mem,
            rx_descriptors,
            tx_descriptors,
            BurstConfig::default(),
        )?;
        Ok(Self { inner })
    }

    /// 启动一次mem2mem拷贝并阻塞至完成: 把`src`搬运到`dst`。
    ///
    /// 要求`dst.len() == src.len()`且二者均为GDMA可达内存; 长度非32字节
    /// 对齐时按esp-hal描述符要求处理。返回实际搬运结果错误。
    pub fn copy(&mut self, dst: &mut [u8], src: &[u8]) -> Result<(), DmaError> {
        let len = dst.len().min(src.len());
        let transfer = self.inner.start_transfer(&mut dst[..len], &src[..len])?;
        transfer.wait()
    }
}

/// 一次性阻塞GDMA内存拷贝。适合偶发的大块搬运。
pub fn copy_blocking(
    channel: impl DmaChannelConvert<AnyGdmaChannel<'static>>,
    peripheral: impl DmaEligible,
    rx_descriptors: &'static mut [DmaDescriptor],
    tx_descriptors: &'static mut [DmaDescriptor],
    dst: &mut [u8],
    src: &[u8],
) -> Result<(), DmaError> {
    let mut copier = GdmaCopy::new(channel, peripheral, rx_descriptors, tx_descriptors)?;
    copier.copy(dst, src)
}
