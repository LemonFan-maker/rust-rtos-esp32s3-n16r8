use core::fmt;

use littlefs2::consts::{U16, U256};
use littlefs2::driver;
use littlefs2::fs::{
    Allocation, File as LfsFile, Filesystem as LfsFilesystem, OpenOptions as LfsOpenOptions,
    ReadDir as LfsReadDir,
};
use littlefs2::io::{
    self as lfsio, Error as LfsError, SeekFrom as LfsSeekFrom,
};
use littlefs2::path::PathBuf;

use super::storage::{FLASH_SECTOR_SIZE, FLASH_WORD_SIZE, FlashStorage, StorageError};

const LFS_CACHE_SIZE: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsError {
    Storage(StorageError),
    Corrupt,
    NotFound,
    AlreadyExists,
    NotADirectory,
    NotAFile,
    DirectoryNotEmpty,
    InvalidParam,
    PathTooLong,
    NameTooLong,
    NoSpace,
    Full,
    TooManyOpenFiles,
    InvalidHandle,
    NotMounted,
    MountFailed,
    FormatFailed,
    Other(LfsError),
}

impl From<StorageError> for FsError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<LfsError> for FsError {
    fn from(e: LfsError) -> Self {
        match e {
            LfsError::Success => Self::InvalidParam,
            LfsError::Io => Self::Storage(StorageError::WriteError),
            LfsError::Corruption => Self::Corrupt,
            LfsError::NoSuchEntry => Self::NotFound,
            LfsError::EntryAlreadyExisted => Self::AlreadyExists,
            LfsError::PathNotDir => Self::NotADirectory,
            LfsError::PathIsDir => Self::NotAFile,
            LfsError::DirNotEmpty => Self::DirectoryNotEmpty,
            LfsError::BadFileDescriptor => Self::InvalidHandle,
            LfsError::FileTooBig => Self::Full,
            LfsError::Invalid => Self::InvalidParam,
            LfsError::NoSpace => Self::NoSpace,
            LfsError::NoMemory => Self::Full,
            LfsError::NoAttribute => Self::NotFound,
            LfsError::FilenameTooLong => Self::NameTooLong,
            other => Self::Other(other),
        }
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
            Self::Other(e) => write!(f, "littlefs error: {:?}", e),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub file_type: FileType,
    pub size: u32,
    pub name: heapless::String<64>,
}

impl Metadata {
    pub fn is_file(&self) -> bool {
        matches!(self.file_type, FileType::File)
    }

    pub fn is_dir(&self) -> bool {
        matches!(self.file_type, FileType::Directory)
    }
}

fn tail_name(path: &str, out: &mut heapless::String<64>) -> Result<(), FsError> {
    let tail = match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    };
    if tail.len() > out.capacity() {
        return Err(FsError::NameTooLong);
    }
    out.push_str(tail).map_err(|_| FsError::NameTooLong)?;
    Ok(())
}

fn make_path(path: &str) -> Result<PathBuf, FsError> {
    let bytes = path.as_bytes();
    if bytes.is_empty() {
        return Err(FsError::InvalidParam);
    }
    if bytes.len() > littlefs2::consts::PATH_MAX {
        return Err(FsError::PathTooLong);
    }
    if !bytes.is_ascii() {
        return Err(FsError::InvalidParam);
    }
    if bytes.contains(&0u8) {
        return Err(FsError::InvalidParam);
    }
    Ok(PathBuf::from(bytes))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct OpenOptions {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub create_new: bool,
    pub append: bool,
    pub truncate: bool,
}

impl OpenOptions {
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

    pub const fn read(mut self, read: bool) -> Self {
        self.read = read;
        self
    }

    pub const fn write(mut self, write: bool) -> Self {
        self.write = write;
        self
    }

    pub const fn create(mut self, create: bool) -> Self {
        self.create = create;
        self
    }

    pub const fn create_new(mut self, create_new: bool) -> Self {
        self.create_new = create_new;
        self
    }

    pub const fn append(mut self, append: bool) -> Self {
        self.append = append;
        self
    }

    pub const fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    pub const fn read_only() -> Self {
        Self::new().read(true)
    }

    pub const fn write_only() -> Self {
        Self::new().write(true).create(true).truncate(true)
    }

    pub const fn read_write() -> Self {
        Self::new().read(true).write(true)
    }

    pub const fn append_mode() -> Self {
        Self::new().write(true).create(true).append(true)
    }

    fn apply(&self, o: &mut LfsOpenOptions) {
        o.read(self.read);
        o.write(self.write);
        if self.create_new {
            o.create_new(true);
        } else {
            o.create(self.create);
        }
        o.append(self.append);
        o.truncate(self.truncate);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SeekFrom {
    Start(u32),
    End(i64),
    Current(i64),
}

pub struct File<'a, 'b, 'c, S: driver::Storage> {
    inner: &'c LfsFile<'a, 'b, S>,
}

impl<'a, 'b, 'c, S: driver::Storage> File<'a, 'b, 'c, S> {
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize, FsError> {
        self.inner.read(buffer).map_err(FsError::from)
    }

    pub fn read_exact(&self, buffer: &mut [u8]) -> Result<(), FsError> {
        use lfsio::Read;
        self.inner.read_exact(buffer).map_err(FsError::from)
    }

    pub fn write(&self, data: &[u8]) -> Result<usize, FsError> {
        self.inner.write(data).map_err(FsError::from)
    }

    pub fn write_all(&self, data: &[u8]) -> Result<(), FsError> {
        use lfsio::Write;
        self.inner.write_all(data).map_err(FsError::from)
    }

    pub fn seek(&self, pos: SeekFrom) -> Result<u32, FsError> {
        let lp = match pos {
            SeekFrom::Start(o) => LfsSeekFrom::Start(o),
            SeekFrom::End(o) => LfsSeekFrom::End(i64_to_i32(o)?),
            SeekFrom::Current(o) => LfsSeekFrom::Current(i64_to_i32(o)?),
        };
        let n = self.inner.seek(lp).map_err(FsError::from)?;
        usize_to_u32(n)
    }

    pub fn position(&self) -> Result<u32, FsError> {
        let n = self
            .inner
            .seek(LfsSeekFrom::Current(0))
            .map_err(FsError::from)?;
        usize_to_u32(n)
    }

    pub fn size(&self) -> Result<u32, FsError> {
        let n = self.inner.len().map_err(FsError::from)?;
        usize_to_u32(n)
    }

    pub fn sync(&self) -> Result<(), FsError> {
        self.inner.sync().map_err(FsError::from)
    }

    pub fn truncate(&self, size: u32) -> Result<(), FsError> {
        self.inner.set_len(size as usize).map_err(FsError::from)
    }
}

fn i64_to_i32(v: i64) -> Result<i32, FsError> {
    i32::try_from(v).map_err(|_| FsError::InvalidParam)
}

fn usize_to_u32(v: usize) -> Result<u32, FsError> {
    u32::try_from(v).map_err(|_| FsError::InvalidParam)
}

pub struct LfsDevice<'d, const BLOCK_COUNT: usize> {
    flash: FlashStorage<'d>,
    last_error: Option<StorageError>,
}

impl<'d, const BLOCK_COUNT: usize> LfsDevice<'d, BLOCK_COUNT> {
    pub fn new(mut flash: FlashStorage<'d>) -> Result<Self, StorageError> {
        flash.init()?;
        let cfg = *flash.config();
        if cfg.partition_offset % FLASH_SECTOR_SIZE != 0 {
            return Err(StorageError::AlignmentError);
        }
        if (cfg.partition_size / FLASH_SECTOR_SIZE) < BLOCK_COUNT as u32 {
            return Err(StorageError::OutOfBounds);
        }
        if (cfg.partition_offset as u64 + (BLOCK_COUNT as u64) * FLASH_SECTOR_SIZE as u64)
            > cfg.total_size as u64
        {
            return Err(StorageError::OutOfBounds);
        }
        Ok(Self {
            flash,
            last_error: None,
        })
    }

    pub fn take_last_error(&mut self) -> Option<StorageError> {
        self.last_error.take()
    }

    pub fn flash(&self) -> &FlashStorage<'d> {
        &self.flash
    }

    fn fail(&mut self, e: StorageError) -> lfsio::Error {
        self.last_error = Some(e);
        lfsio::Error::Io
    }
}

impl<'d, const BLOCK_COUNT: usize> driver::Storage for LfsDevice<'d, BLOCK_COUNT> {
    const READ_SIZE: usize = FLASH_WORD_SIZE as usize;
    const WRITE_SIZE: usize = FLASH_WORD_SIZE as usize;
    const BLOCK_SIZE: usize = FLASH_SECTOR_SIZE as usize;
    const BLOCK_COUNT: usize = BLOCK_COUNT;
    const BLOCK_CYCLES: isize = 500;

    type CACHE_SIZE = U256;
    type LOOKAHEAD_SIZE = U16;

    fn read(&mut self, off: usize, buf: &mut [u8]) -> lfsio::Result<usize> {
        match self.flash.read(off as u32, buf) {
            Ok(()) => Ok(buf.len()),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn write(&mut self, off: usize, data: &[u8]) -> lfsio::Result<usize> {
        match self.flash.write(off as u32, data) {
            Ok(()) => Ok(data.len()),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn erase(&mut self, off: usize, len: usize) -> lfsio::Result<usize> {
        match self.flash.erase(off as u32, len as u32) {
            Ok(()) => Ok(len),
            Err(e) => Err(self.fail(e)),
        }
    }
}

pub struct Dir<'a, 'b, 'c, S: driver::Storage> {
    inner: &'c mut LfsReadDir<'a, 'b, S>,
}

impl<'a, 'b, 'c, S: driver::Storage> Iterator for Dir<'a, 'b, 'c, S> {
    type Item = Result<Metadata, FsError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next() {
            None => None,
            Some(Err(e)) => Some(Err(FsError::from(e))),
            Some(Ok(entry)) => Some(entry_to_metadata(entry)),
        }
    }
}

fn entry_to_metadata(entry: littlefs2::fs::DirEntry) -> Result<Metadata, FsError> {
    let m = entry.metadata();
    let mut name = heapless::String::<64>::new();
    let name_str: &str = entry.file_name().as_ref();
    tail_name(name_str, &mut name)?;
    Ok(Metadata {
        file_type: if m.is_dir() {
            FileType::Directory
        } else {
            FileType::File
        },
        size: usize_to_u32(m.len())?,
        name,
    })
}

pub struct FileSystem<'a, S: driver::Storage> {
    inner: LfsFilesystem<'a, S>,
}

impl<'a, S: driver::Storage> FileSystem<'a, S> {
    pub fn allocate() -> Allocation<S> {
        Allocation::new()
    }

    pub fn mount<'m>(
        alloc: &'m mut Allocation<S>,
        device: &'m mut S,
    ) -> Result<FileSystem<'m, S>, FsError> {
        let inner = LfsFilesystem::mount(alloc, device).map_err(|e| match e {
            LfsError::Corruption | LfsError::NoSuchEntry => FsError::Corrupt,
            other => FsError::from(other),
        })?;
        Ok(FileSystem { inner })
    }

    pub fn format(device: &mut S) -> Result<(), FsError> {
        LfsFilesystem::format(device).map_err(|_| FsError::FormatFailed)?;
        Ok(())
    }

    fn probe_mounted(alloc: &mut Allocation<S>, device: &mut S) -> bool {
        Self::mount(alloc, device).is_ok()
    }

    pub fn mount_or_format(
        alloc: &'a mut Allocation<S>,
        device: &'a mut S,
    ) -> Result<(Self, bool), FsError> {
        if Self::probe_mounted(alloc, device) {
            Ok((Self::mount(alloc, device)?, false))
        } else {
            Self::format(device)?;
            Ok((Self::mount(alloc, device)?, true))
        }
    }

    pub fn total_blocks(&self) -> u32 {
        self.inner.total_blocks() as u32
    }

    pub fn used_blocks(&self) -> Result<u32, FsError> {
        let avail = self.available_blocks()?;
        let total = self.inner.total_blocks();
        Ok((total - avail) as u32)
    }

    pub fn free_blocks(&self) -> Result<u32, FsError> {
        let n = self.available_blocks()?;
        Ok(n as u32)
    }

    fn available_blocks(&self) -> Result<usize, FsError> {
        self.inner.available_blocks().map_err(FsError::from)
    }

    pub fn total_bytes(&self) -> u32 {
        self.inner.total_space() as u32
    }

    pub fn free_bytes(&self) -> Result<u32, FsError> {
        let n = self
            .inner
            .available_space()
            .map_err(FsError::from)?;
        n.try_into().map_err(|_| FsError::InvalidParam)
    }

    pub fn with_file<R>(
        &self,
        path: &str,
        options: OpenOptions,
        f: impl FnOnce(&mut File<'_, '_, '_, S>) -> Result<R, FsError>,
    ) -> Result<R, FsError> {
        let p = make_path(path)?;
        let nested: lfsio::Result<Result<R, FsError>> =
            self.inner
                .open_file_with_options_and_then(
                    |o| {
                        options.apply(o);
                        o
                    },
                    &p,
                    |file| Ok(f(&mut File { inner: file })),
                );
        match nested {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(FsError::from(e)),
        }
    }

    pub fn read_file(&self, path: &str, buffer: &mut [u8]) -> Result<usize, FsError> {
        self.with_file(
            path,
            OpenOptions::read_only(),
            |file| file.read(buffer),
        )
    }

    pub fn read_file_at(
        &self,
        path: &str,
        offset: u32,
        buffer: &mut [u8],
    ) -> Result<usize, FsError> {
        self.with_file(path, OpenOptions::read_only(), |file| {
            file.seek(SeekFrom::Start(offset))?;
            file.read(buffer)
        })
    }

    pub fn write_file(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        self.with_file(path, OpenOptions::write_only(), |file| {
            file.write_all(data)
        })
    }

    pub fn append_file(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        self.with_file(path, OpenOptions::append_mode(), |file| {
            file.write_all(data)
        })
    }

    pub fn write_file_at(&self, path: &str, offset: u32, data: &[u8]) -> Result<(), FsError> {
        self.with_file(
            path,
            OpenOptions::new().write(true).create(true),
            |file| {
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(data)
            },
        )
    }

    pub fn file_size(&self, path: &str) -> Result<u32, FsError> {
        self.with_file(path, OpenOptions::read_only(), |file| file.size())
    }

    pub fn truncate(&self, path: &str, size: u32) -> Result<(), FsError> {
        self.with_file(
            path,
            OpenOptions::new().write(true),
            |file| file.truncate(size),
        )
    }

    pub fn remove(&self, path: &str) -> Result<(), FsError> {
        let p = make_path(path)?;
        self.inner.remove(&p).map_err(FsError::from)
    }

    pub fn rename(&self, old_path: &str, new_path: &str) -> Result<(), FsError> {
        let from = make_path(old_path)?;
        let to = make_path(new_path)?;
        self.inner.rename(&from, &to).map_err(FsError::from)
    }

    pub fn metadata(&self, path: &str) -> Result<Metadata, FsError> {
        let p = make_path(path)?;
        let m = self.inner.metadata(&p).map_err(FsError::from)?;
        let mut name = heapless::String::<64>::new();
        let path_str: &str = (*p).as_ref();
        tail_name(path_str, &mut name)?;
        Ok(Metadata {
            file_type: if m.is_dir() {
                FileType::Directory
            } else {
                FileType::File
            },
            size: usize_to_u32(m.len())?,
            name,
        })
    }

    pub fn exists(&self, path: &str) -> Result<bool, FsError> {
        match self.metadata(path) {
            Ok(_) => Ok(true),
            Err(FsError::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn create_dir(&self, path: &str) -> Result<(), FsError> {
        let p = make_path(path)?;
        self.inner.create_dir(&p).map_err(FsError::from)
    }

    pub fn create_dir_all(&self, path: &str) -> Result<(), FsError> {
        let mut current = heapless::String::<256>::new();
        for component in path.split('/').filter(|s| !s.is_empty()) {
            current.push('/').map_err(|_| FsError::PathTooLong)?;
            current.push_str(component).map_err(|_| FsError::PathTooLong)?;
            match self.create_dir(current.as_str()) {
                Ok(()) => {}
                Err(FsError::AlreadyExists) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        self.remove(path)
    }

    pub fn read_dir_with<R>(
        &self,
        path: &str,
        f: impl FnOnce(&mut Dir<'_, '_, '_, S>) -> Result<R, FsError>,
    ) -> Result<R, FsError> {
        let p = make_path(path)?;
        let nested: lfsio::Result<Result<R, FsError>> =
            self.inner
                .read_dir_and_then(&p, |rd| Ok(f(&mut Dir { inner: rd })));
        match nested {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(FsError::from(e)),
        }
    }

    pub fn read_dir_collect<const N: usize>(
        &self,
        path: &str,
        out: &mut heapless::Vec<Metadata, N>,
    ) -> Result<usize, FsError> {
        out.clear();
        self.read_dir_with(path, |dir| {
            for item in dir {
                out.push(item?).map_err(|_| FsError::Full)?;
            }
            Ok(out.len())
        })
    }

    pub fn unmount(self) -> Result<(), FsError> {
        drop(self);
        Ok(())
    }
}

const _: () = {
    assert!(LFS_CACHE_SIZE % FLASH_WORD_SIZE as usize == 0);
    assert!(FLASH_SECTOR_SIZE as usize % LFS_CACHE_SIZE == 0);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_options() {
        let opts = OpenOptions::new().read(true).write(true).create(true);

        assert!(opts.read);
        assert!(opts.write);
        assert!(opts.create);
        assert!(!opts.truncate);
    }

    #[test]
    fn test_seek_from() {
        let start = SeekFrom::Start(100);
        let end = SeekFrom::End(-50);
        let current = SeekFrom::Current(10);

        assert!(matches!(start, SeekFrom::Start(100)));
        assert!(matches!(end, SeekFrom::End(-50)));
        assert!(matches!(current, SeekFrom::Current(10)));
    }

    #[test]
    fn test_make_path_validation() {
        assert!(make_path("/data/log.txt").is_ok());
        assert_eq!(make_path("").unwrap_err(), FsError::InvalidParam);
        assert_eq!(
            make_path("a\0b").unwrap_err(),
            FsError::InvalidParam
        );
        assert_eq!(
            make_path("中文").unwrap_err(),
            FsError::InvalidParam
        );
        let mut huge = heapless::String::<256>::new();
        for _ in 0..256 {
            huge.push('x').unwrap();
        }
        assert_eq!(make_path(huge.as_str()).unwrap_err(), FsError::PathTooLong);
    }

    #[test]
    fn test_tail_name() {
        let mut name = heapless::String::<64>::new();
        tail_name("/data/log.txt", &mut name).unwrap();
        assert_eq!(name.as_str(), "log.txt");

        let mut root = heapless::String::<64>::new();
        tail_name("/", &mut root).unwrap();
        assert_eq!(root.as_str(), "");
    }
}
