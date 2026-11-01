//! Preserve no-overwrite semantics on filesystems without exclusive rename/link.
use crate::output::OutputPublication;
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::{self, Seek, Write},
    path::Path,
};
use tempfile::{NamedTempFile, PersistError, TempPath};

fn unsupported(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Unsupported
        // ENOTSUP on macOS; keep this explicit even if std classifies it differently.
        || (cfg!(target_os = "macos") && error.raw_os_error() == Some(45))
}

pub(crate) fn publish(temporary: NamedTempFile, path: &Path) -> Result<OutputPublication> {
    publish_with(
        temporary,
        path,
        |file, path| file.persist_noclobber(path),
        io::copy,
    )
}

fn publish_with<P, C>(
    temporary: NamedTempFile,
    path: &Path,
    persist: P,
    copy: C,
) -> Result<OutputPublication>
where
    P: FnOnce(NamedTempFile, &Path) -> std::result::Result<File, PersistError>,
    C: FnOnce(&mut File, &mut File) -> io::Result<u64>,
{
    let mut temporary = match persist(temporary, path) {
        Ok(_) => return Ok(OutputPublication::NativeNoClobber),
        Err(error) if unsupported(&error.error) => error.file,
        Err(error) => {
            return Err(error.error).with_context(|| {
                format!(
                    "publish completed image without overwrite: {}",
                    path.display()
                )
            });
        }
    };
    // No existence-check + ordinary rename: that could overwrite another output.
    // O_EXCL/create_new reserves the actual destination even if a racer created it.
    let absolute = std::path::absolute(path).context("resolve output publication path")?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&absolute)
        .with_context(|| format!("exclusively create fallback output: {}", path.display()))?;
    // Construct the cleanup guard only AFTER successful exclusive creation, so an
    // AlreadyExists failure never owns/deletes somebody else's file.
    let mut destination = NamedTempFile::from_parts(file, TempPath::try_from_path(absolute)?);
    let expected = temporary
        .as_file()
        .metadata()
        .context("read completed image size")?
        .len();
    temporary
        .as_file_mut()
        .rewind()
        .context("rewind completed image for publication")?;
    let copied = copy(temporary.as_file_mut(), destination.as_file_mut()).with_context(|| {
        format!(
            "copy completed image to fallback output (check free space): {}",
            path.display()
        )
    })?;
    ensure!(
        copied == expected && destination.as_file().metadata()?.len() == expected,
        "incomplete output publication copy: {}",
        path.display()
    );
    destination
        .as_file_mut()
        .flush()
        .context("flush fallback image output")?;
    destination
        .as_file()
        .sync_all()
        .context("sync fallback image output")?;
    destination
        .keep()
        .map_err(|e| e.error)
        .context("retain completed fallback image output")?;
    // Both source temp and failed destination guards clean up on returned errors.
    // A process/power interruption during this fallback can leave a partial final
    // name. Callers import only after success; the native path remains preferred.
    Ok(OutputPublication::ExclusiveCopy)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(dir: &Path) -> NamedTempFile {
        let mut file = NamedTempFile::new_in(dir).unwrap();
        file.write_all(b"complete encoded image\0with all bytes")
            .unwrap();
        file
    }
    fn no_rename(file: NamedTempFile, _: &Path) -> std::result::Result<File, PersistError> {
        Err(PersistError {
            error: io::Error::new(
                io::ErrorKind::Unsupported,
                "injected unsupported exclusive rename",
            ),
            file,
        })
    }
    #[test]
    fn unsupported_rename_uses_exclusive_copy_from_start_and_cleans_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.dng");
        let file = source(dir.path());
        let expected = std::fs::read(file.path()).unwrap();
        assert!(matches!(
            publish_with(file, &path, no_rename, io::copy).unwrap(),
            OutputPublication::ExclusiveCopy
        ));
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn fallback_never_overwrites_or_deletes_an_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.dng");
        // Simulate a target appearing during the native publication attempt.
        let result = publish_with(
            source(dir.path()),
            &path,
            |file, path| {
                std::fs::write(path, b"other process's image").unwrap();
                no_rename(file, path)
            },
            |_, _| panic!("must not copy over an existing output"),
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"other process's image");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn failed_or_short_copy_removes_partial_destination_and_source() {
        for short in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("output.dng");
            let error = publish_with(source(dir.path()), &path, no_rename, |_, output| {
                output.write_all(b"partial")?;
                if short {
                    Ok(7)
                } else {
                    Err(io::Error::other("injected write failure"))
                }
            })
            .unwrap_err();
            assert!(format!("{error:#}").contains(if short {
                "incomplete output publication"
            } else {
                "copy completed image"
            }));
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn permission_and_io_errors_do_not_trigger_copy_fallback() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Other,
            io::ErrorKind::AlreadyExists,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("output.dng");
            let result = publish_with(
                source(dir.path()),
                &path,
                |file, _| {
                    Err(PersistError {
                        error: io::Error::new(kind, "injected native failure"),
                        file,
                    })
                },
                |_, _| panic!("unexpected fallback"),
            );
            assert!(result.is_err());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn native_publication_remains_preferred_and_no_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.tif");
        assert!(matches!(
            publish(source(dir.path()), &path).unwrap(),
            OutputPublication::NativeNoClobber
        ));
        let expected = std::fs::read(&path).unwrap();
        assert!(publish(source(dir.path()), &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn fallback_preserves_even_a_dangling_destination_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.dng");
        let target = dir.path().join("nonexistent");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(publish_with(source(dir.path()), &path, no_rename, io::copy).is_err());
        assert_eq!(std::fs::read_link(&path).unwrap(), target);
        assert!(!target.exists());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_error_45_is_an_unsupported_publication_operation() {
        assert!(unsupported(&io::Error::from_raw_os_error(45)));
    }
}
