use std::io;
use std::path::PathBuf;

const DEFAULT_PANE_SPLIT_PERCENT: u16 = 50;
const MIN_PANE_SPLIT_PERCENT: u16 = 10;
const MAX_PANE_SPLIT_PERCENT: u16 = 90;

pub fn load_pane_split_percent() -> u16 {
    let path = pane_split_state_path();
    let Ok(content) = std::fs::read_to_string(path) else {
        return DEFAULT_PANE_SPLIT_PERCENT;
    };
    content
        .trim()
        .parse::<u16>()
        .ok()
        .map(clamp_pane_split_percent)
        .unwrap_or(DEFAULT_PANE_SPLIT_PERCENT)
}

pub fn save_pane_split_percent(percent: u16) -> io::Result<()> {
    let path = pane_split_state_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, clamp_pane_split_percent(percent).to_string())
}

pub fn clamp_pane_split_percent(percent: u16) -> u16 {
    percent.clamp(MIN_PANE_SPLIT_PERCENT, MAX_PANE_SPLIT_PERCENT)
}

fn pane_split_state_path() -> PathBuf {
    crate::xdg::state_home()
        .join("fz1")
        .join("pane-split-percent")
}

#[cfg(test)]
mod tests {
    use super::clamp_pane_split_percent;

    #[test]
    fn pane_split_is_clamped() {
        assert_eq!(clamp_pane_split_percent(0), 10);
        assert_eq!(clamp_pane_split_percent(50), 50);
        assert_eq!(clamp_pane_split_percent(100), 90);
    }
}
