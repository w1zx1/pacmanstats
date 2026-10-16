use crate::logparse::{Event, Kind};
use chrono::NaiveDate;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Day {
    pub date: NaiveDate,
    pub events: Vec<Event>,
    pub n_installed: usize,
    pub n_upgraded: usize, // includes downgraded
    pub n_removed: usize,
    pub n_reinstalled: usize,
    /// Net change for the day (churn cancelled). Computed lazily via `diff()`.
    pub diff: Vec<DiffEntry>,
}

#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub pkg: String,
    /// None = package absent, Some(v) = present with version.
    pub before: Option<String>,
    pub after: Option<String>,
}

impl DiffEntry {
    pub fn kind(&self) -> DiffKind {
        match (&self.before, &self.after) {
            (None, Some(_)) => DiffKind::Added,
            (Some(_), None) => DiffKind::Removed,
            (Some(_), Some(_)) => DiffKind::Changed,
            (None, None) => DiffKind::Changed, // unreachable
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Added,
    Removed,
    Changed,
}

/// Net per-package change within one day.
///
/// For each package, take state before the first event of the day and
/// state after the last event of the day:
/// - installed v        => before=None,        after=Some(v)
/// - upgraded o -> n    => before=Some(o),     after=Some(n)
/// - downgraded o -> n  => before=Some(o),     after=Some(n)
/// - reinstalled v      => before=Some(v),     after=Some(v)
/// - removed v          => before=Some(v),     after=None
///
/// If before == after the package nets to zero change and is skipped.
/// Example: installed A, installed B, removed A  =>  diff shows only +B.
pub fn day_diff(events: &[Event]) -> Vec<DiffEntry> {
    let mut by_pkg: BTreeMap<&str, Vec<&Event>> = BTreeMap::new();
    for ev in events {
        by_pkg.entry(ev.pkg.as_str()).or_default().push(ev);
    }
    let mut out = Vec::new();
    for (pkg, evs) in by_pkg {
        let before = evs.first().and_then(|e| e.before());
        let after = evs.last().and_then(|e| e.after());
        if before == after {
            continue;
        }
        out.push(DiffEntry {
            pkg: pkg.to_string(),
            before,
            after,
        });
    }
    out.sort_by(|a, b| a.pkg.cmp(&b.pkg));
    out
}

pub fn group_by_day(mut events: Vec<Event>) -> Vec<Day> {
    events.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.time.cmp(&b.time))
            .then(a.pkg.cmp(&b.pkg))
    });
    let mut map: BTreeMap<NaiveDate, Vec<Event>> = BTreeMap::new();
    for ev in events {
        map.entry(ev.date).or_default().push(ev);
    }
    map.into_iter()
        .map(|(date, events)| {
            let n_installed = events.iter().filter(|e| e.kind == Kind::Installed).count();
            let n_upgraded = events
                .iter()
                .filter(|e| e.kind == Kind::Upgraded || e.kind == Kind::Downgraded)
                .count();
            let n_removed = events.iter().filter(|e| e.kind == Kind::Removed).count();
            let n_reinstalled = events
                .iter()
                .filter(|e| e.kind == Kind::Reinstalled)
                .count();
            let diff = day_diff(&events);
            Day {
                date,
                events,
                n_installed,
                n_upgraded,
                n_removed,
                n_reinstalled,
                diff,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logparse::parse_log;

    #[test]
    fn diff_cancels_install_then_remove() {
        let text = "\
[2026-10-16T10:00:00+0300] [ALPM] installed foo (1.0-1)\n\
[2026-10-16T10:01:00+0300] [ALPM] installed bar (2.0-1)\n\
[2026-10-16T10:02:00+0300] [ALPM] removed foo (1.0-1)\n";
        let evs = parse_log(text);
        let d = day_diff(&evs);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].pkg, "bar");
        assert_eq!(d[0].before, None);
        assert_eq!(d[0].after.as_deref(), Some("2.0-1"));
    }

    #[test]
    fn diff_collapses_double_upgrade() {
        let text = "\
[2026-10-16T10:00:00+0300] [ALPM] upgraded foo (1.0-1 -> 1.1-1)\n\
[2026-10-16T11:00:00+0300] [ALPM] upgraded foo (1.1-1 -> 1.2-1)\n";
        let evs = parse_log(text);
        let d = day_diff(&evs);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].before.as_deref(), Some("1.0-1"));
        assert_eq!(d[0].after.as_deref(), Some("1.2-1"));
    }

    #[test]
    fn diff_skips_noop_reinstall() {
        let text = "[2026-10-16T10:00:00+0300] [ALPM] reinstalled foo (1.0-1)\n";
        let evs = parse_log(text);
        assert!(day_diff(&evs).is_empty());
    }

    #[test]
    fn diff_upgrade_then_remove_is_removed() {
        let text = "\
[2026-10-16T10:00:00+0300] [ALPM] upgraded foo (1.0-1 -> 1.1-1)\n\
[2026-10-16T11:00:00+0300] [ALPM] removed foo (1.1-1)\n";
        let evs = parse_log(text);
        let d = day_diff(&evs);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].before.as_deref(), Some("1.0-1"));
        assert_eq!(d[0].after, None);
    }

    #[test]
    fn diff_remove_then_install_same_version_is_noop() {
        let text = "\
[2026-10-16T10:00:00+0300] [ALPM] removed foo (1.0-1)\n\
[2026-10-16T11:00:00+0300] [ALPM] installed foo (1.0-1)\n";
        let evs = parse_log(text);
        assert!(day_diff(&evs).is_empty());
    }
}
