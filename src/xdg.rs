use std::path::{Path, PathBuf};

pub fn data_home() -> PathBuf {
    platform_data_home()
}

pub fn state_home() -> PathBuf {
    platform_state_home()
}

#[cfg(not(target_os = "windows"))]
fn platform_data_home() -> PathBuf {
    resolve_xdg_base_dir("XDG_DATA_HOME", ".local/share")
}

#[cfg(not(target_os = "windows"))]
fn platform_state_home() -> PathBuf {
    resolve_xdg_base_dir("XDG_STATE_HOME", ".local/state")
}

#[cfg(target_os = "windows")]
fn platform_data_home() -> PathBuf {
    resolve_windows_base_dir("APPDATA", "AppData\\Roaming")
}

#[cfg(target_os = "windows")]
fn platform_state_home() -> PathBuf {
    resolve_windows_base_dir("LOCALAPPDATA", "AppData\\Local")
}

#[cfg(not(target_os = "windows"))]
fn resolve_xdg_base_dir(var_name: &str, fallback_suffix: &str) -> PathBuf {
    if let Some(path) = env_absolute_dir(var_name) {
        return path;
    }

    home_dir().join(fallback_suffix)
}

#[cfg(target_os = "windows")]
fn resolve_windows_base_dir(var_name: &str, fallback_suffix: &str) -> PathBuf {
    if let Some(path) = env_absolute_dir(var_name) {
        return path;
    }

    user_profile_dir().join(fallback_suffix)
}

fn env_absolute_dir(var_name: &str) -> Option<PathBuf> {
    let value = std::env::var_os(var_name)?;
    if value.is_empty() {
        return None;
    }

    let path = PathBuf::from(value);
    if path.is_absolute() { Some(path) } else { None }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(".").to_path_buf())
}

#[cfg(target_os = "windows")]
fn user_profile_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .filter(|profile| !profile.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(".").to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{data_home, state_home};

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn defaults_match_xdg_suffixes() {
        unsafe {
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_STATE_HOME");
            std::env::set_var("HOME", "/tmp/fz1-home");
        }

        assert_eq!(
            data_home(),
            std::path::PathBuf::from("/tmp/fz1-home/.local/share")
        );
        assert_eq!(
            state_home(),
            std::path::PathBuf::from("/tmp/fz1-home/.local/state")
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn empty_and_relative_xdg_values_are_ignored() {
        unsafe {
            std::env::set_var("HOME", "/tmp/fz1-home");
            std::env::set_var("XDG_DATA_HOME", "");
        }
        assert_eq!(
            data_home(),
            std::path::PathBuf::from("/tmp/fz1-home/.local/share")
        );

        unsafe {
            std::env::set_var("XDG_STATE_HOME", "relative/state");
        }
        assert_eq!(
            state_home(),
            std::path::PathBuf::from("/tmp/fz1-home/.local/state")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn defaults_match_windows_profile_folders() {
        unsafe {
            std::env::remove_var("APPDATA");
            std::env::remove_var("LOCALAPPDATA");
            std::env::set_var("USERPROFILE", r"C:\Users\pavel");
        }

        assert_eq!(
            data_home(),
            std::path::PathBuf::from(r"C:\Users\pavel\AppData\Roaming")
        );
        assert_eq!(
            state_home(),
            std::path::PathBuf::from(r"C:\Users\pavel\AppData\Local")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn empty_and_relative_windows_values_are_ignored() {
        unsafe {
            std::env::set_var("USERPROFILE", r"C:\Users\pavel");
            std::env::set_var("APPDATA", "");
            std::env::set_var("LOCALAPPDATA", r"relative\local");
        }

        assert_eq!(
            data_home(),
            std::path::PathBuf::from(r"C:\Users\pavel\AppData\Roaming")
        );
        assert_eq!(
            state_home(),
            std::path::PathBuf::from(r"C:\Users\pavel\AppData\Local")
        );
    }
}
