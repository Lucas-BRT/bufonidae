//! Aggregate only comparable quota reports; unknown accounts are never zeroes.
use crate::backend::HealthStatus;

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Storage {
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub reported: usize,
    pub accounts: usize,
}

impl Storage {
    pub fn collect<'a>(reports: impl IntoIterator<Item = Option<&'a HealthStatus>>) -> Self {
        let mut result = Self::default();
        for report in reports {
            result.accounts += 1;
            let Some(HealthStatus::Ok { total, used, free }) = report else {
                continue;
            };
            let total = total.or_else(|| used.zip(*free).and_then(|(u, f)| u.checked_add(f)));
            let Some(total) = total.filter(|&total| total > 0) else {
                continue;
            };
            let Some(used) = used.or_else(|| free.map(|free| total.saturating_sub(free))) else {
                continue;
            };
            result.total = result.total.saturating_add(total);
            result.used = result.used.saturating_add(used);
            result.free = result.free.saturating_add(total.saturating_sub(used));
            result.reported += 1;
        }
        result
    }

    pub fn percent(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f64 / self.total as f64 * 100.0).clamp(0.0, 100.0) as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregates_reports_without_treating_unknown_accounts_as_empty() {
        let a = HealthStatus::Ok {
            total: Some(100),
            used: Some(40),
            free: None,
        };
        let b = HealthStatus::Ok {
            total: Some(200),
            used: None,
            free: Some(150),
        };
        let unknown = HealthStatus::Ok {
            total: None,
            used: Some(7),
            free: None,
        };
        let failed = HealthStatus::Unreachable("offline".into());
        let usage = Storage::collect([Some(&a), Some(&b), Some(&unknown), Some(&failed), None]);
        assert_eq!(
            usage,
            Storage {
                total: 300,
                used: 90,
                free: 210,
                reported: 2,
                accounts: 5
            }
        );
        assert_eq!(usage.percent(), 30.0);
    }
    #[test]
    fn handles_over_quota_missing_totals_and_empty_reports() {
        let over = HealthStatus::Ok {
            total: Some(100),
            used: Some(120),
            free: None,
        };
        let usage = Storage::collect([Some(&over)]);
        assert_eq!(usage.used, 120);
        assert_eq!(usage.free, 0);
        assert_eq!(usage.percent(), 100.0);
        let derived = HealthStatus::Ok {
            total: None,
            used: Some(20),
            free: Some(80),
        };
        assert_eq!(Storage::collect([Some(&derived)]).total, 100);
        let overflow = HealthStatus::Ok {
            total: None,
            used: Some(u64::MAX),
            free: Some(1),
        };
        assert_eq!(Storage::collect([Some(&overflow)]).reported, 0);
        assert_eq!(Storage::collect([]).percent(), 0.0);
    }
}
