use std::{
    io::{Read, Seek, Write},
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::{DownloadCandidate, DownloadError};

const MAX_COMPONENT_UTF16: usize = 120;
const MAX_RELATIVE_PATH_UTF16: usize = 220;
const MAX_AUTHOR_UTF16: usize = 80;
const MAX_PUBLISHED_UTF16: usize = 40;

pub fn sanitize_component(value: &str) -> String {
    let mut sanitized = String::new();
    let mut pending_space = false;
    let mut previous_invalid = false;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = !sanitized.is_empty();
            continue;
        }
        if pending_space {
            sanitized.push(' ');
            pending_space = false;
        }
        if character < '\u{20}'
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
        {
            if !previous_invalid {
                sanitized.push('_');
            }
            previous_invalid = true;
        } else {
            sanitized.push(character);
            previous_invalid = false;
        }
    }
    while sanitized.ends_with(['.', ' ']) {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        sanitized.push('_');
    }
    let stem = sanitized.split('.').next().unwrap_or_default();
    if is_reserved_windows_name(stem) {
        sanitized.insert(0, '_');
    }
    truncate_utf16(&sanitized, MAX_COMPONENT_UTF16)
}

pub fn build_work_directory(candidate: &DownloadCandidate) -> PathBuf {
    let author = bounded_component(
        candidate.author_name.as_deref().unwrap_or("未知作者"),
        MAX_AUTHOR_UTF16,
    );
    let published = bounded_component(
        candidate.published_at.as_deref().unwrap_or("未知时间"),
        MAX_PUBLISHED_UTF16,
    );
    let title = sanitize_component(candidate.title.as_deref().unwrap_or("无标题"));
    let suffix = format!("_{}", candidate.aweme_id);
    let fixed = published.encode_utf16().count() + 1 + suffix.encode_utf16().count();
    let author_width = author.encode_utf16().count();
    let separator_width = 1;
    let title_limit = MAX_COMPONENT_UTF16
        .saturating_sub(fixed)
        .min(MAX_RELATIVE_PATH_UTF16.saturating_sub(author_width + separator_width + fixed));
    let title = truncate_utf16(&title, title_limit.max(1));
    PathBuf::from(author).join(format!("{published}_{title}{suffix}"))
}

pub fn contained_path(root: &Path, relative: &Path) -> Result<PathBuf, DownloadError> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(path_error("下载路径必须是非空相对路径"));
    }
    let mut safe = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => safe.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(path_error("下载路径超出目标目录"));
            }
        }
    }
    if safe.as_os_str().is_empty() {
        return Err(path_error("下载路径不能为空"));
    }
    Ok(root.join(safe))
}

pub fn asset_ownership_key(root: &Path, aweme_id: &str, stable_key: &str, quality: &str) -> String {
    let root = normalize_root_identity(root);
    let mut digest = Sha256::new();
    for field in [root.as_str(), aweme_id, stable_key, quality] {
        digest.update((field.len() as u64).to_le_bytes());
        digest.update(field.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub fn partial_filename(asset_id: i64, nonce: u64, extension: &str) -> String {
    let extension: String = extension
        .trim_start_matches('.')
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .take(16)
        .collect();
    let extension = if extension.is_empty() {
        "bin"
    } else {
        &extension
    };
    format!(".asset-{asset_id}-{nonce}.{extension}.part")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileObservation {
    Missing,
    File { len: u64 },
    NotFile,
}

pub trait ResumeFile: Read + Write + Seek + Send {}

impl<T> ResumeFile for T where T: Read + Write + Seek + Send {}

pub trait GuardedPath: Send {
    fn inspect(&self) -> Result<FileObservation, DownloadError>;
    fn remove(&self) -> Result<(), DownloadError>;
    fn create_new(&self) -> Result<Box<dyn Write + Send>, DownloadError>;
    fn open_existing_for_resume(&self) -> Result<Box<dyn ResumeFile + '_>, DownloadError> {
        Err(path_error("下载路径不支持安全续传"))
    }
}

pub trait RootAccess {
    fn guard<'a>(
        &'a self,
        root: &Path,
        relative: &Path,
    ) -> Result<Box<dyn GuardedPath + 'a>, DownloadError>;
    fn guard_for_create<'a>(
        &'a self,
        root: &Path,
        relative: &Path,
    ) -> Result<Box<dyn GuardedPath + 'a>, DownloadError> {
        self.guard(root, relative)
    }
    fn atomic_rename(&self, root: &Path, from: &Path, to: &Path) -> Result<(), DownloadError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FilesystemRootAccess;

impl RootAccess for FilesystemRootAccess {
    fn guard<'a>(
        &'a self,
        root: &Path,
        relative: &Path,
    ) -> Result<Box<dyn GuardedPath + 'a>, DownloadError> {
        guarded_path(root, relative, false)
    }

    fn guard_for_create<'a>(
        &'a self,
        root: &Path,
        relative: &Path,
    ) -> Result<Box<dyn GuardedPath + 'a>, DownloadError> {
        guarded_path(root, relative, true)
    }

    fn atomic_rename(&self, root: &Path, from: &Path, to: &Path) -> Result<(), DownloadError> {
        guarded_rename(root, from, to)
    }
}

fn bounded_component(value: &str, limit: usize) -> String {
    truncate_utf16(&sanitize_component(value), limit)
}

fn truncate_utf16(value: &str, limit: usize) -> String {
    let mut width = 0;
    value
        .chars()
        .take_while(|character| {
            let next = width + character.len_utf16();
            if next <= limit {
                width = next;
                true
            } else {
                false
            }
        })
        .collect()
}

fn is_reserved_windows_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(
            upper.strip_prefix("COM"),
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        )
        || matches!(
            upper.strip_prefix("LPT"),
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        )
}

fn normalize_root_identity(root: &Path) -> String {
    let mut parts = Vec::new();
    let raw = root.to_string_lossy().replace('\\', "/");
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value),
        }
    }
    let prefix = if raw.starts_with('/') { "/" } else { "" };
    format!("{prefix}{}", parts.join("/")).to_lowercase()
}

fn path_error(message: &str) -> DownloadError {
    DownloadError {
        code: "download_path_unsafe".to_string(),
        message: message.to_string(),
    }
}

#[cfg(not(windows))]
mod platform {
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        path::{Path, PathBuf},
    };

    use super::{
        contained_path, path_error, DownloadError, FileObservation, GuardedPath, ResumeFile,
    };

    pub(super) struct NativeGuardedPath {
        root: PathBuf,
        path: PathBuf,
        create_parents: bool,
    }

    impl GuardedPath for NativeGuardedPath {
        fn inspect(&self) -> Result<FileObservation, DownloadError> {
            validate_chain(&self.root, &self.path)?;
            match fs::metadata(&self.path) {
                Ok(metadata) if metadata.is_file() => Ok(FileObservation::File {
                    len: metadata.len(),
                }),
                Ok(_) => Ok(FileObservation::NotFile),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(FileObservation::Missing)
                }
                Err(_) => Err(path_error("无法检查下载文件")),
            }
        }

        fn remove(&self) -> Result<(), DownloadError> {
            validate_chain(&self.root, &self.path)?;
            match fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(path_error("无法删除不安全的临时文件")),
            }
        }

        fn create_new(&self) -> Result<Box<dyn Write + Send>, DownloadError> {
            if !self.create_parents {
                return Err(path_error("下载目录未锁定用于创建"));
            }
            let parent = self
                .path
                .parent()
                .ok_or_else(|| path_error("下载文件缺少父目录"))?;
            fs::create_dir_all(parent).map_err(|_| path_error("无法创建下载目录"))?;
            validate_chain(&self.root, parent)?;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.path)
                .map_err(|_| path_error("无法创建下载临时文件"))?;
            Ok(Box::new(file))
        }

        fn open_existing_for_resume(&self) -> Result<Box<dyn ResumeFile + '_>, DownloadError> {
            validate_chain(&self.root, &self.path)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.path)
                .map_err(|_| path_error("无法安全打开下载临时文件"))?;
            validate_chain(&self.root, &self.path)?;
            Ok(Box::new(file))
        }
    }

    pub(super) fn guard(
        root: &Path,
        relative: &Path,
        create: bool,
    ) -> Result<Box<dyn GuardedPath>, DownloadError> {
        let path = contained_path(root, relative)?;
        Ok(Box::new(NativeGuardedPath {
            root: root.to_path_buf(),
            path,
            create_parents: create,
        }))
    }

    pub(super) fn rename(root: &Path, from: &Path, to: &Path) -> Result<(), DownloadError> {
        let source = contained_path(root, from)?;
        let destination = contained_path(root, to)?;
        validate_chain(root, &source)?;
        let parent = destination
            .parent()
            .ok_or_else(|| path_error("下载文件缺少父目录"))?;
        fs::create_dir_all(parent).map_err(|_| path_error("无法创建下载目录"))?;
        validate_chain(root, parent)?;
        fs::rename(source, destination).map_err(|_| path_error("无法提交下载文件"))
    }

    fn validate_chain(root: &Path, target: &Path) -> Result<(), DownloadError> {
        let relative = target
            .strip_prefix(root)
            .map_err(|_| path_error("下载路径超出目标目录"))?;
        let mut current = root.to_path_buf();
        for component in std::iter::once(None).chain(relative.components().map(Some)) {
            if let Some(component) = component {
                current.push(component.as_os_str());
            }
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(path_error("下载路径包含符号链接"))
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => return Err(path_error("无法安全检查下载路径")),
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        cell::RefCell,
        ffi::OsStr,
        fs::File,
        io::Write,
        mem::size_of,
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        path::{Path, PathBuf},
        ptr,
    };

    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, HANDLE,
            INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{
            CreateDirectoryW, CreateFileW, FileAttributeTagInfo, FileDispositionInfo, FileIdInfo,
            GetFileInformationByHandleEx, GetFileSizeEx, SetFileInformationByHandle, CREATE_NEW,
            DELETE, FILE_ADD_FILE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_DISPOSITION_INFO,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
            FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_RENAME_INFO, FILE_SHARE_READ,
            FILE_SHARE_WRITE, FILE_WRITE_DATA, OPEN_EXISTING,
        },
    };

    use super::{
        contained_path, path_error, DownloadError, FileObservation, GuardedPath, ResumeFile,
    };

    struct OwnedHandle(HANDLE);
    // Windows kernel handles are process-wide values without thread affinity.
    // This wrapper owns one handle exclusively and only closes it on Drop, so
    // moving that ownership to the background download task is safe.
    unsafe impl Send for OwnedHandle {}
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub(super) struct NativeGuardedPath {
        path: PathBuf,
        parent_path: PathBuf,
        parent_identity: Option<DirectoryIdentity>,
        _parent_guards: Vec<OwnedHandle>,
        write_parent: Option<OwnedHandle>,
        missing_parent: bool,
        create_allowed: bool,
        child: RefCell<Option<OwnedHandle>>,
    }

    impl GuardedPath for NativeGuardedPath {
        fn inspect(&self) -> Result<FileObservation, DownloadError> {
            if self.missing_parent {
                return Ok(FileObservation::Missing);
            }
            self.verify_parent()?;
            if self.child.borrow().is_none() {
                let Some(handle) = open_child(&self.path)? else {
                    return Ok(FileObservation::Missing);
                };
                *self.child.borrow_mut() = Some(handle);
            }
            let child = self.child.borrow();
            let handle = child.as_ref().expect("child handle");
            let info = attribute_info(handle.0)?;
            if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(path_error("下载文件是重解析点"));
            }
            if info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                return Ok(FileObservation::NotFile);
            }
            let mut len = 0i64;
            if unsafe { GetFileSizeEx(handle.0, &mut len) } == 0 || len < 0 {
                return Err(path_error("无法读取下载文件大小"));
            }
            Ok(FileObservation::File { len: len as u64 })
        }

        fn remove(&self) -> Result<(), DownloadError> {
            if matches!(self.inspect()?, FileObservation::Missing) {
                return Ok(());
            }
            let child = self.child.borrow();
            let handle = child
                .as_ref()
                .ok_or_else(|| path_error("下载文件句柄丢失"))?;
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            if unsafe {
                SetFileInformationByHandle(
                    handle.0,
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            } == 0
            {
                return Err(path_error("无法删除下载临时文件"));
            }
            Ok(())
        }

        fn create_new(&self) -> Result<Box<dyn Write + Send>, DownloadError> {
            if !self.create_allowed || self.missing_parent {
                return Err(path_error("下载目录未锁定用于创建"));
            }
            self.verify_parent()?;
            let handle = create_file(
                &self.path,
                FILE_WRITE_DATA,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
            )?;
            let file = unsafe { File::from_raw_handle(handle.0.cast()) };
            std::mem::forget(handle);
            Ok(Box::new(file))
        }

        fn open_existing_for_resume(&self) -> Result<Box<dyn ResumeFile + '_>, DownloadError> {
            if self.missing_parent {
                return Err(path_error("下载临时文件不存在"));
            }
            self.verify_parent()?;
            *self.child.borrow_mut() = None;
            let handle = create_file(
                &self.path,
                FILE_READ_DATA | FILE_WRITE_DATA,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            )?;
            let info = attribute_info(handle.0)?;
            if info.FileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY) != 0
            {
                return Err(path_error("下载临时文件不是安全的普通文件"));
            }
            let file = unsafe { File::from_raw_handle(handle.0.cast()) };
            std::mem::forget(handle);
            Ok(Box::new(file))
        }
    }

    pub(super) fn guard(
        root: &Path,
        relative: &Path,
        create: bool,
    ) -> Result<Box<dyn GuardedPath>, DownloadError> {
        Ok(Box::new(NativeGuardedPath::new(root, relative, create)?))
    }

    pub(super) fn rename(root: &Path, from: &Path, to: &Path) -> Result<(), DownloadError> {
        let source = NativeGuardedPath::new(root, from, false)?;
        let destination = NativeGuardedPath::new(root, to, true)?;
        if source.missing_parent || destination.missing_parent {
            return Err(path_error("下载重命名目录不存在"));
        }
        source.verify_parent()?;
        destination.verify_parent()?;
        if !matches!(source.inspect()?, FileObservation::File { .. }) {
            return Err(path_error("下载临时文件不存在"));
        }
        let child = source.child.borrow();
        let source_handle = child
            .as_ref()
            .ok_or_else(|| path_error("下载文件句柄丢失"))?;
        let destination_parent = destination
            .write_parent
            .as_ref()
            .ok_or_else(|| path_error("下载目标父目录未锁定"))?;
        let target = rename_target(destination_parent.0, &destination.path)?;
        let name = target.file_name;
        let byte_len =
            std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * size_of::<u16>();
        let words = (byte_len + size_of::<usize>() - 1) / size_of::<usize>();
        let mut buffer = vec![0usize; words];
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = target.root_directory;
            (*info).FileNameLength = (name.len() * size_of::<u16>()) as u32;
            ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
        }
        let status = unsafe {
            let mut io_status = IoStatusBlock::default();
            NtSetInformationFile(
                source_handle.0,
                &mut io_status,
                info.cast(),
                byte_len as u32,
                FILE_RENAME_INFORMATION_CLASS,
            )
        };
        if status < 0 {
            let code = unsafe { RtlNtStatusToDosError(status) };
            return Err(path_error(&format!(
                "无法提交下载文件（NTSTATUS {status:#x}，Windows 错误 {code}）"
            )));
        }
        Ok(())
    }

    impl NativeGuardedPath {
        fn new(root: &Path, relative: &Path, create: bool) -> Result<Self, DownloadError> {
            let path = contained_path(root, relative)?;
            let parent = path
                .parent()
                .ok_or_else(|| path_error("下载文件缺少父目录"))?;
            let parent_path = parent.to_path_buf();
            let parent_relative = parent
                .strip_prefix(root)
                .map_err(|_| path_error("下载路径超出目标目录"))?;
            let mut current = root.to_path_buf();
            let mut guards = Vec::new();
            guards
                .push(open_directory(&current).map_err(|_| path_error("无法安全打开下载根目录"))?);
            let mut missing_parent = false;
            for component in parent_relative.components() {
                current.push(component.as_os_str());
                match open_directory(&current) {
                    Ok(handle) => guards.push(handle),
                    Err(error) if is_missing(error) && create => {
                        create_directory(&current)?;
                        guards.push(
                            open_directory(&current)
                                .map_err(|_| path_error("无法锁定新下载目录"))?,
                        );
                    }
                    Err(error) if is_missing(error) => {
                        missing_parent = true;
                        break;
                    }
                    Err(_) => return Err(path_error("无法安全打开下载目录")),
                }
            }
            let write_parent = if create && !missing_parent {
                Some(
                    open_directory_with_access(&current, FILE_READ_ATTRIBUTES | FILE_ADD_FILE)
                        .map_err(|_| path_error("无法锁定可写下载父目录"))?,
                )
            } else {
                None
            };
            let parent_identity = if missing_parent {
                None
            } else {
                Some(directory_identity(
                    guards
                        .last()
                        .ok_or_else(|| path_error("下载父目录句柄丢失"))?
                        .0,
                )?)
            };
            Ok(Self {
                path,
                parent_path,
                parent_identity,
                _parent_guards: guards,
                write_parent,
                missing_parent,
                create_allowed: create,
                child: RefCell::new(None),
            })
        }

        fn verify_parent(&self) -> Result<(), DownloadError> {
            let expected = self
                .parent_identity
                .ok_or_else(|| path_error("下载父目录身份缺失"))?;
            let current = open_directory(&self.parent_path)
                .map_err(|_| path_error("无法重新验证下载父目录"))?;
            if directory_identity(current.0)? != expected {
                return Err(path_error("下载父目录已被替换"));
            }
            Ok(())
        }
    }

    fn open_directory(path: &Path) -> Result<OwnedHandle, u32> {
        open_directory_with_access(path, FILE_READ_ATTRIBUTES)
    }

    fn open_directory_with_access(path: &Path, access: u32) -> Result<OwnedHandle, u32> {
        let wide = wide(path.as_os_str());
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(unsafe { GetLastError() });
        }
        let owned = OwnedHandle(handle);
        let info = attribute_info(handle).map_err(|_| unsafe { GetLastError() })?;
        if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        {
            return Err(0);
        }
        Ok(owned)
    }

    fn open_child(path: &Path) -> Result<Option<OwnedHandle>, DownloadError> {
        match create_file(
            path,
            FILE_READ_ATTRIBUTES | DELETE,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
        ) {
            Ok(handle) => Ok(Some(handle)),
            Err(_)
                if matches!(
                    unsafe { GetLastError() },
                    ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn create_file(
        path: &Path,
        access: u32,
        disposition: u32,
        flags: u32,
    ) -> Result<OwnedHandle, DownloadError> {
        let wide = wide(path.as_os_str());
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                disposition,
                flags,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(path_error("无法安全打开下载文件"))
        } else {
            Ok(OwnedHandle(handle))
        }
    }

    fn create_directory(path: &Path) -> Result<(), DownloadError> {
        let wide = wide(path.as_os_str());
        if unsafe { CreateDirectoryW(wide.as_ptr(), ptr::null()) } == 0 {
            return Err(path_error("无法创建下载目录"));
        }
        Ok(())
    }

    fn attribute_info(handle: HANDLE) -> Result<FILE_ATTRIBUTE_TAG_INFO, DownloadError> {
        let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
        if unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        } == 0
        {
            Err(path_error("无法验证下载路径句柄"))
        } else {
            Ok(info)
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct DirectoryIdentity {
        volume_serial: u64,
        file_id: [u8; 16],
    }

    fn directory_identity(handle: HANDLE) -> Result<DirectoryIdentity, DownloadError> {
        let mut info = FILE_ID_INFO::default();
        if unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        } == 0
        {
            return Err(path_error("无法读取下载父目录身份"));
        }
        Ok(DirectoryIdentity {
            volume_serial: info.VolumeSerialNumber,
            file_id: info.FileId.Identifier,
        })
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    fn is_missing(error: u32) -> bool {
        matches!(error, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND)
    }

    struct RenameTarget {
        root_directory: HANDLE,
        file_name: Vec<u16>,
    }

    const FILE_RENAME_INFORMATION_CLASS: u32 = 10;

    #[repr(C)]
    union IoStatusValue {
        status: i32,
        pointer: *mut std::ffi::c_void,
    }

    #[repr(C)]
    struct IoStatusBlock {
        value: IoStatusValue,
        information: usize,
    }

    impl Default for IoStatusBlock {
        fn default() -> Self {
            unsafe { std::mem::zeroed() }
        }
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn NtSetInformationFile(
            file_handle: HANDLE,
            io_status_block: *mut IoStatusBlock,
            file_information: *mut std::ffi::c_void,
            length: u32,
            file_information_class: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }

    fn rename_target(parent: HANDLE, destination: &Path) -> Result<RenameTarget, DownloadError> {
        if parent.is_null() {
            return Err(path_error("下载目标父目录句柄为空"));
        }
        let leaf = destination
            .file_name()
            .ok_or_else(|| path_error("下载目标文件名无效"))?;
        let file_name: Vec<u16> = leaf.encode_wide().collect();
        if file_name.is_empty() {
            return Err(path_error("下载目标文件名为空"));
        }
        Ok(RenameTarget {
            root_directory: parent,
            file_name,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rename_target_uses_verified_parent_handle_and_leaf_only_name() {
            let parent = 0x1234usize as HANDLE;
            let target = rename_target(parent, Path::new("C:/archive/author/work/video.mp4"))
                .expect("rename target");

            assert_eq!(target.root_directory, parent);
            assert_eq!(String::from_utf16(&target.file_name).unwrap(), "video.mp4");
        }
    }
}

fn guarded_path<'a>(
    root: &Path,
    relative: &Path,
    create: bool,
) -> Result<Box<dyn GuardedPath + 'a>, DownloadError> {
    platform::guard(root, relative, create)
}

fn guarded_rename(root: &Path, from: &Path, to: &Path) -> Result<(), DownloadError> {
    platform::rename(root, from, to)
}
