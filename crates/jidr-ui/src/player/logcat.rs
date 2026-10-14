//! Logcat lines (threadtime format) for the device window's logcat panel.

use crate::LogLine;

/// Keep this many lines in memory per device.
pub const KEEP: usize = 5000;

/// `10-13 14:02:07.318  4821  4821 I ActivityManager: Start proc …`
pub fn parse(line: &str) -> Option<LogLine> {
    let mut it = line.split_whitespace();
    let date = it.next()?;
    let time = it.next()?;
    let pid = it.next()?;
    let _tid = it.next()?;
    let level = it.next()?;
    if date.len() != 5
        || !time.contains(':')
        || level.len() != 1
        || !pid.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    // The rest is "Tag: message"; tags may contain spaces before the colon.
    let after_level = line.find(&format!(" {level} "))? + 3;
    let rest = line.get(after_level..)?.trim_start();
    let (tag, msg) = rest.split_once(':').unwrap_or((rest, ""));
    Some(LogLine {
        ts: format!("{date} {time}").into(),
        pid: pid.into(),
        level: level.into(),
        tag: tag.trim().into(),
        msg: msg.trim_start().into(),
    })
}

pub fn level_rank(level: &str) -> usize {
    match level {
        "V" => 0,
        "D" => 1,
        "I" => 2,
        "W" => 3,
        "E" => 4,
        "F" | "A" => 5,
        _ => 0,
    }
}

/// Lines at or above `min_level` (0 = V … 5 = F) that contain `filter` in
/// the tag, pid or message (case-insensitive).
pub fn visible(lines: &[LogLine], min_level: usize, filter: &str) -> Vec<LogLine> {
    let f = filter.trim().to_lowercase();
    lines
        .iter()
        .filter(|l| level_rank(&l.level) >= min_level)
        .filter(|l| {
            f.is_empty()
                || l.tag.to_lowercase().contains(&f)
                || l.pid.contains(&f)
                || l.msg.to_lowercase().contains(&f)
        })
        .cloned()
        .collect()
}

/// "E AndroidRuntime  FATAL EXCEPTION: main" for the collapsed bar.
pub fn short(l: &LogLine) -> String {
    format!("{} {}  {}", l.level, l.tag, l.msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_threadtime_lines() {
        let l = parse("10-13 14:02:07.318  4821  4821 I ActivityManager: Start proc 4821:com.example.shop/u0a190").unwrap();
        assert_eq!(l.ts, "10-13 14:02:07.318");
        assert_eq!(l.pid, "4821");
        assert_eq!(l.level, "I");
        assert_eq!(l.tag, "ActivityManager");
        assert_eq!(l.msg, "Start proc 4821:com.example.shop/u0a190");
        assert!(parse("--------- beginning of main").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn filters_by_level_and_text() {
        let lines: Vec<_> = [
            "10-13 14:02:07.318  4821  4821 I ActivityManager: Start proc",
            "10-13 14:02:08.115  1288  1288 W Magisk: su: request from uid 10190",
            "10-13 14:02:08.640  4821  4821 E AndroidRuntime: FATAL EXCEPTION: main",
        ]
        .iter()
        .filter_map(|l| parse(l))
        .collect();
        assert_eq!(visible(&lines, 3, "").len(), 2);
        assert_eq!(visible(&lines, 0, "magisk").len(), 1);
        assert_eq!(visible(&lines, 0, "4821").len(), 2);
        assert_eq!(short(&lines[2]), "E AndroidRuntime  FATAL EXCEPTION: main");
    }
}
