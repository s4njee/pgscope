//! Passwords, in the OS keychain and nowhere else.

use crate::error::{AppError, Result};

const SERVICE: &str = "pgscope";

/// Handle to one profile's entry in the OS keychain (Keychain on macOS,
/// Credential Manager on Windows, the Secret Service on Linux).
///
/// Keyed by profile id rather than name, so renaming a connection does not
/// orphan its password. Constructing this does not touch the store or prove the
/// entry exists — only the calls below do.
///
/// # Arguments
/// * `profile_id` — `&str`: the profile's id, used as the account name under the
///   fixed `pgscope` service.
///
/// # Returns
/// `Result<keyring::Entry>` — the handle to that entry; `Err(AppError::Keychain)`
/// if the platform credential store cannot be addressed at all.
fn entry(profile_id: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, profile_id).map_err(|e| AppError::Keychain(e.to_string()))
}

/// Store a password in the OS keychain, replacing any previous one.
///
/// May block and, on some platforms, prompt the user to unlock the keychain —
/// so this belongs on a command thread, not in a hot path.
///
/// # Arguments
/// * `profile_id` — `&str`: the profile whose entry is written.
/// * `password` — `&str`: the plaintext password to store.
///
/// # Returns
/// `Result<()>` — `Ok` once the password is committed to the keychain;
/// `Err(AppError::Keychain)` if the store rejected the write or stayed locked.
pub fn set_password(profile_id: &str, password: &str) -> Result<()> {
    entry(profile_id)?
        .set_password(password)
        .map_err(|e| AppError::Keychain(e.to_string()))
}

/// Returns None when no password is stored — a valid state for trust/peer auth.
///
/// # Arguments
/// * `profile_id` — `&str`: the profile whose entry is read.
///
/// # Returns
/// `Result<Option<String>>` — the stored password, or `Ok(None)` when no entry
/// exists; `Err(AppError::Keychain)` on a genuine keychain failure.
pub fn get_password(profile_id: &str) -> Result<Option<String>> {
    match entry(profile_id)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Keychain(e.to_string())),
    }
}

/// Remove a profile's stored password, called when the profile is deleted.
///
/// Absence is success, not failure — see the arm below — so this is safe to call
/// for a profile that used trust auth and never stored one. Only a genuine
/// keychain failure errors.
///
/// # Arguments
/// * `profile_id` — `&str`: the profile whose entry is removed.
///
/// # Returns
/// `Result<()>` — `Ok` if the entry was deleted or was already absent;
/// `Err(AppError::Keychain)` only on a real keychain failure.
pub fn delete_password(profile_id: &str) -> Result<()> {
    match entry(profile_id)?.delete_credential() {
        Ok(()) => Ok(()),
        // Deleting a profile that never had a password is not an error.
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Keychain(e.to_string())),
    }
}
