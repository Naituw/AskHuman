//! Attachments (plan §3.7): files the Agent attached to a question, which the phone downloads in
//! chunks, and files or images the phone attaches to its answers, which it uploads in chunks.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::Actor;
use crate::ios::crypto;
use crate::ios::wire::{
    AssetChunk, AssetError, AssetGet, UploadAck, UploadBegin, UploadCancel, UploadChunk,
    ASSET_CHUNK_BYTES, MAX_ASSET_BYTES, MAX_ASSET_CHUNKS_PER_GET, MAX_REPLY_ATTACHMENTS,
};

/// An upload nobody referenced is dropped after this long.
const UPLOAD_IDLE_MS: i64 = 30 * 60 * 1000;
/// Staged upload files are removed from disk after this long, whatever happened to them.
const UPLOAD_FILE_MAX_AGE_MS: i64 = 24 * 3_600_000;
const MAX_STAGED_UPLOADS_PER_PHONE: usize = 12;
const MAX_STAGED_BYTES_PER_PHONE: u64 = 64 * 1024 * 1024;
/// The phone gets an acknowledgement after this many chunks (and after the last one).
const ACK_EVERY_CHUNKS: u64 = 4;

pub(crate) struct Upload {
    phone_id: String,
    name: String,
    mime: String,
    size: u64,
    chunks: u64,
    sha256: String,
    request_id: Option<String>,
    next: u64,
    received: u64,
    hasher: Sha256,
    path: PathBuf,
    file: Option<std::fs::File>,
    complete: bool,
    touched_ms: i64,
}

/// A finished upload, ready to be attached to an answer.
#[derive(Debug, Clone)]
pub(crate) struct UploadedFile {
    pub name: String,
    pub mime: String,
    pub path: PathBuf,
}

#[derive(Default)]
pub(crate) struct Uploads {
    by_id: HashMap<String, Upload>,
}

impl Uploads {
    fn staged_for(&self, phone_id: &str) -> (usize, u64) {
        self.by_id
            .values()
            .filter(|u| u.phone_id == phone_id)
            .fold((0, 0), |(n, bytes), u| (n + 1, bytes + u.size))
    }
}

/// File names come from the network: keep one safe path component.
fn safe_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .take(120)
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

fn chunk_count(size: u64) -> u64 {
    size.div_ceil(ASSET_CHUNK_BYTES as u64).max(1)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Actor {
    // ------------------------------------------------------------ downloads

    pub(super) fn on_asset_get(&mut self, phone_id: &str, get: AssetGet) {
        let fail = |this: &mut Self, reason: &str| {
            this.send(
                phone_id,
                &AssetError {
                    asset_id: get.asset_id.clone(),
                    reason: reason.to_string(),
                },
            );
        };
        let Some(source) = self.requests.find_asset(&get.asset_id) else {
            fail(self, "notFound");
            return;
        };
        if source.size > MAX_ASSET_BYTES {
            fail(self, "tooLarge");
            return;
        }
        // Attachments are small (<= 10 MB) and read from the local disk: a plain read keeps the
        // actor's state free of in-flight transfers.
        let data = match std::fs::read(&source.path) {
            Ok(data) if data.len() as u64 <= MAX_ASSET_BYTES => data,
            Ok(_) => {
                fail(self, "tooLarge");
                return;
            }
            Err(_) => {
                fail(self, "notFound");
                return;
            }
        };
        let total = chunk_count(data.len() as u64);
        if get.from >= total {
            fail(self, "internal");
            return;
        }
        let count = get.count.clamp(1, MAX_ASSET_CHUNKS_PER_GET as u64);
        let end = (get.from + count).min(total);
        let sha256 = (get.from == 0).then(|| hex(&Sha256::digest(&data)));
        for index in get.from..end {
            let start = index as usize * ASSET_CHUNK_BYTES;
            let stop = (start + ASSET_CHUNK_BYTES).min(data.len());
            self.send(
                phone_id,
                &AssetChunk {
                    asset_id: get.asset_id.clone(),
                    index,
                    total,
                    size: data.len() as u64,
                    sha256: if index == 0 { sha256.clone() } else { None },
                    data: crypto::b64u(&data[start..stop]),
                },
            );
        }
    }

    // ------------------------------------------------------------ uploads

    fn upload_ack(
        &mut self,
        phone_id: &str,
        id: &str,
        next: u64,
        status: &str,
        reason: Option<&str>,
    ) {
        self.send(
            phone_id,
            &UploadAck {
                upload_id: id.to_string(),
                next_index: next,
                status: status.to_string(),
                reason: reason.map(str::to_string),
            },
        );
    }

    pub(super) fn on_upload_begin(&mut self, phone_id: &str, begin: UploadBegin) {
        let id = begin.upload_id.clone();
        let reject = |this: &mut Self, reason: &str| {
            this.upload_ack(phone_id, &id, 0, "error", Some(reason));
        };
        if !crypto::is_valid_id(&id) {
            return reject(self, "invalid");
        }
        if self.uploads.by_id.contains_key(&id) {
            // A repeated begin (lost ack): report where the transfer stands.
            let next = self.uploads.by_id[&id].next;
            let owned = self.uploads.by_id[&id].phone_id == phone_id;
            return if owned {
                self.upload_ack(phone_id, &id, next, "ok", None)
            } else {
                reject(self, "invalid")
            };
        }
        if begin.size == 0 || begin.size > MAX_ASSET_BYTES {
            return reject(self, "tooLarge");
        }
        if begin.chunks != chunk_count(begin.size) {
            return reject(self, "invalid");
        }
        let hash_ok =
            begin.sha256.len() == 64 && begin.sha256.bytes().all(|b| b.is_ascii_hexdigit());
        if !hash_ok {
            return reject(self, "invalid");
        }
        // Reply attachments belong to a pending question.
        match begin.scope.request_id.as_deref() {
            Some(request_id) if self.requests.find(request_id).is_some() => {}
            Some(_) => return reject(self, "gone"),
            None => return reject(self, "invalid"),
        }
        let (count, bytes) = self.uploads.staged_for(phone_id);
        if count >= MAX_STAGED_UPLOADS_PER_PHONE || bytes + begin.size > MAX_STAGED_BYTES_PER_PHONE
        {
            return reject(self, "busy");
        }
        let for_request = self
            .uploads
            .by_id
            .values()
            .filter(|u| u.phone_id == phone_id && u.request_id == begin.scope.request_id)
            .count();
        if for_request >= MAX_REPLY_ATTACHMENTS {
            return reject(self, "tooMany");
        }
        let dir = self.store.dir().join("uploads").join(&id);
        let path = dir.join(safe_file_name(&begin.name));
        let file = match create_private_file(&dir, &path) {
            Ok(file) => file,
            Err(_) => return reject(self, "internal"),
        };
        let now = self.now();
        self.uploads.by_id.insert(
            id.clone(),
            Upload {
                phone_id: phone_id.to_string(),
                name: safe_file_name(&begin.name),
                mime: begin.mime.chars().take(100).collect(),
                size: begin.size,
                chunks: begin.chunks,
                sha256: begin.sha256.to_ascii_lowercase(),
                request_id: begin.scope.request_id.clone(),
                next: 0,
                received: 0,
                hasher: Sha256::new(),
                path,
                file: Some(file),
                complete: false,
                touched_ms: now,
            },
        );
        self.upload_ack(phone_id, &id, 0, "ok", None);
    }

    pub(super) fn on_upload_chunk(&mut self, phone_id: &str, chunk: UploadChunk) {
        let id = chunk.upload_id.clone();
        let now = self.now();
        let Some(upload) = self.uploads.by_id.get_mut(&id) else {
            return self.upload_ack(phone_id, &id, 0, "error", Some("gone"));
        };
        if upload.phone_id != phone_id {
            return self.upload_ack(phone_id, &id, 0, "error", Some("gone"));
        }
        upload.touched_ms = now;
        if upload.complete {
            let next = upload.next;
            return self.upload_ack(phone_id, &id, next, "complete", None);
        }
        // Out of order (a lost chunk or a replay): tell the phone where to continue.
        if chunk.index != upload.next {
            let next = upload.next;
            return self.upload_ack(phone_id, &id, next, "ok", None);
        }
        let data = match crypto::b64u_decode(&chunk.data) {
            Some(data) if data.len() <= ASSET_CHUNK_BYTES => data,
            _ => return self.fail_upload(phone_id, &id, "invalid"),
        };
        let is_last = upload.next + 1 == upload.chunks;
        let expected = if is_last {
            upload.size - upload.received
        } else {
            ASSET_CHUNK_BYTES as u64
        };
        if data.len() as u64 != expected {
            return self.fail_upload(phone_id, &id, "invalid");
        }
        let written = upload
            .file
            .as_mut()
            .map(|file| file.write_all(&data))
            .unwrap_or_else(|| Err(std::io::Error::other("closed")));
        if written.is_err() {
            return self.fail_upload(phone_id, &id, "internal");
        }
        upload.hasher.update(&data);
        upload.received += data.len() as u64;
        upload.next += 1;
        let next = upload.next;
        if is_last {
            let digest = hex(&upload.hasher.clone().finalize());
            if digest != upload.sha256 {
                return self.fail_upload(phone_id, &id, "checksum");
            }
            if let Some(file) = upload.file.take() {
                let _ = file.sync_all();
            }
            upload.complete = true;
            self.upload_ack(phone_id, &id, next, "complete", None);
        } else if next % ACK_EVERY_CHUNKS == 0 {
            self.upload_ack(phone_id, &id, next, "ok", None);
        }
    }

    fn fail_upload(&mut self, phone_id: &str, id: &str, reason: &str) {
        self.discard_upload(id);
        self.upload_ack(phone_id, id, 0, "error", Some(reason));
    }

    pub(super) fn on_upload_cancel(&mut self, phone_id: &str, cancel: UploadCancel) {
        if self
            .uploads
            .by_id
            .get(&cancel.upload_id)
            .is_some_and(|u| u.phone_id == phone_id)
        {
            self.discard_upload(&cancel.upload_id);
        }
    }

    fn discard_upload(&mut self, id: &str) {
        if let Some(upload) = self.uploads.by_id.remove(id) {
            drop(upload.file);
            if let Some(dir) = upload.path.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    /// Takes finished uploads for an answer to `request_id`: every id must exist, be complete
    /// and belong to this phone and request. They leave the table either way (consumed).
    pub(super) fn take_uploads(
        &mut self,
        phone_id: &str,
        request_id: &str,
        ids: &[String],
    ) -> Result<Vec<UploadedFile>, String> {
        let mut files = Vec::with_capacity(ids.len());
        for id in ids {
            let usable = self.uploads.by_id.get(id).is_some_and(|u| {
                u.phone_id == phone_id
                    && u.complete
                    && u.request_id.as_deref().is_none_or(|r| r == request_id)
            });
            if !usable {
                return Err("attachment is not available".into());
            }
        }
        for id in ids {
            if let Some(upload) = self.uploads.by_id.remove(id) {
                files.push(UploadedFile {
                    name: upload.name,
                    mime: upload.mime,
                    path: upload.path,
                });
            }
        }
        Ok(files)
    }

    pub(super) fn drop_phone_uploads(&mut self, phone_id: &str) {
        let ids: Vec<String> = self
            .uploads
            .by_id
            .iter()
            .filter(|(_, u)| u.phone_id == phone_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.discard_upload(&id);
        }
    }

    /// Uploads that were never used, and ones whose request already ended.
    pub(super) fn prune_uploads(&mut self, now: i64) {
        let stale: Vec<String> = self
            .uploads
            .by_id
            .iter()
            .filter(|(_, u)| {
                now - u.touched_ms > UPLOAD_IDLE_MS
                    || u.request_id
                        .as_deref()
                        .is_some_and(|r| self.requests.find(r).is_none())
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            self.discard_upload(&id);
        }
    }

    /// Removes staged files older than a day (also from earlier daemon runs).
    pub(super) fn sweep_upload_files(&self) {
        let root = self.store.dir().join("uploads");
        let Ok(entries) = std::fs::read_dir(&root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let id = entry.file_name().to_string_lossy().to_string();
            if self.uploads.by_id.contains_key(&id) {
                continue;
            }
            let age_ms = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .map(|e| e.as_millis() as i64)
                .unwrap_or(0);
            if age_ms > UPLOAD_FILE_MAX_AGE_MS {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
}

fn create_private_file(dir: &Path, path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    }
}
