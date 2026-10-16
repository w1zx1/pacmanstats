use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Installed,
    Upgraded,
    Downgraded,
    Removed,
    Reinstalled,
}

impl Kind {
    pub fn short(self) -> char {
        match self {
            Kind::Installed => '+',
            Kind::Upgraded => '~',
            Kind::Downgraded => 'v',
            Kind::Removed => '-',
            Kind::Reinstalled => '=',
        }
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub date: NaiveDate,
    pub time: String,
    pub kind: Kind,
    pub pkg: String,
    /// For installed/removed/reinstalled: the single version.
    /// For upgraded/downgraded: old version.
    pub old_ver: Option<String>,
    /// For installed/removed/reinstalled: the single version (same as old_ver).
    /// For upgraded/downgraded: new version.
    pub new_ver: Option<String>,
}

impl Event {
    pub fn version_display(&self) -> String {
        match self.kind {
            Kind::Upgraded | Kind::Downgraded => format!(
                "{} -> {}",
                self.old_ver.as_deref().unwrap_or("?"),
                self.new_ver.as_deref().unwrap_or("?")
            ),
            _ => self
                .new_ver
                .clone()
                .or(self.old_ver.clone())
                .unwrap_or_default(),
        }
    }

    /// State before this single event: None = absent, Some(v) = present with version.
    pub fn before(&self) -> Option<String> {
        match self.kind {
            Kind::Installed => None,
            Kind::Upgraded | Kind::Downgraded | Kind::Removed | Kind::Reinstalled => {
                self.old_ver.clone()
            }
        }
    }

    /// State after this single event.
    pub fn after(&self) -> Option<String> {
        match self.kind {
            Kind::Removed => None,
            Kind::Installed
            | Kind::Upgraded
            | Kind::Downgraded
            | Kind::Reinstalled => self.new_ver.clone(),
        }
    }
}

/// Parse one line like:
/// `[2026-10-16T19:40:20+0300] [ALPM] upgraded foo (1.0-1 -> 2.0-1)`
fn parse_line(line: &str) -> Option<Event> {
    // Fast pre-filter.
    if !line.contains("[ALPM]") {
        return None;
    }
    // Timestamp: between first [ and ].
    let ts_end = line.find(']')?;
    let ts_raw = line[1..ts_end].trim();
    // Date = YYYY-MM-DD (first 10 chars), time = HH:MM:SS (chars 11..19).
    if ts_raw.len() < 19 {
        return None;
    }
    let date_str = &ts_raw[0..10];
    let time_str = ts_raw[11..19].to_string();
    let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;

    // Action: after "[ALPM] ".
    let alpm = line.find("[ALPM]")?;
    let rest = line[alpm + 6..].trim_start();
    let (action, tail) = rest.split_once(' ').unwrap_or((rest, ""));
    let kind = match action {
        "installed" => Kind::Installed,
        "upgraded" => Kind::Upgraded,
        "downgraded" => Kind::Downgraded,
        "removed" => Kind::Removed,
        "reinstalled" => Kind::Reinstalled,
        _ => return None,
    };
    let tail = tail.trim();
    // tail = "pkgname (ver)" or "pkgname (old -> new)"
    let paren = tail.find(" (")?;
    let pkg = tail[..paren].trim().to_string();
    if pkg.is_empty() {
        return None;
    }
    let ver_part = tail[paren + 2..].strip_suffix(')')?.trim().to_string();

    let (old_ver, new_ver) = match kind {
        Kind::Upgraded | Kind::Downgraded => {
            let (old, new) = ver_part.split_once(" -> ")?;
            (
                Some(old.trim().to_string()),
                Some(new.trim().to_string()),
            )
        }
        _ => (Some(ver_part.clone()), Some(ver_part)),
    };

    Some(Event {
        date,
        time: time_str,
        kind,
        pkg,
        old_ver,
        new_ver,
    })
}

pub fn parse_log(text: &str) -> Vec<Event> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(ev) = parse_line(line) {
            out.push(ev);
        }
    }
    out.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.time.cmp(&b.time))
            .then(a.pkg.cmp(&b.pkg))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
[2026-10-16T10:00:00+0300] [ALPM] installed foo (1.0-1)\n\
[2026-10-16T10:01:00+0300] [ALPM] installed bar (2.0-1)\n\
[2026-10-16T10:02:00+0300] [ALPM] removed foo (1.0-1)\n\
[2026-10-16T10:03:00+0300] [ALPM] upgraded baz (1.0-1 -> 1.1-1)\n\
[2026-10-16T10:04:00+0300] [PACMAN] Running 'pacman -S foo'\n\
[2026-10-17T10:00:00+0300] [ALPM] reinstalled qux (3.0-1)\n\
[2026-10-17T10:01:00+0300] [ALPM] downgraded quux (2.0-1 -> 1.0-1)\n";

    #[test]
    fn parses_kinds() {
        let evs = parse_log(SAMPLE);
        assert_eq!(evs.len(), 6);
        assert_eq!(evs[0].pkg, "foo");
        assert_eq!(evs[0].kind, Kind::Installed);
        assert_eq!(evs[2].kind, Kind::Removed);
        assert_eq!(evs[3].kind, Kind::Upgraded);
        assert_eq!(evs[3].old_ver.as_deref(), Some("1.0-1"));
        assert_eq!(evs[3].new_ver.as_deref(), Some("1.1-1"));
        assert_eq!(evs[5].kind, Kind::Downgraded);
    }
}
