//! Preserve access-control metadata when a text edit replaces an inode.
#[cfg(unix)]
use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
use std::{
    fs::{File, Permissions},
    io,
    path::Path,
};

#[derive(PartialEq, Eq)]
pub struct Metadata {
    #[cfg(unix)]
    identity: (u64, u64),
    #[cfg(unix)]
    owner: (u32, u32),
    #[cfg(target_os = "linux")]
    attributes: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
}

impl Metadata {
    pub fn capture(file: &File) -> io::Result<Self> {
        if !cfg!(any(target_os = "linux", target_os = "macos", windows)) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Access-control-preserving replacement is unsupported on this platform",
            ));
        }
        #[cfg(unix)]
        let metadata = file.metadata()?;
        #[cfg(not(unix))]
        let _ = file;
        Ok(Self {
            #[cfg(unix)]
            identity: (metadata.dev(), metadata.ino()),
            #[cfg(unix)]
            owner: (metadata.uid(), metadata.gid()),
            #[cfg(target_os = "linux")]
            attributes: attributes(file)?,
        })
    }

    pub fn preserve(
        &self,
        _source: &File,
        target: &File,
        permissions: &Permissions,
    ) -> io::Result<()> {
        #[cfg(unix)]
        {
            let current = target.metadata()?;
            if self.owner != (current.uid(), current.gid()) {
                // No elevation: if ownership cannot be retained, abort the edit.
                if unsafe { libc::fchown(target.as_raw_fd(), self.owner.0, self.owner.1) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            // chmod alone loses named ACL entries and may retain ACLs inherited
            // from the temporary file's parent. Restore the exact original set.
            let current = attributes(target)?;
            for name in current
                .keys()
                .filter(|name| !self.attributes.contains_key(*name))
            {
                let name = std::ffi::CString::new(name.as_slice()).map_err(io::Error::other)?;
                if unsafe { libc::fremovexattr(target.as_raw_fd(), name.as_ptr()) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            // Restore user attributes while the temporary file is writable;
            // the original access ACL can make even its owner read-only.
            let mut original: Vec<_> = self.attributes.iter().collect();
            original.sort_by_key(|(name, _)| name.as_slice() == b"system.posix_acl_access");
            for (name, value) in original {
                if current.get(name) == Some(value) {
                    continue;
                }
                let name = std::ffi::CString::new(name.as_slice()).map_err(io::Error::other)?;
                if unsafe {
                    libc::fsetxattr(
                        target.as_raw_fd(),
                        name.as_ptr(),
                        value.as_ptr().cast(),
                        value.len(),
                        0,
                    )
                } != 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            // Native copying retains extended ACLs and xattrs, without copying
            // stale timestamps onto newly edited contents. Failure is closed.
            if unsafe {
                libc::fcopyfile(
                    _source.as_raw_fd(),
                    target.as_raw_fd(),
                    std::ptr::null_mut(),
                    libc::COPYFILE_ACL | libc::COPYFILE_XATTR,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        target.set_permissions(permissions.clone())?;
        #[cfg(target_os = "linux")]
        if attributes(target)? != self.attributes {
            return Err(io::Error::other(
                "Cannot preserve original extended permissions",
            ));
        }
        let final_metadata = target.metadata()?;
        #[cfg(unix)]
        if (final_metadata.uid(), final_metadata.gid()) != self.owner {
            return Err(io::Error::other("Cannot preserve original file ownership"));
        }
        if final_metadata.permissions() != *permissions {
            return Err(io::Error::other(
                "Cannot preserve original file permissions",
            ));
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn attributes(file: &File) -> io::Result<std::collections::BTreeMap<Vec<u8>, Vec<u8>>> {
    use std::ffi::CString;
    const MAX_METADATA: usize = 1024 * 1024;
    fn read_bytes(mut read: impl FnMut(*mut libc::c_void, usize) -> isize) -> io::Result<Vec<u8>> {
        // A concurrent xattr resize is retried only a bounded number of times.
        for _ in 0..3 {
            let size = read(std::ptr::null_mut(), 0);
            if size < 0 {
                return Err(io::Error::last_os_error());
            }
            let size = size as usize;
            if size > MAX_METADATA {
                return Err(io::Error::other("Extended metadata exceeds 1 MiB"));
            }
            let mut bytes = vec![0; size];
            let read = read(bytes.as_mut_ptr().cast(), bytes.len());
            if read >= 0 {
                if read as usize > bytes.len() {
                    continue;
                }
                bytes.truncate(read as usize);
                return Ok(bytes);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ERANGE) {
                return Err(error);
            }
        }
        Err(io::Error::other(
            "Extended metadata changed; read the file again",
        ))
    }
    let names = match read_bytes(|buffer, size| unsafe {
        libc::flistxattr(file.as_raw_fd(), buffer.cast(), size)
    }) {
        Ok(names) => names,
        Err(error) if error.raw_os_error() == Some(libc::ENOTSUP) => return Ok(Default::default()),
        Err(error) => return Err(error),
    };
    let mut attributes = std::collections::BTreeMap::new();
    let mut total = names.len();
    for name in names
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let key = CString::new(name).map_err(io::Error::other)?;
        let value = read_bytes(|buffer, size| unsafe {
            libc::fgetxattr(file.as_raw_fd(), key.as_ptr(), buffer, size)
        })?;
        total += value.len();
        if total > MAX_METADATA {
            return Err(io::Error::other("Extended metadata exceeds 1 MiB"));
        }
        attributes.insert(name.to_vec(), value);
    }
    Ok(attributes)
}

pub fn replace(temporary: &Path, path: &Path, existing: bool) -> io::Result<()> {
    #[cfg(windows)]
    if existing {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn ReplaceFileW(
                replaced: *const u16,
                replacement: *const u16,
                backup: *const u16,
                flags: u32,
                exclude: *mut std::ffi::c_void,
                reserved: *mut std::ffi::c_void,
            ) -> i32;
        }
        let replaced: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let replacement: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        // Unlike plain rename, ReplaceFile retains the original DACL and streams.
        // Never set IGNORE_MERGE_ERRORS: preserving access control is mandatory.
        if unsafe {
            ReplaceFileW(
                replaced.as_ptr(),
                replacement.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    let _ = existing;
    std::fs::rename(temporary, path)
}
