use crate::state::{Attachment, AttachmentContent};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const MAX_ATTACHMENTS: usize = 8;
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_PIXELS: u64 = 32_000_000;

pub fn load(path: &Path) -> Result<Attachment, String> {
    let metadata = path
        .metadata()
        .map_err(|e| format!("Cannot attach {}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err("Choose a regular file.".into());
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("Files must be smaller than 10 MiB.".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    from_bytes(
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        &bytes,
    )
}

pub fn from_bytes(name: String, bytes: &[u8]) -> Result<Attachment, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("Files must be smaller than 10 MiB.".into());
    }
    if let Ok(format) = image::guess_format(bytes) {
        if !matches!(
            format,
            image::ImageFormat::Png
                | image::ImageFormat::Jpeg
                | image::ImageFormat::WebP
                | image::ImageFormat::Gif
        ) {
            return Err("Supported images: PNG, JPEG, WebP, and GIF.".into());
        }
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let (width, height) = image::ImageReader::with_format(Cursor::new(bytes), format)
            .into_dimensions()
            .map_err(|e| e.to_string())?;
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err("Images must be smaller than 32 megapixels.".into());
        }
        return from_image(
            name,
            reader
                .decode()
                .map_err(|e| format!("Cannot decode image: {e}"))?,
        );
    }
    if bytes.len() > MAX_TEXT_BYTES {
        return Err("Text files must be smaller than 256 KiB.".into());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Attach a UTF-8 text file or an image (PNG, JPEG, WebP, GIF).")?;
    if text.contains('\0') {
        return Err("This binary file is not supported.".into());
    }
    Ok(Attachment {
        id: Uuid::new_v4(),
        name,
        size: bytes.len(),
        content: AttachmentContent::Text(text.into()),
    })
}

fn from_image(name: String, image: image::DynamicImage) -> Result<Attachment, String> {
    if u64::from(image.width()) * u64::from(image.height()) > MAX_PIXELS {
        return Err("Images must be smaller than 32 megapixels.".into());
    }
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let bytes = png.into_inner();
    if bytes.len() > MAX_FILE_BYTES {
        return Err("The decoded image exceeds 10 MiB. Choose a smaller image.".into());
    }
    Ok(Attachment {
        id: Uuid::new_v4(),
        name,
        size: bytes.len(),
        content: AttachmentContent::Image {
            base64: STANDARD.encode(bytes).into(),
            width: image.width(),
            height: image.height(),
        },
    })
}

pub fn thumbnail(attachment: &Attachment) -> Option<eframe::egui::ColorImage> {
    preview(attachment, 160, 100)
}

pub fn preview(
    attachment: &Attachment,
    max_width: u32,
    max_height: u32,
) -> Option<eframe::egui::ColorImage> {
    let AttachmentContent::Image { base64, .. } = &attachment.content else {
        return None;
    };
    let bytes = STANDARD.decode(base64.as_bytes()).ok()?;
    let decoded = image::load_from_memory(&bytes).ok()?;
    let image = decoded
        .thumbnail(
            max_width.min(decoded.width()),
            max_height.min(decoded.height()),
        )
        .to_rgba8();
    Some(eframe::egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

pub fn image_bytes(attachment: &Attachment) -> Result<Vec<u8>, String> {
    let AttachmentContent::Image { base64, .. } = &attachment.content else {
        return Err("This attachment is not an image.".into());
    };
    STANDARD
        .decode(base64.as_bytes())
        .map_err(|_| "Invalid image data.".into())
}

/// Export through a temporary file so exiting during background work cannot
/// truncate a previously saved image. Only report success after replacement.
pub fn save_png_atomic(image: &Attachment, path: &Path) -> Result<(), String> {
    use std::io::Write;
    let bytes = image_bytes(image)?;
    // Preserve the explicit file chooser's existing-symlink behavior.
    let destination = if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        path.canonicalize()
            .map_err(|e| format!("Could not save image: {e}"))?
    } else {
        path.to_owned()
    };
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".hfx-image-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        if let Ok(metadata) = std::fs::metadata(&destination) {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, &destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Could not save image: {e}"))
}

pub fn from_data_url(name: String, url: &str) -> Result<Attachment, String> {
    let (header, data) = url.split_once(',').ok_or("Invalid image data URL")?;
    if !header.starts_with("data:image/") || !header.ends_with(";base64") {
        return Err("Image output must be a base64 image data URL.".into());
    }
    if data.len() > MAX_FILE_BYTES.div_ceil(3) * 4 {
        return Err("Image output exceeds 10 MiB.".into());
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| "Invalid base64 image output")?;
    let attachment = from_bytes(name, &bytes)?;
    if !matches!(attachment.content, AttachmentContent::Image { .. }) {
        return Err("Provider output is not a supported image.".into());
    }
    Ok(attachment)
}

/// Only local file URIs are interpreted as attachments. Ordinary pasted text is untouched.
pub fn file_uris(text: &str) -> Option<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if matches!(line, "copy" | "cut") && paths.is_empty() {
            continue;
        }
        let url = reqwest::Url::parse(line).ok()?;
        if url.scheme() != "file" {
            return None;
        }
        if url
            .host_str()
            .is_some_and(|host| host != "localhost" && !host.is_empty())
        {
            return None;
        }
        paths.push(url.to_file_path().ok()?);
    }
    (!paths.is_empty()).then_some(paths)
}

/// Read only in response to the user's explicit paste command.
pub fn clipboard_files_or_image() -> Result<Vec<Attachment>, String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat, get_contents};
        for mime in ["text/uri-list", "x-special/gnome-copied-files"] {
            if let Ok((pipe, _)) = get_contents(
                ClipboardType::Regular,
                Seat::Unspecified,
                MimeType::Specific(mime),
            ) {
                let mut text = String::new();
                if pipe.take(64 * 1024).read_to_string(&mut text).is_ok()
                    && let Some(paths) = file_uris(&text)
                {
                    return load_paths(&paths);
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    if let Ok(paths) =
        clipboard_win::get_clipboard::<Vec<PathBuf>, _>(clipboard_win::formats::FileList)
    {
        if !paths.is_empty() {
            return load_paths(&paths);
        }
    }
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
    if let Ok(text) = clipboard.get_text()
        && let Some(paths) = file_uris(&text)
    {
        return load_paths(&paths);
    }
    let image = clipboard.get_image().map_err(|_| "Clipboard has no supported files or image. Copy a file or screenshot, then paste in the chat.")?;
    if image.width as u64 * image.height as u64 > MAX_PIXELS {
        return Err("Images must be smaller than 32 megapixels.".into());
    }
    let rgba = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.into_owned(),
    )
    .ok_or("Invalid clipboard image")?;
    Ok(vec![from_image(
        "Pasted image.png".into(),
        image::DynamicImage::ImageRgba8(rgba),
    )?])
}

pub fn load_paths(paths: &[PathBuf]) -> Result<Vec<Attachment>, String> {
    if paths.len() > MAX_ATTACHMENTS {
        return Err("Attach up to 8 files at a time.".into());
    }
    paths.iter().map(|path| load(path)).collect()
}

#[cfg(test)]
pub(crate) fn test_image() -> Attachment {
    from_image("fixture.png".into(), image::DynamicImage::new_rgba8(24, 16)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_uris_are_decoded_and_ordinary_text_is_preserved() {
        assert_eq!(
            file_uris("copy\nfile:///tmp/my%20file.rs\nfile:///tmp/a.png").unwrap(),
            vec![
                PathBuf::from("/tmp/my file.rs"),
                PathBuf::from("/tmp/a.png")
            ]
        );
        assert!(file_uris("let x = 42;").is_none());
        assert!(file_uris("file://remote.example/secret").is_none());
        assert!(file_uris("file:///tmp/a\nhello").is_none());
    }
    #[test]
    fn text_and_images_are_real_attachments_with_limits() {
        let text = from_bytes("main.rs".into(), "Hej 👋".as_bytes()).unwrap();
        assert!(matches!(text.content, AttachmentContent::Text(_)));
        assert!(from_bytes("binary".into(), &[0, 1, 2]).is_err());
        assert!(from_bytes("big.txt".into(), &vec![b'a'; MAX_TEXT_BYTES + 1]).is_err());
        let attachment =
            from_image("pixel.png".into(), image::DynamicImage::new_rgba8(2, 3)).unwrap();
        let AttachmentContent::Image {
            base64,
            width,
            height,
        } = &attachment.content
        else {
            panic!()
        };
        assert_eq!((*width, *height), (2, 3));
        let decoded = from_bytes(
            "again.png".into(),
            &STANDARD.decode(base64.as_bytes()).unwrap(),
        )
        .unwrap();
        assert_eq!(thumbnail(&decoded).unwrap().size, [2, 3]);
    }

    #[test]
    fn image_export_atomically_replaces_existing_files_and_cleans_up_failed_writes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("result.png");
        std::fs::write(&path, "previous image").unwrap();
        let text = from_bytes("notes.txt".into(), b"not an image").unwrap();
        assert!(save_png_atomic(&text, &path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous image");
        let image = test_image();
        save_png_atomic(&image, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), image_bytes(&image).unwrap());
        let invalid = root.path().join("directory.png");
        std::fs::create_dir(&invalid).unwrap();
        assert!(save_png_atomic(&image, &invalid).is_err());
        assert!(invalid.is_dir());
        assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".hfx-image-")
        }));
    }

    #[test]
    fn returned_images_validate_payloads_and_export_real_png_bytes() {
        let image = test_image();
        let bytes = image_bytes(&image).unwrap();
        let url = format!("data:image/png;base64,{}", STANDARD.encode(&bytes));
        let returned = from_data_url("returned.png".into(), &url).unwrap();
        assert_eq!(image_bytes(&returned).unwrap(), bytes);
        assert_eq!(preview(&returned, 12, 12).unwrap().size, [12, 8]);
        assert!(from_data_url("bad.png".into(), "data:image/png;base64,invalid!").is_err());
        assert!(from_data_url("bad.png".into(), "data:text/plain;base64,aGVsbG8=").is_err());
        assert!(from_data_url("bad.png".into(), "data:image/png;base64,aGVsbG8=").is_err());
        assert!(
            from_data_url(
                "bad.png".into(),
                &format!(
                    "data:image/png;base64,{}",
                    "a".repeat(MAX_FILE_BYTES.div_ceil(3) * 4 + 1)
                )
            )
            .is_err()
        );
        let text = from_bytes("text.txt".into(), b"hello").unwrap();
        assert!(image_bytes(&text).is_err());
    }
}
