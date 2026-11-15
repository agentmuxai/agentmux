// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Writing a small file into a container's `/tmp`.

use crate::backend::container::{ContainerError, ContainerManager};

impl ContainerManager {
    /// Write `bytes` to `/tmp/<file_name>` in the container as the exec user, mode 0600; returns the path. The turn
    /// prompt and the install files of the non-npm CLIs (hash locks too large for an argument) go this way.
    pub(super) async fn upload_tmp_file(&self, container_name: &str, file_name: &str, bytes: &[u8]) -> Result<String, ContainerError> {
        // No usable uid means no usable file: at 0600 a root-owned prompt is
        // unreadable by the exec user, and /tmp's sticky bit makes it
        // un-unlinkable too. Failing here is a clear, retryable error; writing
        // it anyway would be a permission-denied on open plus a leaked file.
        let (uid, gid) = self.exec_identity(container_name).await.ok_or_else(|| {
            ContainerError::NotAvailable(format!(
                "could not resolve the exec user of container {container_name}; \
                 cannot write a readable file"
            ))
        })?;

        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_uid(uid);
        header.set_gid(gid);
        header.set_mtime(0);
        header.set_cksum();

        let mut archive = tar::Builder::new(Vec::new());
        archive
            .append_data(&mut header, file_name, bytes)
            .map_err(|e| ContainerError::NotAvailable(format!("tar build failed: {e}")))?;
        let tar_bytes = archive
            .into_inner()
            .map_err(|e| ContainerError::NotAvailable(format!("tar finish failed: {e}")))?;

        self.inner
            .docker
            .upload_to_container(
                container_name,
                Some(bollard::container::UploadToContainerOptions {
                    path: "/tmp",
                    no_overwrite_dir_non_dir: "true",
                }),
                tar_bytes.into(),
            )
            .await?;

        Ok(format!("/tmp/{file_name}"))
    }
}
