//! WSL as seen from Windows: the installed distributions and their virtual disks.
//!
//! The registry view is the source of truth: each distribution is a subkey of
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss` whose `BasePath` holds its `ext4.vhdx`.
//! `rayx wsl status` and `rayx wsl compact` build on this module; `doctor` reports the sizes.

use std::path::PathBuf;

/// A WSL distribution registered for the current user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distribution {
    /// The registry key name, a GUID in braces.
    pub id: String,
    /// `DistributionName`, for example `Ubuntu-24.04`.
    pub name: String,
    /// `BasePath`: the directory that holds the distribution's virtual disk.
    pub base_path: PathBuf,
    /// WSL version, 1 or 2.
    pub version: Option<u32>,
    /// `DefaultUid`: the user the distribution starts as (1000 is the first created user).
    pub default_uid: Option<u32>,
}

impl Distribution {
    /// The virtual disk file of a WSL 2 distribution.
    pub fn vhdx(&self) -> PathBuf {
        let base = self.base_path.to_string_lossy();
        // The registry stores `\\?\C:\...`; the prefix is valid for file APIs, but plain paths
        // read better and compare equal to what the user types.
        let plain = base.strip_prefix(r"\\?\").unwrap_or(&base);
        PathBuf::from(plain).join("ext4.vhdx")
    }
}

/// The distributions registered in the current user's registry; empty off Windows.
pub fn registry_distributions() -> Vec<Distribution> {
    #[cfg(windows)]
    {
        registry::distributions()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
mod registry {
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, FILETIME};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, REG_DWORD, REG_SZ,
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW,
    };

    use super::Distribution;

    const LXSS: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// An open registry key, closed on drop.
    struct Key(HKEY);

    impl Key {
        fn open(parent: HKEY, path: &str, access: u32) -> Option<Key> {
            let mut key: HKEY = std::ptr::null_mut();
            let path = wide(path);
            // SAFETY: `path` is NUL-terminated and `key` is a live out-pointer.
            let status = unsafe { RegOpenKeyExW(parent, path.as_ptr(), 0, access, &mut key) };
            (status == ERROR_SUCCESS).then_some(Key(key))
        }

        fn string(&self, name: &str) -> Option<String> {
            let (kind, data) = self.value(name)?;
            if kind != REG_SZ {
                return None;
            }
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .take_while(|&unit| unit != 0)
                .collect();
            Some(String::from_utf16_lossy(&units))
        }

        fn dword(&self, name: &str) -> Option<u32> {
            let (kind, data) = self.value(name)?;
            (kind == REG_DWORD && data.len() >= 4)
                .then(|| u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        }

        fn value(&self, name: &str) -> Option<(u32, Vec<u8>)> {
            let name = wide(name);
            let mut kind = 0u32;
            let mut size = 0u32;
            // SAFETY: a null data pointer asks only for the type and the size in bytes.
            let status = unsafe {
                RegQueryValueExW(
                    self.0,
                    name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    std::ptr::null_mut(),
                    &mut size,
                )
            };
            if status != ERROR_SUCCESS {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            // SAFETY: `data` holds `size` writable bytes.
            let status = unsafe {
                RegQueryValueExW(
                    self.0,
                    name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut size,
                )
            };
            (status == ERROR_SUCCESS).then(|| {
                data.truncate(size as usize);
                (kind, data)
            })
        }
    }

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from `RegOpenKeyExW` and is closed exactly once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    pub fn distributions() -> Vec<Distribution> {
        let Some(lxss) = Key::open(
            HKEY_CURRENT_USER,
            LXSS,
            KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS,
        ) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut length = name.len() as u32;
            let mut last_write = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            // SAFETY: `name` holds `length` UTF-16 units and the other out-pointers are null or live.
            let status = unsafe {
                RegEnumKeyExW(
                    lxss.0,
                    index,
                    name.as_mut_ptr(),
                    &mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut last_write,
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            let id = String::from_utf16_lossy(&name[..length as usize]);
            let Some(key) = Key::open(lxss.0, &id, KEY_QUERY_VALUE) else {
                continue;
            };
            let (Some(distro), Some(base)) =
                (key.string("DistributionName"), key.string("BasePath"))
            else {
                continue;
            };
            found.push(Distribution {
                id,
                name: distro,
                base_path: PathBuf::from(base),
                version: key.dword("Version"),
                default_uid: key.dword("DefaultUid"),
            });
        }
        found
    }
}
