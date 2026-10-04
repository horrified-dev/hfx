//! Real Linux ACL/xattr fixtures, with no setfacl dependency or privileged setup.
use super::*;
use serde_json::json;
use std::{
    ffi::CString,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, PermissionsExt},
    },
};

fn acl(permission: u16) -> Vec<u8> {
    // Linux POSIX ACL xattr ABI: version, then little-endian tag/perm/id entries.
    let mut bytes = 2u32.to_le_bytes().to_vec();
    for (tag, permission, id) in [
        (1u16, 6u16, u32::MAX),
        (2, permission, 65534),
        (4, 4, u32::MAX),
        (16, 4, u32::MAX),
        (32, 4, u32::MAX),
    ] {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(permission.to_le_bytes());
        bytes.extend(id.to_le_bytes());
    }
    bytes
}
fn set_attr(path: &Path, name: &str, value: &[u8]) {
    let file = std::fs::File::open(path).unwrap();
    let name = CString::new(name).unwrap();
    let result = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
}
fn get_attr(path: &Path, name: &str) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).unwrap();
    let name = CString::new(name).unwrap();
    let size = unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENODATA) {
        return None;
    }
    assert!(size >= 0, "{}", std::io::Error::last_os_error());
    let mut bytes = vec![0; size as usize];
    assert_eq!(
        unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        },
        size
    );
    Some(bytes)
}

#[test]
fn exact_edits_and_whole_file_writes_preserve_deny_acls_ownership_and_xattrs() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("code.rs");
    std::fs::write(&path, "original").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    set_attr(&path, "system.posix_acl_access", &acl(0));
    set_attr(&path, "user.hfx-note", b"retain this metadata");
    let original_acl = get_attr(&path, "system.posix_acl_access").unwrap();
    let original = path.metadata().unwrap();
    // The canonical target's metadata must survive an in-workspace symlink too.
    std::os::unix::fs::symlink("code.rs", root.path().join("link.rs")).unwrap();
    edit(
        root.path(),
        "link.rs",
        &json!({"old_text":"original","new_text":"changed","expected_sha256":sha256("original")}),
    )
    .unwrap();
    for content in ["changed", "written"] {
        if content == "written" {
            write(root.path(), "code.rs", &json!({"content":content})).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        assert_eq!(
            get_attr(&path, "system.posix_acl_access").unwrap(),
            original_acl
        );
        assert_eq!(
            get_attr(&path, "user.hfx-note").unwrap(),
            b"retain this metadata"
        );
        let current = path.metadata().unwrap();
        assert_eq!(
            (current.uid(), current.gid(), current.permissions()),
            (original.uid(), original.gid(), original.permissions())
        );
        assert!(
            root.path()
                .join("link.rs")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }
}

#[test]
fn read_only_files_keep_their_acl_and_user_attributes_during_atomic_edits() {
    for mode in [0o444, 0o440] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("code.rs");
        std::fs::write(&path, "original").unwrap();
        set_attr(&path, "system.posix_acl_access", &acl(0));
        set_attr(&path, "user.hfx-note", b"retain on read-only files too");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let original_acl = get_attr(&path, "system.posix_acl_access").unwrap();
        edit(
            root.path(),
            "code.rs",
            &json!({"old_text":"original","new_text":"changed","expected_sha256":null}),
        )
        .unwrap();
        write(root.path(), "code.rs", &json!({"content":"written"})).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "written");
        assert_eq!(
            get_attr(&path, "system.posix_acl_access").unwrap(),
            original_acl
        );
        assert_eq!(
            get_attr(&path, "user.hfx-note").unwrap(),
            b"retain on read-only files too"
        );
        assert_eq!(path.metadata().unwrap().mode() & 0o777, mode);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[test]
fn concurrent_acl_changes_are_rejected_even_when_permission_bits_do_not_change() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("code.rs");
    std::fs::write(&path, "original").unwrap();
    set_attr(&path, "system.posix_acl_access", &acl(0));
    let before = snapshot(&path, MAX_EDIT_FILE_BYTES).unwrap().unwrap();
    set_attr(&path, "system.posix_acl_access", &acl(4));
    assert_eq!(before.permissions, path.metadata().unwrap().permissions());
    assert!(
        replace(root.path(), "code.rs", &path, Some(&before), "changed")
            .unwrap_err()
            .contains("File changed")
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    assert_eq!(get_attr(&path, "system.posix_acl_access").unwrap(), acl(4));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn parent_default_acls_are_not_leaked_onto_replaced_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("code.rs");
    std::fs::write(&path, "original").unwrap();
    assert!(get_attr(&path, "system.posix_acl_access").is_none());
    set_attr(root.path(), "system.posix_acl_default", &acl(0));
    edit(
        root.path(),
        "code.rs",
        &json!({"old_text":"original","new_text":"changed","expected_sha256":null}),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed");
    assert!(get_attr(&path, "system.posix_acl_access").is_none());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn concurrent_inode_replacement_is_rejected_even_with_identical_contents() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("code.rs");
    std::fs::write(&path, "original").unwrap();
    let before = snapshot(&path, MAX_EDIT_FILE_BYTES).unwrap().unwrap();
    let replacement = root.path().join("replacement.rs");
    std::fs::write(&replacement, "original").unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    assert!(replace(root.path(), "code.rs", &path, Some(&before), "changed").is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}
