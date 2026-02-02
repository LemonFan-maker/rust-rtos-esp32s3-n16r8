//! LittleFS 文件系统封装
//!
//! 提供基于 littlefs2 的文件系统操作 API

use core::fmt;
use super::storage::{FlashStorage, StorageError};

/// 文件系统错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    /// 存储层错误
    Storage(StorageError),
    /// 文件系统损坏
    Corrupt,
    /// 文件/目录不存在
    NotFound,
    /// 文件/目录已存在
    AlreadyExists,
    /// 不是目录
    NotADirectory,
    /// 不是文件
    NotAFile,
    /// 目录非空
    DirectoryNotEmpty,
    /// 无效参数
    InvalidParam,
    /// 路径过长
    PathTooLong,
    /// 文件名过长
    NameTooLong,
    /// 空间不足
    NoSpace,
    /// 文件系统已满
    Full,
    /// 打开的文件过多
    TooManyOpenFiles,
    /// 无效的文件句柄
    InvalidHandle,
    /// 文件系统未挂载
    NotMounted,
    /// 挂载失败
    MountFailed,
    /// 格式化失败
    FormatFailed,
    /// IO 错误
    IoError,
}

impl From<StorageError> for FsError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(e) => write!(f, "Storage error: {}", e),
            Self::Corrupt => write!(f, "Filesystem corrupt"),
            Self::NotFound => write!(f, "Not found"),
            Self::AlreadyExists => write!(f, "Already exists"),
            Self::NotADirectory => write!(f, "Not a directory"),
            Self::NotAFile => write!(f, "Not a file"),
            Self::DirectoryNotEmpty => write!(f, "Directory not empty"),
            Self::InvalidParam => write!(f, "Invalid parameter"),
            Self::PathTooLong => write!(f, "Path too long"),
            Self::NameTooLong => write!(f, "Name too long"),
            Self::NoSpace => write!(f, "No space"),
            Self::Full => write!(f, "Filesystem full"),
            Self::TooManyOpenFiles => write!(f, "Too many open files"),
            Self::InvalidHandle => write!(f, "Invalid handle"),
            Self::NotMounted => write!(f, "Not mounted"),
            Self::MountFailed => write!(f, "Mount failed"),
            Self::FormatFailed => write!(f, "Format failed"),
            Self::IoError => write!(f, "IO error"),
        }
    }
}

/// 文件类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// 普通文件
    File,
    /// 目录
    Directory,
}

/// 文件元数据
#[derive(Debug, Clone)]
pub struct Metadata {
    /// 文件类型
    pub file_type: FileType,
    /// 文件大小 (目录为 0)
    pub size: u32,
    /// 文件名
    pub name: heapless::String<64>,
}

impl Metadata {
    /// 是否为文件
    pub fn is_file(&self) -> bool {
        matches!(self.file_type, FileType::File)
    }

    /// 是否为目录
    pub fn is_dir(&self) -> bool {
        matches!(self.file_type, FileType::Directory)
    }
}

/// 文件打开选项
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenOptions {
    /// 读取权限
    pub read: bool,
    /// 写入权限
    pub write: bool,
    /// 如果不存在则创建
    pub create: bool,
    /// 创建新文件 (如果存在则失败)
    pub create_new: bool,
    /// 追加模式
    pub append: bool,
    /// 截断文件
    pub truncate: bool,
}

impl OpenOptions {
    /// 创建新的打开选项
    pub const fn new() -> Self {
        Self {
            read: false,
            write: false,
            create: false,
            create_new: false,
            append: false,
            truncate: false,
        }
    }

    /// 设置读取权限
    pub const fn read(mut self, read: bool) -> Self {
        self.read = read;
        self
    }

    /// 设置写入权限
    pub const fn write(mut self, write: bool) -> Self {
        self.write = write;
        self
    }

    /// 设置创建标志
    pub const fn create(mut self, create: bool) -> Self {
        self.create = create;
        self
    }

    /// 设置创建新文件标志
    pub const fn create_new(mut self, create_new: bool) -> Self {
        self.create_new = create_new;
        self
    }

    /// 设置追加模式
    pub const fn append(mut self, append: bool) -> Self {
        self.append = append;
        self
    }

    /// 设置截断标志
    pub const fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    /// 只读打开
    pub const fn read_only() -> Self {
        Self::new().read(true)
    }

    /// 只写打开 (创建或截断)
    pub const fn write_only() -> Self {
        Self::new().write(true).create(true).truncate(true)
    }

    /// 读写打开
    pub const fn read_write() -> Self {
        Self::new().read(true).write(true)
    }

    /// 追加模式打开
    pub const fn append_mode() -> Self {
        Self::new().write(true).create(true).append(true)
    }
}

/// 文件句柄
pub struct File<'a> {
    /// 文件系统引用
    fs: &'a FileSystem,
    /// 内部文件 ID
    id: u32,
    /// 打开选项
    options: OpenOptions,
    /// 当前位置
    position: u32,
    /// 文件大小 (缓存)
    size: u32,
}

impl<'a> File<'a> {
    /// 读取数据
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, FsError> {
        if !self.options.read {
            return Err(FsError::InvalidParam);
        }

        // 计算可读取的字节数
        let available = self.size.saturating_sub(self.position) as usize;
        let to_read = core::cmp::min(buffer.len(), available);

        if to_read == 0 {
            return Ok(0);
        }

        // 调用底层读取
        let read = self.fs.read_file_internal(self.id, self.position, &mut buffer[..to_read])?;
        self.position += read as u32;

        Ok(read)
    }

    /// 写入数据
    pub fn write(&mut self, data: &[u8]) -> Result<usize, FsError> {
        if !self.options.write {
            return Err(FsError::InvalidParam);
        }

        // 调用底层写入
        let written = self.fs.write_file_internal(self.id, self.position, data)?;
        self.position += written as u32;

        // 更新文件大小
        if self.position > self.size {
            self.size = self.position;
        }

        Ok(written)
    }

    /// 写入全部数据
    pub fn write_all(&mut self, data: &[u8]) -> Result<(), FsError> {
        let mut offset = 0;
        while offset < data.len() {
            let written = self.write(&data[offset..])?;
            if written == 0 {
                return Err(FsError::NoSpace);
            }
            offset += written;
        }
        Ok(())
    }

    /// 移动文件指针
    pub fn seek(&mut self, pos: SeekFrom) -> Result<u32, FsError> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.size as i64 + offset,
            SeekFrom::Current(offset) => self.position as i64 + offset,
        };

        if new_pos < 0 {
            return Err(FsError::InvalidParam);
        }

        self.position = new_pos as u32;
        Ok(self.position)
    }

    /// 获取当前位置
    pub fn position(&self) -> u32 {
        self.position
    }

    /// 获取文件大小
    pub fn size(&self) -> u32 {
        self.size
    }

    /// 同步文件到存储
    pub fn sync(&mut self) -> Result<(), FsError> {
        self.fs.sync_file_internal(self.id)
    }

    /// 截断文件到指定大小
    pub fn truncate(&mut self, size: u32) -> Result<(), FsError> {
        if !self.options.write {
            return Err(FsError::InvalidParam);
        }

        self.fs.truncate_file_internal(self.id, size)?;
        self.size = size;

        if self.position > size {
            self.position = size;
        }

        Ok(())
    }
}

/// 文件指针位置
#[derive(Debug, Clone, Copy)]
pub enum SeekFrom {
    /// 从文件开头
    Start(u32),
    /// 从文件末尾
    End(i64),
    /// 从当前位置
    Current(i64),
}

/// 目录迭代器
pub struct Dir<'a> {
    /// 文件系统引用
    fs: &'a FileSystem,
    /// 内部目录 ID
    id: u32,
    /// 迭代索引
    index: u32,
}

impl<'a> Dir<'a> {
    /// 读取下一个目录项
    pub fn next(&mut self) -> Result<Option<Metadata>, FsError> {
        let result = self.fs.read_dir_internal(self.id, self.index)?;
        if result.is_some() {
            self.index += 1;
        }
        Ok(result)
    }

    /// 重置迭代器到开头
    pub fn rewind(&mut self) {
        self.index = 0;
    }
}

/// 文件系统配置
#[derive(Debug, Clone, Copy)]
pub struct FsConfig {
    /// 块大小
    pub block_size: u32,
    /// 总块数
    pub block_count: u32,
    /// 读缓冲区大小
    pub read_size: u32,
    /// 写缓冲区大小 (编程大小)
    pub prog_size: u32,
    /// 块缓存大小
    pub cache_size: u32,
    /// lookahead 缓冲区大小
    pub lookahead_size: u32,
    /// 块周期 (磨损均衡)
    pub block_cycles: i32,
}

impl Default for FsConfig {
    fn default() -> Self {
        Self {
            block_size: 4096,
            block_count: 0,  // 从存储获取
            read_size: 256,
            prog_size: 256,
            cache_size: 512,
            lookahead_size: 16,
            block_cycles: 500,
        }
    }
}

/// LittleFS 文件系统
pub struct FileSystem {
    /// 存储适配器
    storage: super::storage::littlefs_adapter::LfsStorageAdapter,
    /// 文件系统配置
    config: FsConfig,
    /// 是否已挂载
    mounted: bool,
    /// 下一个文件 ID
    next_file_id: u32,
    /// 下一个目录 ID
    next_dir_id: u32,
}

impl FileSystem {
    /// 创建文件系统实例
    pub fn new(storage: FlashStorage) -> Self {
        let adapter = super::storage::littlefs_adapter::LfsStorageAdapter::new(storage);
        let block_count = adapter.block_count();

        Self {
            storage: adapter,
            config: FsConfig {
                block_count,
                ..Default::default()
            },
            mounted: false,
            next_file_id: 1,
            next_dir_id: 1,
        }
    }

    /// 使用自定义配置创建
    pub fn with_config(storage: FlashStorage, mut config: FsConfig) -> Self {
        let adapter = super::storage::littlefs_adapter::LfsStorageAdapter::new(storage);
        
        if config.block_count == 0 {
            config.block_count = adapter.block_count();
        }

        Self {
            storage: adapter,
            config,
            mounted: false,
            next_file_id: 1,
            next_dir_id: 1,
        }
    }

    /// 挂载文件系统
    ///
    /// # 实现说明
    /// 当前使用简化的魔数检查。完整实现应使用 littlefs2 crate:
    /// ```ignore
    /// use littlefs2::fs::Filesystem;
    /// let mut alloc = Filesystem::allocate();
    /// Filesystem::mount(&mut alloc, storage)?;
    /// ```
    pub fn mount(&mut self) -> Result<(), FsError> {
        if self.mounted {
            return Ok(());
        }

        // 初始化存储
        self.storage.inner_mut().init()?;

        // 简化实现: 读取超级块验证魔数
        // 完整实现应使用 littlefs2::fs::Filesystem::mount()
        let mut buffer = [0u8; 4096];
        self.storage.read(0, 0, &mut buffer)?;
        
        // 检查 littlefs 魔数 "littlefs"
        if &buffer[8..16] != b"littlefs" {
            return Err(FsError::Corrupt);
        }

        self.mounted = true;
        Ok(())
    }

    /// 卸载文件系统
    ///
    /// # 实现说明
    /// 完整实现应使用 littlefs2 crate 的 unmount 方法。
    pub fn unmount(&mut self) -> Result<(), FsError> {
        if !self.mounted {
            return Ok(());
        }

        // 同步所有数据
        self.storage.sync()?;

        // 简化实现: 仅更新状态
        // 完整实现应调用 littlefs2::fs::Filesystem::unmount()

        self.mounted = false;
        Ok(())
    }

    /// 格式化文件系统
    ///
    /// # 实现说明
    /// 当前使用简化实现，只写入基本的魔数。
    /// 完整实现应使用 littlefs2 crate:
    /// ```ignore
    /// use littlefs2::fs::Filesystem;
    /// Filesystem::format(storage)?;
    /// ```
    pub fn format(&mut self) -> Result<(), FsError> {
        // 如果已挂载，先卸载
        if self.mounted {
            self.unmount()?;
        }

        // 初始化存储
        self.storage.inner_mut().init()?;

        // 简化实现: 擦除前几个块并写入超级块
        // 完整实现应使用 littlefs2::fs::Filesystem::format()
        for block in 0..core::cmp::min(4, self.config.block_count) {
            self.storage.erase(block)?;
        }

        // 写入简化的超级块 (包含 littlefs 魔数)
        let mut superblock = [0xFFu8; 4096];
        superblock[8..16].copy_from_slice(b"littlefs");
        superblock[0..4].copy_from_slice(&0x00000002u32.to_le_bytes()); // version
        superblock[4..8].copy_from_slice(&self.config.block_size.to_le_bytes());
        
        self.storage.prog(0, 0, &superblock)?;
        self.storage.sync()?;

        Ok(())
    }

    /// 检查是否已挂载
    pub fn is_mounted(&self) -> bool {
        self.mounted
    }

    /// 获取配置
    pub fn config(&self) -> &FsConfig {
        &self.config
    }

    /// 获取已用空间 (块数)
    ///
    /// # 实现说明
    /// 当前返回 0，完整实现应使用 littlefs2 的 fs_size() 方法。
    pub fn used_blocks(&self) -> Result<u32, FsError> {
        if !self.mounted {
            return Err(FsError::NotMounted);
        }

        // 占位实现 - 完整实现应使用 littlefs2::fs::Filesystem::size()
        
        Ok(0) // 占位
    }

    /// 获取可用空间 (块数)
    pub fn free_blocks(&self) -> Result<u32, FsError> {
        let used = self.used_blocks()?;
        Ok(self.config.block_count.saturating_sub(used))
    }

    /// 获取总空间 (字节)
    pub fn total_bytes(&self) -> u32 {
        self.config.block_count * self.config.block_size
    }

    // ==================== 文件操作 ====================

    /// 打开文件
    ///
    /// # 实现说明
    /// 当前为占位实现，返回模拟的 File 结构。
    /// 完整实现应使用 littlefs2 crate 的 file_open 方法。
    pub fn open(&self, path: &str, options: OpenOptions) -> Result<File<'_>, FsError> {
        if !self.mounted {
            return Err(FsError::NotMounted);
        }

        // 占位实现 - 完整实现应使用 littlefs2::fs::Filesystem::open()
        let id = self.allocate_file_id();
        let size = if options.truncate { 0 } else { self.get_file_size(path)? };

        Ok(File {
            fs: self,
            id,
            options,
            position: if options.append { size } else { 0 },
            size,
        })
    }

    /// 创建文件
    pub fn create(&self, path: &str) -> Result<File<'_>, FsError> {
        self.open(path, OpenOptions::write_only())
    }

    /// 删除文件
    ///
    /// # 实现说明
    /// 当前为占位实现。完整实现应使用 littlefs2 的 remove 方法。
    pub fn remove(&self, path: &str) -> Result<(), FsError> {
        if !self.mounted {
            return Err(FsError::NotMounted);
        }

        // 占位实现 - 完整实现应使用 littlefs2::fs::Filesystem::remove()
        let _ = path;
        Ok(())
    }

    /// 重命名文件/目录
    ///
    /// # 实现说明
    /// 当前为占位实现。完整实现应使用 littlefs2 的 rename 方法。
    pub fn rename(&self, old_path: &str, new_path: &str) -> Result<(), FsError> {
        if !self.mounted {
            return Err(FsError::NotMounted);
        }

        // 占位实现 - 完整实现应使用 littlefs2::fs::Filesystem::rename()
        let _ = (old_path, new_path);
        Ok(())
    }

    /// 获取文件元数据
    ///
    /// # 实现说明
    /// 当前返回默认值。完整实现应使用 littlefs2 的 stat 方法。
    pub fn metadata(&self, path: &str) -> Result<Metadata, FsError> {
        if !self.mounted {
            return Err(FsError::NotMounted);
        }

        // 占位实现 - 完整实现应使用 littlefs2::fs::Filesystem::metadata()
        let _ = path;
        
        Ok(Metadata {
            file_type: FileType::File,
            size: 0,
            name: heapless::String::new(),
        })
    }

    /// 检查文件是否存在
    pub fn exists(&self, path: &str) -> Result<bool, FsError> {
        match self.metadata(path) {
            Ok(_) => Ok(true),
            Err(FsError::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }

    // ==================== 目录操作 ====================

    /// 创建目录
    ///
    /// # 实现说明
    /// 当前为占位实现。完整实现应使用 littlefs2 的 mkdir 方法。
    pub fn create_dir(&self, path: &str) -> Result<(), FsError> {
        if !self.mounted {
            return Err(FsError::NotMounted);
        }

        // 占位实现 - 完整实现应使用 littlefs2::fs::Filesystem::create_dir()
        let _ = path;
        Ok(())
    }

    /// 创建目录 (包括父目录)
    pub fn create_dir_all(&self, path: &str) -> Result<(), FsError> {
        if !self.mounted {
            return Err(FsError::NotMounted);
        }

        // 逐级创建目录
        let mut current_path = heapless::String::<256>::new();
        
        for component in path.split('/').filter(|s| !s.is_empty()) {
            current_path.push('/').map_err(|_| FsError::PathTooLong)?;
            current_path.push_str(component).map_err(|_| FsError::PathTooLong)?;
            
            match self.create_dir(current_path.as_str()) {
                Ok(()) => {}
                Err(FsError::AlreadyExists) => {}
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    /// 删除空目录
    pub fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        self.remove(path)
    }

    /// 打开目录进行遍历
    ///
    /// # 实现说明
    /// 当前为占位实现。完整实现应使用 littlefs2 的 dir_open 方法。
    pub fn read_dir(&self, path: &str) -> Result<Dir<'_>, FsError> {
        if !self.mounted {
            return Err(FsError::NotMounted);
        }

        // 占位实现 - 完整实现应使用 littlefs2::fs::Filesystem::read_dir()
        let _ = path;
        let id = self.allocate_dir_id();

        Ok(Dir {
            fs: self,
            id,
            index: 0,
        })
    }

    // ==================== 内部方法 ====================

    fn allocate_file_id(&self) -> u32 {
        // 简化实现，实际需要原子操作
        // self.next_file_id.fetch_add(1, Ordering::Relaxed)
        1
    }

    fn allocate_dir_id(&self) -> u32 {
        // 简化实现
        1
    }

    fn get_file_size(&self, _path: &str) -> Result<u32, FsError> {
        // 占位实现 - 完整实现应使用 littlefs2::fs::Filesystem::metadata()
        Ok(0)
    }

    fn read_file_internal(&self, _id: u32, _offset: u32, buffer: &mut [u8]) -> Result<usize, FsError> {
        // 占位实现 - 完整实现应使用 littlefs2 文件读取 API
        Ok(buffer.len())
    }

    fn write_file_internal(&self, _id: u32, _offset: u32, data: &[u8]) -> Result<usize, FsError> {
        // 占位实现 - 完整实现应使用 littlefs2 文件写入 API
        Ok(data.len())
    }

    fn sync_file_internal(&self, _id: u32) -> Result<(), FsError> {
        // 占位实现 - 完整实现应使用 littlefs2 文件同步 API
        self.storage.inner().config(); // 保持对 storage 的引用
        Ok(())
    }

    fn truncate_file_internal(&self, _id: u32, _size: u32) -> Result<(), FsError> {
        // 占位实现 - 完整实现应使用 littlefs2 文件截断 API
        Ok(())
    }

    fn read_dir_internal(&self, _id: u32, _index: u32) -> Result<Option<Metadata>, FsError> {
        // 占位实现 - 完整实现应使用 littlefs2 目录读取 API
        Ok(None)
    }
}

impl Drop for FileSystem {
    fn drop(&mut self) {
        if self.mounted {
            let _ = self.unmount();
        }
    }
}

/// 便捷宏: 简化文件读取
#[macro_export]
macro_rules! read_file {
    ($fs:expr, $path:expr) => {{
        let mut file = $fs.open($path, $crate::fs::OpenOptions::read_only())?;
        let mut buffer = [0u8; 1024];
        let size = file.read(&mut buffer)?;
        &buffer[..size]
    }};
}

/// 便捷宏: 简化文件写入
#[macro_export]
macro_rules! write_file {
    ($fs:expr, $path:expr, $data:expr) => {{
        let mut file = $fs.create($path)?;
        file.write_all($data)?;
        file.sync()?;
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_options() {
        let opts = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true);
        
        assert!(opts.read);
        assert!(opts.write);
        assert!(opts.create);
        assert!(!opts.truncate);
    }

    #[test]
    fn test_seek_from() {
        // 测试 SeekFrom 枚举
        let start = SeekFrom::Start(100);
        let end = SeekFrom::End(-50);
        let current = SeekFrom::Current(10);
        
        // 只验证构造
        assert!(matches!(start, SeekFrom::Start(100)));
        assert!(matches!(end, SeekFrom::End(-50)));
        assert!(matches!(current, SeekFrom::Current(10)));
    }
}
