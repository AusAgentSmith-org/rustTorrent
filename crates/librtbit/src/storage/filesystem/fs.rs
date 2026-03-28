use std::{
    fs::OpenOptions,
    io::IoSlice,
    path::{Path, PathBuf},
};

use anyhow::Context;
use tracing::warn;

use crate::{
    storage::{StorageFactoryExt, filesystem::opened_file::OurFileExt},
    torrent_state::{ManagedTorrentShared, TorrentMetadata},
};

use crate::storage::{StorageFactory, TorrentStorage};

use super::opened_file::OpenedFile;

#[derive(Default, Clone, Copy)]
pub struct FilesystemStorageFactory {}

impl StorageFactory for FilesystemStorageFactory {
    type Storage = FilesystemStorage;

    fn create(
        &self,
        shared: &ManagedTorrentShared,
        _metadata: &TorrentMetadata,
    ) -> anyhow::Result<FilesystemStorage> {
        Ok(FilesystemStorage {
            output_folder: shared.options.output_folder.clone(),
            opened_files: Default::default(),
        })
    }

    fn clone_box(&self) -> crate::storage::BoxStorageFactory {
        self.boxed()
    }
}

pub struct FilesystemStorage {
    pub(super) output_folder: PathBuf,
    pub(super) opened_files: Vec<OpenedFile>,
}

impl FilesystemStorage {
    pub(super) fn take_fs(&self) -> anyhow::Result<Self> {
        Ok(Self {
            opened_files: self
                .opened_files
                .iter()
                .map(|f| f.take_clone())
                .collect::<anyhow::Result<Vec<_>>>()?,
            output_folder: self.output_folder.clone(),
        })
    }
}

impl TorrentStorage for FilesystemStorage {
    fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        self.opened_files
            .get(file_id)
            .context("no such file")?
            .lock_read()?
            .pread_exact(offset, buf)
    }

    fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        let of = self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        return of.try_mark_sparse()?.pwrite_all(offset, buf);
        #[cfg(not(windows))]
        return of.lock_read()?.pwrite_all(offset, buf);
    }

    fn pwrite_all_vectored(
        &self,
        file_id: usize,
        offset: u64,
        bufs: [IoSlice<'_>; 2],
    ) -> anyhow::Result<usize> {
        let of = self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        return of.try_mark_sparse()?.pwrite_all_vectored(offset, bufs);
        #[cfg(not(windows))]
        return of.lock_read()?.pwrite_all_vectored(offset, bufs);
    }

    fn remove_file(&self, _file_id: usize, filename: &Path) -> anyhow::Result<()> {
        Ok(std::fs::remove_file(self.output_folder.join(filename))?)
    }

    fn ensure_file_length(&self, file_id: usize, len: u64) -> anyhow::Result<()> {
        let f = &self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        f.try_mark_sparse()?;
        Ok(f.lock_read()?.set_len(len)?)
    }

    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        Ok(Box::new(Self {
            opened_files: self
                .opened_files
                .iter()
                .map(|f| f.take_clone())
                .collect::<anyhow::Result<Vec<_>>>()?,
            output_folder: self.output_folder.clone(),
        }))
    }

    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        let path = self.output_folder.join(path);
        if !path.is_dir() {
            anyhow::bail!("cannot remove dir: {path:?} is not a directory")
        }
        // Attempt removal directly to avoid TOCTOU race between empty check and remove.
        // std::fs::remove_dir only removes empty directories; if non-empty it errors.
        match std::fs::remove_dir(&path) {
            Ok(()) => Ok(()),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::DirectoryNotEmpty) => {
                warn!("did not remove {path:?} as it was not empty");
                Ok(())
            }
            Err(e) => Err(e).with_context(|| format!("error removing {path:?}")),
        }
    }

    fn init(
        &mut self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<()> {
        let mut files = Vec::<OpenedFile>::new();
        for file_details in metadata.file_infos.iter() {
            let mut full_path = self.output_folder.clone();
            let relative_path = &file_details.relative_filename;
            full_path.push(relative_path);

            if file_details.attrs.padding {
                files.push(OpenedFile::new_dummy());
                continue;
            };
            std::fs::create_dir_all(full_path.parent().context("bug: no parent")?)?;
            let f = if shared.options.allow_overwrite {
                OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(&full_path)
                    .with_context(|| format!("error opening {full_path:?} in read/write mode"))?
            } else {
                // create_new does not seem to work with read(true), so calling this twice.
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&full_path)
                    .with_context(|| {
                        format!(
                            "error creating a new file (because allow_overwrite = false) {:?}",
                            &full_path
                        )
                    })?;
                OpenOptions::new().read(true).write(true).open(&full_path)?
            };
            files.push(OpenedFile::new(full_path.clone(), f));
        }

        self.opened_files = files;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::TorrentStorage;

    /// Helper to create a FilesystemStorage with `n` real files in a temp directory.
    /// Returns (storage, tempdir) -- tempdir must be kept alive for the duration of the test.
    fn make_test_storage(n: usize) -> (FilesystemStorage, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut opened_files = Vec::new();
        for i in 0..n {
            let path = dir.path().join(format!("file_{i}.dat"));
            let f = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            opened_files.push(OpenedFile::new(path, f));
        }
        let storage = FilesystemStorage {
            output_folder: dir.path().to_owned(),
            opened_files,
        };
        (storage, dir)
    }

    #[test]
    fn test_filesystem_storage_write_and_read() {
        let (storage, _dir) = make_test_storage(1);

        // Ensure the file is long enough.
        storage.ensure_file_length(0, 1024).unwrap();

        let write_data = b"hello, world! this is test data.";
        storage.pwrite_all(0, 0, write_data).unwrap();

        let mut read_buf = vec![0u8; write_data.len()];
        storage.pread_exact(0, 0, &mut read_buf).unwrap();

        assert_eq!(&read_buf, write_data);
    }

    #[test]
    fn test_filesystem_storage_write_at_offset() {
        let (storage, _dir) = make_test_storage(1);

        storage.ensure_file_length(0, 4096).unwrap();

        let write_data = b"offset_data";
        let offset = 2048;
        storage.pwrite_all(0, offset, write_data).unwrap();

        let mut read_buf = vec![0u8; write_data.len()];
        storage.pread_exact(0, offset, &mut read_buf).unwrap();

        assert_eq!(&read_buf, write_data);
    }

    #[test]
    fn test_filesystem_storage_piece_hash_verification() {
        use sha1w::ISha1;

        let (storage, _dir) = make_test_storage(1);

        storage.ensure_file_length(0, 16384).unwrap();

        // Write a "piece" of known data.
        #[allow(clippy::cast_possible_truncation)]
        let piece_data: Vec<u8> = (0..16384u32).map(|i| (i % 256) as u8).collect();
        storage.pwrite_all(0, 0, &piece_data).unwrap();

        // Compute expected hash.
        let mut hasher = sha1w::Sha1::new();
        hasher.update(&piece_data);
        let expected_hash = hasher.finish();

        // Read back and compute hash.
        let mut read_buf = vec![0u8; 16384];
        storage.pread_exact(0, 0, &mut read_buf).unwrap();

        let mut hasher2 = sha1w::Sha1::new();
        hasher2.update(&read_buf);
        let actual_hash = hasher2.finish();

        assert_eq!(expected_hash, actual_hash);
    }

    #[test]
    fn test_storage_handles_sparse_writes() {
        let (storage, _dir) = make_test_storage(1);

        storage.ensure_file_length(0, 8192).unwrap();

        // Write at non-sequential offsets (sparse writes).
        let data_a = b"chunk_a";
        let data_b = b"chunk_b";

        // Write chunk B first (at offset 4096), then chunk A (at offset 0).
        storage.pwrite_all(0, 4096, data_b).unwrap();
        storage.pwrite_all(0, 0, data_a).unwrap();

        // Read both back.
        let mut buf_a = vec![0u8; data_a.len()];
        let mut buf_b = vec![0u8; data_b.len()];
        storage.pread_exact(0, 0, &mut buf_a).unwrap();
        storage.pread_exact(0, 4096, &mut buf_b).unwrap();

        assert_eq!(&buf_a, data_a);
        assert_eq!(&buf_b, data_b);
    }

    #[test]
    fn test_storage_file_boundaries() {
        // Test writing to multiple files (pieces spanning file boundaries).
        let (storage, _dir) = make_test_storage(3);

        for i in 0..3 {
            storage.ensure_file_length(i, 1024).unwrap();
        }

        // Write different data to each file.
        let data = [b"file0_data", b"file1_data", b"file2_data"];
        for (i, d) in data.iter().enumerate() {
            storage.pwrite_all(i, 0, *d).unwrap();
        }

        // Read back from each file and verify.
        for (i, d) in data.iter().enumerate() {
            let mut buf = vec![0u8; d.len()];
            storage.pread_exact(i, 0, &mut buf).unwrap();
            assert_eq!(&buf, *d, "file {i} data mismatch");
        }
    }

    #[test]
    fn test_storage_ensure_file_length() {
        let (storage, dir) = make_test_storage(1);

        // Set to 4096 bytes.
        storage.ensure_file_length(0, 4096).unwrap();
        let meta = std::fs::metadata(dir.path().join("file_0.dat")).unwrap();
        assert_eq!(meta.len(), 4096);

        // Grow to 8192 bytes.
        storage.ensure_file_length(0, 8192).unwrap();
        let meta = std::fs::metadata(dir.path().join("file_0.dat")).unwrap();
        assert_eq!(meta.len(), 8192);
    }

    #[test]
    fn test_storage_remove_file() {
        let (storage, dir) = make_test_storage(2);

        let file0_path = dir.path().join("file_0.dat");
        let file1_path = dir.path().join("file_1.dat");
        assert!(file0_path.exists());
        assert!(file1_path.exists());

        // Remove file 0.
        storage.remove_file(0, Path::new("file_0.dat")).unwrap();
        assert!(!file0_path.exists());
        // File 1 should still exist.
        assert!(file1_path.exists());
    }

    #[test]
    fn test_storage_take() {
        let (storage, _dir) = make_test_storage(1);

        storage.ensure_file_length(0, 1024).unwrap();
        storage.pwrite_all(0, 0, b"before_take").unwrap();

        let taken = storage.take().unwrap();

        // The taken storage should be able to read the same data.
        let mut buf = vec![0u8; 11];
        taken.pread_exact(0, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"before_take");
    }

    #[test]
    fn test_storage_invalid_file_id() {
        let (storage, _dir) = make_test_storage(1);

        // File ID 5 does not exist.
        let result = storage.pread_exact(5, 0, &mut [0u8; 10]);
        assert!(result.is_err());

        let result = storage.pwrite_all(5, 0, &[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn test_storage_remove_directory_if_empty() {
        let (storage, dir) = make_test_storage(0);

        // Create a subdirectory.
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();

        // Remove empty directory should succeed.
        storage
            .remove_directory_if_empty(Path::new("subdir"))
            .unwrap();
        assert!(!sub.exists());
    }

    #[test]
    fn test_storage_remove_directory_not_empty() {
        let (storage, dir) = make_test_storage(0);

        // Create a subdirectory with a file inside.
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("file.txt"), b"content").unwrap();

        // Removing a non-empty directory should not delete it (just warn).
        storage
            .remove_directory_if_empty(Path::new("subdir"))
            .unwrap();
        assert!(sub.exists());
    }
}
