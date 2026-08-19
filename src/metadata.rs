//! Rules for recognizing macOS metadata names.

use std::ffi::OsStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataKind {
    DsStore,
    AppleDouble,
    MacosxDirectory,
}

/// Classifies one path component without applying host path semantics.
#[must_use]
pub fn classify_component(component: &[u8]) -> Option<MetadataKind> {
    if component == b".DS_Store" {
        Some(MetadataKind::DsStore)
    } else if component.starts_with(b"._") {
        Some(MetadataKind::AppleDouble)
    } else if component == b"__MACOSX" {
        Some(MetadataKind::MacosxDirectory)
    } else {
        None
    }
}

/// Classifies a host filesystem filename without lossy Unicode conversion.
#[must_use]
pub fn classify_filesystem_name(name: &OsStr) -> Option<MetadataKind> {
    classify_component(name.as_encoded_bytes())
}

/// Returns whether a ZIP entry name identifies macOS-generated metadata.
///
/// ZIP paths always use `/` as their component separator, independent of the
/// host operating system. Matching bytes instead of decoded text also avoids
/// changing legacy, non-UTF-8 ZIP filenames.
#[must_use]
pub fn is_macos_metadata(entry_name: &[u8]) -> bool {
    entry_name
        .split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .any(|component| classify_component(component).is_some())
}

#[cfg(test)]
mod tests {
    use super::{MetadataKind, classify_component, classify_filesystem_name, is_macos_metadata};
    use std::ffi::OsStr;

    #[test]
    fn classifies_exact_metadata_names() {
        assert_eq!(
            classify_filesystem_name(OsStr::new(".DS_Store")),
            Some(MetadataKind::DsStore)
        );
        assert_eq!(
            classify_filesystem_name(OsStr::new("._image.png")),
            Some(MetadataKind::AppleDouble)
        );
        assert_eq!(
            classify_filesystem_name(OsStr::new("__MACOSX")),
            Some(MetadataKind::MacosxDirectory)
        );
    }

    #[test]
    fn component_classifier_preserves_similar_names() {
        for name in [
            &b"README.md"[..],
            &b"DS_Store.txt"[..],
            &b".DS_Store.backup"[..],
            &b"my.DS_Store"[..],
            &b"foo._bar"[..],
            &b"image._backup"[..],
            &b".gitignore"[..],
            &b".env"[..],
            &b".hidden"[..],
            &b"MACOSX"[..],
            &b"__MACOSX-file"[..],
            &b"foo__MACOSX"[..],
        ] {
            assert_eq!(classify_component(name), None, "{name:?}");
        }
    }

    #[test]
    fn classifies_unicode_appledouble_names() {
        assert_eq!(
            classify_filesystem_name(OsStr::new("._照片.jpg")),
            Some(MetadataKind::AppleDouble)
        );
        assert_eq!(classify_filesystem_name(OsStr::new("照片.jpg")), None);
    }

    #[cfg(unix)]
    #[test]
    fn classifies_non_utf8_appledouble_without_lossy_conversion() {
        use std::os::unix::ffi::OsStrExt;

        let name = OsStr::from_bytes(b"._photo-\xff.jpg");
        assert_eq!(
            classify_filesystem_name(name),
            Some(MetadataKind::AppleDouble)
        );
    }

    #[test]
    fn recognizes_required_metadata_components() {
        for name in [
            ".DS_Store",
            "folder/.DS_Store",
            "foo/bar/.DS_Store",
            "._foo",
            "folder/._foo",
            "foo/bar/._image.png",
            "__MACOSX/",
            "__MACOSX/._foo",
            "__MACOSX/folder/._image.png",
            "foo/__MACOSX/file",
        ] {
            assert!(is_macos_metadata(name.as_bytes()), "{name}");
        }
    }

    #[test]
    fn preserves_unrelated_hidden_and_similarly_named_entries() {
        for name in [
            "README.md",
            "DS_Store.txt",
            "foo._bar",
            "image.png",
            ".gitignore",
            ".env",
            ".hidden",
            "MACOSX/",
            "__MACOSX-file.txt",
            "foo/__MACOSX-file.txt",
        ] {
            assert!(!is_macos_metadata(name.as_bytes()), "{name}");
        }
    }

    #[test]
    fn uses_zip_separators_on_every_host() {
        assert!(is_macos_metadata(b"folder/.DS_Store"));
        assert!(!is_macos_metadata(b"folder\\.DS_Store"));
    }
}
