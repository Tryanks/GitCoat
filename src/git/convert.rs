//! Conversions between gitoxide values and the backend-independent model.

use gix::bstr::ByteSlice;

use super::{EntryKind, GitError, Oid, Timestamp, TreeEntry};

pub(super) fn oid_from_gix(id: &gix::oid) -> Oid {
    Oid::from_validated(id.to_hex().to_string())
}

pub(super) fn oid_to_gix(oid: &Oid) -> Result<gix::ObjectId, GitError> {
    gix::ObjectId::from_hex(oid.as_str().as_bytes())
        .map_err(|error| GitError::invalid(format!("bad object id {oid}: {error}")))
}

pub(super) fn entry_kind(mode: gix::objs::tree::EntryMode) -> EntryKind {
    use gix::objs::tree::EntryKind as K;
    match mode.kind() {
        K::Tree => EntryKind::Dir,
        K::Blob => EntryKind::File,
        K::BlobExecutable => EntryKind::Executable,
        K::Link => EntryKind::Symlink,
        K::Commit => EntryKind::Submodule,
    }
}

/// The six-digit octal mode Git prints for `mode`.
pub(super) fn mode_string(mode: gix::objs::tree::EntryMode) -> String {
    format!("{:06o}", mode.value())
}

pub(super) fn tree_entry(
    name: &[u8],
    mode: gix::objs::tree::EntryMode,
    id: &gix::oid,
    size: Option<u64>,
) -> TreeEntry {
    TreeEntry {
        name: name.to_vec(),
        name_display: name.to_str_lossy().into_owned(),
        kind: entry_kind(mode),
        mode: mode_string(mode),
        oid: oid_from_gix(id),
        size,
    }
}

pub(super) fn timestamp(signature: &gix::actor::SignatureRef<'_>) -> Timestamp {
    match signature.time() {
        Ok(time) => Timestamp {
            unix: time.seconds,
            tz_offset_minutes: time.offset / 60,
        },
        Err(_) => Timestamp {
            unix: signature.seconds(),
            tz_offset_minutes: 0,
        },
    }
}

/// Whether `data` looks binary: a NUL byte within the first 8 KiB.
pub(super) fn looks_binary(data: &[u8]) -> bool {
    let probe = &data[..data.len().min(8 * 1024)];
    memchr::memchr(0, probe).is_some()
}

/// Map a "missing object" style error from an internal lookup (an id we got
/// from the repository itself) to a repository error.
pub(super) fn repo_error(context: &str, error: impl std::fmt::Display) -> GitError {
    GitError::repo(format!("{context}: {error}"))
}
