//! Rules for recognizing macOS metadata in ZIP entry names.

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
        .any(|component| {
            component == b".DS_Store" || component.starts_with(b"._") || component == b"__MACOSX"
        })
}

#[cfg(test)]
mod tests {
    use super::is_macos_metadata;

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
